use rand::RngCore;

/// 随机生成一个用于 tracker 通信的 transaction id
#[inline]
pub fn gen_tran_id() -> u32 {
    rand::rng().next_u32()
}
