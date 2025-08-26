//! BBR 拥塞控制。应用层上的 BBR 实现，主要是为了解决从 peer 下载资源，速率动态的最大化。
//! BBR 核心的 RTT 不是容易就能利用 TCP 的 Timestamp 拿到，需要操作原始的 Socket。这
//! 个实现成本较大，现阶段先从应用层处理 RTT。这里有个特殊的点需要注意，带宽受限可能并不真
//! 的是带宽受限，很有可能是对端 peer 硬盘性能导致的延时增加。不过在当前场景下，我们完全可
//! 以将其作为带宽受限处理。
//!
//! ## 名词：
//!
//!     cwnd_gain：拥塞窗口增益，控制拥塞窗口相对于 BDP 的比例
//!     pacing_gain：步调增益，用于控制分组发送的时间间隔比例
//!     inflight：已发送，待确认的数据
//!     cwnd：一个 RTprop 内，发送出去待确认的上限。一个 cwnd 分成 n 个分组，间隔发送
//!
//! ## 公式：
//!
//!     单个分组的往返时间：RTProp = 应用受限阶段测量（10S 变化一次）
//!     瓶颈链路带宽：bltbw = 带宽受限阶段，6～10RTProp 内的最大交付速率
//!     带宽延迟积：BDP = bltbw * RTprop
//!     拥塞窗口：cwnd = cwnd_gain * BDP
//!     分组发送间隔：pacing_rate = pacing_gain * bltbw
//!     下一个分组发送时间：next_send_time = Now() + packet.size / pacing_rate
//!
//! ## 参考资料：
//! [bbr 草案](https://datatracker.ietf.org/doc/html/draft-cardwell-iccrg-bbr-congestion-control)\
//! [linux bbr 实现](https://github.com/torvalds/linux/blob/master/net/ipv4/tcp_bbr.c#L160)

use std::cmp::max;
use std::hash::Hash;
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(target_has_atomic = "64")]
use std::sync::atomic::AtomicU64;
#[cfg(not(target_has_atomic = "64"))]
use portable_atomic::AtomicU64;

use doro_util::win_minmax::Minmax;
use doro_util::{datetime, default_logger, if_else};
use fnv::FnvHashMap;
use futures::future::BoxFuture;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tracing::{Level, info};

use crate::BBRState::{Drain, ProbeBW, ProbeRtprop, Startup};

default_logger!(Level::DEBUG);

/// 带宽计算基准，作用是消除小包微秒级别的交付导致速率计算异常高。
///
///     例如，1MSS，在 1us 内交付了。如果不做消尖，其速率如下为：
///     1MSS / 0.0000001s = 1500B * 1_000_000 = 12000b * 1_000_000 = 12_000_000_000 = 12Gbps
///     消尖后如下：
///     12000b * 1_000_000 / (2 ^ 24) = 715bps
///
/// 取 24 没有特别原因，只是这个值合适，最高支持到 11.99Gbps 传输速率（一个包 1MSS 大小，最高 715bps * (1 ^ 24) ~= 11.99 Gbps）
const BW_SCALE: u8 = 24;
const BW_UNIT: u32 = 1 << BW_SCALE;

/// pacing_gain 计算基准
const BBR_SCALE: u8 = 8;
const BBR_UNIT: u32 = 1 << BBR_SCALE;

