use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Ret<T: Serialize> {
    /// 业务逻辑是否成功
    pub status_code: i32,

    /// 附带消息
    pub msg: String,

    /// 响应数据
    pub data: Option<T>,
}

impl<T: Serialize> Ret<T> {
    #[allow(dead_code)]
    pub fn default_err(msg: String) -> Self {
        Self::err(-1, msg)
    }

    #[allow(dead_code)]
    pub fn default_ok() -> Self {
        Self {
            status_code: 0,
            msg: "success".to_string(),
            data: None,
        }
    }

    #[allow(dead_code)]
    pub fn ok(data: T) -> Self {
        Self {
            status_code: 0,
            msg: "success".to_string(),
            data: Some(data),
        }
    }

    #[allow(dead_code)]
    pub fn err(status_code: i32, msg: String) -> Self {
        Self {
            status_code,
            msg,
            data: None,
        }
    }
}