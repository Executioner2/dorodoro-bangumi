use crate::default_logger;
use tracing::Level;

// 注意，这个注册了之后，所有的单元测试 mod 都会使用这个 logger，所以不要在测试 mod 里重复注册。
default_logger!(Level::TRACE);