/// pacing_gain 定义，用整数代替浮点
const BBR_PACING_GAIN: [u32; 10] = [
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

/// 微秒到秒
const MICROS_PER_SEC: u64 = 1_000_000;

/// 最小往返时间间隔窗口 10 秒
const BBR_MIN_RTT_WIN_SEC: u32 = 10;

/// ProbeRtprop 阶段持续时长
const BBR_PROBE_RTT_MODE_US: u32 = 200_000;

/// 发送速率比理想值略低 1%，可以减少队列堆积概率
const BBR_PACING_MARGIN_PERCENT: u32 = 1;

/// BBR Startup 阶段，快速增益值。使用 2/ln(2)，虽然后来 BBR 团队重新推到出 4ln(2) 这个新的值（原因
/// 听说是 BBR 团队丢失了 2/ln(2) 的推导过程）。不过由于 4ln(2) 并没有应用到 linux 中（[参考源码](https://github.com/torvalds/linux/blob/master/net/ipv4/tcp_bbr.c#L160) ），
/// 为了系统可靠性，先采用 2/ln(2)。
const BBR_HIGH_GAIN: u32 = BBR_UNIT * 2885 / 1000 + 1;
/// 就是 1/(2/ln(2)) => 1 / 2.885 罢了
const BBR_DRAIN_GAIN: u32 = BBR_UNIT * 1000 / 2885;
/// 窗口增益，指数增长
const BBR_CWND_GAIN: u32 = BBR_UNIT * 2;

/// Startup 阶段，判断 bw 增幅是否超过 1.25
const BBR_FULL_BW_THRESH: u32 = BBR_UNIT * 5 / 4;
/// 进过 3 轮，增幅没有超过 [`BBR_FULL_BW_THRESH`]
const BBR_FULL_BW_CNT: u8 = 3;

/// ProbeBW 阶段的周期长度
const CYCLE_LEN: usize = 8;

/// 带宽窗口长度
const BBR_BW_RTTS: u32 = 10;

/// cwnd 最小值
const BBR_CWND_MIN_TARGET: u32 = 4;
/// 初始的 cwnd 窗口大小
const TCP_INIT_CWND: u32 = 10;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct StartupState {
    /// 窗口乘数
    multiplier: u32,

    /// 连续满足 25% 增长的轮次
    full_bw_cnt: u8,

    /// 最小瓶颈带宽观测值
    min_bltbw: u64,
}

impl Default for StartupState {
    fn default() -> Self {
        Self {
            multiplier: 1,
            full_bw_cnt: 0,
            min_bltbw: 0,
        }
    }
}

#[derive(Clone, Eq, PartialEq, Hash, Debug, Default)]
#[allow(dead_code)]
struct ProbeBWState {
    /// 一个周期中的第 n 个 RTT（一共 8 个周期）
    index: u8,
}

/// BBR 状态机
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
enum BBRState {
    /// 连接建立，指数型增加 cwnd。快速从应用受限转入到带宽受限。
    /// 连续三个 RTprop 交付速率不增加 25%，进入带宽受限状态
    ///
    /// - 测量 RTprop
    /// - 将链路注满（可以理解为把水管注满水）
    Startup,

    /// 排空阶段
    ///
    /// - Startup 阶段会探索带宽上限，造成排队分组注入，因此需
    ///   要排空，使得 inflight = BDP
    /// - 排空排队分组，能够在低延迟下保持高速率
    Drain,

    /// 高速发送阶段，周期性（8 个 RTT 一个周期）探索上限。按照
    /// [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0] 的倍
    /// 率设置发送间隔。如果速率增加，则提高 bltbw，如果速率不变，
    /// 延迟增加，则在 10 个 RTT 后，降低速率（避免波动导致变速）
    ///
    ProbeBW,

    /// RTprop 测量阶段。在高速发送阶段创造 2% 的时间条件测量
    /// RTprop。通常是 10 秒一个周期，花 200ms 测量 RTprop，
    /// 应用本身受限的情况无需进入此状态。其目的是为了适应路由
    /// 变化，在应用受限阶段的低延迟下，重新计算下 cwnd（拥塞
    /// 窗口）大小
    ProbeRtprop,
}

/// 包参数
struct Packet {
    /// 包的发送时间
    sendtime: u64,

    /// 在发这个包时，接收到的数据量
    delivered: u64,

    /// 在发这个包时，的微秒时间戳
    delivered_time: u64,

    /// 发这个包时是否处于应用受限状态
    app_limited: bool,
}

/// 速率样本
#[derive(Debug, Clone)]
struct RateSample {
    /// 此时间间隔的起始时间戳（单位为毫秒）
    #[allow(dead_code)]
    prior_mstamp: u64,

    /// 在 "prior_mstamp" 时刻的已交付数据包总量（tp->delivered 值）
    prior_delivered: u64,

    /// 在此时间间隔内交付的数据包数量（可能为负值，表示计数器回绕）
    delivered: u64,

    /// 时间间隔的总时长（单位为微秒）
    interval_us: i64,

    /// 最后一个被(S)ACK确认的数据包的RTT（单位为微秒，-1表示无效值）
    rtt_us: i64,

    /// 此样本是否来自管道中存在空闲的应用受限数据包？
    is_app_limited: bool,
}

/// 解析包 id
trait PacketId<Key> {
    /// 从发送数据中解析出 id。用于 RTT 计算
    fn packet_id_parse(&self) -> Option<Key>;

    /// 包大小
    fn packet_size(&self) -> u32;
}

/// 载入要发送出去的包
trait LoadSendPackage<Packet, Key>
where
    Packet: PacketId<Key>,
{
    /// 获取要发送到数据
    fn get_package_callback(&mut self, next_time: Duration) -> BoxFuture<Option<Packet>>;
}

/// 读取回调
trait BBRReadCallback<Packet, Key>
where
    Packet: PacketId<Key>,
{
    /// 接收数据
    fn recv<'a>(
        &'a self, read: &'a mut OwnedReadHalf, addr: &'a SocketAddr,
    ) -> BoxFuture<'a, Packet>;

    /// 处理数据
    fn handle_data(&mut self, data: Packet);
}

enum Command<K: Hash + Eq + 'static + Send> {
    /// 确认包
    Ack((Option<K>, u32)),
}

type CommandChannel<K> = Option<(Sender<Command<K>>, Receiver<Command<K>>)>;

