//! bug 多多，不要使用

#![cfg_attr(
    debug_assertions,
    allow(dead_code, unused_imports, unused_variables, unused_mut)
)]

#[cfg(test)]
mod tests;

use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use doro_util::timer::CountdownTimer;
use doro_util::win_minmax::Minmax;
use doro_util::{datetime, if_else};
use libc::{socklen_t, tcp_connection_info};
use tracing::debug;

use crate::bt::base_peer::rate_control::bbr::State::*;

/// 带宽扩大
const BW_SCALE: u8 = 24;
const BW_UNIT: u64 = 1 << BW_SCALE;

/// 避免小数
const BBR_SCALE: u8 = 8;
const BBR_UNIT: u32 = 1 << BBR_SCALE;

/// 发送速率比理想值略低 1%，可以减少队列堆积概率
const PACING_MARGIN_PERCENT: u64 = 1;

/// 计算间隔
const CALCULATE_INTERVAL: Duration = Duration::from_secs(1);

/// ProbeRtt 持续时间
const PROBE_RTT_MODE_MS: u64 = 200;

/// 微秒到秒
const MICROS_PER_SEC: u64 = 1_000_000;

/// 窗口大小
const CWND_MIN_TARGET: u32 = 4;

/// Startup 阶段，判断 bw 增幅是否超过 1.25
const FULL_BW_THRESH: u32 = BBR_UNIT * 5 / 4;

/// 连续 3 轮没有增长，则判定为带宽受限
const FULL_BW_CNT: u32 = 3;

/// 周期长度
const CYCLE: u8 = 8;

/// ProbeBW 阶段，带宽有效周期
const BW_RTTS: u32 = 10;

/// 初始的 cwnd 窗口大小
const TCP_INIT_CWND: u32 = 10;

/// RTT 平滑因子
const RTT_ALPHA: u32 = BBR_UNIT * 32 / 256;

/// RTT 计算间隔时间
const RTT_INTERVAL_MS: u64 = 10 * 1000;

/// Startup 阶段，快速增益值。
const HIGH_GAIN: u32 = BBR_UNIT * 2885 / 1000;
/// Drain 阶段，负增益值
const DRAIN_GAIN: u32 = BBR_UNIT * 1000 / 2885;
/// 窗口增益值
const CWND_GAIN: u32 = BBR_UNIT * 2;
/// ProbeBW 阶段的 发送速率增益值
const PACING_GAIN: [u32; 10] = [
    BBR_UNIT * 5 / 4,
    BBR_UNIT * 3 / 4,
    BBR_UNIT,
    BBR_UNIT,
    BBR_UNIT,
    BBR_UNIT,
    BBR_UNIT,
    BBR_UNIT,
    BBR_UNIT,
    BBR_UNIT,
];

/* 长期带宽（long-term => LT BW）相关参数。即长期处于一个稳定状态的带宽 */
/// lt bw 采样间隔轮次
#[allow(dead_code)]
const LT_INTVL_MIN_RTTS: u32 = 4;
/// 如果丢包率到达 20 %，则进入 lt bw 阶段
#[allow(dead_code)]
const LT_LOSS_THRESH: u32 = 50;
/// 如果两个 bw 的区间小于等于 1/8，则取两个值的平均值
#[allow(dead_code)]
const LT_BW_RATIO: u32 = BBR_UNIT / 8;
/// 如果两个带宽的差值小于等于 4 kbit/sec，则取两个值的平均值
#[allow(dead_code)]
const LT_BW_DIFF: u32 = 4000 / 8;
/// 如果处于 lt 状态，则在经过 48 个 rtt 后再次检查 lt 状态
#[allow(dead_code)]
const LT_BW_MAX_RTTS: u32 = 48;

/// 重传判定倍率
const ARQ_RATE: u32 = 2;

#[derive(Clone, Default)]
pub struct Throttle {
    /// 累计读取到的字节数
    recv_size: Arc<AtomicU64>,

    /// 距离上一次 ack 后收到的数据包
    delivered: Arc<AtomicU32>,

    /// 累计确认的数据包
    acc_delivered: Arc<AtomicU64>,

    /// 上一次确认的时间戳
    delivered_time: Arc<AtomicU64>,

    /// 发送速率，pkts/s
    pacing_rate: Arc<AtomicU64>,

