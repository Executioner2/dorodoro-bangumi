//! **基于 rc4 算法的 bt 加密传输实现**
//!
//! 握手流程：
//! 1. A->B: Diffie Hellman Ya, PadA
//! 2. B->A: Diffie Hellman Yb, PadB
//! 3. A->B: HASH('req1', S), HASH('req2', SKEY) xor HASH('req3', S), ENCRYPT(VC, crypto_provide, len(PadC), PadC, len(IA)), ENCRYPT(IA)
//! 4. B->A: ENCRYPT(VC, crypto_select, len(padD), padD), ENCRYPT2(Payload Stream)
//! 5. A->B: ENCRYPT2(Payload Stream)
//!
//! 若需快速失败行为，可使用以下条件立即断开对等节点连接。若倾向更隐蔽的识别模式，可延后断开连接。满足任一条件即可判定握手无效：
//!
//! **步骤2（由B方终止）**
//! 1. 若A方在30秒内发送字节数少于96字节
//! 2. 若A方发送字节数超过608字节
//!
//! **步骤3（由A方终止）**
//! 1. 若B方在30秒内发送字节数少于96字节
//! 2. 若B方发送字节数超过608字节
//!
//! **步骤4（由B方终止）**
//! 1. 若A方在连接启动后628字节内（同步点）未发送正确的S哈希值
//! 2. 若A方在S哈希值后未发送受支持的SKEY哈希值
//! 3. 若SKEY哈希值后VC无法被正确解码
//! 4. 若未支持任何crypto_provide选项或位字段被清零
//! 5. 此后连接终止交由下一协议层处理
//!
//! **步骤5（由A方终止）**
//! 1. 若在连接启动后616字节内（同步点）无法正确解码VC
//! 2. 若选定的加密方法未被提供
//! 3. 此后连接终止交由下一协议层处理
//!
//! 参考资料：
//! - [Message Stream Encryption](https://web.archive.org/web/20120206163648/http://wiki.vuze.com/w/Message_Stream_Encryption#Implementation_Notes_for_BitTorrent_Clients)
//! - [libtorrent pe_crypto.cpp](https://github.com/arvidn/libtorrent/blob/RC_2_0/src/pe_crypto.cpp#L329)

#[cfg(test)]
mod tests;

use crate::bt::socket::{Crypto, TcpStreamWrapper};
use anyhow::{Result, anyhow};
use bytes::BytesMut;
use doro_util::buffer::ByteBuffer;
use doro_util::bytes_util::Bytes2Int;
use doro_util::net::{AsyncReadExtExt, TcpStreamExt};
use lazy_static::lazy_static;
use num_bigint::BigUint;
use num_traits::Num;
use rand::{Rng, RngCore};
use rc4::cipher::StreamCipherCoreWrapper;
use rc4::consts::*;
use rc4::{KeyInit, Rc4, Rc4Core, StreamCipher};
use sha1::{Digest, Sha1};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::debug;

lazy_static! {
    static ref PRIME: BigUint = BigUint::from_str_radix("FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A63A36210000000000090563", 16).unwrap();

    static ref G: BigUint = BigUint::from(2u32);
}

/// 加解密器
pub type Rc4Cipher = StreamCipherCoreWrapper<Rc4Core<U20>>;

/// 本地私钥长度
/// 160 bits = 20 bytes
const LOCAL_PRIVATE_KEY_LEN: usize = 20;

/// 共享密钥长度
const SHARE_KEY_LEN: usize = 96;

/// PAD 填充的最大长度，超出可以断开链接
const PAD_MAX_LEN: usize = 512;

/// VC 最大偏移长度，也就是最多读取多少个字节，找到 VC
const VC_MAX_OFFSET: usize = 616;

/// VC
const VC: [u8; 8] = [0u8; 8];

/// crypto option 长度
const CRYPTO_OPTION_LEN: usize = 4;

/// pad 长度单位
const PAD_LEN_UNIT: usize = 2;

/// 空数据块，偏移加密数据位置
const DISCARD: [u8; 1024] = [0u8; 1024];

/// 30 秒的读取超时限定
const TIMEOUT: Duration = Duration::from_secs(30);

/// 加密提供类型
#[derive(Eq, PartialEq, Debug)]
pub enum CryptoProvide {
    /// 明文数据传输
    Plaintext = 0x01,

    /// Rc4 加密传输
    Rc4 = 0x02,
}

/// 密钥类型
enum KeyType {
    /// 发送端密钥
    KeyA,

