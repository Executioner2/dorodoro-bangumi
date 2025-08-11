//! 暴力探测算法。
//! 大体思路：
//! 1. 如果当前响应速率在 %15 以内，则 cwnd * 1.25。
//! 2. 如果当前响应速率低于 %15，则 cwnd * 0.75。
//! 3. 如果响应速率在 cwnd 增长了 1.25 之后的 3s 内，速率 <= 之前的速率，则 cwnd * 0.75。
//!
//! 使用方其实只需要三个参数：
//! - cwnd: 拥塞窗口大小。
//! - rate: 发送速率。
//! - inflight: 传输中的包。
//!
//! todo - 大体来说是可以用，但有一些小细节需要完善。例如：
//!      - 上下浮动相对来说有延迟，不平滑，会受到毛刺影响
//!      - 暂时无法进行速率估算，因为需要测量 rtt

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use doro_util::collection::FixedQueue;
use doro_util::win_minmax::Minmax;
use doro_util::{datetime, if_else};
use tracing::{debug, level_enabled, trace, Level};

use super::{PacketAck, PacketSend, RateControl};

#[derive(Clone, Default, Debug)]
pub struct Dashbord {
    /// 拥塞窗口大小
    cwnd: Arc<AtomicU32>,

    /// 传输中的包
    inflight: Arc<AtomicU32>,

    /// 累计确认的字节数
    acked_bytes: Arc<AtomicU64>,

    /// 近期平均带宽
    bw: Arc<Mutex<TimeFixeQueue>>,
    // 发送速率
    // rate: Arc<AtomicU32>,
}

impl Dashbord {
    pub fn new() -> Self {
        Self {
            cwnd: Arc::new(AtomicU32::new(MIN_CWND)),
            inflight: Arc::new(AtomicU32::new(0)),
            acked_bytes: Arc::new(AtomicU64::new(0)),
            bw: Arc::new(Mutex::new(TimeFixeQueue::new(BW_RATE_THRESH))),
        }
    }
}

impl Dashbord {
    pub fn update_bw(&self, bw: u32) {
        let item = (bw, datetime::now_millis() as u64);
        match self.bw.lock().as_mut() {
            Ok(x) => {
                x.push(item);
            }
            Err(p) => {
                p.get_mut().push(item);
            }
        }
    }
}

impl RateControl for Dashbord {
    fn cwnd(&self) -> u32 {
        self.cwnd.load(Ordering::Relaxed)
    }

    fn inflight(&self) -> u32 {
        self.inflight.load(Ordering::Relaxed)
    }

    fn acked_bytes(&self) -> u64 {
        self.acked_bytes.load(Ordering::Relaxed)
    }

    fn bw(&self) -> u64 {
        match self.bw.lock().as_mut() {
            Ok(x) => x.avg(),
            Err(p) => p.get_mut().avg(),
        }
    }

    fn clear_ing(&self) {
        self.inflight.store(0, Ordering::Relaxed);
        self.cwnd.store(MIN_CWND, Ordering::Relaxed);
    }
}

impl PacketSend for Dashbord {
    fn send(&self, _write_size: u32) {
        self.inflight.fetch_add(1, Ordering::Relaxed);
    }
}

/// 避免处理小数
const SCALE: u8 = 8;
const UNIT: u32 = 1 << SCALE;

/// cwnd 高增益
const CWND_HIGHT_GAIN: u32 = UNIT * 5 / 4;

/// cwnd 低增益
const CWND_LOW_GAIN: u32 = UNIT * 4 / 5;

/// 带宽速率浮动门限
const BW_UP_THRESH: u64 = UNIT as u64 * 5 / 4;

/// 高增益探索保持门限
const BW_UP_KEEP_THRESH: u64 = UNIT as u64 * 11 / 10;

/// 带宽速率浮动门限
const BW_DOWN_THRESH: u64 = UNIT as u64 * 4 / 5;

/// 1s 内的毫秒数
const MILLIS_PER_SEC: u64 = 1000;