    /// 已发送，等待接收的包
    inflight: Arc<AtomicU32>,

    /// 发送窗口
    cwnd: Arc<AtomicU32>,

    /// 近期最小 rtt
    rtt: Arc<AtomicU32>,

    /// 是否应用受限。0 表示未受限，否则表示从 acc_delivered 到 app_limit 范围内的包都是应用受限的
    app_limited: Arc<AtomicU64>,
}

impl Throttle {
    fn ack(&self, size: u64, inflight: u32) {
        self.recv_size.fetch_add(size, Ordering::Relaxed);
        self.delivered.fetch_add(1, Ordering::Relaxed);
        self.acc_delivered.fetch_add(1, Ordering::Relaxed);
        self.delivered_time
            .store(datetime::now_micros() as u64, Ordering::Relaxed);
        self.inflight
            .store(inflight.min(self.inflight() - 1), Ordering::Relaxed);
        // self.inflight_dec();
    }

    // fn take_sample(&self) -> (u64, u32, u64) {
    //     let recv_size = self.recv_size.load(Ordering::Relaxed);
    //     let delivered = self.delivered.swap(0, Ordering::Relaxed);
    //     let mut app_limited = self.app_limited.load(Ordering::Relaxed);
    //     if app_limited != 0 && self.acc_delivered() >= app_limited {
    //         self.app_limited.store(0, Ordering::Relaxed);
    //         app_limited = 0;
    //     }
    //     (recv_size, delivered, app_limited)
    // }

    fn take_app_limited(&self) -> u64 {
        let mut app_limited = self.app_limited.load(Ordering::Relaxed);
        if app_limited != 0 && self.acc_delivered() >= app_limited {
            self.app_limited.store(0, Ordering::Relaxed);
            app_limited = 0;
        }
        app_limited
    }

    pub fn pacing_rate(&self) -> u64 {
        self.pacing_rate.load(Ordering::Relaxed)
    }

    pub fn cwnd(&self) -> u32 {
        self.cwnd.load(Ordering::Relaxed)
    }

    fn set_cwnd(&self, cwnd: u32) {
        self.cwnd.store(cwnd, Ordering::Relaxed);
    }

    fn set_pacing_rate(&self, rate: u64) {
        self.pacing_rate.store(rate, Ordering::Relaxed);
    }

    fn acc_delivered(&self) -> u64 {
        self.acc_delivered.load(Ordering::Relaxed)
    }

    fn delivered_time(&self) -> u64 {
        self.delivered_time.load(Ordering::Relaxed)
    }

    pub fn inflight(&self) -> u32 {
        self.inflight.load(Ordering::Relaxed)
    }

    fn inflight_inc(&self) -> u32 {
        self.inflight.fetch_add(1, Ordering::Relaxed)
    }

    /// 判断数据发送时是否处于应用受限状态，如果是，那么接下来直到 app_limited 返回内的 ack 样本都是应用受限的
    fn sent_app_limited(&self) {
        let inflight = self.inflight();
        let cwnd = self.cwnd();
        if inflight < cwnd {
            let n = self.acc_delivered() + (cwnd - inflight) as u64;
            self.app_limited
                .store(if_else!(n > 0, n, 1), Ordering::Relaxed);
        }
    }

    /// 获取下一次发送间隔
    fn get_next_send_time(&self, packet_size: u32) -> CountdownTimer {
        let us = packet_size as u64 * MICROS_PER_SEC / self.pacing_rate();
        CountdownTimer::new(Duration::from_micros(us))
    }

    /// 发送下一个数据包，会计算下一次发送间隔，增加 inflight 数据数量，并检测应用受限状态。
    pub fn send_next(&self, packet_size: u32) -> CountdownTimer {
        self.inflight_inc();
        self.sent_app_limited();
        self.get_next_send_time(packet_size)
    }
}

#[derive(Eq, PartialEq, Clone, Copy, Debug)]
enum State {
    /// 启动时
    Startup,

    /// 排空阶段
    Drain,

    /// 带宽受限
    ProbeBW,

    /// rtt 测量阶段
    ProbeRtt,
}

/// 速率样本
struct RateSample {
    /// 收到的数据包
    delivered: u32,

    /// 间隔时间，微秒
    interval_us: u64,

    /// 往返时间
    rtt: u32,

