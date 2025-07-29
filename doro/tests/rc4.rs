//! rc4 加密解密实现 demo

use byteorder::{BigEndian, WriteBytesExt};
use doro::base_peer::reserved;
use doro::protocol::{BIT_TORRENT_PAYLOAD_LEN, BIT_TORRENT_PROTOCOL, BIT_TORRENT_PROTOCOL_LEN};
use doro_util::bytes_util::Bytes2Int;
use doro_util::default_logger;
use lazy_static::lazy_static;
use num_bigint::BigUint;
use num_traits::Num;
use rand::{Rng, RngCore};
use rc4::cipher::StreamCipherCoreWrapper;
use rc4::consts::*;
use rc4::{KeyInit, Rc4, Rc4Core, StreamCipher};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{Level, info};

default_logger!(Level::DEBUG);

lazy_static! {
    static ref PRIME: BigUint = BigUint::from_str_radix("FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A63A36210000000000090563", 16).unwrap();

    static ref G: BigUint = BigUint::from(2u32);
}

/// 加解密器
type Rc4Encrypt = StreamCipherCoreWrapper<Rc4Core<U20>>;

/// 本地私钥长度
/// 160 bits = 20 bytes
const LOCAL_PRIVATE_KEY_LEN: usize = 20;

/// 共享密钥长度
const SHARE_KEY_LEN: usize = 96;

/// VC
const VC: [u8; 8] = [0u8; 8];

/// crypto option 长度
const CRYPTO_OPTION_LEN: usize = 4;

/// 空数据块，偏移加密数据位置
const DISCARD: [u8; 1024] = [0u8; 1024];

/// 加密提供类型
enum CryptoProvide {
    // 明文数据传输
    Plaintext = 0x01,

    // Rc4 加密传输
    Rc4 = 0x02,
}

enum KeyType {
    KeyA,
    KeyB,
}

impl KeyType {
    pub fn as_bytes(&self) -> &'static [u8; 4] {
        match self {
            KeyType::KeyA => b"keyA",
            KeyType::KeyB => b"keyB",
        }
    }
}

/// 生成本地私钥
fn generate_private_key() -> BigUint {
    let mut private_key = [0u8; LOCAL_PRIVATE_KEY_LEN];
    rand::rng().fill_bytes(&mut private_key);
    BigUint::from_bytes_be(&private_key)
}

/// 根据本地私钥生成公钥
fn generate_public_key(lprk: &BigUint) -> BigUint {
    G.clone().modpow(lprk, &PRIME)
}

/// 生成密钥对
fn generate_key_pair() -> (BigUint, BigUint) {
    let lprk = generate_private_key();
    let lpuk = generate_public_key(&lprk);
    (lprk, lpuk)
}

/// 生成随机 pad 数据
fn generate_pad() -> Vec<u8> {
    let len = rand::rng().random_range(0..=512usize);
    let mut pad = vec![0u8; len];
    rand::rng().fill_bytes(&mut pad);
    pad
}

/// 发送本地公钥
fn send_secret(lpuk: &BigUint) -> Vec<u8> {
    let mut data = lpuk.to_bytes_be().to_vec();
    let mut pad = generate_pad();
    data.append(&mut pad);
    data
}

/// 接收到远程公钥
fn compute_secret(data: &mut [u8]) -> BigUint {
    if data.len() < SHARE_KEY_LEN {
        panic!("data len is too short");
    }

    BigUint::from_bytes_be(&data[..SHARE_KEY_LEN])
}

/// 准备发送方的加密密钥
fn encrypt_key(s: &BigUint, skey: &[u8], key_type: KeyType) -> Vec<u8> {
    let mut hasher = Sha1::new();
    hasher.update(key_type.as_bytes());
    hasher.update(s.to_bytes_be());
    hasher.update(skey);
    hasher.finalize().to_vec()
}

/// 生成解密器
fn generate_decryptor(s: &BigUint, skey: &[u8], key_type: KeyType) -> Rc4Encrypt {
    let key = encrypt_key(s, skey, key_type);
    let mut decryptor = Rc4::<U20>::new(key.as_slice().into());
    let mut discard = DISCARD;
    decryptor.apply_keystream(&mut discard); // 安全规定，使用前需要初始化，即丢弃 1024 个字节的密钥流
    decryptor
}

