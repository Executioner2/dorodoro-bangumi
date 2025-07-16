#[cfg(test)]
mod tests;

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};
use async_trait::async_trait;
use json_value::JsonValue;

pub type Code = u32;

#[async_trait]
pub trait RouteHandler: Send + Sync {
    async fn handle(&self, body: Option<&[u8]>) -> Result<Vec<u8>>;
}

pub enum HandlerFnType<F1, F2> {
    OnlyContext(F1),
    ContextAndBody(F2),
}

#[async_trait]
pub trait HandlerTrait<Ret>: Send + Sync {
    async fn call(&self, json_value: Option<JsonValue>) -> Result<Ret>;
}

pub struct HandlerWrapper<Ret> {
    pub handle: Arc<dyn HandlerTrait<Ret> + Send + Sync>,
}

#[async_trait]
impl<Ret> RouteHandler for HandlerWrapper<Ret>
where
    Ret: Serialize + 'static + Send + Sync,
{
    async fn handle(&self, body: Option<&[u8]>) -> Result<Vec<u8>> {
        let body_data = match body {
            Some(b) => Some(serde_json::from_slice(b)?),
            None => None,
        };

        let ret = self.handle.call(body_data).await?;
        Ok(serde_json::to_vec(&ret)?)
    }
}

pub struct NoneInputHandler<F, Fut> {
    pub f: F,
    pub _phantom: std::marker::PhantomData<Fut>,
}

#[async_trait]
impl<F, Fut, Ret> HandlerTrait<Ret> for NoneInputHandler<F, Fut>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Ret> + Send + Sync,
    Ret: Serialize + Send + Sync + 'static,
{
    async fn call(&self, _: Option<JsonValue>) -> Result<Ret> {
        Ok((self.f)().await)
    }
}

pub struct HasInputHandler<F, Fut> {
    pub f: F,
    pub _phantom: std::marker::PhantomData<Fut>,
}

#[async_trait]
impl<F, Fut, Ret> HandlerTrait<Ret> for HasInputHandler<F, Fut>
where
    F: Fn(JsonValue) -> Fut + Send + Sync,
    Fut: Future<Output = Ret> + Send + Sync,
    Ret: Serialize + Send + Sync + 'static,
{
    async fn call(&self, json_value: Option<JsonValue>) -> Result<Ret> {
        let json_value = json_value.ok_or_else(|| anyhow!("Missing request params"))?;
        Ok((self.f)(json_value).await)
    }
}

#[derive(Default)]
pub struct Router {
    routes: RwLock<HashMap<u32, Arc<dyn RouteHandler>>>
}

impl Router {
    pub fn global() -> &'static Self {
        static ROUTER: OnceLock<Router> = OnceLock::new();
        ROUTER.get_or_init(Router::default)
    }

    pub fn register_handler(&self, code: Code, handler: Arc<dyn RouteHandler>) {
        if self.routes.write().unwrap().insert(code, handler).is_some() {
            panic!("Route for code {} already registered", code)
        }
    }

    pub async fn handle_request(&self, code: Code, body: Option<&[u8]>) -> Result<Vec<u8>> {
        let handler = self.routes.read().unwrap()
            .get(&code)
            .ok_or_else(|| anyhow!("No handler for code {}", code))
            .cloned()?;
        handler.handle(body).await
    }
}

#[macro_export]
macro_rules! register_route {
    ($register_fn:ident, $code:expr, $handler:ident, true) => {
        #[ctor::ctor]
        fn $register_fn() {
            use crate::core::router::{HandlerWrapper, Router, HasInputHandler};
            let wrapper = HandlerWrapper {
                handle: std::sync::Arc::new(HasInputHandler {
                    f: $handler,
                    _phantom: std::marker::PhantomData,
                }),
            };
            Router::global().register_handler($code, std::sync::Arc::new(wrapper))
        }
    };
    ($register_fn:ident, $code:expr, $handler:ident, false) => {
        #[ctor::ctor]
        fn $register_fn() {
            use crate::core::router::{HandlerWrapper, Router, NoneInputHandler};
            let wrapper = HandlerWrapper {
                handle: std::sync::Arc::new(NoneInputHandler {
                    f: $handler,
                    _phantom: std::marker::PhantomData,
                }),
            };
            Router::global().register_handler($code, std::sync::Arc::new(wrapper))
        }
    };
}

pub async fn handle_request(code: Code, body: Option<&[u8]>) -> Result<Vec<u8>> {
    Router::global().handle_request(code, body).await
}