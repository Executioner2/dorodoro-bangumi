use bytes::Bytes;

mod bit_torrent;
mod remote_controller;

/// The BitTorrent protocol identifier.
pub const BIT_TORRENT_PROTOCOL: &[u8] = b"BitTorrent protocol";

/// The length of the BitTorrent payload.
pub const BIT_TORRENT_PAYLOAD_LEN: usize = 48;

/// The Remote Control protocol identifier.
pub const REMOTE_CONTROL_PROTOCOL: &[u8] = b"Remote control protocol";

/// The length of the Remote Control payload.
pub const REMOTE_CONTROL_PAYLOAD_LEN: usize = 0;

pub enum Identifier {
    BitTorrent,
    RemoteControl,
}

pub struct Protocol {
    pub id: Identifier,
    pub payload: Bytes
}