/// 构造第三步的握手包
/// 3.  **A -> B：** `HASH('req1', S)`, `HASH('req2', SKEY) xor HASH('req3', S)`, `ENCRYPT(VC, crypto_provide, len(PadC), PadC, len(IA))`, `ENCRYPT(IA)`
fn build_step3_packet(
    rc4: &mut Rc4Encrypt,
    s: &BigUint,
    skey: &[u8],
    vc: &[u8],
    crypto_provide: u32,
    pad_c: &[u8],
    ia: &[u8],
) -> Vec<u8> {
    // 计算 req1
    let hash_req1 = {
        let mut hasher = Sha1::new();
        hasher.update(b"req1");
        hasher.update(s.to_bytes_be());
        hasher.finalize().to_vec()
    };

    // 计算 req2 xor req3
    let hash_xor = {
        let hash_req2 = {
            let mut hasher = Sha1::new();
            hasher.update(b"req2");
            hasher.update(skey);
            hasher.finalize().to_vec()
        };

        let hash_req3 = {
            let mut hasher = Sha1::new();
            hasher.update(b"req3");
            hasher.update(s.to_bytes_be());
            hasher.finalize().to_vec()
        };

        hash_req2
            .iter()
            .zip(hash_req3.iter())
            .map(|(a, b)| a ^ b)
            .collect::<Vec<_>>()
    };

    // 构建明文数据
    let mut plain_part3 = Vec::new();
    plain_part3.extend_from_slice(vc);
    plain_part3.extend(&crypto_provide.to_be_bytes());
    plain_part3.extend(&(pad_c.len() as u16).to_be_bytes());
    plain_part3.extend(pad_c);
    plain_part3.extend(&(ia.len() as u16).to_be_bytes());

    // 加密明文数据
    let mut encrypted_part3 = plain_part3.clone();
    rc4.apply_keystream(&mut encrypted_part3);

    // 加密发送方初始载荷数据
    let mut encrypted_ia = ia.to_vec();
    rc4.apply_keystream(&mut encrypted_ia);

    let mut packet = Vec::new();
    packet.extend_from_slice(&hash_req1);
    packet.extend_from_slice(&hash_xor);
    packet.extend_from_slice(&encrypted_part3);
    packet.extend_from_slice(&encrypted_ia);

    packet
}

/// 解密第 4 步的握手数据
fn decrypt_step4_packet(remote_encrypt: &mut Rc4Encrypt, recv: &mut [u8]) -> usize {
    // 解析出 VC
    let mut pos = 0usize;

    while pos < recv.len() {
        if recv.len() - pos < 8 {
            // 剩余数据不足一个 VC，断定这个不是有效的握手包
            info!("剩余数据不足一个 VC\tpos: {}\tlen: {}", pos, recv.len());
            return 0;
        }
        remote_encrypt.apply_keystream(&mut recv[pos..pos + 8]);
        if recv[pos..pos + 8] == VC {
            info!("找到VC");
            break;
        }
        pos += 8;
    }

    if pos >= recv.len() {
        // 没有找到 VC，断定这个不是有效的握手包
        info!("没有找到VC");
        return 0;
    }

    pos += 8; // 跳过 VC

    // 解析出 crypto_select
    if recv.len() < pos + 4 {
        return 0;
    }
    remote_encrypt.apply_keystream(&mut recv[pos..pos + CRYPTO_OPTION_LEN]);
    let crypto_select = u32::from_be_slice(&recv[pos..pos + CRYPTO_OPTION_LEN]);
    info!("加密方式: {:?}", crypto_select);
    if crypto_select & CryptoProvide::Rc4 as u32 == 0
        && crypto_select & CryptoProvide::Plaintext as u32 == 0
    {
        // 不支持的加密方式
        info!("不支持的加密方式");
        return 0;
    }

    pos += 4; // 跳过 crypto_select

    // 解析出 PadD 长度
    if recv.len() < pos + 2 {
        return 0;
    }
    remote_encrypt.apply_keystream(&mut recv[pos..pos + 2]);
    let pad_d_len = u16::from_be_slice(&recv[pos..pos + 2]) as usize;
    info!("pad_d_len: {}", pad_d_len);
    pos += 2; // 跳过 PadD 长度

    remote_encrypt.apply_keystream(&mut recv[pos..pos + pad_d_len]);
    pos += pad_d_len; // 跳过 PadD

    if crypto_select & CryptoProvide::Rc4 as u32 != 0 {
        // 对载荷解密
        remote_encrypt.apply_keystream(&mut recv[pos..]);
    }

    pos
}

/// 加密载荷数据
fn encrypt_payload(local_encrypt: &mut Rc4Encrypt, payload: &mut [u8]) {
    local_encrypt.apply_keystream(payload);
}

/// 解密载荷数据
fn decrypt_payload(remote_encrypt: &mut Rc4Encrypt, payload: &mut [u8]) {
    remote_encrypt.apply_keystream(payload);
}

#[test]
fn test() {
    let (xa, ya) = generate_key_pair();
    let (xb, yb) = generate_key_pair();

    let s1 = ya.modpow(&xb, &PRIME);
    let s2 = yb.modpow(&xa, &PRIME);
    assert_eq!(s1, s2);
    info!("s1: {}\n s2: {}", s1, s2);
}