/// bbr 拥塞控制
struct BBRCongestion<T, P, K>
where
    T: LoadSendPackage<P, K>,
    P: PacketId<K> + Into<Vec<u8>>,
    K: Hash + Eq + 'static + Send,
{
    /// 拥塞窗口增益
    cwnd_gain: u32,

    /// 发送步调增益，ProbeBW 状态用到。一个周期的第一个 RTT 为激励探测
    /// 设置为 1.25，第二个 RTT 为排空传输设置为 0.75，其余为稳定阶段 1.0。
    /// 激励探测时如果速率提高，则将 bltbw 增加 1.25。
    pacing_gain: u32,

    /// 待确认数据，单位是数据包数量
    inflight: u32,

    /// 拥塞窗口，在应用在 TCP 中的单位是 MSS （报文段）。在这里可以当作请求
    /// 一次 piece
    ///
    /// 计算公式：cwnd = cwnd_gain * BDP = cwnd_gain * bltbw * RTProp
    cwnd: u32,

    /// 保存的拥塞窗口。一般是从 ProbeRtprop 状态恢复到之前到窗口值
    prior_cwnd: u32,

    /// ProbeRtprop 状态持续时长（微秒）
    probe_rtt_done_stamp: u64,

    /// 是否完成了 ProbeRtprop 的测量
    probe_rtt_round_done: bool,

    /// 最小延迟，单位微秒
    min_rtt_us: u32,

    /// 最小延迟更新的时间戳，精确到微秒
    min_rtt_stamp: u64,

    /// 发送速率 Bps
    pacing_rate: u64,

    /// 最大带宽，单位 pkts/us << 24（每微秒交付的包）
    bw: Minmax,

    /// 带宽受限上限值
    full_bw: u32,

    /// 带宽受限连续状态
    full_bw_cnt: u8,

    /// 是否进入带宽受限阶段
    full_bw_reached: bool,

    /// 发送的数据块大小，类似 MSS 大小
    block_size: u32,

    /// bbr 状态机
    state: BBRState,

    /// rtt 计时
    wait_ack: FnvHashMap<K, Packet>,

    /// bbr 回调处理
    bbr_callback: T,

    /// 写入数据
    write: OwnedWriteHalf,

    /// 接收 ReadHandle 的消息
    channel: CommandChannel<K>,

    /// 累计收到的数据包
    delivered: u64,

    /// 下一个 rtt 的累计确认数据包
    next_rtt_delivered: u64,

    /// 最近一次收到ack的时间戳（微秒）
    delivered_time: u64,

    /// 是否记录 rtt
    #[allow(dead_code)]
    record_rtt: bool,

    /// ProbeBW 阶段周期的相位下标
    cycle_idx: usize,

    /// ProbeBW 阶段，上一次周期相位更新的时间戳
    cycle_mstamp: u64,

    /// rtt 计数
    rtt_cnt: u32,

    /// 受限带宽（即受到链路或对端限速的带宽）
    lt_bw: u32,

    /// 当前的传输速率是否受到限制，为固定速率
    lt_use_bw: bool,

    /// 长期 rtt 计数
    #[allow(dead_code)]
    lt_rtt_cnt: u32,

    /// 是否开启一个新的 rtt 计数周期
    round_start: bool,

    _marker: PhantomData<P>,
}

