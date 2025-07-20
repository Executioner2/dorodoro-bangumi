#[cfg(test)]
mod tests;

use core::fmt::{Display, Formatter};
use std::time::{Duration, Instant};

/// 倒计时器
#[derive(Clone)]
pub struct CountdownTimer {
    instant: Instant,
    offset: Duration,
}

impl CountdownTimer {
    pub fn new(duration: Duration) -> Self {
        Self {
            instant: Instant::now() + duration,
            offset: duration,
        }
    }
    
    pub fn is_finished(&self) -> bool {
        Instant::now() >= self.instant
    }
    
    pub fn remaining(&self) -> Duration {
        self.instant.saturating_duration_since(Instant::now())
    }
    
    /// 标准库的阻塞剩余时间，不要在 async 上用，否则会阻塞整个线程   
    /// 注意，支持的精度受限于系统时钟精度，理论支持纳秒级，就会以纳秒级精度休眠    
    pub fn std_wait_reamining(&self) {
        if !self.is_finished() {
            std::thread::sleep(self.remaining());
        }
    }
    
    /// 适用于 tokio 的异步等待剩余时间，可以用于 async 函数中  
    /// 小于 1ms 的休眠会以自旋的方式进行
    pub async fn tokio_wait_reamining(&self) {
        while !self.is_finished() {
            let t = self.remaining();
            if t.as_millis() >= 1 {
                tokio::time::sleep(t).await;
            }
        }
    }
}

impl Display for CountdownTimer {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.offset)
    }
}