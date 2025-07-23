//! api module   
//! 一些口头上的约定：
//! 1. 1000 以内的 code 为保留 code，不能在正式代码中使用。
//! 2. 每个 api 的 code 使用范围在 100 以内。
//! 3. 每个 controoler 的第 100 位 code 为保留位。     
//!     即 1100 是保留位，1101 - 1199 是这个 api 的可用范围。

pub mod task_api;
pub mod rss_api; 