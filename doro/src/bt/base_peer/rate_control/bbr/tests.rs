use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsRawFd, RawFd};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use byteorder::{BigEndian, WriteBytesExt};
use bytes::Bytes;
use dashmap::DashMap;
use doro_util::bytes_util::Bytes2Int;
use doro_util::collection::FixedQueue;
use doro_util::log;
use doro_util::net::FutureRet;
use doro_util::option_ext::OptionExt;
use doro_util::timer::CountdownTimer;
use libc::socklen_t;
use nix::sys::socket;
use nix::sys::socket::{
    AddressFamily, CmsgIterator, ControlMessageOwned, MsgFlags, SockFlag, SockType, SockaddrIn, SockaddrStorage, socket, sockopt
};
use nix::sys::time::TimeVal;
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{Sender, channel};
use tracing::{Level, error, info};

use crate::base_peer::peer_resp::PeerResp;
use crate::base_peer::peer_resp::RespType::*;
use crate::bt::base_peer::rate_control::bbr::{BBRRateControl, TcpConnectionInfo, Throttle};

const BLOCK_SIZE: u32 = 1 << 14;
const MSS: u32 = 17;

type Idx = (u32, u32);

/// 网络传输速率测试
#[tokio::test]
async fn test_net_rate() {
    let stream = TcpStream::connect("192.168.2.177:8000").await.unwrap();
    let (read, mut wrtie) = stream.into_split();
    let (tx, mut rx) = channel(1000);
    let rc = BBRRateControl::new(MSS);
    let throttle = rc.throttle();
    let inflight = Arc::new(DashMap::default());
    let mut idx: Idx = (0u32, 0u32);

    async_read(read, tx, rc, inflight.clone());
    let mut ct = CountdownTimer::new(Duration::from_secs(0));

    ct = uniform_send_data(&mut wrtie, &throttle, &mut ct, &inflight, &mut idx).await;

    loop {
        tokio::select! {
            res = rx.recv() => {
                if let Some(_size) = res {
                    ct = uniform_send_data(&mut wrtie, &throttle, &mut ct, &inflight, &mut idx).await;
                } else {
                    error!("中断了 channel");
                    break;
                }
            }
        }
    }
}

/// 匀速发送数据
async fn uniform_send_data(
    write: &mut OwnedWriteHalf, throttle: &Throttle, ct: &mut CountdownTimer,
    _inflight: &Arc<DashMap<(u32, u32), Instant>>, idx: &mut Idx,
) -> CountdownTimer {
    let mut pst = ct.clone();
    while throttle.inflight() < throttle.cwnd() {
        pst.tokio_wait_reamining().await;
        let key = next_idx(idx);
        // inflight.insert(key.clone(), Instant::now());
        pst = throttle.send_next(MSS);
        send_data(write, key.0, key.1).await;
    }
    pst
}

fn next_idx(idx: &mut Idx) -> (u32, u32) {
    let res = *idx;
    if idx.1 > u32::MAX - BLOCK_SIZE {
        idx.1 = 0;
        idx.0 += 1;
    } else {
        idx.1 += BLOCK_SIZE;
    }
    res
}

async fn send_data(write: &mut OwnedWriteHalf, piece_idx: u32, block_offset: u32) {
    let mut data = Vec::with_capacity(17);
    data.write_u32::<BigEndian>(13).unwrap();
    data.write_u8(6).unwrap();
    data.write_u32::<BigEndian>(piece_idx).unwrap();
    data.write_u32::<BigEndian>(block_offset).unwrap();
    data.write_u32::<BigEndian>(BLOCK_SIZE).unwrap();
    tokio::io::AsyncWriteExt::write_all(write, &data)
        .await
        .unwrap();
}

/// 异步读取
fn async_read(
    read: OwnedReadHalf, tx: Sender<u32>, rc: BBRRateControl,
    inflight: Arc<DashMap<(u32, u32), Instant>>,
) {
    tokio::spawn(do_async_read(read, tx, rc, inflight));
}

