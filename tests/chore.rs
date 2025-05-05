//! 这个是一些杂项的验证测试

use dashmap::DashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant};
use tokio::select;
use tokio::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// 测试发送了消息后，再发送取消信号，会不会处理取消信号
///
/// 结论：会的，不过要注意处理 rx 接收到 None 的情况，这种表示发送端已关闭，可以退出循环了。
#[tokio::test]
#[ignore]
async fn test_tokio_select() {
    let layer = fmt::layer()
        .with_line_number(true)
        .with_thread_names(false)
        .with_thread_ids(true)
        .with_writer(std::io::stderr);
    tracing_subscriber::registry().with(layer).init();

    let (tx, mut rx) = channel(10);
    let cancel = CancellationToken::new();
    let cancel2 = cancel.clone();

    tokio::spawn(async move {
        tx.send("hello").await.unwrap();
        // tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        cancel2.cancel();
    });

    let mut tsuzuki_masika = true;
    while tsuzuki_masika {
        tsuzuki_masika = select! {
            _ = cancel.cancelled() => {
                info!("取消信号已收到");
                false
            }
            result = rx.recv() => {
                match result {
                    Some(msg) => {
                        info!("接收到了消息：{}", msg);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        true
                    }
                    None => {
                        info!("接收者已关闭");
                        false
                    }
                }
            }
        }
    }
}

/// 测试 oneshot channel 是否可以多次使用
///
/// 结论：不行
#[tokio::test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
async fn test_oneshot_channel() {
    let (tx, mut rx) = tokio::sync::oneshot::channel::<&'static str>();

    let t1 = tokio::spawn(async move {
        while let Ok(msg) = rx.try_recv() {
            println!("接收到了消息：{}", msg);
        }
    });

    tx.send("hello").unwrap();
    t1.await.unwrap();
    // tx.send("world").unwrap()
}

/// 测试在循环的 select! 中新建 future
#[ignore]
#[tokio::test]
async fn test_tokio_select_new_instance() {
    let (send, mut recv) = channel(100);
    let t = tokio::spawn(async move {
        let mut k = 0;
        let mut test = Test::new(k);
        for _ in 0..4 {
            select! {
                _ = &mut test => {
                    println!("[{k}] Ready 了");
                    k += 1;
                    test = Test::new(k);
                }
                res = recv.recv() => {
                    match res {
                        Some(res) => println!("接收到了消息\t{}", res),
                        None => println!("啥都没有"),
                    }
                }
            }
        }
    });

    send.send("hello").await.unwrap();
    send.send("world").await.unwrap();
    t.await.unwrap();
}

#[derive(Debug)]
enum State {
    Start,
    End,
}

#[allow(dead_code)]
#[derive(Debug)]
struct Test {
    id: usize,
    state: State,
}

impl Test {
    fn new(id: usize) -> Self {
        println!("新建了Test\tid: {}", id);
        Self {
            id,
            state: State::Start,
        }
    }
}

impl Future for Test {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        println!("触发了poll\t this: {:?}", this);
        match this.state {
            State::Start => {
                this.state = State::End;
                let waker = cx.waker().clone();
                // tokio::time::sleep(Duration::from_secs(5)).await;
                waker.wake();
                Poll::Pending
            }
            State::End => Poll::Ready(()),
        }
    }
}

/// 测试时间戳和时间间隔
#[test]
fn test_instant_duration() {
    let start = Instant::now();
    thread::sleep(Duration::from_secs(1));
    let end = start.elapsed();
    thread::sleep(Duration::from_secs(1));
    let end2 = start.elapsed();
    println!("{:?}", end);
    println!("{:?}", end2);
}

/// 这个会死锁
#[ignore]
#[cfg_attr(miri, ignore)]
#[tokio::test]
async fn test_dashmap_deadlock() {
    let map = Arc::new(DashMap::new());
    map.insert(1, "");

    let k = 1;
    let value = map.get(&k); // 这里获取时，会拿到分段锁
    let remove = map.remove(&k); // 前面的锁还没释放，这里再次尝试拿锁，拿不到，产生死锁
    println!("{:?}", value);
    println!("{:?}", remove);
}
