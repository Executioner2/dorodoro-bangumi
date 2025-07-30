use std::thread;
use std::time::Duration;

use tracing::info;

use crate::timer::CountdownTimer;

/// 测试计时器是否正常工作
#[test]
#[ignore]
fn test_timer() {
    let ct = CountdownTimer::new(Duration::from_secs(1));
    while !ct.is_finished() {
        let t = ct.remaining();
        info!("剩余时间: {:?}", t);
        if t >= Duration::from_millis(100) {
            thread::sleep(Duration::from_millis(100))
        }
    }
}