async fn do_async_read(
    mut read: OwnedReadHalf, tx: Sender<u32>, mut rc: BBRRateControl,
    _inflight: Arc<DashMap<(u32, u32), Instant>>,
) {
    let fd = read.as_ref().as_raw_fd();
    let addr = read.peer_addr().unwrap();
    let mut bt_resp = PeerResp::new(&mut read, &addr);
    loop {
        tokio::select! {
            res = &mut bt_resp => {
                // let i = Instant::now();
                match res {
                    FutureRet::Ok(Normal(_msg_type, buf)) => {
                        let packet_size = buf.len() as u32 + 13;
                        // if let Some(key) = get_packet_id(&buf) {
                            // if let Some((_, instant)) = inflight.remove(&key) {
                            //     let rtt = instant.elapsed().as_micros() as u32;
                            //     rc.ack(packet_size as u64, rtt);
                            //     // println!("接收接收!\tkey: [{:?}]", key);
                            // } else {
                            //     panic!("丢包了！！")
                            // }
                        // } else {
                        //     panic!("丢包了！！")
                        // }
                        // let ni = get_net_info(fd).unwrap();
                        rc.ack(packet_size as u64, fd);
                        tx.send(packet_size).await.unwrap();
                    },
                    FutureRet::Ok(Heartbeat) => {
                        info!("心跳包");
                    }
                    _ => {
                        eprintln!("断开了链接");
                        break;
                    }
                }
                bt_resp = PeerResp::new(&mut read, &addr);
            }
        }
    }
}

#[allow(dead_code)]
fn get_packet_id(buf: &Bytes) -> Option<(u32, u32)> {
    if buf.len() < 13 {
        return None;
    }

    let piece_idx = u32::from_be_slice(&buf[0..4]);
    let block_offset = u32::from_be_slice(&buf[4..8]);
    Some((piece_idx, block_offset))
}

/// 测试普通方案的请求速率
#[ignore]
#[tokio::test]
async fn test_normal_net_rate() {
    let inflight = Arc::new(AtomicU64::new(0));
    let read_size = Arc::new(AtomicU64::new(0));
    let current_n = Arc::new(AtomicU64::new(4));

    let (tx, mut rx) = channel(1000);
    let stream = TcpStream::connect("192.168.2.177:8000").await.unwrap();
    let (read, mut write) = stream.into_split();
    normal_async_read(read, inflight.clone(), read_size.clone(), tx);
    tick_update_cwnd(inflight.clone(), read_size.clone(), current_n.clone());

    request_block(&mut write, &current_n, &inflight).await;

    loop {
        tokio::select! {
            _ = rx.recv() => {
                request_block(&mut write, &current_n, &inflight).await;
            }
        }
    }
}

async fn request_block(
    write: &mut OwnedWriteHalf, current_n: &Arc<AtomicU64>, inflight: &Arc<AtomicU64>,
) {
    while current_n.load(Ordering::Relaxed) > inflight.load(Ordering::Relaxed) {
        inflight.fetch_add(1, Ordering::Relaxed);
        send_data(write, 0, 0).await;
    }
}

/// 定时更新窗口大小
fn tick_update_cwnd(
    inflight: Arc<AtomicU64>, read_size: Arc<AtomicU64>, current_n: Arc<AtomicU64>,
) {
    tokio::spawn(do_tick_update_cwnd(inflight, read_size, current_n));
}

async fn do_tick_update_cwnd(
    _inflight: Arc<AtomicU64>, read_size: Arc<AtomicU64>, current_n: Arc<AtomicU64>,
) {
    let start = tokio::time::Instant::now() + Duration::from_secs(1);
    let mut tick = tokio::time::interval_at(start, Duration::from_secs(1));
    let mut window = FixedQueue::new(5);
    let mut prev_recv_size = 0;
    let mut sum = 0;
    let mut prev_rate = 0;

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let recv_size = read_size.load(Ordering::Acquire);
                let rate = recv_size - prev_recv_size;
                prev_recv_size = recv_size;
                sum += rate;
                window.push(rate).map_ext(|prev| sum -= prev);
                let current_n = update(rate, prev_rate, &current_n);
                info!("并发请求数: {}\t平均速率: {:.2} Mib/s", current_n, sum as f64 / window.len() as f64 / 1024.0 / 1024.0);
                prev_rate = rate;
            }
        }
    }
}

