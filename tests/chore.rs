//! 这个是一些杂项的验证测试

use dashmap::DashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::select;
use tokio::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;
use tracing::{info, Level};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use dorodoro_bangumi::default_logger;

default_logger!(Level::DEBUG);

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
            info!("接收到了消息：{}", msg);
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
                    info!("[{k}] Ready 了");
                    k += 1;
                    test = Test::new(k);
                }
                res = recv.recv() => {
                    match res {
                        Some(res) => info!("接收到了消息\t{}", res),
                        None => info!("啥都没有"),
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
        info!("新建了Test\tid: {}", id);
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
        info!("触发了poll\t this: {:?}", this);
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
    info!("{:?}", end);
    info!("{:?}", end2);
}

/// 这个会死锁
#[ignore]
#[cfg_attr(miri, ignore)]
#[tokio::test]
async fn test_dashmap_deadlock1() {
    let map = Arc::new(DashMap::new());
    map.insert(1, "");

    let k = 1;
    let value = map.get(&k); // 这里获取时，会拿到分段锁
    let remove = map.remove(&k); // 前面的锁还没释放，这里再次尝试拿锁，拿不到，产生死锁
    info!("{:?}", value);
    info!("{:?}", remove);
}

/// 这个也会死锁
#[ignore]
#[cfg_attr(miri, ignore)]
#[tokio::test]
async fn test_dashmap_deadlock2() {
    let map = Arc::new(DashMap::new());
    map.insert(1u32, "xx");
    
    info!("before: {:?}", map.get(&1).unwrap().value());
    
    // 这样写会有死锁风险，所以避免在拿到分段锁中，再进行一次可能会对这个 dashmap 进行操作的调用
    // if let Some(item) = map.get(&1) {
    //     info!("enter if let");
    //     do_test_dashmap_deadlock2(map.clone(), item.key()).await;
    // }
    
    info!("done");
}

#[allow(dead_code)]
async fn do_test_dashmap_deadlock2(map: Arc<DashMap<u32, &str>>, key: &u32) {
    info!("enter do dd2");
    let value = map.get_mut(key);
    info!("get lock success");
    // let value = map.remove(key);
    info!("value: {:?}", value);
}

struct B {
    val: Option<i32>,
}

struct A {
    b: Option<B>,
}

#[test]
fn test_option() {
    fn f() -> Option<i32> {
        // let x = Some(A { b: Some(B { val: Some(10) }) });
        let x = Some(A { b: None });
        let v = x?.b?.val?;
        info!("{}", v);
        Some(v)
    }
    let v = f();
    info!("{:?}", v);
}

async fn test_fu(id: u32, name: &str, delay: u64) -> u32 {
    info!("{name} start");
    tokio::time::sleep(Duration::from_secs(delay)).await;
    info!("{name} end");
    id
}

/// 测试 futures
#[tokio::test]
#[ignore]
#[cfg_attr(miri, ignore)]
async fn test_futures_unordered() {
    let mut futures = FuturesUnordered::new();
    futures.push(test_fu(1, "task1", 10));
    futures.push(test_fu(2, "task2", 3));
    futures.push(test_fu(3, "task3", 5));

    while let Some(res) = futures.next().await {
        info!("result: {}", res);
    }
}