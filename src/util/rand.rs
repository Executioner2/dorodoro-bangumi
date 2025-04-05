use rand::RngCore;

/// 随机生成一个用于 tracker 通信的 transaction id
pub fn rand_transaction_id() -> u32 {
    rand::rng().next_u32()
}

// ===========================================================================
// TEST
// ===========================================================================

#[test]
fn test_rand_transaction_id() {
    println!("transaction id: {}", rand_transaction_id())
}