impl<T, P, K> BBRCongestion<T, P, K>
where
    T: LoadSendPackage<P, K>,
    P: PacketId<K> + Into<Vec<u8>>,
    K: Hash + Eq + 'static + Send,
{
    pub fn new<ReadCallback, ReadPacket>(
        stream: TcpStream, bbr_callback: T, block_size: u32, read_callback: ReadCallback,
    ) -> Self
    where
        ReadCallback: BBRReadCallback<ReadPacket, K> + Send + 'static,
        ReadPacket: PacketId<K> + 'static + Send,
    {
        let (read, write) = stream.into_split();
        let (tx, rx) = channel(100);

        let bbr_read = BBRRead::new(read, read_callback, tx.clone());
        tokio::spawn(bbr_read.run());

        Self {
            cwnd_gain: BBR_HIGH_GAIN,
            pacing_gain: BBR_HIGH_GAIN,
            inflight: 0,
            cwnd: TCP_INIT_CWND,
            prior_cwnd: 0,
            probe_rtt_done_stamp: 0,
            probe_rtt_round_done: false,
            min_rtt_us: 0,
            min_rtt_stamp: 0,
            pacing_rate: ((TCP_INIT_CWND * BBR_HIGH_GAIN) >> BBR_SCALE) as u64,
            bw: Minmax::new(),
            full_bw: 0,
            full_bw_cnt: 0,
            full_bw_reached: false,
            block_size,
            state: Startup,
            wait_ack: FnvHashMap::default(),
            bbr_callback,
            write,
            channel: Some((tx, rx)),
            delivered: 0,
            next_rtt_delivered: 0,
            delivered_time: datetime::now_micros() as u64,
            record_rtt: true,
            cycle_idx: 0,
            cycle_mstamp: 0,
            rtt_cnt: 0,
            lt_bw: 0,
            lt_use_bw: false,
            lt_rtt_cnt: 0,
            round_start: false,
            _marker: PhantomData,
        }
    }

    /// 发送数据
    pub async fn send(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.inflight >= self.cwnd {
            return Ok(());
        }

        let data = self
            .bbr_callback
            .get_package_callback(self.next_send_time())
            .await;
        if data.is_none() {
            return Ok(());
        }

        let data = data.unwrap();
        let id = data.packet_id_parse().unwrap();
        let packet = Packet {
            sendtime: datetime::now_micros() as u64,
            delivered: self.delivered,
            delivered_time: self.delivered_time,
            app_limited: self.cwnd <= self.inflight + 1,
        };

        self.wait_ack.insert(id, packet);
        let data: Vec<u8> = data.into();
        self.write.write_all(&data).await.unwrap();
        self.inflight += 1;

        Ok(())
    }

    /// 获取下一次发送时间（微秒级精度）
    pub fn next_send_time(&self) -> Duration {
        let packet_size = self.block_size;
        let next = Duration::from_micros(packet_size as u64 * MICROS_PER_SEC / self.pacing_rate);
        info!(
            "cwnd: {}\tpacing_rate: {}\tpacket_size: {}\t下一次发送时间: {:?}\tmin rtt: {}us\t",
            self.cwnd, self.pacing_rate, packet_size, next, self.min_rtt_us
        );
        next
    }

    /// 根据 min RTT 和 瓶颈链路带宽计算出来的理论最大流动量（单位为包的数量）
    fn theory_inflight_limit(&self, bw: u32, gain: u32) -> u32 {
        let mut inflight = self.bdp(bw, gain);
        if self.state == ProbeBW && self.cycle_idx == 0 {
            // 多增加一点，确保小带宽下也能超过 BDP。比如 BDP 为 3，这种情况
            // 乘以增益的 1.25，忽略小数后是没有变化的。
            inflight += 2;
        }
        inflight
    }

    /// 根据瓶颈链路带宽和 min RTT（RTprop）计算 bdp：
    ///
    ///     bdp = ceil(bw * min_rtt * gain)
    ///
    /// # Arguments
    ///
    /// * `bw`: 瓶颈链路带宽
    /// * `gain`: 增益
    ///
    /// returns: u32 返回 带宽延迟积
    fn bdp(&self, bw: u32, gain: u32) -> u32 {
        if self.min_rtt_us == u32::MAX {
            // 一般刚开始建立连接，还没有 rtt 时
            return TCP_INIT_CWND;
        }

        let bdp = bw as u64 * self.min_rtt_us as u64;

        // 带宽延迟积乘上增益，然后缩放（bw 是放大了 BBR_UNIT 的，这里需要缩小）。再使用 BW_UNIT 进行
        // 毛刺处理，避免初始阶段，小包过快得到 ack，导致速率拔尖的高。
        ((bdp * gain as u64) >> BBR_SCALE).div_ceil(BW_UNIT as u64) as u32
    }

    /// 如果因为限速导致带宽一直处于一个稳定值，则在 48 个 rtt 内保持这个速率，
    /// 即设置 pacing_gain 为 1。否则正常更新 bltbw。
    fn lt_bw_sampling(&mut self, _rs: &RateSample) {}

    /// 获取近期带宽，考虑受到限速的情况
    fn bw(&self) -> u32 {
        if_else!(self.lt_use_bw, self.lt_bw, self.max_bw())
    }

    /// 获取近期最大带宽，不考虑是否受到限速
    fn max_bw(&self) -> u32 {
        self.bw.minmax_get()
    }

    /// 更新带宽
    fn update_bw(&mut self, rs: &RateSample) {
        self.round_start = false;
        if rs.delivered == 0 || rs.rtt_us <= 0 {
            return;
        }

        // 通过样本发送时的确认数量，和 bbr 下一个 rtt 的确认数量对比。
        // 可以确定当前样本是否已经是下一个 rtt 的样本了。保证先发送，后
        // 抵达的迷路数据包不会影响 rtt 测量。如果是正常的数据样本，则设置
        // round_start 为 true，即允许进行 min rtt 更新
        if rs.prior_delivered >= self.next_rtt_delivered {
            self.rtt_cnt += 1;
            self.round_start = true;
            self.next_rtt_delivered = self.delivered
        }

        self.lt_bw_sampling(rs);

        let bw = (rs.delivered * BW_UNIT as u64 / rs.interval_us as u64) as u32;
        if !rs.is_app_limited || bw >= self.max_bw() {
            self.bw.minmax_running_max(BBR_BW_RTTS, self.rtt_cnt, bw);
        }
    }

    /// 估算 ack 确认前的空档期内，偏差的 inflight 值。这个其实是因为在 tcp 中，
    /// ack 确认有可能会等待下个响应到来时，再顺带 ack 上一个请求。但是在应用层上
    /// 的 bbr 实现，基本上可以不用关心这个问题，所以这里先暂时不实现。
    fn update_ack_aggregation(&mut self, _rs: &RateSample) {}

    /// 检查是否可以进入到下一个相位
    fn is_next_cycle_phase(&self) -> bool {
        // 检查是否充满了链路通道，即距离上一次相位切换，至少要大于 min rtt
        let is_full_length = (self.delivered_time - self.cycle_mstamp) as u32 > self.min_rtt_us;

        // 在无增益的情况下使用，那么只需要判断是否充满链路即可
        if self.pacing_gain == BBR_UNIT {
            return is_full_length;
        }

        let bw = self.max_bw();
        if self.pacing_gain > BBR_UNIT {
            return is_full_length
                && self.inflight >= self.theory_inflight_limit(bw, self.pacing_gain);
        }

        // 增益小于 1.0 的情况，只要当前流动量数据小于等于 1.0 增益下的理论最大流动量，或者已经充满了链路
        // 通道，即可进行下一个相位。避免坚持当前负增益而导致链路利用率不足
        is_full_length || self.inflight <= self.theory_inflight_limit(bw, BBR_UNIT)
    }

    /// 更新周期相位
    fn advance_cycle_phase(&mut self) {
        self.cycle_idx = (self.cycle_idx + 1) & (CYCLE_LEN - 1);
        self.cycle_mstamp = self.delivered_time
    }

    /// 更新周期相位，只有当前状态是 ProbeBW，并且距离上一次更新间隔了一定时间，
    /// 且处于带宽受限状态
    fn update_cycle_phase(&mut self) {
        if self.state == ProbeBW && self.is_next_cycle_phase() {
            self.advance_cycle_phase();
        }
    }

    /// 检查是否进入带宽受限
    fn check_full_bw_reached(&mut self, rs: &RateSample) {
        if self.full_bw_reached || !self.round_start || rs.is_app_limited {
            return;
        }

        let bw_thresh = (self.full_bw * BBR_FULL_BW_THRESH) >> BBR_SCALE;
        info!("当前带宽最大值: {}\t受限值: {}", self.max_bw(), bw_thresh);
        if self.max_bw() >= bw_thresh {
            // 当前带宽最大值大于等于受限值，则更新受限值
            self.full_bw = self.max_bw();
            self.full_bw_cnt = 0;
        } else {
            self.full_bw_cnt += 1;
            self.full_bw_reached = self.full_bw_cnt >= BBR_FULL_BW_CNT;
        }
    }

    /// 重置为 ProbeBW 状态
    fn reset_probe_bw_mode(&mut self) {
        info!("进入 ProbeBW 状态");
        self.state = ProbeBW;
        self.cycle_idx = rand::random_range(0..CYCLE_LEN);
        self.advance_cycle_phase();
    }

    /// 检查 Drain 状态
    fn check_drain(&mut self, _rs: &RateSample) {
        if self.state == Startup && self.full_bw_reached {
            info!("进入 Drain 状态");
            self.state = Drain;
        }

        let bw = self.max_bw();
        if self.state == Drain && self.inflight <= self.theory_inflight_limit(bw, BBR_UNIT) {
            // 充满了链路，进入 ProbeBW 状态
            self.reset_probe_bw_mode()
        }
    }

    /// 重置为 Startup 状态
    fn reset_startup_mode(&mut self) {
        info!("进入 Startup 状态");
        self.state = Startup;
    }

    /// 重置状态。如果触及到带宽上限，进入 ProbeBW。否则进入 Startup
    fn reset_mode(&mut self) {
        if self.full_bw_reached {
            self.reset_probe_bw_mode();
        } else {
            self.reset_startup_mode();
        }
    }

    /// 保存拥塞窗口，便于 ProbeRtprop 状态后恢复
    fn save_cwnd(&mut self) {
        self.prior_cwnd = self.cwnd;
    }

    /// 检查 ProbeRtprop 状态是否完成
    fn check_probe_rtt_done(&mut self) {
        let now = datetime::now_micros() as u64;
        if self.probe_rtt_done_stamp == 0 || now < self.probe_rtt_done_stamp {
            return;
        }

        self.min_rtt_stamp = now;
        self.cwnd = max(self.cwnd, self.prior_cwnd);
        self.reset_mode();
    }

    /// 更新最小往返时间
    fn update_min_rtt(&mut self, rs: &RateSample) {
        let now = datetime::now_micros() as u64;
        let filter_expired =
            now >= self.min_rtt_stamp + BBR_MIN_RTT_WIN_SEC as u64 * MICROS_PER_SEC;

        // 这个包的 rtt 小于 min rtt（当前最小），或者 min rtt 已经过时（超过了 10s）。则更新 min rtt
        if rs.rtt_us >= 0 && ((rs.rtt_us as u32) < self.min_rtt_us || filter_expired) {
            self.min_rtt_us = rs.rtt_us as u32;
            self.min_rtt_stamp = now;
        }

        if BBR_PROBE_RTT_MODE_US > 0 && filter_expired && self.state != ProbeRtprop {
            info!("进入 ProbeRtprop 状态");
            self.state = ProbeRtprop; // 切换到 rtt 测量状态
            self.save_cwnd();
            self.probe_rtt_done_stamp = 0;
        }

        if self.state == ProbeRtprop {
            // 持续等待待确认数据小于等于最小 cwnd
            if self.probe_rtt_done_stamp == 0 && self.inflight <= BBR_CWND_MIN_TARGET {
                self.probe_rtt_done_stamp = now + BBR_PROBE_RTT_MODE_US as u64;
                self.probe_rtt_round_done = false;
                self.next_rtt_delivered = self.delivered;
            } else if self.probe_rtt_done_stamp > 0 {
                if self.round_start {
                    self.probe_rtt_round_done = true;
                }
                if self.probe_rtt_round_done {
                    self.check_probe_rtt_done();
                }
            }
        }
    }

    /// 更新增益值
    fn update_gains(&mut self) {
        match self.state {
            Startup => {
                self.cwnd_gain = BBR_HIGH_GAIN;
                self.pacing_gain = BBR_HIGH_GAIN;
            }
            Drain => {
                self.cwnd_gain = BBR_DRAIN_GAIN;
                self.pacing_gain = BBR_HIGH_GAIN;
            }
            ProbeBW => {
                self.cwnd_gain = BBR_CWND_GAIN;
                self.pacing_gain =
                    if_else!(self.lt_use_bw, BBR_UNIT, BBR_PACING_GAIN[self.cycle_idx]);
            }
            ProbeRtprop => {
                self.cwnd_gain = BBR_UNIT;
                self.pacing_gain = BBR_UNIT;
            }
        }
    }

    /// 更新数值
    fn update_model(&mut self, rs: RateSample) {
        self.update_bw(&rs); // 更新带宽
        self.update_ack_aggregation(&rs); // 更新最大流动数据与已确认数据的差值
        self.update_cycle_phase(); // 更新 ProbeBW 状态需要的周期相位下标
        self.check_full_bw_reached(&rs); // 检查是否处于带宽受限状态
        self.check_drain(&rs); // 检查 Drain 状态
        self.update_min_rtt(&rs); // 更新最小 rtt
        self.update_gains(); // 更新窗口和发送速率的增益值
    }

    /// 接收速率由 pkts/us 转换为 Bps
    fn rate_bytes_per_sec(&self, mut rate: u64, gain: u32) -> u64 {
        rate *= self.block_size as u64;
        rate *= gain as u64;
        rate >>= BBR_SCALE;

        // 因为 bw 的单位是 pkts/us，所以扩大为 Bps。扩大的同时，略低于预期的 1%，
        // 减少波动或额外发送数据造成的缓存队列堆积
        rate *= MICROS_PER_SEC / 100 * (100 - BBR_PACING_MARGIN_PERCENT as u64);
        rate >> BW_SCALE
    }

    /// 带宽转发送速率
    fn bw_to_pacing_rate(&self, mut rate: u64, gain: u32) -> u64 {
        rate = self.rate_bytes_per_sec(rate, gain);
        rate
    }

    /// 设置发送速率
    fn set_pacing_rate(&mut self, bw: u32, gain: u32) {
        let rate = self.bw_to_pacing_rate(bw as u64, gain);
        self.pacing_rate = rate;
    }

    /// 设置拥塞窗口大小
    fn set_cwnd(&mut self, bw: u32, gain: u32) {
        let mut cwnd = self.cwnd;
        let target_cwnd = self.theory_inflight_limit(bw, gain);
        info!("目标cwnd: {}", target_cwnd);

        if self.full_bw_reached {
            cwnd = target_cwnd.min(cwnd + 1);
        } else if cwnd < target_cwnd || self.delivered < TCP_INIT_CWND as u64 {
            cwnd += 1;
        }

        self.cwnd = cwnd.max(BBR_CWND_MIN_TARGET);
        if self.state == ProbeRtprop {
            self.cwnd = self.cwnd.min(BBR_CWND_MIN_TARGET);
        }
    }

    /// 处理指令
    fn handle_command(&mut self, cmd: Command<K>) {
        match cmd {
            Command::Ack(ack) => self.ack(ack),
        }
    }

    /// 确认数据包
    ///
    /// 每一次 ack 都需要计算：
    ///
    ///  **瓶颈带宽**（`bottleneck_bandwidth`）= 滑动窗口内（10 个 RTT）的最大（交付数据量 / 时间间隔）。
    ///  **最小 RTT**（`min_rtt`）= 滑动窗口内（10 秒）的最小 RTT。
    ///
    /// 根据当前所处状态，更新计算：
    ///
    ///  **发送速率**（`pacing_rate`） = pacing_gain * bottleneck_bandwidth
    ///  **拥塞窗口**（`cwnd`） = max(cwnd_gain * bottleneck_bandwidth * min_rtt, 4)
    fn ack(&mut self, (key, _size): (Option<K>, u32)) {
        if key.is_none() {
            return;
        }
        let instant = self.wait_ack.remove(&key.unwrap());
        if instant.is_none() {
            return;
        }

        let now = datetime::now_micros() as u64;
        self.delivered += 1;
        self.delivered_time = now;
        self.inflight -= 1;

        let packet = instant.unwrap();
        let rs = RateSample {
            prior_mstamp: packet.sendtime,
            prior_delivered: packet.delivered,
            delivered: self.delivered - packet.delivered,
            interval_us: now as i64 - packet.delivered_time as i64,
            rtt_us: now as i64 - packet.sendtime as i64,
            is_app_limited: packet.app_limited,
        };

        self.update_model(rs);
        let bw = self.bw();
        self.set_pacing_rate(bw, self.pacing_gain);
        self.set_cwnd(bw, self.cwnd_gain);
    }
}

