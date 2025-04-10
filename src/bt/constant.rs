/// UDP tracker 相关常量
pub mod udp_tracker {
    use std::time::Duration;

    /// connection id 超时时间 - 60秒
    pub const CONNECTION_ID_TIMEOUT: u64 = 60;

    /// 最大重试次数 - 8次
    pub const MAX_RETRY_NUM: u8 = 8;

    /// UDP socket 读取超时时间 - 15秒
    pub const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(15);
    
    /// UDP socket 写入超时时间 - 15秒
    pub const SOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(15);

    /// 默认的 UDP 地址
    pub const DEFAULT_ADDR: &str = "0.0.0.0:0";

    /// UDP 包最大大小
    pub const MAX_PACKET_SIZE: usize = 65535;

    /// UDP 包最大负载大小。 65507 = 65535 - 8(UDP头) - 20(IP头)
    pub const MAX_PAYLOAD_SIZE: usize = 65507;

    /// 协议 ID
    pub const TRACKER_PROTOCOL_ID: u64 = 0x41727101980;

    /// 最小 connect 响应数据包大小
    pub const MIN_CONNECT_RESP_SIZE: usize = 16;

    /// 最小 announce 响应数据包大小
    pub const MIN_ANNOUNCE_RESP_SIZE: usize = 20;

    /// 最小 scrape 响应数据包大小
    pub const MIN_SCRAPE_RESP_SIZE: usize = 12;
}

/// HTTP tracker 相关常量
pub mod http_tracker {
    use std::time::Duration;

    /// UDP socket 读取超时时间 - 15秒
    pub const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
}