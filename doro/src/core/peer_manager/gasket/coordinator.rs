use doro_util::collection::FixedQueue;
use crate::peer::rate_control::RateControl;
use crate::peer_manager::gasket::{PeerExitReason, GasketContext};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, level_enabled, trace, Level};

pub const SCALE: u8 = 8;
pub const UNIT: u32 = 1 << SCALE;

/// 经过了 n 次循环后，进行一次重分配
pub const ALLOC_CNT_THRESH: u32 = 10;

/// 宽带增益门限，如果临时 peer 的带宽超过这个增益，才将其升级为 lt peer
pub const BW_GAIN_THRESH: u32 = UNIT * 5 / 4;

/// 协调器，用于定时统计上传/下载速率，进行分块下载分配
pub struct Coordinator {
    /// gasket context
    ctx: GasketContext,

    /// 速率窗口
    speed_window: FixedQueue<f64>,

    /// 速率总和
    speed_sum: f64,

    /// 统计计数
    alloc_cnt: u32,

    /// 取消 token
    cancel_token: CancellationToken
}

/// a 是否比 b 快 25%
pub fn faster(a: u64, b: u64) -> bool {
    a * UNIT as u64 >= b * BW_GAIN_THRESH as u64
}

impl Coordinator {
    pub fn new(ctx: GasketContext, cancel_token: CancellationToken) -> Self {
        Self {
            ctx,
            speed_window: FixedQueue::new(5),
            speed_sum: 0.0,
            alloc_cnt: 0,
            cancel_token
        }
    }

    fn speed_rate_statistics(&mut self) {
        let mut speed: f64 = 0.0;
        self.ctx.peer_transfer_speed.retain(|_, read_size| {
            speed += *read_size as f64;
            false
        });
        self.speed_window
            .push(speed)
            .map(|head| self.speed_sum -= head);
        self.speed_sum += speed;
        let download = self.ctx.download.load(Ordering::Relaxed);
        let len = self.speed_window.len();
        let file_size = self.ctx.torrent.info.length;
        trace!(
            "下载速度: {:.2} MiB/s\t当前进度: {:.2}%",
            self.speed_sum / len as f64 / 1024.0 / 1024.0,
            download as f64 / file_size as f64 * 100.0
        );
    }

    /// 检查是否可以升级为 lt peer
    async fn checkout_upgrade_lt_peer(&mut self) -> Option<()> {
        let mut temp_peer = None;
        let mut low = (u64::MAX, u64::MAX, None);

        for item in self.ctx.peers.iter() {
            if !item.value().is_lt() {
                temp_peer = Some((*item.key(), item.value().dashbord.bw(), item.addr));
                continue;
            }

            let bw = item.value().dashbord.bw();
            if bw < low.1 {
                low.0 = *item.key();
                low.1 = bw;
                low.2 = Some(item.addr);
            }
        }

        if low.2.is_none() {
            // 没有 lt peer 了，那么直接把当前这个临时 peer 升级为 lt peer
            self.ctx.upgrage_lt_peer(temp_peer?.0)?;
            debug!("将临时 peer 升级为 lt peer，addr: {}", temp_peer?.2);
        } else if faster(temp_peer?.1, low.1) {
            // 替换 peer
            self.ctx.replace_peer(temp_peer?.0, low.0).await;
            debug!("将 {} 替换为 lt peer，停止 {}", temp_peer?.2, low.2?);
        } else {
            // 关闭临时 peer
            let cmd = PeerExitReason::PeriodicPeerReplace;
            self.ctx.notify_peer_stop(temp_peer?.0, cmd).await;
            debug!("关闭临时 peer，addr: {}", temp_peer?.2);
        }

        Some(())
    }

    async fn peer_alloc(&mut self) {
        if level_enabled!(Level::DEBUG) {
            self.printf_peer_status();
        }
        
        if self.alloc_cnt < ALLOC_CNT_THRESH {
            self.alloc_cnt += 1;
            return;
        } else if self.ctx.peers.len() > self.ctx.config().torrent_peer_conn_limit()
            || self.ctx.wait_queue.lock().await.is_empty()
        {
            return;
        }

        self.alloc_cnt = 0;

        // 判断之前开启的临时 peer 是否可以升级为 lt peer
        self.checkout_upgrade_lt_peer().await;

        // 开启一个新的临时 peer
        self.ctx.start_temp_peer().await;
    }
    
    fn printf_peer_status(&self) {
        let mut peers = self.ctx.peers.iter().map(|item| {
            (item.value().addr, item.value().dashbord.bw(), item.value().dashbord.cwnd())
        }).collect::<Vec<_>>();
        peers.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        let mut str = String::new();
        for (addr, bw, cwnd) in peers {
            let (rate, unit) = doro_util::net::rate_formatting(bw);
            str.push_str(&format!("{}: {:.2} {} - [{}]\t", addr, rate, unit, cwnd));
        }
        let len: usize = tokio::task::block_in_place(move || {
            Handle::current().block_on(async {
                self.ctx.wait_queue.lock().await.len()
            })
        });
        debug!("\n当前 peer 状态: {} [wait num: {}]", str, len);
    }
}

impl Coordinator {
    pub async fn run(mut self) {
        let start = Instant::now() + Duration::from_secs(1);
        let mut interval = tokio::time::interval_at(start, Duration::from_secs(1));

        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    break;
                }
                _ = interval.tick() => {
                    if self.ctx.check_finished().await {
                        break;
                    }
                    self.speed_rate_statistics();
                    self.peer_alloc().await;
                }
            }
        }

        info!("协调器已退出");
    }
}