impl<T, Packet, K> BBRCongestion<T, Packet, K>
where
    T: LoadSendPackage<Packet, K> + Send + Sync + 'static,
    Packet: PacketId<K> + 'static + Into<Vec<u8>> + Sync + Send,
    K: Hash + Eq + 'static + Send + Sync,
{
    async fn run(mut self) {
        let mut channel = self.channel.take().unwrap();
        loop {
            tokio::select! {
                cmd = channel.1.recv() => {
                    if let Some(cmd) = cmd {
                        self.handle_command(cmd);
                    } else {
                        break;
                    }
                }
                _ = self.send() => {

                }
            }
        }
    }
}

struct BBRRead<T, Packet, K>
where
    T: BBRReadCallback<Packet, K> + 'static + Send,
    Packet: PacketId<K> + 'static + Send,
    K: Hash + Eq + 'static + Send,
{
    /// 数据读取
    read: OwnedReadHalf,

    /// bbr 回调处理
    read_callback: T,

    /// 发送消息给 BBRCongestion
    send: Sender<Command<K>>,

    _marker: PhantomData<Packet>,
}

impl<T, Packet, K> BBRRead<T, Packet, K>
where
    T: BBRReadCallback<Packet, K> + 'static + Send,
    Packet: PacketId<K> + 'static + Send,
    K: Hash + Eq + 'static + Send,
{
    fn new(read: OwnedReadHalf, read_callback: T, send: Sender<Command<K>>) -> Self {
        Self {
            read,
            read_callback,
            send,
            _marker: PhantomData,
        }
    }
}

