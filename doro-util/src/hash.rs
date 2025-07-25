/// SHA1 字符编码后的长度
pub const SHA1_ENCODEN_LEN: usize = 40;

/// 检查 hash 值是否有效
pub fn check_info_hash(info_hash: &str) -> bool {
    info_hash.len() == SHA1_ENCODEN_LEN && info_hash.chars().all(|c| c.is_ascii_hexdigit())
}
