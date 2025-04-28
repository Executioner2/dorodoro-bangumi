use crate::runtime::DelayedTask;
use std::time::Duration;

/// 测试延迟任务
///
/// 加上 --nocapture
#[ignore]
#[tokio::test]
async fn test_delayed_task() {
    let mut list = vec![];

    async fn task() {
        println!("任务执行了")
    }
    
    let delayed_task = DelayedTask::new(Duration::from_secs(1), task());
    list.push(tokio::spawn(delayed_task));

    let delayed_task = DelayedTask::new(Duration::from_secs(5), task());
    let cancel = delayed_task.cancel_token();
    list.push(tokio::spawn(delayed_task));

    tokio::time::sleep(Duration::from_secs(2)).await;
    cancel.cancel();

    for handle in list {
        handle.await.unwrap();
    }
}