/// 采样更新的时间间隔 1s
const UPDATE_INTERVAL: u64 = MILLIS_PER_SEC;

/// 最大带宽持续时间，三个样本更新周期
const MAX_BW_CWND: u32 = 3;

/// 高增益持续时长，三个样本更新周期
const HIGHT_GAIN_CYCLE: u32 = 3;

/// 低增益持续时长，三个样本更新周期
const LOW_GAIN_CYCLE: u32 = 3;

/// 普通增益持续时长，三个样本更新周期
const NORMAL_GAIN_CYCLE: u32 = 3;

/// 最小拥塞窗口大小
const MIN_CWND: u32 = 4;

/// 最大拥塞窗口大小
const MAX_CWND: u32 = 25;

/// 带宽样本队列长度
const BW_RATE_THRESH: usize = 10;

/// 连续 3 轮没有增长，则判定为带宽受限
const FULL_BW_CNT: u32 = 3;

/// Startup 阶段，快速增益值。
const STARTUP_GAIN: u32 = UNIT * 3 / 2;

/// 带宽受限阈值
const FULL_BW_THRESH: u32 = UNIT / 2;

/// 高增益探索保持率
const MAX_GAIN_KEEP_RATE: u32 = UNIT * 2 / 3;

/// full bw 保持周期
const FULL_BW_KEEP_CYCLE: u32 = 3;

#[derive(Default, Debug)]
pub struct TimeFixeQueue {
    queue: FixedQueue<(u32, u64)>,
    sum: u64,
}

impl TimeFixeQueue {
    pub fn new(size: usize) -> Self {
        Self {
            queue: FixedQueue::new(size),
            sum: 0,
        }
    }

    fn expires(&mut self) {
        let now = datetime::now_millis() as u64;
        let i = self.queue.limit();
        while !self.queue.is_empty() {
            if let Some(x) = self.queue.peek_front() {
                if x.1 + (UPDATE_INTERVAL * i as u64) < now {
                    self.sum -= x.0 as u64;
                    self.queue.pop_front();
                    continue;
                }
            }
            break;
        }
    }

    pub fn push(&mut self, item: (u32, u64)) {
        self.expires();
        self.sum += item.0 as u64;
        if let Some(x) = self.queue.push(item) {
            self.sum -= x.0 as u64;
        }
    }

    pub fn sum(&mut self) -> u64 {
        self.expires();
        self.sum
    }

    pub fn avg(&mut self) -> u64 {
        self.expires();
        self.sum / self.queue.limit() as u64
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn iter(&self) -> std::collections::vec_deque::Iter<(u32, u64)> {
        self.queue.iter()
    }
}

/// 暴力探测
pub struct Probe {
    /// 速率仪表盘
    dashbord: Dashbord,

    /// 传输带宽
    bw: Minmax,

    /// 上一次确认的时间，毫秒
    last_ack_stamp: u64,

    /// 上一次确认的字节数
    last_ack_bytes: u64,

    /// 样本更新计数
    sample_update_cnt: u32,

    /// 窗口增益值
    cwnd_gain: u32,

    /// 窗口高增益计数
    cwnd_high_gain_cnt: u32,

    /// 高增益试探
    cwnd_high_gain: bool,

    /// 窗口低增益计数
    cwnd_low_gain_cnt: u32,

    /// 普通增益计数
    cwnd_normal_gain_cnt: u32,

    /// 最大带宽
    full_bw: u32,

    /// 最大带宽连续无增长计数
    full_bw_cnt: u32,

    /// 最大带宽保持计数
    full_bw_keep_cnt: u32,

    /// 是否进入带宽受限
    full_bw_reached: bool,

    /// 最近一次高带宽
    last_high_bw: u64,

    /// 高增长期间，超过原来速率低次数
    high_gain_valid_cnt: u32,

