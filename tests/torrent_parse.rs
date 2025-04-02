//! 种子文件解析测试

use dorodoro_bangumi::bt::parse::Torrent;
use dorodoro_bangumi::parse::Parse;

#[test]
fn test_parse_torrent_file() {
    if let Ok(torrent) =
        Torrent::parse_torrent("tests/torrent/b3b2b9fbf23bda5fc649c0bbf73e5ed0e07f5fc3.torrent")
    {
        println!("{:?}", torrent);
    }
}
