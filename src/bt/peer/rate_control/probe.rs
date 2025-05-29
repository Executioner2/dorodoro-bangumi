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

use crate::{datetime, if_else};
use crate::peer::rate_control::{PacketAck, PacketSend, RateControl};
use crate::win_minmax::Minmax;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tracing::debug;
use crate::collection::FixedQueue;

#[derive(Clone, Default)]
pub struct Dashbord {
    /// 拥塞窗口大小
    cwnd: Arc<AtomicU32>,

    /// 传输中的包
    inflight: Arc<AtomicU32>,
    
    /// 累计确认的字节数
    acked_bytes: Arc<AtomicU64>,
    
    /// 近期平均带宽
    bw: Arc<AtomicU64>,
    
    // 发送速率
    // rate: Arc<AtomicU32>,
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
        self.bw.load(Ordering::Relaxed)
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

/// 带宽速率浮动门限
const BW_DOWN_THRESH: u64 = UNIT as u64 * 4 / 5;

/// 1s 内的毫秒数
const MILLIS_PER_SEC: u64 = 1000;

/// 采样更新的时间间隔 1s
const UPDATE_INTERVAL: u64 = 1 * MILLIS_PER_SEC;

/// 最大带宽持续时间，三个样本更新周期
const MAX_BW_CWND: u32 = 3;

/// 高增益持续时长，三个样本更新周期
const HIGHT_GAIN_CYCLE: u32 = 3;

/// 低增益持续时长，五个样本更新周期
const LOW_GAIN_CYCLE: u32 = 5;

/// 普通增益持续时长，三个样本更新周期
const NORMAL_GAIN_CYCLE: u32 = 3;

/// 最小拥塞窗口大小
const MIN_CWND: u32 = 4;

/// 连续 3 轮没有增长，则判定为带宽受限
const FULL_BW_CNT: u32 = 3;

/// Startup 阶段，快速增益值。
const STARTUP_GAIN: u32 = UNIT * 2;

/// 带宽受限阈值
const FULL_BW_THRESH: u32 = UNIT * 1 / 2;

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

    /// 是否进入带宽受限
    full_bw_reached: bool,
    
    /// 平均带宽队列
    avg_bw_fq: FixedQueue<u32>,
    
    /// 平均带宽之和
    avg_bw_sum: u64,
}

impl Probe {
    pub fn new() -> Self {
        let dashbord = Dashbord::default();
        dashbord.cwnd.store(MIN_CWND, Ordering::Relaxed);
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
            full_bw_reached: false,
            avg_bw_fq: FixedQueue::new(10),
            avg_bw_sum: 0,
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
            bw: (interval_bytes * MILLIS_PER_SEC + interval_ms - 1) / interval_ms
        }
    }
    
    fn update_avg_bw(&mut self, rs: &RateSample) {
        self.avg_bw_sum += rs.bw;
        self.avg_bw_fq.push(rs.bw as u32).map(|x| {
            self.avg_bw_sum -= x as u64;
        });
        self.dashbord.bw.store(self.avg_bw_sum / self.avg_bw_fq.len() as u64, Ordering::Relaxed);
    }

    fn update_bw(&mut self, rs: &RateSample) -> u32 {
        let origin_bw = self.bw.minmax_get();
        self.bw.minmax_running_max(MAX_BW_CWND, self.sample_update_cnt, rs.bw as u32);
        origin_bw
    }
    
    fn check_full_bw(&mut self) {
        if self.full_bw_reached {
            if self.dashbord.cwnd() <= MIN_CWND || self.bw.minmax_get() < self.full_bw * FULL_BW_THRESH >> SCALE {
                self.full_bw = 0;
                self.full_bw_reached = false;
            } else {
                return;
            }
        }

        let bw_thresh = (self.full_bw as u64 * BW_UP_THRESH >> SCALE) as u32;
        
        if self.bw.minmax_get() > bw_thresh {
            self.full_bw_cnt = 0;
            self.full_bw = self.bw.minmax_get();
        } else {
            self.full_bw_cnt += 1;
            self.full_bw_reached = self.full_bw_cnt >= FULL_BW_CNT;
        }
    }

    fn update_gain(&mut self, bw: u64, origin_bw: u64) {
        if !self.full_bw_reached {
            self.cwnd_gain = if_else!(self.full_bw_cnt == 0, STARTUP_GAIN, UNIT);
            return;
        }

        if self.cwnd_high_gain_cnt > 0 {
            self.cwnd_high_gain_cnt -= 1;
            self.cwnd_high_gain = self.cwnd_high_gain_cnt == 0;
        }
        
        if bw <= origin_bw * BW_DOWN_THRESH >> SCALE {
            self.cwnd_low_gain_cnt += 1;
            self.cwnd_gain = UNIT;
            return;
        }

        if self.cwnd_normal_gain_cnt >= NORMAL_GAIN_CYCLE || bw >= origin_bw * BW_UP_THRESH >> SCALE {
            self.cwnd_normal_gain_cnt = 0;
            self.cwnd_gain = CWND_HIGHT_GAIN;
            self.cwnd_high_gain_cnt = HIGHT_GAIN_CYCLE;
        } else if self.cwnd_high_gain || self.cwnd_low_gain_cnt >= LOW_GAIN_CYCLE {
            self.cwnd_normal_gain_cnt = 0;
            self.cwnd_high_gain = false;
            self.cwnd_gain = CWND_LOW_GAIN;
        } else {
            // 当连续 n 个样本没有增长，则尝试进行增长
            self.cwnd_normal_gain_cnt += 1;
            self.cwnd_gain = UNIT;
        }

        self.cwnd_low_gain_cnt = 0;
    }

    fn update_cwnd(&mut self, gain: u32) {
        let mut cwnd = self.dashbord.cwnd.load(Ordering::Acquire);
        
        if gain == CWND_HIGHT_GAIN || gain == STARTUP_GAIN {
            cwnd += 2; // 防止小窗口增益过小
        }
        
        cwnd = ((cwnd as u64 * gain as u64) >> SCALE) as u32;
        self.dashbord
            .cwnd
            .store(cwnd.max(MIN_CWND), Ordering::Release);
    }

    pub fn dashbord(&self) -> Dashbord {
        self.dashbord.clone()
    }
}

impl PacketAck for Probe {
    fn ack(&mut self, read_size: u32) {
        self.inflight_dec();
        self.dashbord.acked_bytes.fetch_add(read_size as u64, Ordering::Relaxed);

        if self.last_ack_stamp + UPDATE_INTERVAL > datetime::now_millis() as u64 {
            return;
        }

        let rs = self.get_rate_sample();
        let origin_bw = self.update_bw(&rs);
        self.update_avg_bw(&rs);
        self.check_full_bw();
        self.update_gain(rs.bw, origin_bw as u64);
        self.update_cwnd(self.cwnd_gain);
        
        debug!(
            "\ncwnd: {}\tmax bw: {}Mib/s\tnew bw: {}Mib/s\tcwnd_gain: {}\t\
            down thresh: {}\tnew bw bytes: {}\t new_bw <= down_thresh: {}",
            self.dashbord.cwnd(),
            origin_bw / 1024 / 1024,
            rs.bw / 1024 / 1024,
            self.cwnd_gain,
            origin_bw as u64 * BW_DOWN_THRESH >> SCALE,
            rs.bw,
            rs.bw <= origin_bw as u64 * BW_DOWN_THRESH >> SCALE, 
        );
    }
}

struct RateSample {
    bw: u64,
}
