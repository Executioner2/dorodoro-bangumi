use lazy_static::lazy_static;
use nanoid::nanoid;
use rand::RngCore;

pub mod http_tracker;
pub mod udp_tracker;

lazy_static! {
    /// 客户端程序 id
    static ref PEER_ID_RAND: [u8; 20] = {
        let mut id = [0u8; 20];
        // id[0..16].copy_from_slice(b"dorodoro-bangumi"); // 正式开发完成后再改成这个
        id[..].copy_from_slice(nanoid!(20).as_bytes()); // 临时使用完全随机生成的，且每次启动都不一样
        id
    };

    /// 进程 id，发送给 Tracker 的，用于区分多开情况
    static ref PROCESS_KEY: u32 = rand::rng().next_u32();
}

/// 随机生成一个 20 字节的 peer_id。当前版本下，这个 id 程序每次启动时重新生成一次。
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::bt::tracker::gen_peer_id;
///
/// let id = gen_peer_id();
/// println!("peer_id: {:?}", String::from_utf8_lossy(&id));
/// ```
pub fn gen_peer_id() -> [u8; 20] {
    *PEER_ID_RAND
}

/// 随机生成一个 4 字节的 process_key。实际上这个 key 只在程序启动时生成一次。
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::bt::tracker::gen_process_key;
///
/// let key = gen_process_key();
/// println!("process_key: {}", key);
/// ```
pub fn gen_process_key() -> u32 {
    *PROCESS_KEY
}