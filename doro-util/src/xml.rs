use ahash::HashMap;
use quick_xml::Reader;
use quick_xml::events::Event;
use quick_xml::events::attributes::{AttrError, Attribute, Attributes};
use tracing::warn;

/// quick xml 的 attribute 扩展
pub trait AttributeExt {
    /// 检查属性是否包含某个值
    fn contains<N: AsRef<[u8]> + Sized>(&self, val: N) -> bool;
}

/// quick xml 的 attributes 扩展
pub trait AttributesExt {
    /// 尝试获取属性
    fn try_get_attribute<N: AsRef<[u8]> + Sized>(
        &mut self, attr_name: N,
    ) -> Result<Option<Attribute<'_>>, AttrError>;

    /// 转为 hashmap
    fn to_hashmap(&mut self) -> anyhow::Result<AttributesMap<'_>>;
}

/// quick xml 的 reader 扩展
pub trait ReaderExt {
    /// 获取下一个事件，遇到 Eof 事件时返回 None
    fn next_event(&mut self) -> anyhow::Result<Option<Event<'_>>>;
}

impl AttributeExt for Attribute<'_> {
    fn contains<N: AsRef<[u8]> + Sized>(&self, val: N) -> bool {
        let value = self.value.as_ref();
        let target = val.as_ref();

        if target.is_empty() {
            return false;
        }

        value
            .windows(target.len())
            .enumerate()
            .any(|(find_idx, window)| {
                if window != target {
                    return false;
                }

                let end = find_idx + target.len();
                (find_idx == 0 || value[find_idx - 1] == b' ')
                    && (end >= value.len() || value[end] == b' ')
            })
    }
}

impl<'a> AttributesExt for Attributes<'a> {
    fn try_get_attribute<N: AsRef<[u8]> + Sized>(
        &mut self, attr_name: N,
    ) -> Result<Option<Attribute<'a>>, AttrError> {
        for a in self.with_checks(false) {
            let a = a?;
            if a.key.as_ref() == attr_name.as_ref() {
                return Ok(Some(a));
            }
        }
        Ok(None)
    }

    fn to_hashmap(&mut self) -> anyhow::Result<AttributesMap<'_>> {
        let mut ret = HashMap::default();
        for a in self.with_checks(false) {
            let a = a?;
            let key = a.key.as_ref().to_vec();
            ret.insert(key, a.to_owned());
        }
        Ok(AttributesMap(ret))
    }
}

impl ReaderExt for Reader<&[u8]> {
    fn next_event(&mut self) -> anyhow::Result<Option<Event<'_>>> {
        match Reader::read_event(self)? {
            Event::Eof => Ok(None),
            event => Ok(Some(event)),
        }
    }
}

pub struct AttributesMap<'a>(HashMap<Vec<u8>, Attribute<'a>>);

impl<'a> AttributesMap<'a> {
    /// 尝试获取属性
    pub fn try_get_attribute<N: AsRef<[u8]> + Sized>(
        &mut self, attr_name: N,
    ) -> Option<Attribute<'a>> {
        self.0.get(attr_name.as_ref()).cloned()
    }

    /// 获取属性，如果不存在或者出现错误则返回 None
    pub fn get_attribute_value<N: AsRef<[u8]> + Sized>(&mut self, attr_name: N) -> Option<String> {
        let val = self.0.get(attr_name.as_ref())?;
        match val.unescape_value() {
            Ok(val) => Some(val.to_string()),
            Err(e) => {
                warn!("尝试编码属性值出现错误: {e}");
                None
            }
        }
    }
}

// 解码 HTML 实体函数
pub fn decode_html_entity(entity: &str) -> Option<String> {
    let mut ret = None;
    if entity.starts_with("#x") {
        // 处理十六进制实体（如 #x3010）
        let hex_code = &entity[2..entity.len()];
        if let Ok(code_point) = u32::from_str_radix(hex_code, 16) {
            if let Some(c) = std::char::from_u32(code_point) {
                ret = Some(c.to_string());
            }
        }
    } else if entity.starts_with("#") {
        // 处理十进制实体（如 #1234）
        let dec_code = &entity[1..entity.len()];
        if let Ok(code_point) = dec_code.parse::<u32>() {
            if let Some(c) = std::char::from_u32(code_point) {
                ret = Some(c.to_string());
            }
        }
    }
    ret
}