    /// 是否是重传包，rtt >= 2 * min_rtt 则判定为重传包
    #[allow(dead_code)]
    is_arq: bool,

    /// 是否应用受限
    is_app_limited: bool,
}

/// 速率控制器
pub struct BBRRateControl {
    /// 速率油门
    throttle: Throttle,

    /// 带宽状态
    state: State,

    /// 窗口增益
    cwnd_gain: u32,

    /// 发送速率增益
    pacing_gain: u32,

    /// 时间窗口内，最大带宽，单位: pkts/us
    bw: Minmax,

    /// 更新计数
    rtt_cnt: u32,

    /// 进过了一次 rtt 的时间周期
    round_start: bool,

    /// 下一次更新 rtt_cnt 的时间戳
    next_rtt_cnt_mstamp: u64,

    /// 最大带宽
    full_bw: u32,

    /// 最大带宽连续无增长计数
    full_bw_cnt: u32,

    /// 是否进入带宽受限
    full_bw_reached: bool,

    /// 更新间隔计时
    update_interval_timer: Instant,

    /// 更新间隔
    update_interval: Duration,

    /// 最小往返时间
    min_rtt_us: u32,

    /// 上一次更新最小往返时间的时间戳
    min_rtt_stamp: u64,

    /// probe rtt 状态预计完成时间
    probe_rtt_done_stamp: u64,

    /// ProbeBW 阶段循环周期
    cycle_idx: usize,

    /// 上一次更新周期的时间戳
    cycle_mstamp: u64,

    /// 保存着的上一个窗口大小，便于从 ProbeRtt 状态恢复
    prior_cwnd: u32,

    /// 一个请求大小
    mss: u32,

    /// lt bw
    lt_bw: u32,

    /// 是否使用 lt bw
    lt_use_bw: bool,

    /// 是否触发 lt bw 检测
    lt_is_sampling: bool,

    /// 最后一次 lt bw 检查的时间戳
    lt_last_stamp: u64,

    /// 最后一次检查的 lt bw 的交付数量
    lt_last_delivered: u64,

    /// lt bw 检测 rtt 计数
    lt_rtt_cnt: u32,

    /// 最后一次记录到重传包的数量
    lt_last_arq: u64,

    /// 重传计数
    arq: u64,

    /// 上一次采样时间戳
    prior_sampling_stamp: u64,

    /// 上一次采样时的交付数量
    prior_sampling_delivered: u64,
}

impl BBRRateControl {
    pub fn new(mss: u32) -> Self {
        let throttle = Throttle::default();
        throttle.set_cwnd(TCP_INIT_CWND);
        throttle.set_pacing_rate(((TCP_INIT_CWND * HIGH_GAIN) >> BBR_SCALE) as u64);
        throttle.rtt.store(u32::MAX, Ordering::Relaxed);

        Self {
            throttle,
            state: Startup,
            cwnd_gain: HIGH_GAIN,
            pacing_gain: HIGH_GAIN,
            bw: Minmax::new(),
            rtt_cnt: 0,
            round_start: false,
            next_rtt_cnt_mstamp: 0,
            full_bw: 0,
            full_bw_cnt: 0,
            full_bw_reached: false,
            update_interval_timer: Instant::now(),
            update_interval: CALCULATE_INTERVAL,
            min_rtt_us: u32::MAX,
            min_rtt_stamp: 0,
            probe_rtt_done_stamp: 0,
            cycle_idx: 0,
            cycle_mstamp: 0,
            prior_cwnd: 0,
            mss,
            lt_bw: 0,
            lt_use_bw: false,
            lt_is_sampling: false,
            lt_last_stamp: 0,
            lt_last_delivered: 0,
            lt_rtt_cnt: 0,
            lt_last_arq: 0,
            arq: 0,
            prior_sampling_stamp: datetime::now_micros() as u64,
            prior_sampling_delivered: 0,
        }
    }

    pub fn throttle(&self) -> Throttle {
        self.throttle.clone()
    }

