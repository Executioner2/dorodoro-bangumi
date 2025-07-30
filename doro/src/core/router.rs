pub mod ret;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use json_value::JsonValue;
use serde::Serialize;

use crate::router::ret::Ret;

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
pub trait HandlerTrait<T: Serialize + Send + Sync + 'static>: Send + Sync {
    async fn call(&self, json_value: Option<JsonValue>) -> Result<Ret<T>>;
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
impl<F, Fut, T> HandlerTrait<T> for NoneInputHandler<F, Fut>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Result<Ret<T>>> + Send + Sync,
    T: Serialize + Send + Sync + 'static,
{
    async fn call(&self, _: Option<JsonValue>) -> Result<Ret<T>> {
        Ok(handle_ret_after((self.f)().await))
    }
}

pub struct HasInputHandler<F, Fut> {
    pub f: F,
    pub _phantom: std::marker::PhantomData<Fut>,
}

#[async_trait]
impl<F, Fut, T> HandlerTrait<T> for HasInputHandler<F, Fut>
where
    F: Fn(JsonValue) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Ret<T>>> + Send + Sync,
    T: Serialize + Send + Sync + 'static,
{
    async fn call(&self, json_value: Option<JsonValue>) -> Result<Ret<T>> {
        let json_value = json_value.ok_or_else(|| anyhow!("Missing request params"))?;
        Ok(handle_ret_after((self.f)(json_value).await))
    }
}

#[derive(Default)]
pub struct Router {
    routes: RwLock<HashMap<u32, Arc<dyn RouteHandler>>>,
}

impl Router {
    pub fn global() -> &'static Self {
        static ROUTER: OnceLock<Router> = OnceLock::new();
        ROUTER.get_or_init(Router::default)
    }

    pub fn register_handler(&self, code: Code, handler: Arc<dyn RouteHandler>) {
        if self.routes.write().unwrap().insert(code, handler).is_some() {
            panic!("Route for code {code} already registered")
        }
    }

    pub async fn handle_request(&self, code: Code, body: Option<&[u8]>) -> Result<Vec<u8>> {
        let handler = self
            .routes
            .read()
            .unwrap()
            .get(&code)
            .ok_or_else(|| anyhow!("No handler for code {}", code))
            .cloned()?;
        handler.handle(body).await
    }
}

#[macro_export]
macro_rules! register_route {
    ($register_fn:ident, $code:expr, $handler:ident,true) => {
        #[ctor::ctor]
        fn $register_fn() {
            use $crate::core::router::{HandlerWrapper, HasInputHandler, Router};
            let wrapper = HandlerWrapper {
                handle: std::sync::Arc::new(HasInputHandler {
                    f: $handler,
                    _phantom: std::marker::PhantomData,
                }),
            };
            Router::global().register_handler($code, std::sync::Arc::new(wrapper))
        }
    };
    ($register_fn:ident, $code:expr, $handler:ident,false) => {
        #[ctor::ctor]
        fn $register_fn() {
            use $crate::core::router::{HandlerWrapper, NoneInputHandler, Router};
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

fn handle_ret_after<T: Serialize + Send + Sync + 'static>(ret: Result<Ret<T>>) -> Ret<T> {
    ret.unwrap_or_else(|e| Ret::default_err(e.to_string()))
}

pub async fn handle_request(code: Code, body: Option<&[u8]>) -> Result<Vec<u8>> {
    Router::global().handle_request(code, body).await
}