/// 测试生成本地密钥对
#[test]
fn test_generate_local_key() {
    let (lpk, gpk) = generate_key_pair();

    // 打印本地私钥和公钥
    info!("local private key: {}", lpk);
    info!("local public key: {}", gpk);

    info!("local private key len: {}", lpk.to_bytes_be().len());
    info!("local public key len: {}", gpk.to_bytes_be().len());
}

/// 测试发送公钥，看看有没有响应
#[tokio::test]
async fn test_send_peer() {
    // let mut socket = TcpStreamExt::connect("192.168.2.242:3115").await.unwrap();
    let mut socket = TcpStream::connect("106.73.62.197:40370").await.unwrap();
    let (lprk, lpuk) = generate_key_pair();
    let data = send_secret(&lpuk);
    socket.write_all(&data).await.unwrap();

    info!("发送端本地公钥: {:?}", lpuk.to_bytes_be());

    socket.readable().await.unwrap();
    let mut buffer = vec![0u8; 9120];
    let n = socket.read(&mut buffer).await.unwrap();
    let mut data = buffer[..n].to_vec();
    info!("收到对端的公钥响应数据: {:?}", data);

    let remote_public_key = compute_secret(&mut data);
    info!("对端的公钥: {:?}", remote_public_key.to_bytes_be());

    let s = remote_public_key.modpow(&lprk, &PRIME);
    info!("根据本地私钥计算出共享对称密钥s: {:?}", s.to_bytes_be());
    info!("根据对端公钥计算出共享对称密钥s: {}", s);

    let info_hash = hex::decode("c6bbdb50bd685bacf8c0d615bb58a3a0023986ef").unwrap();
    info!("请求下载的资源的 info_hash: {}", hex::encode(&info_hash));

    // 生成解密器
    let mut local_encrypt = generate_decryptor(&s, &info_hash, KeyType::KeyA); // 本地加密器
    let mut remote_encrypt = generate_decryptor(&s, &info_hash, KeyType::KeyB); // 对端加密器

    // 构造第三步握手包
    let data = build_step3_packet(
        &mut local_encrypt,
        &s,
        &info_hash,
        &VC,
        CryptoProvide::Rc4 as u32,
        &[],
        &[],
    );
    socket.write_all(&data).await.unwrap();

    // 接收第四步握手包
    socket.readable().await.unwrap();
    let mut buffer = vec![0u8; 9120];
    let n = socket.read(&mut buffer).await.unwrap();
    let mut data = buffer[..n].to_vec();
    info!("data len: {}", data.len());
    info!("data: {:?}", data);

    let pos = decrypt_step4_packet(&mut remote_encrypt, &mut data);
    info!("解密后 pos: {}", pos);
    info!("解密后 data: {:?}", &data[pos..]);

    info!("向对端发起握手请求");
    let peer_id = doro_util::rand::gen_peer_id();
    info!("peer_id: {:?}", hex::encode(peer_id));

    let mut bytes =
        Vec::with_capacity(1 + BIT_TORRENT_PROTOCOL_LEN as usize + BIT_TORRENT_PAYLOAD_LEN);
    WriteBytesExt::write_u8(&mut bytes, BIT_TORRENT_PROTOCOL_LEN).unwrap();
    std::io::Write::write(&mut bytes, BIT_TORRENT_PROTOCOL).unwrap();
    let ext = reserved::LTEP | reserved::DHT;
    WriteBytesExt::write_u64::<BigEndian>(&mut bytes, ext).unwrap();
    std::io::Write::write(&mut bytes, &info_hash).unwrap();
    std::io::Write::write(&mut bytes, &peer_id).unwrap();
    encrypt_payload(&mut local_encrypt, &mut bytes);
    socket.write_all(&bytes).await.unwrap();

    // 等待握手信息
    socket.readable().await.unwrap();
    let mut handshake_resp = vec![0u8; bytes.len()];
    let size = socket.read(&mut handshake_resp).await.unwrap();
    if size != bytes.len() {
        info!("响应数据长度与预期不符 [{}]\t[{}]", size, bytes.len());
        return;
    }

    info!("握手响应原始数据流: {:?}", handshake_resp);

    decrypt_payload(&mut remote_encrypt, &mut handshake_resp);
    info!("解密握手数据: {:?}", handshake_resp);

    let protocol_len = u8::from_be_bytes([handshake_resp[0]]) as usize;
    info!("协议长度: {}", protocol_len);

    let resp_info_hash = &handshake_resp[1 + protocol_len + 8..1 + protocol_len + 8 + 20];
    let peer_id = &handshake_resp[1 + protocol_len + 8 + 20..];

    info!("对端响应的 info_hash: {}", hex::encode(resp_info_hash));
    info!("对端响应的 peer_id: {}", hex::encode(peer_id));
    assert_eq!(info_hash, resp_info_hash, "没有在讨论同一个资源文件");
}