/// 更新网络状态并返回新的并发数
///
/// 参数：
/// - measured_rate: 最新测量的下载速率 (字节/秒)
/// - latest_rtt: 最新测量的往返时间 (秒)
///
/// 返回：调整后的并发请求数
fn update(measured_rate: u64, previous_rate: u64, current_n: &Arc<AtomicU64>) -> u64 {
    let delta_rate = measured_rate as i64 - previous_rate as i64;
    let mut current = current_n.load(Ordering::Acquire);

    // AIMD 算法调整并发数
    if delta_rate > 0 {
        current += 1;
    } else if delta_rate.abs() as f64 > 0.1 * previous_rate as f64 {
        current = (current as f64 * 0.95).floor() as u64;
    }

    current_n.store(current.clamp(1, 100), Ordering::Release);

    current_n.load(Ordering::Relaxed)
}

/// 异步读取数据
fn normal_async_read(
    read: OwnedReadHalf, inflight: Arc<AtomicU64>, read_size: Arc<AtomicU64>, tx: Sender<u64>,
) {
    tokio::spawn(do_normal_async_read(read, inflight, read_size, tx));
}

async fn do_normal_async_read(
    mut read: OwnedReadHalf, inflight: Arc<AtomicU64>, read_size: Arc<AtomicU64>, tx: Sender<u64>,
) {
    let addr = read.peer_addr().unwrap();
    let mut bt_resp = PeerResp::new(&mut read, &addr);
    loop {
        tokio::select! {
            res = &mut bt_resp => {
                match res {
                    FutureRet::Ok(Normal(_msg_type, buf)) => {
                        let size = buf.len() as u64 + 13;
                        inflight.fetch_sub(1, Ordering::Relaxed);
                        read_size.fetch_add(size, Ordering::Relaxed);
                        tx.send(size).await.unwrap();
                    },
                    FutureRet::Ok(Heartbeat) => {
                        info!("心跳包");
                    }
                    _ => {
                        error!("断开了链接");
                        break;
                    }
                }
                bt_resp = PeerResp::new(&mut read, &addr);
            }
        }
    }
}

// ===========================================================================
// 尝试获取 rtt
// ===========================================================================

/// 从 tcp socket 中获取 rtt
#[ignore]
#[tokio::test]
async fn test_get_tcp_socket_rtt() {
    let sock = socket(
        AddressFamily::Inet,
        SockType::Stream,
        SockFlag::empty(),
        None,
    )
    .unwrap();
    socket::setsockopt(&sock, sockopt::ReceiveTimestamp, &true).unwrap();
    let server_addr = SockaddrIn::from_str("192.168.2.177:8000").unwrap();
    socket::connect(sock.as_raw_fd(), &server_addr).unwrap();

    // 发送数据
    let mut data = Vec::with_capacity(17);
    data.write_u32::<BigEndian>(13).unwrap();
    data.write_u8(6).unwrap();
    data.write_u32::<BigEndian>(0).unwrap();
    data.write_u32::<BigEndian>(0).unwrap();
    data.write_u32::<BigEndian>(5).unwrap();
    socket::send(sock.as_raw_fd(), &data, MsgFlags::empty()).unwrap();

    let mut buffer = vec![0u8; 5 + 13];
    let mut cmsgspace = nix::cmsg_space!(TimeVal);
    let mut iov = [IoSliceMut::new(&mut buffer)];
    let r = socket::recvmsg::<SockaddrStorage>(
        sock.as_raw_fd(),
        &mut iov,
        Some(&mut cmsgspace),
        MsgFlags::empty(),
    )
    .unwrap();

    let mut ci: CmsgIterator = r.cmsgs().unwrap();
    info!("ci: {:?}", ci);
    if let Some(ControlMessageOwned::ScmTimestamp(rtime)) = ci.next() {
        info!("rtt: {}", rtime);
    }

    info!("cmsgspace: {:?}", cmsgspace);
    info!("buffer: {:?}", buffer);
}

