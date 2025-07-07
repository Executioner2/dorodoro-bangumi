use bincode::{config, de, enc};

/// 字节数组转无符号整数
macro_rules! impl_bytes2int {
    ($($t:ty),+) => {
        $(impl Bytes2Int<$t> for $t {
            /// 大端序：高位补零，数据放在右侧
            fn from_be_slice(data: &[u8]) -> Self {
                const SIZE: usize = std::mem::size_of::<$t>();
                let mut bytes = [0u8; SIZE];
                let len = data.len().min(SIZE);
                bytes[SIZE - len..].copy_from_slice(&data[..len]);
                Self::from_be_bytes(bytes)
            }

            /// 小端序：低位补零，数据放在左侧
            fn from_le_slice(data: &[u8]) -> Self {
                const SIZE: usize = std::mem::size_of::<$t>();
                let mut bytes = [0u8; SIZE];
                let len = data.len().min(SIZE);
                bytes[..len].copy_from_slice(&data[..len]);
                Self::from_le_bytes(bytes)
            }
        })+
    }
}

/// 字节数组转无符号整数
pub trait Bytes2Int<T> {
    /// 字节数组转无符号整数，大端字节序
    ///
    /// Examples:
    /// ```
    /// use dorodoro_bangumi::util::bytes_util::Bytes2Int;
    ///
    /// assert_eq!(u32::from_be_slice(&[0x12, 0x34, 0x56, 0x78]), 0x12345678);
    /// assert_eq!(u32::from_be_slice(&[0x56, 0x78]), 0x00005678); // 高位补零
    /// assert_eq!(u32::from_be_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A]), 0x12345678); // 截断超长数据
    ///
    /// assert_eq!(u32::from_be_slice(&[0x12, 0x34, 0x56, 0x78]), u32::from_be_bytes([0x12, 0x34, 0x56, 0x78]));
    /// assert_eq!(u32::from_be_slice(&[0x56, 0x78]), u32::from_be_bytes([0, 0, 0x56, 0x78]));
    /// assert_eq!(u32::from_be_slice(&[0x12, 0x34, 0x56, 0x78, 0x9A]), u32::from_be_bytes([0x12, 0x34, 0x56, 0x78]));
    ///
    /// assert_eq!(u32::from_be_slice(&[]), 0);
    /// assert_eq!(u32::from_be_slice(&[0xFF]), 0x000000FF);
    ///```
    fn from_be_slice(data: &[u8]) -> Self;

    /// 字节数组转无符号整数，小端字节序
    ///
    /// Examples:
    /// ```
    /// use dorodoro_bangumi::util::bytes_util::Bytes2Int;
    ///
    /// assert_eq!(u32::from_le_slice(&[0x78, 0x56, 0x34, 0x12]), 0x12345678);
    /// assert_eq!(u32::from_le_slice(&[0x78, 0x56]), 0x00005678); // 低位补零
    /// assert_eq!(u32::from_le_slice(&[0x9A, 0x78, 0x56, 0x34, 0x12]), 0x3456789A); // 截断超长数据
    ///
    /// assert_eq!(u32::from_le_slice(&[0x78, 0x56, 0x34, 0x12]), u32::from_le_bytes([0x78, 0x56, 0x34, 0x12]));
    /// assert_eq!(u32::from_le_slice(&[0x78, 0x56]), u32::from_le_bytes([0x78, 0x56, 0, 0]));
    /// assert_eq!(u32::from_le_slice(&[0x9A, 0x78, 0x56, 0x34, 0x12]), u32::from_le_bytes([0x9A, 0x78, 0x56, 0x34]));
    ///
    /// assert_eq!(u32::from_le_slice(&[]), 0);
    /// assert_eq!(u32::from_le_slice(&[0xFF]), 0x000000FF);
    ///```
    fn from_le_slice(data: &[u8]) -> Self;
}

impl_bytes2int!(u8, u16, u32, u64, u128);

/// 1 字节位图数组偏移计算
pub fn bitmap_offset<T: Into<usize> + Copy>(size: T) -> (usize, u8) {
    let index = size.into();
    (index >> 3, 1 << (7 - (index & 7) as u8))
}

/// 解码二进制数据
pub fn decode<D: de::Decode<()>>(src: &[u8]) -> D {
    bincode::decode_from_slice(src, config::standard()).unwrap().0
}

/// 编码二进制数据
pub fn encode<T: enc::Encode>(src: &T) -> Vec<u8> {
    bincode::encode_to_vec(src, config::standard()).unwrap()
}