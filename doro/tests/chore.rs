//! 这个是一些杂项的验证测试

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use doro_util::default_logger;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde::{Deserialize, Serialize};
use tokio::select;
use tokio::sync::mpsc::channel;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{Level, info};
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use url::Url;

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

#[ignore]
#[cfg_attr(miri, ignore)]
#[test]
fn test_dashmap_deadlock3() {
    let map = DashMap::new();
    map.insert(2, 2);
    map.insert(1, 1);
    map.insert(3, 3);

    info!("map: {map:?}");


    if let Some(mut item) = map.iter_mut().take(1).next() {
        let ret =  Some((*item.key(), *item.value()));
        if *item.value() + 1 < 2 {
            *item.value_mut() += 1;
        } else {
            // 死锁
            map.remove(item.key());
        }
        info!("ret: {ret:?}")
    }

    info!("map: {map:?}");
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

/// 测试循环依赖的 drop 情况    
/// B 和 C 相互依赖，A 依赖 B
#[test]
#[ignore]
#[allow(dead_code)]
#[cfg_attr(miri, ignore)]
fn test_loop_depend() {
    struct A {
        b: Arc<B>,
    }

    impl Drop for A {
        fn drop(&mut self) {
            info!("drop A");
        }
    }

    struct B {
        c: Arc<C>,
    }

    impl Drop for B {
        fn drop(&mut self) {
            info!("drop B");
        }
    }

    struct C {
        // 这个会有循环依赖
        // b: OnceLock<Arc<B>>,

        // 尝试嵌套一层 weak
        b: OnceLock<Weak<B>>,
    }

    impl Drop for C {
        fn drop(&mut self) {
            info!("drop C");
        }
    }

    // 开始测试
    {
        let c = Arc::new(C {
            b: OnceLock::new(),
        });

        let b = Arc::new(B {
            c: c.clone(),
        });

        // 这里设置了 b 后，导致循环依赖，无法释放 b 和 c
        // if c.b.set(b.clone()).is_err() {
        
        // 换成 weak b
        if c.b.set(Arc::downgrade(&b)).is_err() {
            panic!("init value failed")
        }

        // a 会被正常释放
        let _a = A {
            b: b.clone(),
        };
    }
    // 这里应该 drop a
}

/// 测试 join_set
#[tokio::test] 
#[ignore]
#[cfg_attr(miri, ignore)]
async fn test_join_set() {
    struct Tasks(JoinSet<()>);

    impl Drop for Tasks {
        fn drop(&mut self) {
            info!("drop Tasks");
        }
    }

    struct A;

    impl Drop for A {
        fn drop(&mut self) {
            info!("drop A");
        }
    }

    impl A {
        async fn run(self) {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                info!("tick");
            }
        }
    }

    {
        let mut tasks = Tasks(JoinSet::new());
        tasks.0.spawn(Box::pin(A.run()));
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    info!("5 秒之后，tasks 应该被 drop 了");
    tokio::time::sleep(Duration::from_secs(5)).await;
    info!("再过 5 秒之后，程序结束")
}

/// 复现 dashmap 的无限循环
#[ignore]
#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn test_dashmap_infinite_loop() {
    let dashmap = Arc::new(DashMap::new());
    dashmap.insert(1, 1);
    dashmap.insert(2, 0);

    let mut join_set = JoinSet::new();

    join_set.spawn(Box::pin(test_task(dashmap.clone())));
    join_set.spawn(Box::pin(test_task(dashmap.clone())));

    join_set.join_all().await;
    info!("join_all 结束\ndashmap: {dashmap:#?}")
}

async fn test_task(dashmap: Arc<DashMap<u32, u32>>) {
    // 下面这种写法是错误的！每次都会重复获取新的迭代器，导致无限循环
    // while let Some(mut item) = dashmap.iter_mut().next() {

    // 同时 clippy 也建议我们把下面的 while 换成 for 循环
    // let mut it = dashmap.iter_mut();
    // while let Some(mut item) = it.next() {

    for mut item in dashmap.iter_mut() {
        info!("遍历 dashmap: {} - {}", item.key(), item.value());
        if *item.value() < 2 {
            *item.value_mut() += 1;
            break;
        }
    }
    info!("遍历结束")
}

#[ignore]
#[test]
#[cfg_attr(miri, ignore)]
fn test_hashmap() {
    let mut map = HashMap::new();
    map.insert(1, 1);
    map.insert(2, 2);

    // let mut it = map.iter();
    // while let Some(item) = it.next() {

    for item in map.iter() {
        info!("遍历 hashmap: {} - {}", item.0, item.1);
    }
}

/// 测试 join_set 中的任务完成后，是否会自动从集合中移除
#[ignore]
#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn test_join_set_task_finished() {
    let mut join_set = JoinSet::new();

    join_set.spawn(Box::pin(async move {
        info!("任务开始");
        tokio::time::sleep(Duration::from_secs(1)).await;
        info!("任务完成");
    }));

    info!("join_set 长度: {}", join_set.len());

    tokio::time::sleep(Duration::from_secs(2)).await;

    info!("join_set 长度: {}", join_set.len());

    join_set.join_next().await;

    info!("join_all 结束\tjoin_set 长度: {}", join_set.len());
}

/// 测试解析磁链
#[test]
#[ignore]
fn test_parse_magnet_link() {
    let link = "magnet:?xt=urn:btih:15372d70d8a9eb542f7d153c36d63c2ede788514&tr=http%3a%2f%2ft.nyaatracker.com%2fannounce&tr=http%3a%2f%2ftracker.kamigami.org%3a2710%2fannounce&tr=http%3a%2f%2fshare.camoe.cn%3a8080%2fannounce&tr=http%3a%2f%2fopentracker.acgnx.se%2fannounce&tr=http%3a%2f%2fanidex.moe%3a6969%2fannounce&tr=http%3a%2f%2ft.acg.rip%3a6699%2fannounce&tr=https%3a%2f%2ftr.bangumi.moe%3a9696%2fannounce&tr=udp%3a%2f%2ftr.bangumi.moe%3a6969%2fannounce&tr=http%3a%2f%2fopen.acgtracker.com%3a1096%2fannounce&tr=udp%3a%2f%2ftracker.opentrackr.org%3a1337%2fannounce";
    let url = Url::parse(link).unwrap();
    info!("url: {:?}", url);
    url.query_pairs().for_each(|(k, v)| {
        info!("{}: {}", k, v);
    });
}

/// 测试 json 序列化，结构体改变后是否有影响
#[test]
#[ignore]
fn test_json_serialize() {
    #[derive(Serialize, Deserialize, Debug)]
    struct User {
        name: String,
        age: u32,
    }

    let user = User {
        name: "张三".to_string(),
        age: 18,
    };

    let json = serde_json::to_string(&user).unwrap();
    info!("json: {}", json);

    #[derive(Serialize, Deserialize, Debug)]
    struct User2 {
        name: String,
        email: Option<String>,
    }
    let json_str = "{\"name\":\"张三\",\"age\":18}";
    let user2: User2 = serde_json::from_str(json_str).unwrap();
    info!("user2: {:?}", user2);
}