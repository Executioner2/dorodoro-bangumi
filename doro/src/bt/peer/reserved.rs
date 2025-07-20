//! peer 握手阶段保留位
//! 
//! 详情见 [bep_0004](https://www.bittorrent.org/beps/bep_0004.html)

/// Azureus 消息协议 (Azureus Messaging Protocol)
pub const AZUREUS_MESSAGING: u64 =  1 << 63;

/// BitTorrent 位置感知协议 (BitTorrent Location Aware Protocol)
pub const LOCATION_AWARE: u64 = 1 << 43;

/// LTEP（Libtorrent 扩展协议）   
/// 磁力链接解析用到       
/// 详情见 [bep_0009](https://www.bittorrent.org/beps/bep_0009.html)
pub const LTEP: u64 = 1 << 20;

/// BitTorrent DHT (分布式哈希表)
pub const DHT: u64 = 1;

/// XBT Peer Exchange (XBT 对等交换)
pub const XPE: u64 = 1 << 1;

/// suggest/haveall/havenone/reject/allow fast 扩展   
/// 详情见 [bep_0006](https://www.bittorrent.org/beps/bep_0006.html)
pub const FAST_EXTENSIONS: u64 = 1 << 2;

/// NAT Traversal (NAT 穿透)
pub const NAT_TRAVERSAL: u64 = 1 << 3;

/// Hybrid Torrent Legacy→v2 Upgrade (混合种子升级)
pub const HYBRID_TORRENT_LEGACY: u64 = 1 << 4;

