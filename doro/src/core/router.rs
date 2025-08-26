pub mod ret;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{OnceLock, RwLock};

use anyhow::{Result, anyhow};
use bytes::Bytes;
use doro_util::sync::{ReadLockExt, WriteLockExt};
use json_value::JsonValue;
use serde::Serialize;

use crate::router::ret::Ret;

pub type Code = u32;

/// 没有输入参数的函数
trait AsyncNoneInputFn: Send {
    fn call(&self) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>;
}

/// 有输入参数的函数
trait AsyncHasInputFn: Send {
    fn call(&self, input: JsonValue) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>;
}

struct NoneInputFnPtr<F>(F);

impl<F, Fut, T> AsyncNoneInputFn for NoneInputFnPtr<F>
where
    F: Fn() -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<Ret<T>>> + Send,
    T: serde::Serialize + Send + 'static,
{
    fn call(&self) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>> {
        let future = self.0.clone();
        Box::pin(async move {
            let result = future().await;
            Ok(serde_json::to_vec(&handle_ret_after(result))?)
        })
    }
}

struct HasInputFnPtr<F>(F);

impl<F, Fut, T> AsyncHasInputFn for HasInputFnPtr<F>
where
    F: Fn(JsonValue) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<Ret<T>>> + Send,
    T: serde::Serialize + Send + 'static,
{
    fn call(&self, input: JsonValue) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>> {
        let future = (self.0).clone();
        Box::pin(async move {
            let result = future(input).await;
            Ok(serde_json::to_vec(&handle_ret_after(result))?)
        })
    }
}

/// 函数指针
#[derive(Clone, Copy)]
enum FunctionPtr {
    /// 没有输入参数的函数裸指针
    NoneInput(*const dyn AsyncNoneInputFn),

    /// 有输入参数的函数裸指针
    HasInput(*const dyn AsyncHasInputFn),
}

unsafe impl Send for FunctionPtr {}
unsafe impl Sync for FunctionPtr {}

#[derive(Default)]
pub struct Router {
    routes: RwLock<HashMap<u32, FunctionPtr>>,
}

impl Router {
    pub fn global() -> &'static Self {
        static ROUTER: OnceLock<Router> = OnceLock::new();
        ROUTER.get_or_init(Router::default)
    }

    /// 注册没有输入的函数
    pub fn register_none_input_fn<F, Fut, T>(&self, code: Code, f: F)
    where
        F: Fn() -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Result<Ret<T>>> + Send,
        T: Serialize + Send + 'static,
    {
        let boxed = Box::new(NoneInputFnPtr(f));
        let ptr = Box::into_raw(boxed);
        self.routes
            .write_pe()
            .insert(code, FunctionPtr::NoneInput(ptr));
    }

    /// 注册有输入的函数
    pub fn register_has_input_fn<F, Fut, T>(&self, code: Code, f: F)
    where
        F: Fn(JsonValue) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Result<Ret<T>>> + Send,
        T: Serialize + Send + 'static,
    {
        let boxed = Box::new(HasInputFnPtr(f));
        let ptr = Box::into_raw(boxed);
        self.routes
            .write_pe()
            .insert(code, FunctionPtr::HasInput(ptr));
    }

    pub async fn handle_request(&self, code: Code, body: Option<Bytes>) -> Result<Vec<u8>> {
        let handler = self
            .routes
            .read_pe()
            .get(&code)
            .ok_or_else(|| anyhow!("No handler for code {}", code))
            .cloned()?;

        match handler {
            FunctionPtr::NoneInput(ptr) => {
                let handler = unsafe { &*ptr };
                handler.call().await
            }
            FunctionPtr::HasInput(ptr) => {
                let body_data = match body {
                    Some(b) => serde_json::from_slice(b.as_ref())?,
                    None => return Err(anyhow!("Missing request params")),
                };
                let handler = unsafe { &*ptr };
                handler.call(body_data).await
            }
        }
    }
}

#[macro_export]
macro_rules! register_route {
    ($register_fn:ident, $code:expr, $handler:ident,true) => {
        #[ctor::ctor]
        fn $register_fn() {
            use $crate::core::router::Router;
            Router::global().register_has_input_fn($code, $handler)
        }
    };
    ($register_fn:ident, $code:expr, $handler:ident,false) => {
        #[ctor::ctor]
        fn $register_fn() {
            use $crate::core::router::Router;
            Router::global().register_none_input_fn($code, $handler)
        }
    };
}

fn handle_ret_after<T: Serialize + 'static>(ret: Result<Ret<T>>) -> Ret<T> {
    ret.unwrap_or_else(|e| Ret::default_err(e.to_string()))
}

pub async fn handle_request(code: Code, body: Option<Bytes>) -> Result<Vec<u8>> {
    Router::global().handle_request(code, body).await
}