    /// 获取速率样本
    fn get_rate_sample(&mut self, fd: RawFd) -> RateSample {
        let ni = get_net_info(fd).unwrap();

        // let interval_us = self.update_interval_timer.elapsed().as_micros() as u64;
        self.update_interval_timer = Instant::now();
        // let (recv_size_total, delivered, app_limited) = self.throttle.take_sample();
        // let rtt = self.throttle.take_rtt();
        // let recv_size = recv_size_total - self.prev_read_size;
        // self.prev_read_size = recv_size_total;

        let interval_us = ni.time_stamp - self.prior_sampling_stamp;
        self.prior_sampling_stamp = datetime::now_micros() as u64;
        let delivered = (ni.delivered - self.prior_sampling_delivered) as u32;
        self.prior_sampling_delivered = ni.delivered;
        let app_limited = self.throttle.take_app_limited();

        RateSample {
            delivered,
            interval_us,
            rtt: ni.rtt,
            is_arq: self.min_rtt_us != u32::MAX && ni.rtt >= ARQ_RATE * self.min_rtt_us,
            is_app_limited: app_limited > 0 || self.state == ProbeRtt,
        }
    }

    /// 平滑数据包的 rtt
    fn rtts(&self, rs: &RateSample) -> u32 {
        if rs.delivered == 0 {
            return u32::MAX;
        }

        if self.min_rtt_us == u32::MAX {
            return rs.rtt;
        }

        // 平滑 RTT（指数加权移动平均）
        (rs.rtt * RTT_ALPHA + (BBR_UNIT - RTT_ALPHA) * self.min_rtt_us) >> BBR_SCALE
    }

    /// 获取最大带宽
    fn max_bw(&self) -> u32 {
        self.bw.minmax_get()
    }

    /// 获取带宽，如果处于 lt 状态，则返回 lt_bw
    fn bw(&self) -> u32 {
        if_else!(self.lt_use_bw, self.lt_bw, self.max_bw())
    }

    /// 重置 lt bw 检测的间隔计数变量
    #[allow(dead_code)]
    fn reset_lt_bw_sampling_interval(&mut self) {
        self.lt_last_stamp = self.throttle.delivered_time();
        self.lt_last_delivered = self.throttle.acc_delivered();
        self.lt_last_arq = self.arq;
        self.lt_rtt_cnt = 0;
    }

    /// 重置 lt bw 检测的变量
    #[allow(dead_code)]
    fn reset_lt_bw_sampling(&mut self) {
        self.lt_bw = 0;
        self.lt_use_bw = false;
        self.lt_is_sampling = false;
        self.reset_lt_bw_sampling_interval();
    }

    /// lt bw 计算完成。需要两次检测出 lt bw，并且差值在一定范围内，才会进入 lt bw
    #[allow(dead_code)]
    fn lt_bw_interval_done(&mut self, bw: u32) {
        if self.lt_bw != 0 {
            let diff = self.lt_bw.abs_diff(bw);
            if diff * BBR_UNIT <= LT_BW_RATIO * self.lt_bw || diff < LT_BW_DIFF {
                self.lt_bw = (self.lt_bw + bw) >> 1;
                self.lt_use_bw = true;
                self.pacing_gain = BBR_UNIT;
                self.lt_rtt_cnt = 0;
                debug!("进入 lt bw 状态，bw: {}", self.lt_bw);
                return;
            }
        }

        self.lt_bw = bw;
        self.reset_lt_bw_sampling_interval()
    }

    /// lt bw 检查。如果重传率超过一定数值，则认定已经达到瓶颈，再往上之可能造成更多
    /// 丢包。ProbeBW 中的增益至为 1。保持峰值运行 48 个 rtt 周期。
    #[allow(dead_code)]
    fn lt_bw_sampling(&mut self, rs: &RateSample) {
        if self.lt_use_bw {
            if self.state == ProbeBW && self.round_start && {
                self.lt_rtt_cnt += 1;
                self.lt_rtt_cnt
            } >= LT_BW_MAX_RTTS
            {
                self.reset_lt_bw_sampling();
                self.reset_probe_bw_state();
                debug!("退出 lt bw 阶段");
            }
            return;
        }

        // 如果当前没启用 lt bw 检测，则判断该样本是否是重传样本。
        // 如果是重传样本，那么有可能触发上限，因此开启 lt bw 检测。
        if !self.lt_is_sampling {
            if !rs.is_arq {
                return;
            }
            self.reset_lt_bw_sampling_interval();
            self.lt_is_sampling = true;
        }

        // 如果是应用受限的样本，则重置 lt bw 检测
        if rs.is_app_limited || self.state == Drain {
            self.reset_lt_bw_sampling();
            return;
        }

        if self.round_start {
            self.lt_rtt_cnt += 1;
        }
        if self.lt_rtt_cnt < LT_INTVL_MIN_RTTS {
            return;
        } else if self.lt_rtt_cnt > 4 * LT_INTVL_MIN_RTTS {
            self.reset_lt_bw_sampling();
            return;
        }

        if !rs.is_arq {
            return;
        }

        let arq_cnt = self.arq - self.lt_last_arq;
        let delivered = self.throttle.acc_delivered() - self.lt_last_delivered;
        if delivered == 0 || arq_cnt << BBR_SCALE < LT_LOSS_THRESH as u64 * delivered {
            return; // 重传率不达标
        }

        let t = self.throttle.delivered_time() - self.lt_last_stamp;
        if t < 1000 {
            return; // 两个重传包之间需要间隔 1ms
        }

        let bw = (delivered * BW_UNIT / t) as u32;
        self.lt_bw_interval_done(bw);
    }

