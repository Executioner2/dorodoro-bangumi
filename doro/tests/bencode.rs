use bendy::decoding::{Decoder, Error, FromBencode, Object};
use doro_util::default_logger;
use tracing::{info, Level};

default_logger!(Level::INFO);

// 定义混合数据结构
#[derive(Debug, PartialEq)]
struct MixedData {
    msg_type: u32,
    piece: u32,
    total_size: u32,
    binary_data: Vec<u8>,
}

impl FromBencode for MixedData {
    const EXPECTED_RECURSION_DEPTH: usize = 5;

    fn decode_bencode_object(object: Object) -> Result<Self, Error> {
        // 1. 将Object转换为字典
        let mut dict = match object {
            Object::Dict(d) => d,
            _ => return Err(Error::missing_field("dict")),
        };

        // 2. 准备存储解析值的变量
        let mut msg_type = None;
        let mut piece = None;
        let mut total_size = None;

        // 3. 遍历字典键值对
        while let Some((key, value)) = dict.next_pair()? {
            match key {
                b"msg_type" => {
                    msg_type = Some(i64::decode_bencode_object(value)? as u32);
                }
                b"piece" => {
                    piece = Some(i64::decode_bencode_object(value)? as u32);
                }
                b"total_size" => {
                    total_size = Some(i64::decode_bencode_object(value)? as u32);
                }
                // 忽略未知键
                _ => {}
            }
        }

        // 4. 确保所有必需字段都存在
        let msg_type = msg_type.ok_or(Error::missing_field("msg_type"))?;
        let piece = piece.ok_or(Error::missing_field("piece"))?;
        let total_size = total_size.ok_or(Error::missing_field("total_size"))?;

        // 5. 注意：此时还没有处理二进制数据部分
        // 我们需要在外部处理二进制数据

        Ok(MixedData {
            msg_type,
            piece,
            total_size,
            binary_data: Vec::new(), // 将在外部填充
        })
    }
}

fn parse_mixed_data(data: &[u8]) -> Result<MixedData, Error> {
    // 1. 创建解码器
    let mut decoder = Decoder::new(data);

    // 2. 解析第一个对象（应该是字典）
    let object = match decoder.next_object()? {
        Some(obj) => obj,
        None => return Err(Error::unexpected_token("dict", "dict")),
    };

    // 3. 解析B编码部分
    let mut mixed_data = MixedData::decode_bencode_object(object)?;

    // 4. 获取解码器当前位置（B编码结束位置）
    mixed_data.binary_data = decoder.take_remaining();

    Ok(mixed_data)
}

#[test]
fn test_parse_mixed_data() -> Result<(), Box<dyn std::error::Error>> {
    // 示例数据：B编码字典 + 二进制数据
    let data = b"d8:msg_typei1e5:piecei0e10:total_sizei3425ee\x01\x02\x03\x04\x05\x06\x07\x08";

    // 解析混合数据
    let mixed_data = parse_mixed_data(data).unwrap();

    info!("Parsed data: {mixed_data:#?}");

    // 验证结果
    assert_eq!(mixed_data.msg_type, 1);
    assert_eq!(mixed_data.piece, 0);
    assert_eq!(mixed_data.total_size, 3425);
    assert_eq!(mixed_data.binary_data, vec![1, 2, 3, 4, 5, 6, 7, 8]);

    Ok(())
}
