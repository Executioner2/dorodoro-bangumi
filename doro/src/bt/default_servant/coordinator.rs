use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use doro_util::collection::FixedQueue;
use doro_util::global::Id;
use doro_util::sync::MutexExt;
use tokio::time::Instant;
use tracing::{debug, info, trace};

use crate::base_peer::error::PeerExitReason;

pub const SCALE: u8 = 8;
pub const UNIT: u32 = 1 << SCALE;

/// 经过了 n 次循环后，进行一次重分配
pub const ALLOC_CNT_THRESH: u32 = 10;

/// 宽带增益门限，如果临时 peer 的带宽超过这个增益，才将其升级为 lt peer
pub const BW_GAIN_THRESH: u32 = UNIT * 5 / 4;

pub struct PeerSpeed {
    id: Id,
    addr: SocketAddr,
    bw: u64,
}

impl PeerSpeed {
    pub fn new(id: Id, addr: SocketAddr, bw: u64) -> Self {
        Self { id, addr, bw }
    }
}

#[async_trait]
pub trait PeerSwitch {
    /// 获取 task id
    fn get_task_id(&self) -> Id;

    /// 升级为 lt peer
    fn upgrage_lt_peer(&self, id: Id) -> Option<()>;

    /// 替换 peer
    async fn replace_peer(&self, old_id: Id, new_id: Id) -> Option<()>;

    /// 通知 peer 停止
    async fn notify_peer_stop(&self, id: Id, reason: PeerExitReason);

    /// 开启一个新的临时 peer
    async fn start_temp_peer(&self);

    /// 找到最慢的 lt peer
    fn find_slowest_lt_peer(&self) -> Option<PeerSpeed>;

    /// 找到最快的 temp peer
    fn find_fastest_temp_peer(&self) -> Option<PeerSpeed>;

    /// 拿走累计的 peer 传输速度
    fn take_peer_transfer_speed(&self) -> u64;

    /// 获得下载文件大小
    fn file_length(&self) -> u64;

    /// 已下载的文件大小
    fn download_length(&self) -> u64;

    /// 是否达到 peer 限制
    fn is_peer_limit(&self) -> bool;

    /// 是否完成下载
    fn is_finished(&self) -> bool;
}

/// 协调器，用于定时统计上传/下载速率，进行分块下载分配
pub struct Coordinator<T> {
    /// peer 交换机
    switch: Arc<T>,

    /// 速率窗口
    speed_window: Mutex<FixedQueue<u64>>,

    /// 速率总和
    speed_sum: AtomicU64,

    /// 统计计数
    alloc_cnt: AtomicU32,
}

/// a 是否比 b 快 25%
pub fn faster(a: u64, b: u64) -> bool {
    a * UNIT as u64 >= b * BW_GAIN_THRESH as u64
}

impl<T> Drop for Coordinator<T> {
    fn drop(&mut self) {
        use tracing::info;
        info!("Coordinator 已 drop");
    }
}

impl<T: PeerSwitch> Coordinator<T> {
    pub fn new(switch: Arc<T>) -> Self {
        Coordinator {
            switch,
            speed_window: Mutex::new(FixedQueue::new(5)),
            speed_sum: AtomicU64::new(0),
            alloc_cnt: AtomicU32::new(0),
        }
    }

    fn speed_rate_statistics(&mut self) {
        let speed = self.switch.take_peer_transfer_speed();
        self.speed_window
            .lock_pe()
            .push(speed)
            .map(|head| self.speed_sum.fetch_sub(head, Ordering::Relaxed));
        let speed = self.speed_sum.fetch_add(speed, Ordering::Relaxed) + speed;
        let download = self.switch.download_length();
        let len = self.speed_window.lock_pe().len();
        let file_length = self.switch.file_length();
        trace!(
        // info!(
            "下载速度: {:.2} MiB/s\t当前进度: {:.2}%",
            speed as f64 / len as f64 / 1024.0 / 1024.0,
            download as f64 / file_length as f64 * 100.0
        );
    }

    /// 检查是否可以升级为 lt peer
    async fn checkout_upgrade_lt_peer(&mut self) -> Option<()> {
        let stp = self.switch.find_slowest_lt_peer();
        let ftp = self.switch.find_fastest_temp_peer();

        if ftp.is_none() {
            // 没有 lt peer 了，那么直接把当前这个临时 peer 升级为 lt peer
            self.switch.upgrage_lt_peer(stp.as_ref()?.id)?;
            debug!("将临时 peer 升级为 lt peer，addr: {}", stp.as_ref()?.addr);
        } else if faster(stp.as_ref()?.bw, ftp.as_ref()?.bw) {
            // 替换 peer
            self.switch.replace_peer(ftp.as_ref()?.id, stp.as_ref()?.id).await?;
            debug!("将 {} 替换为 lt peer，停止 {}", ftp.as_ref()?.addr, stp.as_ref()?.addr);
        } else {
            // 关闭临时 peer
            let cmd = PeerExitReason::PeriodicPeerReplace;
            self.switch.notify_peer_stop(stp.as_ref()?.id, cmd).await;
            debug!("关闭临时 peer，addr: {}", stp.as_ref()?.addr);
        }

        Some(())
    }

    async fn peer_alloc(&mut self) {
        // if level_enabled!(Level::DEBUG) {
        //     self.printf_peer_status();
        // }

        if self.alloc_cnt.load(Ordering::Relaxed) < ALLOC_CNT_THRESH {
            self.alloc_cnt.fetch_add(1, Ordering::Relaxed);
            return;
        } else if self.switch.is_peer_limit() {
            return;
        }

        self.alloc_cnt.store(0, Ordering::Relaxed);

        // 判断之前开启的临时 peer 是否可以升级为 lt peer
        self.checkout_upgrade_lt_peer().await;

        // 开启一个新的临时 peer
        self.switch.start_temp_peer().await;
    }

    // fn printf_peer_status(&self) {
    //     let mut peers = self
    //         .ctx
    //         .peers
    //         .iter()
    //         .map(|item| {
    //             (
    //                 item.value().addr,
    //                 item.value().dashbord.bw(),
    //                 item.value().dashbord.cwnd(),
    //             )
    //         })
    //         .collect::<Vec<_>>();
    //     peers.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    //     let mut str = String::new();
    //     for (addr, bw, cwnd) in peers {
    //         let (rate, unit) = doro_util::net::rate_formatting(bw);
    //         str.push_str(&format!("{}: {:.2} {} - [{}]\t", addr, rate, unit, cwnd));
    //     }
    //     let len: usize = tokio::task::block_in_place(move || {
    //         Handle::current().block_on(async { self.ctx.wait_queue.lock().await.len() })
    //     });
    //     debug!("\n当前 peer 状态: {} [wait num: {}]", str, len);
    // }
}

impl<T: PeerSwitch> Coordinator<T> {
    pub async fn run(mut self) {
        let start = Instant::now() + Duration::from_secs(1);
        let mut interval = tokio::time::interval_at(start, Duration::from_secs(1));

        loop {
            interval.tick().await;
            if self.switch.is_finished() {
                break;
            }
            self.speed_rate_statistics();
            self.peer_alloc().await;
        }

        info!("协调器已退出");
    }
}