    /// 接收端密钥
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
fn send_local_public_key(lpuk: &BigUint) -> Vec<u8> {
    let mut data = lpuk.to_bytes_be().to_vec();
    let mut pad = generate_pad();
    data.append(&mut pad);
    data
}

/// 接收到远程公钥，计算出共享密钥
fn compute_secret(data: &mut [u8], lprk: &BigUint) -> BigUint {
    if data.len() < SHARE_KEY_LEN {
        panic!("data len is too short");
    }

    let rpuk = BigUint::from_bytes_be(&data[..SHARE_KEY_LEN]);
    rpuk.modpow(lprk, &PRIME)
}

/// 准备发送方的加密密钥
#[inline]
fn encrypt_key(key_type: KeyType, s: &BigUint, skey: &[u8]) -> Vec<u8> {
    hash(&[key_type.as_bytes(), &s.to_bytes_be(), skey])
}

/// 生成解密器
fn generate_cipher(s: &BigUint, skey: &[u8], key_type: KeyType) -> Rc4Cipher {
    let key = encrypt_key(key_type, s, skey);
    let mut cipher = Rc4::<U20>::new(key.as_slice().into());
    let mut discard = DISCARD;
    cipher.apply_keystream(&mut discard); // 安全规定，使用前需要初始化，即丢弃 1024 个字节的密钥流
    cipher
}

/// sha1 哈希
fn hash(data: &[&[u8]]) -> Vec<u8> {
    let mut hasher = Sha1::new();
    for d in data {
        hasher.update(d);
    }
    hasher.finalize().to_vec()
}

/// 构造第三步，A->B 的握手包
fn encrypt_a2b_handshake_packet(
    rc4: &mut Rc4Cipher,
    s: &BigUint,
    skey: &[u8],
    vc: &[u8],
    crypto_provide: u32,
    pad_c: &[u8],
    ia: &mut [u8],
) -> Vec<u8> {
    let s_bytes = s.to_bytes_be();

    // 计算 req1
    let hash_req1 = hash(&[b"req1", &s_bytes]);

    // 计算 req2 xor req3
    let hash_xor = hash(&[b"req2", skey])
        .iter()
        .zip(hash(&[b"req3", &s_bytes]).iter())
        .map(|(a, b)| a ^ b)
        .collect::<Vec<_>>();

    // 构建加密握手参数 crypto_handshake_args
    let mut rha = Vec::new();
    rha.extend_from_slice(vc);
    rha.extend(&crypto_provide.to_be_bytes());
    rha.extend(&(pad_c.len() as u16).to_be_bytes());
    rha.extend(pad_c);
    rha.extend(&(ia.len() as u16).to_be_bytes());

    // 加密明文数据
    rc4.apply_keystream(&mut rha);

    // 加密发送方初始载荷数据
    rc4.apply_keystream(ia);

    let mut packet = Vec::new();
    packet.extend_from_slice(&hash_req1);
    packet.extend_from_slice(&hash_xor);
    packet.extend_from_slice(&rha);
    packet.extend_from_slice(ia);

    packet
}

/// 解密第四步，B->A 的握手包
async fn decrypt_b2a_handshake_packet(
    socket: &mut TcpStream,
    remote_cipher: &mut Rc4Cipher,
    recv: &mut BytesMut,
) -> Result<(CryptoProvide, usize)> {
    // 解析出 VC
    let mut pos = 0usize;
    let vc_len = VC.len();

    while pos < VC_MAX_OFFSET {
        // 规定最多读取 616 字节
        if recv.len() < pos + vc_len {
            let mut append = vec![0u8; pos + vc_len - recv.len()];
            if let Err(e) = socket.read_exact_with_timeout(&mut append, TIMEOUT).await {
                return Err(anyhow!(
                    "vc 数据不足\t已遍历偏移量[{pos}\t需要读取[{}]个字节\n{:?}",
                    append.len(),
                    e
                ));
            }
            recv.extend_from_slice(append.as_slice())
        }
        remote_cipher.apply_keystream(&mut recv[pos..pos + vc_len]);
        if recv[pos..pos + vc_len] == VC {
            break;
        }
        pos += vc_len;
    }

    if pos >= VC_MAX_OFFSET {
        return Err(anyhow!("握手失败，没有找到 VC"));
    }

    pos += vc_len; // 跳过 VC

    // 解析出 crypto_select 和 len(padD)
    if recv.len() < pos + CRYPTO_OPTION_LEN + PAD_LEN_UNIT {
        let mut append = vec![0u8; CRYPTO_OPTION_LEN + PAD_LEN_UNIT - recv.len()];
        if (socket.read_exact_with_timeout(&mut append, TIMEOUT).await).is_err() {
            return Err(anyhow!("crypto_select 和 len(padD) 长度不足"));
        }
        recv.extend_from_slice(append.as_slice());
    }

    remote_cipher.apply_keystream(&mut recv[pos..pos + CRYPTO_OPTION_LEN + PAD_LEN_UNIT]);
    let crypto_select = u32::from_be_slice(&recv[pos..pos + CRYPTO_OPTION_LEN]);
    let pad_d_len =
        u16::from_be_slice(&recv[pos + CRYPTO_OPTION_LEN..pos + CRYPTO_OPTION_LEN + PAD_LEN_UNIT])
            as usize;
    pos += CRYPTO_OPTION_LEN + PAD_LEN_UNIT; // 跳过 crypto_select

    let crypto_select = if crypto_select & CryptoProvide::Rc4 as u32 != 0 {
        CryptoProvide::Rc4
    } else if crypto_select & CryptoProvide::Plaintext as u32 != 0 {
        CryptoProvide::Plaintext
    } else {
        return Err(anyhow!("不支持的加密方式"));
    };

    if recv.len() < pos + pad_d_len {
        let mut append = ByteBuffer::new(pos + pad_d_len - recv.len());
        match socket
            .read_exact_with_timeout(append.as_mut(), TIMEOUT)
            .await
        {
            Ok(size) => append.resize(size),
            Err(_) => return Err(anyhow!("padD 长度不足")),
        }
        recv.extend_from_slice(append.as_ref());
    }

    // 注意这里要对 pad 数据也进行解密，不然这里的 remote_cipher 流会和对端的 remote_cipher 不一致
    remote_cipher.apply_keystream(&mut recv[pos..pos + pad_d_len]);
    pos += pad_d_len; // 跳过 PadD

    if crypto_select == CryptoProvide::Rc4 {
        // 对载荷解密
        remote_cipher.apply_keystream(&mut recv[pos..]);
    }

    Ok((crypto_select, pos))
}

/// 加密载荷数据
pub fn encrypt_payload(local_cipher: &mut Rc4Cipher, payload: &mut [u8]) {
    local_cipher.apply_keystream(payload);
}

/// 解密载荷数据
pub fn decrypt_payload(remote_cipher: &mut Rc4Cipher, payload: &mut [u8]) {
    remote_cipher.apply_keystream(payload);
}

/// 初始化握手
///
/// # Arguments
///
/// * `socket`: 待握手的 TcpStreamExt
/// * `info_hash`: 种子哈希值
///
/// returns: 正常情况返回 TcpStreamWrapper，包含密钥，实现读取和写入的加解密封装
pub async fn init_handshake(
    mut socket: TcpStream,
    info_hash: &[u8],
    cp: CryptoProvide,
) -> Result<TcpStreamWrapper> {
    debug!("进行 pe crypto 握手");
    if cp == CryptoProvide::Plaintext {
        return Ok(TcpStreamWrapper::new(
            socket,
            Crypto::Plaintext,
            Crypto::Plaintext,
        ));
    }

    // 生成本地端密钥
    let (lprk, lpuk) = generate_key_pair();
    socket.write_all(&send_local_public_key(&lpuk)).await?;
    let mut recv = [0u8; SHARE_KEY_LEN];
    let size = socket.read_extra_with_timeout(&mut recv, TIMEOUT).await?;
    debug!("在分享公钥时，丢弃了[{size}]个字节的填充数据");
    if size > PAD_MAX_LEN {
        return Err(anyhow!("握手失败，长度超出 608 字节"));
    }

    // 计算出共享密钥
    let s = compute_secret(&mut recv, &lprk);

    // 生成解密器
    let mut local_cipher = generate_cipher(&s, info_hash, KeyType::KeyA);
    let mut remote_cipher = generate_cipher(&s, info_hash, KeyType::KeyB);

    // 发送握手包
    let data = encrypt_a2b_handshake_packet(
        &mut local_cipher,
        &s,
        info_hash,
        &VC,
        cp as u32,
        &[],
        &mut [],
    );
    socket.write_all(&data).await?;

    // 不加附加载荷数据，最大 1134 字节
    // VC最大长度（616） + crypto_select（4） + len_padD（2） + padD（512）
    let mut recv = BytesMut::with_capacity(1134);
    socket.read_buf_with_timeout(&mut recv, TIMEOUT).await?;
    let (crypto_select, _) =
        decrypt_b2a_handshake_packet(&mut socket, &mut remote_cipher, &mut recv).await?;
    debug!("加密方式: {:?}", crypto_select);

    let (lc, rc) = if crypto_select == CryptoProvide::Rc4 {
        (
            Crypto::Rc4(Box::new(local_cipher)),
            Crypto::Rc4(Box::new(remote_cipher)),
        )
    } else {
        (Crypto::Plaintext, Crypto::Plaintext)
    };

    Ok(TcpStreamWrapper::new(socket, lc, rc))
}
