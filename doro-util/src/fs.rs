use std::fs::{File as StdFile, OpenOptions as StdOpenOptions};
use std::path::Path;
use tokio::fs;
use tokio::fs::{File, OpenOptions};

/// 为 OpenOptions 添加扩展方法
pub trait OpenOptionsExt {
    /// 打开文件，父目录不存在则创建
    fn open_with_parent_dirs<P: AsRef<Path>>(&self, path: P) -> std::io::Result<StdFile>;
}

/// 为 OpenOptions 添加扩展方法
pub trait AsyncOpenOptionsExt {
    /// 打开文件，父目录不存在则创建
    fn open_with_parent_dirs<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> impl Future<Output = std::io::Result<File>>;
}

impl AsyncOpenOptionsExt for OpenOptions {
    async fn open_with_parent_dirs<P: AsRef<Path>>(&self, path: P) -> std::io::Result<File> {
        let path = path.as_ref();

        // 获取父目录路径
        if let Some(parent) = path.parent() {
            // 若父目录不存在，则递归创建
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }

        // 使用配置打开文件
        self.open(path).await
    }
}

impl OpenOptionsExt for StdOpenOptions {
    fn open_with_parent_dirs<P: AsRef<Path>>(&self, path: P) -> std::io::Result<StdFile> {
        let path = path.as_ref();

        // 获取父目录路径
        if let Some(parent) = path.parent() {
            // 若父目录不存在，则递归创建
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        // 使用配置打开文件
        self.open(path)
    }
}
