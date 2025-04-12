//! 这个是一些杂项的验证测试
use tokio::select;
use tokio::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;
use tracing::info;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::writer::MakeWriterExt;
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
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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
