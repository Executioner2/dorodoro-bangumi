/// 测试是否能正常回收内存，因为有 println，需要添加 --nocapture 保证不会被兜住
#[test]
#[ignore]
fn test_drop_safe() {
    use super::ByteBuffer;
    use bytes::Bytes;
    
    for _ in 0..10000 {
        let buf: Bytes;
        {
            let mut bytes = ByteBuffer::new(1024);
            bytes.as_mut().iter_mut().for_each(|x| *x = 1);
            buf = bytes.take();
        }
        println!("{:?}", buf)
    }
}
