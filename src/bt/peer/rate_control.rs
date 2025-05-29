pub mod bbr;
pub mod probe;

/// 速率控制 trait 定义
pub trait RateControl {
    /// 获取拥塞窗口大小
    /// 
    /// returns: u32 拥塞窗口大小
    fn cwnd(&self) -> u32;
    
    /// 获取正在传输的数据包数量
    /// 
    /// returns: u32 正在传输的数据包数量
    fn inflight(&self) -> u32;
    
    /// 累计确认的字节数
    /// 
    /// returns: u64 累计确认的字节数
    fn acked_bytes(&self) -> u64;
    
    /// 近期的传输速度
    /// 
    /// returns: u64 近期的传输速度
    fn bw(&self) -> u64;
}

/// 数据包发送 trait 定义
pub trait PacketSend: Clone {
    /// 发送数据包
    ///
    /// # Arguments 
    ///
    /// * `write_size`: 发送的数据包大小
    ///
    /// returns: () 
    fn send(&self, write_size: u32);
}

/// 数据包确认 trait 定义
pub trait PacketAck {
    /// 确认收到数据包
    ///
    /// # Arguments 
    ///
    /// * `read_size`: 数据包大小 
    ///
    /// returns: () 
    fn ack(&mut self, read_size: u32);
}