impl<T, Packet, K> BBRRead<T, Packet, K>
where
    T: BBRReadCallback<Packet, K> + 'static + Send,
    Packet: PacketId<K> + 'static + Send,
    K: Hash + Eq + 'static + Send,
{
    async fn run(mut self) {
        let addr = self.read.peer_addr().unwrap();

        loop {
            let res = self.read_callback.recv(&mut self.read, &addr).await;
            let key = res.packet_id_parse();
            let size = res.packet_size();
            self.read_callback.handle_data(res);
            self.send.send(Command::Ack((key, size))).await.unwrap();
        }
    }
}

#[tokio::test]
async fn main() {
    use rd::*;

    let stream = TcpStream::connect("192.168.2.177:8000").await.unwrap();
    let read_count = Arc::new(AtomicU64::new(0));
    let _handle = tokio::spawn(speed_print(read_count.clone()));

    let reader = Reader { read_count };
    let rdf = ReqDataFactory {
        tick: Instant::now(),
        interval: Duration::from_micros(0),
        piece_index: 0,
        block_offset: 0,
    };
    let bbr = BBRCongestion::new(stream, rdf, 15, reader);
    bbr.run().await;
}

pub mod rd {
    use std::net::SocketAddr;
    use std::ops::{Deref, DerefMut};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use byteorder::{BigEndian, WriteBytesExt};
    use bytes::Bytes;
    use doro::base_peer::MsgType;
    use doro::base_peer::peer_resp::PeerResp;
    use doro::base_peer::peer_resp::RespType::Normal;
    use doro_util::bytes_util::Bytes2Int;
    use doro_util::collection::FixedQueue;
    use doro_util::net::FutureRet;
    use doro_util::option_ext::OptionExt;
    use futures::future::BoxFuture;
    use tokio::net::tcp::OwnedReadHalf;
    use tracing::info;