    /// 更新带宽
    fn update_bw(&mut self, rs: &RateSample) {
        self.round_start = false;
        if self.min_rtt_us != u32::MAX && self.throttle.delivered_time() >= self.next_rtt_cnt_mstamp
        {
            self.next_rtt_cnt_mstamp = self.throttle.delivered_time() + self.min_rtt_us as u64;
            self.rtt_cnt += 1;
            self.round_start = true;
        }

        // self.lt_bw_sampling(rs);

        let bw = (rs.delivered as u64 * BW_UNIT).div_ceil(rs.interval_us) as u32;

        if !rs.is_app_limited || bw >= self.bw.minmax_get() {
            self.bw.minmax_running_max(BW_RTTS, self.rtt_cnt, bw);
        }
    }

    /// 检查是否达到带宽瓶颈
    fn check_full_bw_reached(&mut self) {
        if self.full_bw_reached || !self.round_start {
            return;
        }

        let bw_thresh = (self.full_bw * FULL_BW_THRESH) >> BBR_SCALE;

        if self.max_bw() > bw_thresh {
            self.full_bw_cnt = 0;
            self.full_bw = self.max_bw();
        } else {
            self.full_bw_cnt += 1;
            self.full_bw_reached = self.full_bw_cnt >= FULL_BW_CNT;
        }
    }

    /// 重置为稳定阶段
    fn reset_probe_bw_state(&mut self) {
        debug!("进入 ProbeBw 状态");
        self.state = ProbeBW;
        self.cycle_idx = rand::random_range(0..CYCLE as usize); // 避免每次都从增长阶段开始，导致激流涌进
        self.advance_cycle_phase();
        self.update_interval = Duration::from_micros(self.min_rtt_us as u64);
        // self.update_interval = Duration::from_micros(self.min_rtt_us as u64);
        // self.update_interval = CALCULATE_INTERVAL;
    }

    /// 带宽延迟积
    fn bdp(&self, bw: u32, gain: u32) -> u32 {
        if self.min_rtt_us == u32::MAX {
            return TCP_INIT_CWND;
        }

        let bdp = bw as u64 * self.min_rtt_us as u64;
        ((bdp * gain as u64) >> BBR_SCALE as u64).div_ceil(BW_UNIT) as u32

        // let bdp = bw as u64 * self.min_rtt_us as u64 / MICROS_PER_SEC;
        // (bdp * gain as u64 >> BBR_SCALE) as u32
    }

    /// 理论上的最佳流动量
    fn theory_inflight_limit(&self, bw: u32, gain: u32) -> u32 {
        let mut inflight = self.bdp(bw, gain);
        if self.state == ProbeBW && self.cycle_idx == 0 {
            inflight += 2;
        }
        inflight.max(CWND_MIN_TARGET)
    }

    /// 检查 Drain 状态是否完成
    fn check_drain(&mut self) {
        if self.full_bw_reached && self.state == Startup {
            debug!("进入 Drain 状态");
            self.update_interval = Duration::from_micros(self.min_rtt_us as u64);
            self.state = Drain;
        }

        let bw = self.max_bw();
        if self.state == Drain
            && self.throttle.inflight() <= self.theory_inflight_limit(bw, BBR_UNIT)
        {
            self.reset_probe_bw_state();
        }
    }

    /// 更新周期相位
    fn advance_cycle_phase(&mut self) {
        self.cycle_idx = (self.cycle_idx + 1) & (CYCLE as usize - 1);
        self.cycle_mstamp = self.throttle.delivered_time();
    }