    /// 是否使用 start up 增益
    starup_gain: bool,
}

impl Probe {
    pub fn new(dashbord: Dashbord) -> Self {
        Self {
            dashbord,
            bw: Minmax::new(),
            last_ack_stamp: 0,
            last_ack_bytes: 0,
            sample_update_cnt: 0,
            cwnd_gain: UNIT,
            cwnd_high_gain_cnt: 0,
            cwnd_high_gain: false,
            cwnd_low_gain_cnt: 0,
            cwnd_normal_gain_cnt: 0,
            full_bw: 0,
            full_bw_cnt: 0,
            full_bw_keep_cnt: 0,
            full_bw_reached: false,
            last_high_bw: 0,
            high_gain_valid_cnt: 0,
            starup_gain: false,
        }
    }

    fn inflight_dec(&self) {
        self.dashbord.inflight.fetch_sub(1, Ordering::Relaxed);
    }

    fn get_rate_sample(&mut self) -> RateSample {
        let now = datetime::now_millis() as u64;
        let interval_ms = now - self.last_ack_stamp;
        let acked_bytes = self.dashbord.acked_bytes();
        let interval_bytes = acked_bytes - self.last_ack_bytes;
        self.last_ack_stamp = now;
        self.last_ack_bytes = acked_bytes;
        self.sample_update_cnt += 1;

        RateSample {
            bw: (interval_bytes * MILLIS_PER_SEC).div_ceil(interval_ms),
        }
    }

    fn update_avg_bw(&mut self, rs: &RateSample) {
        self.dashbord.update_bw(rs.bw as u32);
    }

    fn update_bw(&mut self, rs: &RateSample) -> u32 {
        let origin_bw = self.bw.minmax_get();
        self.bw
            .minmax_running_max(MAX_BW_CWND, self.sample_update_cnt, rs.bw as u32);
        origin_bw
    }

    fn check_full_bw(&mut self) {
        if self.full_bw_reached {
            if self.dashbord.cwnd() <= MIN_CWND
                || self.bw.minmax_get() < (self.full_bw * FULL_BW_THRESH) >> SCALE
            {
                self.full_bw = 0;
                self.full_bw_reached = false;
            } else {
                return;
            }
        }

        // 等待增益把链路充满
        if self.full_bw_keep_cnt > 0 {
            self.full_bw_keep_cnt -= 1;
            self.full_bw = self.full_bw.max(self.bw.minmax_get());
            self.starup_gain = self.full_bw_keep_cnt == 0;
            return;
        }

        self.starup_gain = false;
        let bw_thresh = ((self.full_bw as u64 * BW_UP_THRESH) >> SCALE) as u32;

        if self.bw.minmax_get() > bw_thresh {
            self.full_bw_keep_cnt = FULL_BW_KEEP_CYCLE;
            self.full_bw_cnt = 0;
            self.full_bw = self.bw.minmax_get();
        } else {
            self.full_bw_cnt += 1;
            self.full_bw_reached = self.full_bw_cnt >= FULL_BW_CNT;
        }
    }

