//! bencoding 的单元测试
use crate::bencoding::{BEncode, BEncoder, BencodeItem, Decoder, ParseError, decode};
use bytes::Bytes;
use hashlink::LinkedHashMap;
use std::collections::HashMap;
use std::io::Read;

/// Helper function to read a file into a byte vector.
fn read_file(path: &str) -> Vec<u8> {
    let mut file = std::fs::File::open(path).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    buffer
}

#[test]
fn test_decode_str() {
    let mut decoder = Decoder::new(Bytes::from(&b"4:spam"[..]));
    let binding = decoder.decode().unwrap();
    let result = binding.as_str().unwrap();
    assert_eq!(result, "spam");
}

#[test]
fn test_decode_invalid_str() {
    let invalid_utf8: Vec<u8> = vec![0xF0, 0x28, 0x8C, 0xBC];
    let mut decoder = Decoder::new(Bytes::from(invalid_utf8));
    let result = decoder.decode();
    assert_eq!(result, Err(ParseError::InvalidByte(0)));
}

#[test]
fn test_decode_int() {
    let mut decoder = Decoder::new(Bytes::from(&b"i42e"[..]));
    let result = decoder.decode().unwrap().value;
    assert_eq!(result, BencodeItem::Int(42));
}

#[test]
fn test_decode_negative_int() {
    let mut decoder = Decoder::new(Bytes::from(&b"i-42e"[..]));
    let result = decoder.decode().unwrap().value;
    assert_eq!(result, BencodeItem::Int(-42));
}

#[test]
fn test_decode_invalid_int() {
    let mut decoder = Decoder::new(Bytes::from_owner(b"iae"));
    let result = decoder.decode();
    assert_eq!(result, Err(ParseError::InvalidByte(1)));
}

#[test]
fn test_decode_list() {
    let mut decoder = Decoder::new(Bytes::from_owner(b"l4:spam4:eggse"));
    let result = decoder.decode().unwrap().value;
    assert_eq!(
        result,
        BencodeItem::List(vec![
            BEncode::new(
                Bytes::from(&b"4:spam"[..]),
                BencodeItem::Str(Bytes::from(&b"spam"[..]))
            ),
            BEncode::new(
                Bytes::from(&b"4:eggs"[..]),
                BencodeItem::Str(Bytes::from(&b"eggs"[..]))
            ),
        ])
    );
}

#[test]
fn test_decode_dict() {
    let mut decoder = Decoder::new(Bytes::from_owner(b"d3:cow3:moo4:spam4:eggse"));
    let result = decoder.decode().unwrap().value;
    let mut expected_dict = HashMap::new();
    expected_dict.insert(
        "cow".to_string(),
        BEncode::new(
            Bytes::from(&b"3:moo"[..]),
            BencodeItem::Str(Bytes::from(&b"moo"[..])),
        ),
    );
    expected_dict.insert(
        "spam".to_string(),
        BEncode::new(
            Bytes::from(&b"4:eggs"[..]),
            BencodeItem::Str(Bytes::from(&b"eggs"[..])),
        ),
    );
    assert_eq!(result, BencodeItem::Dict(expected_dict));
}

#[test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
fn test_decode_torrent() {
    // Read the file into a byte vector
    let path = "tests/resources/test2.torrent";
    let torrent_stream = read_file(path);

    // Decode the torrent file (assuming you have a `encode` function handling Bencoding)
    let result = decode(Bytes::from(torrent_stream)).expect("Failed to encode");

    // Check for required keys in the top-level dictionary
    let required_keys = [
        "announce",
        "created by",
        "creation date",
        "encoding",
        "info",
    ];
    for key in required_keys {
        assert!(result.as_dict().unwrap().contains_key(key));
    }

    // Check for required keys in the "info" dictionary
    let info_dict = result
        .as_dict()
        .unwrap()
        .get("info")
        .unwrap()
        .as_dict()
        .unwrap();
    let required_keys = ["name", "piece length", "pieces"];
    for key in required_keys {
        assert!(info_dict.contains_key(key));
    }
}

#[test]
fn test_encode_str() {
    let binding = "spam".encode();
    assert_eq!(binding, Bytes::from(&b"4:spam"[..]));
}

#[test]
fn test_encode_int() {
    let int = 42.encode();
    assert_eq!(int, Bytes::from(&b"i42e"[..]))
}

#[test]
fn test_encode_list() {
    let list = vec!["spam", "eggs", "123"].encode();
    assert_eq!(list, Bytes::from_owner(b"l4:spam4:eggs3:123e"))
}

#[test]
fn test_encode_dict() {
    let mut dict = LinkedHashMap::new();
    dict.insert("cow".to_string(), "moo");
    dict.insert("spam".to_string(), "eggs");
    let dict = dict.encode();
    assert_eq!(dict, Bytes::from_owner(b"d3:cow3:moo4:spam4:eggse"))
}
