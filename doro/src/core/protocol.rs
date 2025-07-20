use bytes::Bytes;

pub mod bit_torrent;
pub mod remote_control;

/// The BitTorrent protocol identifier.
pub const BIT_TORRENT_PROTOCOL: &[u8] = b"BitTorrent protocol";

/// The length of the BitTorrent protocol identifier.
pub const BIT_TORRENT_PROTOCOL_LEN: u8 = 19;

/// The length of the BitTorrent payload.
pub const BIT_TORRENT_PAYLOAD_LEN: usize = 48;

/// The Remote Control protocol identifier.
pub const REMOTE_CONTROL_PROTOCOL: &[u8] = b"Remote control protocol";

/// The length of the Remote Control protocol identifier.
pub const REMOTE_CONTROL_PROTOCOL_LEN: u8 = 23;

/// The length of the Remote Control payload.
pub const REMOTE_CONTROL_PAYLOAD_LEN: usize = 0;

/// 协议名长度占用字节数
pub const PROTOCOL_SIZE: usize = 1;

#[derive(Debug)]
pub enum Identifier {
    BitTorrent,
    RemoteControl,
}

#[derive(Debug)]
pub struct Protocol {
    pub id: Identifier,
    pub payload: Bytes,
}
