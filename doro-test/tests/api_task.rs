use doro::api::task_api::{Task, TorrentRet, TorrentSource};
use doro::router::ret::Ret;
use doro_test::client_util;
use doro_util::default_logger;
use tracing::{Level, info};

default_logger!(Level::DEBUG);

#[test]
fn test() {
    let link = "magnet:?xt=urn:btih:cfe40ab08b47c19e0f04d4f4b4ae5004708b4402&dn=%5BSweetSub%26LoliHouse%5D%20Takopii%20no%20Genzai%20-%2001%20%5BWebRip%201080p%20HEVC-10bit%20AAC%20ASSx2%5D.mkv";
    let idx = link.find("urn:btih:");
    info!("{:?}", idx);
}

/// 测试解析 torrent 链接，支持磁力链接和文件哈希值
#[tokio::test]
async fn test_parse_torrent_link() {
    let code = 1001;
    // let link = "15372d70d8a9eb542f7d153c36d63c2ede788514";
    // let link = "magnet:?xt=urn:btih:15372d70d8a9eb542f7d153c36d63c2ede788514&tr=http%3a%2f%2ft.nyaatracker.com%2fannounce&tr=http%3a%2f%2ftracker.kamigami.org%3a2710%2fannounce&tr=http%3a%2f%2fshare.camoe.cn%3a8080%2fannounce&tr=http%3a%2f%2fopentracker.acgnx.se%2fannounce&tr=http%3a%2f%2fanidex.moe%3a6969%2fannounce&tr=http%3a%2f%2ft.acg.rip%3a6699%2fannounce&tr=https%3a%2f%2ftr.bangumi.moe%3a9696%2fannounce&tr=udp%3a%2f%2ftr.bangumi.moe%3a6969%2fannounce&tr=http%3a%2f%2fopen.acgtracker.com%3a1096%2fannounce&tr=udp%3a%2f%2ftracker.opentrackr.org%3a1337%2fannounce";
    let link = "magnet:?xt=urn:btih:2da35cf6f641283c91a3806665eefbd2ecef1efa&tr=http%3a%2f%2ft.nyaatracker.com%2fannounce&tr=http%3a%2f%2ftracker.kamigami.org%3a2710%2fannounce&tr=http%3a%2f%2fshare.camoe.cn%3a8080%2fannounce&tr=http%3a%2f%2fopentracker.acgnx.se%2fannounce&tr=http%3a%2f%2fanidex.moe%3a6969%2fannounce&tr=http%3a%2f%2ft.acg.rip%3a6699%2fannounce&tr=https%3a%2f%2ftr.bangumi.moe%3a9696%2fannounce&tr=udp%3a%2f%2ftr.bangumi.moe%3a6969%2fannounce&tr=http%3a%2f%2fopen.acgtracker.com%3a1096%2fannounce&tr=udp%3a%2f%2ftracker.opentrackr.org%3a1337%2fannounce";

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, link).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let torrent_ret: Result<Ret<TorrentRet>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(torrent_ret.is_ok());
    assert_eq!(torrent_ret.as_ref().unwrap().status_code, 0);

    let data = torrent_ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试解析本地 torrent 文件
#[tokio::test]
async fn test_parse_torrent_file() {
    let code = 1002;
    // let file_path = "./tests/resources/test6.torrent";
    let file_path = "/Users/zhaoyuxi/Downloads/cfe40ab08b47c19e0f04d4f4b4ae5004708b4402.torrent";

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, file_path).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let torrent_ret: Result<Ret<TorrentRet>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(torrent_ret.is_ok());
    assert_eq!(torrent_ret.as_ref().unwrap().status_code, 0);

    let data = torrent_ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试从本地文件中添加任务是否成功
#[tokio::test]
async fn test_add_task_from_local_file() {
    let code = 1003;
    let task = Task {
        task_name: Some("test".to_string()),
        download_path: Some("./download".to_string()),
        // source: TorrentSource::LocalFile("./tests/resources/test7.torrent".to_string()),
        source: TorrentSource::LocalFile("/Users/zhaoyuxi/Downloads/c6bbdb50bd685bacf8c0d615bb58a3a0023986ef.torrent".to_string()),
    };

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, task).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let torrent_ret: Result<Ret<bool>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(torrent_ret.is_ok());
    assert_eq!(torrent_ret.as_ref().unwrap().status_code, 0);

    let data = torrent_ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}

