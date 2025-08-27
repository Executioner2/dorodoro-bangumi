use anyhow::Result;
use doro_util::sync::ReadLockExt;

use crate::config::{ClientAuth, ConfigInner};
use crate::context::Context;
use crate::mapper::context::{ContextEntity, ContextMapper};

/// 还原默认设置
pub async fn reset_default_config() -> Result<ConfigInner> {
    let config = ConfigInner::default();
    update_config(config.clone()).await?;
    Ok(config)
}

/// 获取全局配置
pub fn get_config() -> Result<ConfigInner> {
    Ok(Context::get_config().inner().read_pe().clone())
}

/// 更新全局配置
pub async fn update_config(config: ConfigInner) -> Result<bool> {
    Context::set_config(config);
    let conn = Context::get_conn().await?;
    let ce = ContextEntity {
        config: Some(Context::get_config().clone()),
        ..Default::default()
    };
    Ok(conn.store_context(ce)? == 1)
}

/// 更新用户认证信息
pub async fn update_auth(auth: ClientAuth) -> Result<bool> {
    Context::update_auth(auth);
    let conn = Context::get_conn().await?;
    let ce = ContextEntity {
        config: Some(Context::get_config().clone()),
        ..Default::default()
    };
    Ok(conn.store_context(ce)? == 1)
}
