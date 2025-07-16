use serde::{Deserialize, Serialize};
use tracing::info;
use crate::register_route;
use doro_macro::route;
use crate::client::common::Ret;

#[derive(Serialize, Deserialize, Debug)]
pub struct Task {
    /// 任务名
    pub task_name: Option<String>,

    /// 下载路径
    pub download_path: Option<String>,

    /// 种子文件的路径
    pub file_path: String,
}

#[route(code = 1001)]
pub async fn add_task(task: Task) -> Ret<bool> {
    info!("task: {:#?}", task);
    Ret::ok(true)
}