/// 测试从磁力链接中添加任务是否成功
#[tokio::test]
async fn test_add_task_from_magnet_link() {
    let code = 1003;
    // let link = "magnet:?xt=urn:btih:2da35cf6f641283c91a3806665eefbd2ecef1efa&tr=http%3a%2f%2ft.nyaatracker.com%2fannounce&tr=http%3a%2f%2ftracker.kamigami.org%3a2710%2fannounce&tr=http%3a%2f%2fshare.camoe.cn%3a8080%2fannounce&tr=http%3a%2f%2fopentracker.acgnx.se%2fannounce&tr=http%3a%2f%2fanidex.moe%3a6969%2fannounce&tr=http%3a%2f%2ft.acg.rip%3a6699%2fannounce&tr=https%3a%2f%2ftr.bangumi.moe%3a9696%2fannounce&tr=udp%3a%2f%2ftr.bangumi.moe%3a6969%2fannounce&tr=http%3a%2f%2fopen.acgtracker.com%3a1096%2fannounce&tr=udp%3a%2f%2ftracker.opentrackr.org%3a1337%2fannounce";
    // let task = Task {
    //     task_name: Some("[黒ネズミたち] 去唱卡拉OK吧！ / Karaoke Iko! - 04 (ABEMA 1920x1080 AVC AAC MP4) [751.7 MB]".to_string()),
    //     download_path: Some("./download".to_string()),
    //     source: TorrentSource::MagnetURI(link.to_string()),
    // };

    let link = "magnet:?xt=urn:btih:c6bbdb50bd685bacf8c0d615bb58a3a0023986ef&dn=%5BNekomoe%20kissaten%5D%5BHibi%20wa%20Sugiredo%20Meshi%20Umashi%5D%5B08%5D%5B1080p%5D%5BJPTC%5D.mp4&xl=564643524&tr=udp%3A%2F%2Ftr.bangumi.moe%3A6969%2Fannounce&tr=http%3A%2F%2Ft.nyaatracker.com%2Fannounce&tr=http%3A%2F%2Fopen.acgtracker.com%3A1096%2Fannounce&tr=http%3A%2F%2Fopen.nyaatorrents.info%3A6544%2Fannounce&tr=http%3A%2F%2Ft2.popgo.org%3A7456%2Fannonce&tr=http%3A%2F%2Fshare.camoe.cn%3A8080%2Fannounce&tr=http%3A%2F%2Fopentracker.acgnx.se%2Fannounce&tr=http%3A%2F%2Ftracker.acgnx.se%2Fannounce&tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce&tr=http%3A%2F%2Ft.acg.rip%3A6699%2Fannounce&tr=http%3A%2F%2Ftr.bangumi.moe%3A6969%2Fannounce&tr=http%3A%2F%2Fsukebei.tracker.wf%3A8888%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2F9.rarbg.com%3A2810%2Fannounce&tr=udp%3A%2F%2Ftracker.openbittorrent.com%3A6969%2Fannounce&tr=http%3A%2F%2Fopenbittorrent.com%3A80%2Fannounce&tr=http%3A%2F%2Ftracker.openbittorrent.com%3A80%2Fannounce&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce&tr=udp%3A%2F%2Fwww.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Ftracker.zerobytes.xyz%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Ftracker.tiny-vps.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.moeking.me%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.dler.org%3A6969%2Fannounce&tr=udp%3A%2F%2Fretracker.lanta-net.ru%3A2710%2Fannounce&tr=udp%3A%2F%2Fopen.tracker.cl%3A1337%2Fannounce&tr=udp%3A%2F%2Fopentracker.i2p.rocks%3A6969%2Fannounce&tr=https%3A%2F%2Fopentracker.i2p.rocks%3A443%2Fannounce&tr=udp%3A%2F%2Fopen.demonii.com%3A1337%2Fannounce&tr=udp%3A%2F%2Fipv4.tracker.harry.lu%3A80%2Fannounce&tr=udp%3A%2F%2Fexplodie.org%3A6969%2Fannounce&tr=https%3A%2F%2Ftracker.nanoha.org%3A443%2Fannounce&tr=https%3A%2F%2Ftracker.cyber-hub.net%3A443%2Fannounce&tr=https%3A%2F%2Ftr.burnabyhighstar.com%3A443%2Fannounce";
    let task = Task {
        task_name: Some("[Nekomoe kissaten][Hibi wa Sugiredo Meshi Umashi][08][1080p][JPTC].mp4".to_string()),
        download_path: Some("./download".to_string()),
        source: TorrentSource::MagnetURI(link.to_string()),
    };

    let mut client = client_util::client().await.unwrap();
    let rf = client.request(code, task).await.unwrap();
    let ret = rf.await;
    client_util::verification_result(&code, &ret);

    let torrent_ret: Result<Ret<bool>, _> = serde_json::from_slice(ret.body.as_ref());
    assert!(torrent_ret.is_ok());
    assert_eq!(torrent_ret.as_ref().unwrap().status_code, 0);

    let data = torrent_ret.unwrap().data;
    assert!(data.is_some());
    info!("data: {:?}", data);
}