    fn update_gain(&mut self, bw: u64, origin_bw: u64) {
        if !self.full_bw_reached {
            self.cwnd_gain = if_else!(self.starup_gain, STARTUP_GAIN, UNIT);
            return;
        }

        if self.cwnd_high_gain_cnt > 0 {
            self.cwnd_high_gain_cnt -= 1;
            self.cwnd_gain = UNIT;

            if bw * UNIT as u64 >= self.last_high_bw * BW_UP_KEEP_THRESH {
                // 在 HIGHT_GAIN_CYCLE 个周期内，保持住了增长，说明这
                // 次探索的增益是有效的那么将探索的临时增益更新为长期增益
                self.high_gain_valid_cnt += 1;
                trace!(
                    "增益计数 +1！bw: {}\tb: {}",
                    self.high_gain_valid_cnt * UNIT,
                    HIGHT_GAIN_CYCLE * MAX_GAIN_KEEP_RATE
                );
            }

            if self.high_gain_valid_cnt * UNIT >= HIGHT_GAIN_CYCLE * MAX_GAIN_KEEP_RATE {
                self.cwnd_high_gain_cnt = 0;
            } else {
                self.cwnd_high_gain = self.cwnd_high_gain_cnt == 0;
            }
        }

        // 增益探索阶段也要进行低增益检查，如果探索阶段也是处于低增益
        // 状态，根据设置的周期，有可能提前退出增益探索阶段，转而进入
        // 低增益状态。
        if bw * UNIT as u64 <= origin_bw * BW_DOWN_THRESH {
            self.cwnd_low_gain_cnt += 1;
            self.cwnd_gain = UNIT;
        }

        if self.cwnd_normal_gain_cnt >= NORMAL_GAIN_CYCLE {
            trace!("进行高增益");
            self.cwnd_low_gain_cnt = 0;
            self.cwnd_normal_gain_cnt = 0;
            self.high_gain_valid_cnt = 0;
            self.cwnd_gain = CWND_HIGHT_GAIN;
            self.cwnd_high_gain_cnt = HIGHT_GAIN_CYCLE;
            self.last_high_bw = origin_bw;
        } else if self.cwnd_high_gain || self.cwnd_low_gain_cnt >= LOW_GAIN_CYCLE {
            trace!("进行低增益");
            self.cwnd_low_gain_cnt = 0;
            self.cwnd_normal_gain_cnt = 0;
            self.cwnd_high_gain = false;
            self.cwnd_gain = CWND_LOW_GAIN;
        } else if self.cwnd_high_gain_cnt == 0 {
            // 当连续 n 个样本没有增长，则尝试进行增长
            self.cwnd_normal_gain_cnt += 1;
            self.cwnd_gain = UNIT;
        }
    }

    fn update_cwnd(&mut self, gain: u32) {
        let mut cwnd = self.dashbord.cwnd.load(Ordering::Acquire);

        if gain == CWND_HIGHT_GAIN || gain == STARTUP_GAIN {
            cwnd += 2; // 防止小窗口增益过小
        } else if gain == CWND_LOW_GAIN {
            cwnd -= 1;
        }

        cwnd = ((cwnd as u64 * gain as u64) >> SCALE) as u32;
        self.dashbord
            .cwnd
            .store(cwnd.clamp(MIN_CWND, MAX_CWND), Ordering::Release);
    }

    pub fn dashbord(&self) -> Dashbord {
        self.dashbord.clone()
    }
}

impl PacketAck for Probe {
    fn ack(&mut self, read_size: u32) {
        self.inflight_dec();
        self.dashbord
            .acked_bytes
            .fetch_add(read_size as u64, Ordering::Relaxed);

        if self.last_ack_stamp + UPDATE_INTERVAL > datetime::now_millis() as u64 {
            return;
        }

        let rs = self.get_rate_sample();
        let origin_bw = self.update_bw(&rs);
        self.update_avg_bw(&rs);
        self.check_full_bw();
        self.update_gain(rs.bw, origin_bw as u64);
        self.update_cwnd(self.cwnd_gain);

        if level_enabled!(Level::DEBUG) {
            let (rate1, unit1) = doro_util::net::rate_formatting(origin_bw);
            let (rate2, unit2) = doro_util::net::rate_formatting(rs.bw);
            debug!(
                "\ncwnd: {}\tmax bw: {:.2}{}\tnew bw: {:.2}{}\tcwnd_gain: {}\t\
                down thresh: {}\tnew bw bytes: {}\t new_bw <= down_thresh: {}\tfbr: {}",
                self.dashbord.cwnd(),
                rate1,
                unit1,
                rate2,
                unit2,
                self.cwnd_gain,
                (origin_bw as u64 * BW_DOWN_THRESH) >> SCALE,
                rs.bw,
                rs.bw <= (origin_bw as u64 * BW_DOWN_THRESH) >> SCALE,
                self.full_bw_reached
            );
        }
    }
}

struct RateSample {
    bw: u64,
}