/// 从 udp 中获取 rtt
#[ignore]
#[tokio::test]
async fn test_get_udp_socket_rtt() {
    // Set up
    let message = "Ohayō!".as_bytes();
    let in_socket = socket(
        AddressFamily::Inet,
        SockType::Datagram,
        SockFlag::empty(),
        None,
    )
    .unwrap();
    socket::setsockopt(&in_socket, sockopt::ReceiveTimestamp, &true).unwrap();
    let localhost = SockaddrIn::from_str("127.0.0.1:0").unwrap();
    socket::bind(in_socket.as_raw_fd(), &localhost).unwrap();
    let address: SockaddrIn = socket::getsockname(in_socket.as_raw_fd()).unwrap();
    // Get initial time
    let time0 = SystemTime::now();
    // Send the message
    let iov = [IoSlice::new(message)];
    let flags = MsgFlags::empty();
    let l = socket::sendmsg(in_socket.as_raw_fd(), &iov, &[], flags, Some(&address)).unwrap();
    assert_eq!(message.len(), l);
    // Receive the message
    let mut buffer = vec![0u8; message.len()];
    let mut cmsgspace = nix::cmsg_space!(TimeVal);
    let mut iov = [IoSliceMut::new(&mut buffer)];
    let r =
        socket::recvmsg::<SockaddrIn>(in_socket.as_raw_fd(), &mut iov, Some(&mut cmsgspace), flags)
            .unwrap();
    let rtime = match r.cmsgs().unwrap().next() {
        Some(ControlMessageOwned::ScmTimestamp(rtime)) => rtime,
        Some(_) => panic!("Unexpected control message"),
        None => panic!("No control message"),
    };
    // Check the final time
    let time1 = SystemTime::now();
    // the packet's received timestamp should lie in-between the two system
    // times, unless the system clock was adjusted in the meantime.
    let rduration = Duration::new(rtime.tv_sec() as u64, rtime.tv_usec() as u32 * 1000);
    info!(
        "rtime: {}\trtt: {:?}",
        rtime,
        time1.duration_since(UNIX_EPOCH).unwrap() - rduration
    );
    assert!(time0.duration_since(UNIX_EPOCH).unwrap() <= rduration);
    assert!(rduration <= time1.duration_since(UNIX_EPOCH).unwrap());
    // Close socket
}

