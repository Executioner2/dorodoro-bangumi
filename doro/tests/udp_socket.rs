//! 进行 udp socket 结构选型的实践
//!
//! 两种方式：
//!  1. 所有 UDP Tracker 共享同一个 socket
//!  2. 一个 UDP Tracker 对应一个 socket
//!
//! 比较点：吞吐量
//!
//! # 服务端处理逻辑
//!
//! ``` rust
//! use std::net::UdpSocket;
//!
//! let socket = UdpSocket::bind("192.168.2.177:8000").unwrap();
//! let mut buf = [0; 20];
//! loop {
//!     if let Ok((_, addr)) = socket.recv_from(&mut buf) {
//!         socket.send_to(buf.as_ref(), addr).unwrap();
//!     }
//! }
//! ```
//!
//! 测试结果：
//!
//! 方案1：所有 UDP Tracker 共享同一个 socket
//!
//!     十个线程收发 20 字节：
//!     第一次 接收到的字节数: 4987680Byte[4.76MB] 发送的字节数: 4987880Byte[4.76MB]
//!     第二次 接收到的字节数: 5293700Byte[5.05MB] 发送的字节数: 5293820Byte[5.05MB]
//!     第三次 接收到的字节数: 5075920Byte[4.84MB] 发送的字节数: 5076120Byte[4.84MB]
//!     十个线程收发 1024 字节：
//!     第一次 接收到的字节数: 246957056Byte[235.52MB] 发送的字节数: 246967296Byte[235.53MB]
//!     第二次 接收到的字节数: 230904832Byte[220.21MB] 发送的字节数: 230915072Byte[220.22MB]
//!     第三次 接收到的字节数: 259508224Byte[247.49MB] 发送的字节数: 259518464Byte[247.50MB]
//!
//! 方案2：一个 UDP Tracker 对应一个 socket
//!
//!     十个线程收发 20 字节：
//!     第一次 接收到的字节数: 5120800Byte[4.88MB] 发送的字节数: 5120980Byte[4.88MB]
//!     第二次 接收到的字节数: 5025180Byte[4.79MB] 发送的字节数: 5025380Byte[4.79MB]
//!     第三次 接收到的字节数: 4809140Byte[4.59MB] 发送的字节数: 4809340Byte[4.59MB]
//!     十个线程收发 1024 字节：
//!     第一次 接收到的字节数: 246234112Byte[234.83MB] 发送的字节数: 246243328Byte[234.84MB]
//!     第二次 接收到的字节数: 243945472Byte[232.64MB] 发送的字节数: 243955712Byte[232.65MB]
//!     第三次 接收到的字节数: 275804160Byte[263.03MB] 发送的字节数: 275814400Byte[263.04MB]
//!
//! 结论：吞吐量基本差不多。但是，在测试时，通过观察 mac 的活动监视器，发现方案2的CPU负载对比方案1是显著的低，具体截图
//! 见 tests/udp_socket 目录。因此设计上直接采用`方案2`

use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{sleep, spawn};
use std::time::Duration;

use doro_util::default_logger;
use tracing::{Level, debug};

default_logger!(Level::DEBUG);

/// 方案1：所有 UDP Tracker 共享同一个 socket
fn scheme1() {
    let recv = Arc::new(AtomicUsize::new(0));
    let send = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").unwrap());

    for i in 0..10 {
        let stop = stop.clone();
        let recv = recv.clone();
        let send = send.clone();
        let socket = socket.clone();

        spawn(move || {
            loop {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let mut byte = [255u8; 1024];
                byte[0] = i as u8;
                let x = socket.send_to(&byte, "192.168.2.177:8000").unwrap();
                send.fetch_add(x, Ordering::Relaxed);
                let (n, _addr) = socket.recv_from(&mut byte).unwrap();
                recv.fetch_add(n, Ordering::Relaxed);
            }
        });
    }

    // 测试60秒的吞吐量
    sleep(Duration::from_secs(60));
    stop.store(true, Ordering::Release);

    let recv = recv.load(Ordering::Acquire);
    let send = send.load(Ordering::Acquire);

    // 十个线程收发 20 字节：
    // 第一次 接收到的字节数: 4987680Byte[4.76MB] 发送的字节数: 4987880Byte[4.76MB]
    // 第二次 接收到的字节数: 5293700Byte[5.05MB] 发送的字节数: 5293820Byte[5.05MB]
    // 第三次 接收到的字节数: 5075920Byte[4.84MB] 发送的字节数: 5076120Byte[4.84MB]
    // 十个线程收发 1024 字节：
    // 第一次 接收到的字节数: 246957056Byte[235.52MB] 发送的字节数: 246967296Byte[235.53MB]
    // 第二次 接收到的字节数: 230904832Byte[220.21MB] 发送的字节数: 230915072Byte[220.22MB]
    // 第三次 接收到的字节数: 259508224Byte[247.49MB] 发送的字节数: 259518464Byte[247.50MB]
    debug!(
        "接收到的字节数: {}Byte[{:.2}MB] 发送的字节数: {}Byte[{:.2}MB]",
        recv,
        recv as f64 / 1048576f64,
        send,
        send as f64 / 1048576f64
    );
}

/// 方案2：一个 UDP Tracker 对应一个 socket
fn scheme2() {
    let recv = Arc::new(AtomicUsize::new(0));
    let send = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    for i in 0..10 {
        let stop = stop.clone();
        let recv = recv.clone();
        let send = send.clone();

        spawn(move || {
            let socket = UdpSocket::bind("0.0.0.0:0").unwrap();
            loop {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let mut byte = [255u8; 1024];
                byte[0] = i as u8;
                let x = socket.send_to(&byte, "192.168.2.177:8000").unwrap();
                send.fetch_add(x, Ordering::Relaxed);
                let (n, _addr) = socket.recv_from(&mut byte).unwrap();
                recv.fetch_add(n, Ordering::Relaxed);
            }
        });
    }

    // 测试60秒的吞吐量
    sleep(Duration::from_secs(60));
    stop.store(true, Ordering::Release);

    let recv = recv.load(Ordering::Acquire);
    let send = send.load(Ordering::Acquire);

    // 十个线程收发 20 字节：
    // 第一次 接收到的字节数: 5120800Byte[4.88MB] 发送的字节数: 5120980Byte[4.88MB]
    // 第二次 接收到的字节数: 5025180Byte[4.79MB] 发送的字节数: 5025380Byte[4.79MB]
    // 第三次 接收到的字节数: 4809140Byte[4.59MB] 发送的字节数: 4809340Byte[4.59MB]
    // 十个线程收发 1024 字节：
    // 第一次 接收到的字节数: 246234112Byte[234.83MB] 发送的字节数: 246243328Byte[234.84MB]
    // 第二次 接收到的字节数: 243945472Byte[232.64MB] 发送的字节数: 243955712Byte[232.65MB]
    // 第三次 接收到的字节数: 275804160Byte[263.03MB] 发送的字节数: 275814400Byte[263.04MB]
    debug!(
        "接收到的字节数: {}Byte[{:.2}MB] 发送的字节数: {}Byte[{:.2}MB]",
        recv,
        recv as f64 / 1048576f64,
        send,
        send as f64 / 1048576f64
    );
}

#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_scheme1() {
    scheme1();
}

#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_scheme2() {
    scheme2();
}