    /// 检查是否可以更新到下一个状态
    fn is_next_cycle_phase(&self) -> bool {
        // 距离上一次切换相位至少经过了一个 min rtt 周期
        let is_full_length =
            (self.throttle.delivered_time() - self.cycle_mstamp) as u32 > self.min_rtt_us;

        if self.pacing_gain == BBR_UNIT {
            return is_full_length;
        }

        let bw = self.max_bw();

        if self.pacing_gain > BBR_UNIT {
            return is_full_length
                && self.throttle.inflight() >= self.theory_inflight_limit(bw, self.pacing_gain);
        }

        is_full_length || self.throttle.inflight() <= self.theory_inflight_limit(bw, BBR_UNIT)
    }

    /// 更新 ProbeBW 阶段相位下标
    fn update_cycle_phase(&mut self) {
        if self.state == ProbeBW && self.is_next_cycle_phase() {
            self.advance_cycle_phase();
        }
    }

    /// 保存拥塞窗口
    fn save_cwnd(&mut self) {
        self.prior_cwnd = self.throttle.cwnd();
    }

    /// 进入 Startup 状态
    fn reset_startup_state(&mut self) {
        debug!("进入 Startup 状态");
        self.state = Startup;
        self.update_sampling_interval();
    }

    /// 重置状态。如果触及到带宽上限，进入 ProbeBW。否则进入 Startup
    fn reset_mode(&mut self) {
        if self.full_bw_reached {
            self.reset_probe_bw_state();
        } else {
            self.reset_startup_state();
        }
    }

    /// 检查 ProbeRtt 阶段是否完成
    fn check_probe_rtt_done(&mut self) {
        let now = datetime::now_millis() as u64;
        if self.probe_rtt_done_stamp == 0 || now < self.probe_rtt_done_stamp {
            return;
        }

        self.min_rtt_stamp = now;
        self.throttle
            .set_cwnd(self.prior_cwnd.max(self.throttle.cwnd()));
        debug!(
            "ProbeRtt 状态结束，当前 min rtt: {}\tcwnd: {}\tbw: {}\t发送间隔: {}",
            self.min_rtt_us,
            self.throttle.cwnd(),
            self.max_bw(),
            self.throttle.get_next_send_time(self.mss)
        );
        self.reset_mode();
    }

    /// 更新最小往返时间
    fn update_min_rtt(&mut self, rs: &RateSample) {
        // 检查是否需要更新最小往返时间
        let now = datetime::now_millis() as u64;
        let filter_expired = now >= self.min_rtt_stamp + RTT_INTERVAL_MS;
        let rtt = self.rtts(rs);

        if rtt != u32::MAX && (rtt < self.min_rtt_us || filter_expired) {
            self.min_rtt_us = rtt;
            self.min_rtt_stamp = now;
        }

        if filter_expired && self.state == ProbeBW {
            debug!("进入 ProbeRtt 状态");
            self.state = ProbeRtt;
            self.save_cwnd();
            self.probe_rtt_done_stamp = 0;

            // 缩短更新间隔，可提高检测频次，避免 ProbeRtt 阶段占用大量时间
            self.update_interval = Duration::from_micros(self.min_rtt_us as u64);
        }

        if self.state == ProbeRtt {
            if self.probe_rtt_done_stamp == 0 && self.throttle.inflight() <= CWND_MIN_TARGET {
                self.probe_rtt_done_stamp = now + PROBE_RTT_MODE_MS;
            } else if self.probe_rtt_done_stamp > 0 {
                self.check_probe_rtt_done();
            }
        }
    }

    /// 更新增益
    fn update_gains(&mut self) {
        match self.state {
            Startup => {
                self.cwnd_gain = HIGH_GAIN;
                self.pacing_gain = HIGH_GAIN;
            }
            Drain => {
                self.cwnd_gain = HIGH_GAIN;
                self.pacing_gain = DRAIN_GAIN;
            }
            ProbeBW => {
                self.cwnd_gain = CWND_GAIN;
                self.pacing_gain = if_else!(self.lt_use_bw, BBR_UNIT, PACING_GAIN[self.cycle_idx]);
            }
            ProbeRtt => {
                self.cwnd_gain = BBR_UNIT;
                self.pacing_gain = BBR_UNIT;
            }
        }
    }