    use crate::{BBRReadCallback, LoadSendPackage, PacketId};

    pub struct ReqDataFactory {
        pub tick: Instant,
        pub interval: Duration,
        pub piece_index: u32,
        pub block_offset: u32,
    }

    unsafe impl Send for ReqDataFactory {}
    unsafe impl Sync for ReqDataFactory {}

    pub struct BytesWrapper {
        inner: Bytes,
    }

    impl Deref for BytesWrapper {
        type Target = Bytes;

        fn deref(&self) -> &Self::Target {
            &self.inner
        }
    }

    impl DerefMut for BytesWrapper {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.inner
        }
    }

    impl From<BytesWrapper> for Vec<u8> {
        fn from(wrapper: BytesWrapper) -> Vec<u8> {
            wrapper.inner.to_vec()
        }
    }

    unsafe impl Send for BytesWrapper {}
    unsafe impl Sync for BytesWrapper {}

    impl PacketId<(u32, u32)> for BytesWrapper {
        fn packet_id_parse(&self) -> Option<(u32, u32)> {
            if self.inner.len() < 15 {
                return None;
            }

            Some((
                u32::from_be_slice(&self.inner[5..9]),
                u32::from_be_slice(&self.inner[9..13]),
            ))
        }

        fn packet_size(&self) -> u32 {
            self.inner.len() as u32
        }
    }

    impl LoadSendPackage<BytesWrapper, (u32, u32)> for ReqDataFactory {
        fn get_package_callback(&mut self, next_time: Duration) -> BoxFuture<Option<BytesWrapper>> {
            Box::pin(async move {
                let offset = 16384;
                let mut t = self.tick.elapsed();
                while t < self.interval {
                    tokio::time::sleep(self.interval - t).await;
                    t = self.tick.elapsed();
                }
                self.tick = Instant::now();
                self.interval = next_time;
                let mut data = Vec::with_capacity(17);
                data.write_u32::<BigEndian>(13).unwrap();
                data.write_u8(6).unwrap();
                data.write_u32::<BigEndian>(self.piece_index).unwrap();
                data.write_u32::<BigEndian>(self.block_offset).unwrap();
                data.write_u32::<BigEndian>(offset).unwrap();

                if u32::MAX - offset < self.block_offset {
                    self.block_offset = 0;
                    self.piece_index += 1;
                } else {
                    self.block_offset += offset;
                }

                Some(BytesWrapper {
                    inner: Bytes::from_owner(data),
                })
            })
        }
    }

    /// 读取处理
    #[derive(Clone)]
    pub struct Reader {
        pub read_count: Arc<AtomicU64>,
    }

    impl PacketId<(u32, u32)> for (MsgType, BytesWrapper) {
        fn packet_id_parse(&self) -> Option<(u32, u32)> {
            if self.0 != MsgType::Piece || self.1.inner.len() < 8 {
                return None;
            }
            Some((
                u32::from_be_slice(&self.1.inner[0..4]),
                u32::from_be_slice(&self.1.inner[4..8]),
            ))
        }

        fn packet_size(&self) -> u32 {
            (self.1.inner.len() + 5) as u32
        }
    }

    impl BBRReadCallback<(MsgType, BytesWrapper), (u32, u32)> for Reader {
        fn recv<'a>(
            &self, read: &'a mut OwnedReadHalf, addr: &'a SocketAddr,
        ) -> BoxFuture<'a, (MsgType, BytesWrapper)> {
            Box::pin(async move {
                if let FutureRet::Ok(Normal(msg_type, data)) = PeerResp::new(read, addr).await {
                    (msg_type, BytesWrapper { inner: data })
                } else {
                    panic!("不是正常的响应")
                }
            })
        }

        fn handle_data(&mut self, data: (MsgType, BytesWrapper)) {
            self.read_count
                .fetch_add((data.1.inner.len() + 5) as u64, Ordering::Relaxed);
        }
    }

    /// 速率播报
    pub async fn speed_print(read_count: Arc<AtomicU64>) {
        let start = tokio::time::Instant::now() + Duration::from_secs(1);
        let mut tick = tokio::time::interval_at(start, Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut queue = FixedQueue::new(10);
        let mut sum = 0;

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let read_count = read_count.swap(0, Ordering::Relaxed);
                    sum += read_count;
                    queue.push(read_count).map_ext(|val| sum -= val);
                    info!("当前速率: {}Mib/s", sum / queue.len() as u64 / 1024 / 1024);
                }
            }
        }
    }
}