/// 从 tcp socket 中获取 tcp_info
#[ignore]
#[tokio::test]
async fn test_get_tcp_socket_tcp_info() {
    let stream = TcpStream::connect("192.168.2.177:8000").await.unwrap();
    let (mut read, mut write) = stream.into_split();
    let fd = read.as_ref().as_raw_fd();
    let info = get_tco_info(fd).unwrap();
    info!(
        "tcpi_srtt: {}\ttcpi_rttcur: {}\ttcpi_rttvar: {}\ttcp_option: {}\ttcpi_rxpackets: {}\ttcpi_snd_sbbytes: {}",
        info.tcpi_srtt,
        info.tcpi_rttcur,
        info.tcpi_rttvar,
        info.tcpi_options,
        info.tcpi_rxpackets,
        info.tcpi_snd_sbbytes
    );

    send_data(&mut write, 0, 0).await;

    let info = get_tco_info(fd).unwrap();
    info!(
        "tcpi_srtt: {}\ttcpi_rttcur: {}\ttcpi_rttvar: {}\ttcp_option: {}\ttcpi_rxpackets: {}\ttcpi_snd_sbbytes: {}",
        info.tcpi_srtt,
        info.tcpi_rttcur,
        info.tcpi_rttvar,
        info.tcpi_options,
        info.tcpi_rxpackets,
        info.tcpi_snd_sbbytes
    );

    let addr = read.peer_addr().unwrap();
    let x = PeerResp::new(&mut read, &addr).await;
    // let mut buffer = vec![0u8; 5 + 13];
    // loop {
    //     read.readable().await.unwrap();
    //     let info = get_tco_info(fd).unwrap();
    //     println!("tcpi_srtt: {}\ttcpi_rttcur: {}\ttcpi_rttvar: {}tcpi_rxpackets: {}", info.tcpi_srtt, info.tcpi_rttcur, info.tcpi_rttvar, info.tcpi_rxpackets);
    //
    //     read.read_buf(&mut buffer).await.unwrap();
    //     let info = get_tco_info(fd).unwrap();
    //     println!("tcpi_srtt: {}\ttcpi_rttcur: {}\ttcpi_rttvar: {}tcpi_rxpackets: {}", info.tcpi_srtt, info.tcpi_rttcur, info.tcpi_rttvar, info.tcpi_rxpackets);
    // }
    let info = get_tco_info(fd).unwrap();
    info!(
        "tcpi_srtt: {}\ttcpi_rttcur: {}\ttcpi_rttvar: {}\ttcpi_rxpackets: {}\ttcpi_txpackets: {}\ttcpi_rxretransmitpackets: {}\ttcp_option: {}\ttcpi_snd_sbbytes: {}",
        info.tcpi_srtt,
        info.tcpi_rttcur,
        info.tcpi_rttvar,
        info.tcpi_rxpackets,
        info.tcpi_txpackets,
        info.tcpi_txretransmitpackets,
        info.tcpi_options,
        info.tcpi_snd_sbbytes
    );

    let info = get_tco_info(fd).unwrap();
    info!(
        "tcpi_srtt: {}\ttcpi_rttcur: {}\ttcpi_rttvar: {}\ttcpi_rxpackets: {}\ttcpi_txpackets: {}\ttcpi_rxretransmitpackets: {}\ttcp_option: {}\ttcpi_snd_sbbytes: {}",
        info.tcpi_srtt,
        info.tcpi_rttcur,
        info.tcpi_rttvar,
        info.tcpi_rxpackets,
        info.tcpi_txpackets,
        info.tcpi_txretransmitpackets,
        info.tcpi_options,
        info.tcpi_snd_sbbytes
    );

    let info = get_tco_info(fd).unwrap();
    info!(
        "tcpi_srtt: {}\ttcpi_rttcur: {}\ttcpi_rttvar: {}\ttcpi_rxpackets: {}\ttcpi_txpackets: {}\ttcpi_rxretransmitpackets: {}\ttcp_option: {}\ttcpi_snd_sbbytes: {}",
        info.tcpi_srtt,
        info.tcpi_rttcur,
        info.tcpi_rttvar,
        info.tcpi_rxpackets,
        info.tcpi_txpackets,
        info.tcpi_txretransmitpackets,
        info.tcpi_options,
        info.tcpi_snd_sbbytes
    );

    let info = get_tco_info(fd).unwrap();
    info!(
        "tcpi_srtt: {}\ttcpi_rttcur: {}\ttcpi_rttvar: {}\ttcpi_rxpackets: {}\ttcpi_txpackets: {}\ttcpi_rxretransmitpackets: {}\ttcp_option: {}\ttcpi_snd_sbbytes: {}",
        info.tcpi_srtt,
        info.tcpi_rttcur,
        info.tcpi_rttvar,
        info.tcpi_rxpackets,
        info.tcpi_txpackets,
        info.tcpi_txretransmitpackets,
        info.tcpi_options,
        info.tcpi_snd_sbbytes
    );

    if let FutureRet::Ok(Normal(msg_type, buf)) = x {
        info!("msg_type: {:?}", msg_type);
    }
}

#[cfg(target_os = "macos")]
fn get_tco_info(fd: RawFd) -> Option<TcpConnectionInfo> {
    let mut info = std::mem::MaybeUninit::<TcpConnectionInfo>::uninit();
    let mut len = size_of::<TcpConnectionInfo>() as socklen_t;
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
            Some(info)
        } else {
            None
        }
    }
}