    /// 更新采样间隔
    fn update_sampling_interval(&mut self) {
        if self.state == Startup && self.min_rtt_us != u32::MAX {
            self.update_interval = Duration::from_micros(self.min_rtt_us as u64);
        }
    }

    /// 更新数值以及状态
    fn update_model(&mut self, rs: &RateSample) {
        self.update_bw(rs);
        self.check_full_bw_reached();
        self.check_drain();
        self.update_cycle_phase();
        self.update_min_rtt(rs);
        self.update_gains();
        self.update_sampling_interval();
    }

    /// 带宽转发送速率
    fn bw_to_pacing_rate(&self, mut rate: u64, gain: u32) -> u64 {
        rate *= self.mss as u64;
        rate *= gain as u64;
        rate >>= BBR_SCALE as u64;
        rate *= MICROS_PER_SEC / 100 * (100 - PACING_MARGIN_PERCENT);
        rate >> BW_SCALE
    }

    /// 更新发送速率
    fn set_pacing_rate(&mut self, bw: u32, gain: u32) {
        let rate = self.bw_to_pacing_rate(bw as u64, gain);

        if self.full_bw_reached || rate > self.throttle.pacing_rate() {
            self.throttle.set_pacing_rate(rate);
        }
    }

    /// 更新发送窗口
    fn set_cwnd(&mut self, bw: u32, gain: u32, rs: &RateSample) {
        let mut cwnd = self.throttle.cwnd();
        let target_cwnd = self.theory_inflight_limit(bw, gain);

        if self.full_bw_reached {
            cwnd = target_cwnd.min(cwnd + rs.delivered);
        } else if self.state == Startup
            || cwnd < target_cwnd
            || self.prior_sampling_delivered < TCP_INIT_CWND as u64
        {
            cwnd += rs.delivered; // 不要升得太猛，避免波动造成的 min rtt 变高，bdp 异常的高
        }

        cwnd = cwnd.max(CWND_MIN_TARGET);
        if self.state == ProbeRtt {
            cwnd = cwnd.min(CWND_MIN_TARGET);
        }
        self.throttle.set_cwnd(cwnd);
    }

