use anyhow::Result;
use doro_macro::route;

use crate::entity::{Group, Quarter, Resource, Source};
use crate::router::ret::Ret;
use crate::{bangumi_service, register_route};

// ===========================================================================
// API
// ===========================================================================

/// 获取番剧列表
#[route(code = 1201)]
pub async fn list_bangumi(
    #[param] crawler: String, quarter: Option<Quarter>,
) -> Result<Ret<Vec<Resource>>> {
    let ret = bangumi_service::list_bangumi(crawler, quarter).await?;
    Ok(Ret::ok(ret))
}

/// 获取季度列表
#[route(code = 1202)]
pub async fn list_quarter(#[param] crawler: String) -> Result<Ret<Vec<Quarter>>> {
    let ret = bangumi_service::list_quarter(crawler).await?;
    Ok(Ret::ok(ret))
}

/// 获取资源分组列表
#[route(code = 1203)]
pub async fn list_source_group(
    #[param] crawler: String, #[param] resource_id: String,
) -> Result<Ret<Vec<Group>>> {
    let ret = bangumi_service::list_source_group(crawler, resource_id).await?;
    Ok(Ret::ok(ret))
}

/// 获取订阅源列表
#[route(code = 1204)]
pub async fn list_subscribe_sources(
    #[param] crawler: String, #[param] resource_id: String,
) -> Result<Ret<Vec<Vec<Source>>>> {
    let ret = bangumi_service::list_subscribe_sources(crawler, resource_id).await?;
    Ok(Ret::ok(ret))
}

/// 搜索资源
#[route(code = 1205)]
pub async fn search_resource(
    #[param] crawler: String, #[param] keyword: String,
) -> Result<Ret<(Vec<Resource>, Vec<Source>)>> {
    let ret = bangumi_service::search_resource(crawler, keyword).await?;
    Ok(Ret::ok(ret))
}
