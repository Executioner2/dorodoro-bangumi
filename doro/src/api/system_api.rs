use anyhow::Result;
use doro_macro::route;

use crate::config::{ClientAuth, ConfigInner};
use crate::{register_route, system_service};
use crate::router::ret::Ret;

/// 还原默认设置
#[route(code = 1301)]
async fn reset_default_config() -> Result<Ret<ConfigInner>> {
    let ret = system_service::reset_default_config().await?;
    Ok(Ret::ok(ret))
}

/// 获取系统配置
#[route(code = 1302)]
async fn get_config() -> Result<Ret<ConfigInner>> {
    let ret = system_service::get_config()?;
    Ok(Ret::ok(ret))
}

/// 更新系统配置
#[route(code = 1303)]
async fn update_config(config: ConfigInner) -> Result<Ret<bool>> {
    let ret = system_service::update_config(config).await?;
    Ok(Ret::ok(ret))
}

/// 更新客户端认证信息
#[route(code = 1304)]
async fn update_auth(auth: ClientAuth) -> Result<Ret<bool>> {
    let ret = system_service::update_auth(auth).await?;
    Ok(Ret::ok(ret))
}