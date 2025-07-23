use core::fmt::{Display, Formatter};
use thiserror::Error;

#[derive(Eq, PartialEq, Debug, Copy, Clone)]
pub enum ErrorType {
    /// 普通异常
    Exception,

    /// 致命错误
    DeadlyError,
}

impl Display for ErrorType {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            ErrorType::Exception => write!(f, "Exception"),
            ErrorType::DeadlyError => write!(f, "DeadlyError"),
        }
    }
}

#[derive(Error, Debug)]
pub struct ErrorReason {
    error_type: ErrorType,

    #[source]
    source: anyhow::Error,
}

impl Display for ErrorReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.error_type, self.source)
    }
}

impl ErrorReason {
    pub fn new(error_type: ErrorType, source: anyhow::Error) -> Self {
        Self { error_type, source }
    }

    pub fn deadly_error(source: anyhow::Error) -> Self {
        Self::new(ErrorType::DeadlyError, source)
    }

    pub fn exception(source: anyhow::Error) -> Self {
        Self::new(ErrorType::Exception, source)
    }

    pub fn error_type(&self) -> ErrorType {
        self.error_type
    }

    pub fn source(&self) -> &anyhow::Error {
        &self.source
    }
}

#[derive(Error, Debug)]
pub enum PeerExitReason {
    /// 正常退出
    #[error("Normal")]
    Normal,

    /// 客户端主动退出
    #[error("ClientExit")]
    ClientExit,

    /// 没有任务的退出
    #[error("NotHasJob")]
    NotHasJob,

    /// 周期性的临时 peer 替换
    #[error("PeriodicPeerReplace")]
    PeriodicPeerReplace,

    /// 异常退出
    #[error("{0}")]
    Exception(ErrorReason),

    /// 下载任务完成
    #[error("DownloadFinished")]
    DownloadFinished,
}

pub fn deadly_error(source: anyhow::Error) -> PeerExitReason {
    PeerExitReason::Exception(ErrorReason::new(ErrorType::DeadlyError, source))
}

pub fn exception(source: anyhow::Error) -> PeerExitReason {
    PeerExitReason::Exception(ErrorReason::new(ErrorType::Exception, source))
}