    /// 确认数据包
    pub fn ack(&mut self, packet_size: u64, fd: RawFd) {
        let inflight = get_inflight(fd, self.mss);
        self.throttle.ack(packet_size, inflight);

        if self.bw() == 0 || self.update_interval_timer.elapsed() >= self.update_interval {
            let rs = self.get_rate_sample(fd);
            self.update_model(&rs);

            let bw = self.max_bw();
            self.set_pacing_rate(bw, self.pacing_gain);
            self.set_cwnd(bw, self.cwnd_gain, &rs);

            let origin_state = self.state;
            if origin_state != ProbeRtt && origin_state != ProbeBW {
                // if origin_state != ProbeBW {
                // if true {
                debug!(
                    "\ninflight: {}\tcwnd: {}\t\
                    send rate: {}\t发送间隔: {}\t\
                    max_bw: {}\tmin rtt: {:?}\t\
                    inflight limit: {}\tstate: {:?}\t\
                    is_app_limited: {}\tinterval: {:?}\t\
                    delivered: {}\t
                    ",
                    self.throttle.inflight(),
                    self.throttle.cwnd(),
                    self.throttle.pacing_rate(),
                    self.throttle.get_next_send_time(self.mss),
                    self.max_bw(),
                    Duration::from_micros(self.min_rtt_us as u64),
                    self.theory_inflight_limit(bw, self.cwnd_gain),
                    self.state,
                    rs.is_app_limited,
                    Duration::from_micros(rs.interval_us),
                    rs.delivered,
                );
            }
        }
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
struct TcpConnectionInfo {
    tcpi_state: u8,      /* connection state */
    tcpi_snd_wscale: u8, /* Window scale for send window */
    tcpi_rcv_wscale: u8, /* Window scale for receive window */
    __pad1: u8,
    tcpi_options: u32, /* TCP options supported */
    // #define TCPCI_OPT_TIMESTAMPS    0x00000001 /* Timestamps enabled */
    // #define TCPCI_OPT_SACK          0x00000002 /* SACK enabled */
    // #define TCPCI_OPT_WSCALE        0x00000004 /* Window scaling enabled */
    // #define TCPCI_OPT_ECN           0x00000008 /* ECN enabled */
    tcpi_flags: u32, /* flags */
    // #define TCPCI_FLAG_LOSSRECOVERY 0x00000001
    // #define TCPCI_FLAG_REORDERING_DETECTED  0x00000002
    tcpi_rto: u32,          /* retransmit timeout in ms */
    tcpi_maxseg: u32,       /* maximum segment size supported */
    tcpi_snd_ssthresh: u32, /* slow start threshold in bytes */
    tcpi_snd_cwnd: u32,     /* send congestion window in bytes */
    tcpi_snd_wnd: u32,      /* send widnow in bytes */
    tcpi_snd_sbbytes: u32,  /* bytes in send socket buffer, including in-flight data */
    tcpi_rcv_wnd: u32,      /* receive window in bytes*/
    tcpi_rttcur: u32,       /* most recent RTT in ms */
    tcpi_srtt: u32,         /* average RTT in ms */
    tcpi_rttvar: u32,       /* RTT variance */
    tcpi_tfo: u32,
    // tcpi_tfo_cookie_req:1,             /* Cookie requested? */
    // tcpi_tfo_cookie_rcv:1,             /* Cookie received? */
    // tcpi_tfo_syn_loss:1,               /* Fallback to reg. TCP after SYN-loss */
    // tcpi_tfo_syn_data_sent:1,             /* SYN+data has been sent out */
    // tcpi_tfo_syn_data_acked:1,             /* SYN+data has been fully acknowledged */
    // tcpi_tfo_syn_data_rcv:1,             /* Server received SYN+data with a valid cookie */
    // tcpi_tfo_cookie_req_rcv:1,             /* Server received cookie-request */
    // tcpi_tfo_cookie_sent:1,             /* Server announced cookie */
    // tcpi_tfo_cookie_invalid:1,             /* Server received an invalid cookie */
    // tcpi_tfo_cookie_wrong:1,             /* Our sent cookie was wrong */
    // tcpi_tfo_no_cookie_rcv:1,             /* We did not receive a cookie upon our request */
    // tcpi_tfo_heuristics_disable:1,             /* TFO-heuristics disabled it */
    // tcpi_tfo_send_blackhole:1,             /* A sending-blackhole got detected */
    // tcpi_tfo_recv_blackhole:1,             /* A receiver-blackhole got detected */
    // tcpi_tfo_onebyte_proxy:1,             /* A proxy acknowledges all but one byte of the SYN */
    // __pad2:17,
    tcpi_txpackets: u64,
    tcpi_txbytes: u64,
    tcpi_txretransmitbytes: u64,
    tcpi_rxpackets: u64,
    tcpi_rxbytes: u64,
    tcpi_rxoutoforderbytes: u64,
    tcpi_txretransmitpackets: u64,
}

/// 网络信息
pub struct NetInfo {
    /// rtt
    rtt: u32,

    /// 等待确认的数据
    inflight: u32,

    /// 累计被确认的包
    delivered: u64,

    /// 采集时的时间戳，单位微秒
    time_stamp: u64,

    /// 丢包数（重传的也视为丢包）
    #[allow(dead_code)]
    loss: u64,
}

#[cfg(target_os = "macos")]
pub fn get_net_info(fd: RawFd) -> Option<NetInfo> {
    let mut info = std::mem::MaybeUninit::<TcpConnectionInfo>::uninit();
    let mut len = size_of::<tcp_connection_info>() as socklen_t;
    unsafe {
        let ret = libc::getsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_CONNECTION_INFO,
            info.as_mut_ptr().cast(),
            &mut len,
        );

        if ret == 0 {
            let info = info.assume_init();
            let ni = NetInfo {
                rtt: info.tcpi_srtt * 1000, // 原单位是 ms
                inflight: info.tcpi_snd_sbbytes,
                delivered: info.tcpi_rxpackets,
                time_stamp: datetime::now_micros() as u64,
                loss: info.tcpi_txretransmitpackets,
            };
            Some(ni)
        } else {
            None
        }
    }
}

/// 获取当前连接的 inflight 数据
#[cfg(target_os = "macos")]
pub fn get_inflight(fd: RawFd, mss: u32) -> u32 {
    let ni = get_net_info(fd).unwrap();
    ni.inflight / mss
}