// ===========================================================================
// TEST
// ===========================================================================

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use std::time::{Duration, Instant};

    use byteorder::{BigEndian, WriteBytesExt};
    use doro_util::buffer::ByteBuffer;
    use doro_util::datetime;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;
    use tracing::info;

    use super::rd::*;
    use crate::BBRCongestion;

    #[ignore]
    #[tokio::test]
    async fn test() {
        let stream = TcpStream::connect("192.168.2.177:8000").await.unwrap();
        let read_count = Arc::new(AtomicU64::new(0));
        let _handle = tokio::spawn(speed_print(read_count.clone()));

        let reader = Reader { read_count };
        let rdf = ReqDataFactory {
            tick: Instant::now(),
            interval: Duration::from_micros(0),
            piece_index: 0,
            block_offset: 0,
        };
        let bbr = BBRCongestion::new(stream, rdf, 15, reader);
        bbr.run().await;
    }

    /// 本地局域网 rtt 测试
    #[ignore]
    #[tokio::test]
    async fn test_local_rtt() {
        let mut stream = TcpStream::connect("192.168.2.177:8000").await.unwrap();
        let mut data = Vec::with_capacity(17);
        data.write_u32::<BigEndian>(13).unwrap();
        WriteBytesExt::write_u8(&mut data, 6).unwrap();
        data.write_u32::<BigEndian>(0).unwrap();
        data.write_u32::<BigEndian>(0).unwrap();
        data.write_u32::<BigEndian>(16384).unwrap();

        let mut res = ByteBuffer::new(16384 + 13);

        let t = datetime::now_micros();
        tokio::io::AsyncWriteExt::write_all(&mut stream, &data)
            .await
            .unwrap();

        stream.readable().await.unwrap();
        stream.read_exact(res.as_mut()).await.unwrap();

        let end = datetime::now_micros() - t;
        info!("耗时: {:?}", Duration::from_micros(end as u64));
    }
}
