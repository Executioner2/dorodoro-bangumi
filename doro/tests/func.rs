use std::any::TypeId;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::Value as JsonValue;

trait AsyncNoneInputFn: Send {
    fn call(&self) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>;
}

trait AsyncHasInputFn: Send {
    fn call(&self, input: JsonValue) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>>;
}

struct NoneInputFnPtr<F>(F);

impl<F, Fut, T> AsyncNoneInputFn for NoneInputFnPtr<F>
where 
    F: Fn() -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<T>> + Send,
    T: serde::Serialize + Send, 
{

    fn call(&self) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>> {
        let future = self.0.clone();
        Box::pin(async move {
            let result = future().await?;
            Ok(serde_json::to_vec(&result)?)
        })
    }
}

struct HasInputFnPtr<F>(F);

impl<F, Fut, T> AsyncHasInputFn for HasInputFnPtr<F>
where 
    F: Fn(JsonValue) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<T>> + Send,
    T: serde::Serialize + Send
{

    fn call(&self, input: JsonValue) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send>> {
        let future = (self.0).clone();
        Box::pin(async move {
            let result = future(input).await?;
            Ok(serde_json::to_vec(&result)?)
        })
    }
}

// 函数存储条目
#[derive(Clone, Copy)]
enum Handler {
    NoneInput(*const dyn AsyncNoneInputFn),
    HasInput(*const dyn AsyncHasInputFn),
}

// 函数存储结构
#[derive(Default)]
pub struct FunctionStorage {
    functions: HashMap<TypeId, Handler>,
}

impl FunctionStorage {
    pub fn new() -> Self {
        Self::default()
    }

    // 存储无输入函数
    pub fn store_none_input<F, Fut, T>(&mut self, f: F) -> TypeId
    where
        F: Fn() -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Result<Ret<T>>> + Send,
        T: Serialize + Send,
    {
        let type_id = TypeId::of::<F>();
        let wrapper = Box::new(NoneInputFnPtr(f));
        let ptr = Box::into_raw(wrapper);
        self.functions.insert(type_id, Handler::NoneInput(ptr));
        type_id
    }

    // 存储有输入函数
    pub fn store_has_input<F, Fut, T>(&mut self, f: F) -> TypeId
    where
        F: Fn(JsonValue) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Result<Ret<T>>> + Send,
        T: Serialize + Send,
    {
        let type_id = TypeId::of::<F>();
        let wrapper = Box::new(HasInputFnPtr(f));
        let ptr = Box::into_raw(wrapper);
        self.functions.insert(type_id, Handler::HasInput(ptr));
        type_id
    }

    // 获取函数处理器
    fn get_handler(&self, type_id: TypeId) -> Option<Handler> {
        self.functions.get(&type_id).cloned()
    }

    // 删除函数
    pub fn remove_function(&mut self, type_id: TypeId) {
        if let Some(entry) = self.functions.remove(&type_id) {
            match entry {
                Handler::NoneInput(ptr) => {
                    // 安全地释放内存
                    unsafe {
                        let _ = Box::from_raw(ptr as *mut dyn AsyncNoneInputFn);
                    }
                }
                Handler::HasInput(ptr) => {
                    // 安全地释放内存
                    unsafe {
                        let _ = Box::from_raw(ptr as *mut dyn AsyncHasInputFn);
                    }
                }
            }
        }
    }
}

impl Drop for FunctionStorage {
    fn drop(&mut self) {
        // 移除所有函数以确保内存安全释放
        let keys: Vec<TypeId> = self.functions.keys().cloned().collect();
        for key in keys {
            self.remove_function(key);
        }
    }
}

// 实现 Send 和 Sync 确保线程安全
unsafe impl Send for Handler {}

impl Handler {
    // 消耗处理器执行调用
    pub async fn call(self, json_value: Option<JsonValue>) -> Result<Vec<u8>> {
        match self {
            Handler::NoneInput(ptr) => {
                let handler = unsafe { &*ptr };
                handler.call().await
            }
            Handler::HasInput(ptr) => {
                let handler = unsafe { &*ptr };
                let json_value = json_value.ok_or_else(|| anyhow!("Input is missing"))?;
                handler.call(json_value).await
            }
        }
    }
}

// 擦除后的返回类型
#[derive(Serialize)]
pub struct RetErased<T: Serialize> {
    pub code: i32,
    pub msg: String,
    pub data: Option<T>,
}

// 通用返回类型
#[derive(Serialize)]
pub struct Ret<T: Serialize> {
    pub code: i32,
    pub msg: String,
    pub data: Option<T>,
}

// 示例使用
#[tokio::test]
async fn test_func_call() {
    // 创建函数存储
    let mut storage = FunctionStorage::new();

    // 创建并存储无输入函数
    let none_input_id = storage.store_none_input(|| async {
        Ok(Ret {
            code: 0,
            msg: "Success".to_string(),
            data: Some("Hello".to_string()),
        })
    });

    // 创建并存储有输入函数
    let has_input_id = storage.store_has_input(|input: JsonValue| async move {
        let value: i32 = serde_json::from_value(input).map_err(|e| anyhow!(e))?;
        Ok(Ret {
            code: 0,
            msg: "Success".to_string(),
            data: Some(value * 2),
        })
    });

    // 从存储中获取处理器
    let handler1 = storage.get_handler(none_input_id).unwrap();
    let handler2 = storage.get_handler(has_input_id).unwrap();

    let result2 = tokio::spawn(async move {
        handler2.call(Some(serde_json::json!(5))).await.unwrap()
    });

    // 调用处理器
    let result1 = handler1.call(None).await.unwrap();
    let result2 = result2.await.unwrap();

    assert_eq!(
        String::from_utf8(result1).unwrap(),
        r#"{"code":0,"msg":"Success","data":"Hello"}"#
    );
    assert_eq!(
        String::from_utf8(result2).unwrap(),
        r#"{"code":0,"msg":"Success","data":10}"#
    );

    // 清理函数（释放不了的，因为本身就是函数地址了）
    storage.remove_function(none_input_id);
    storage.remove_function(has_input_id);
}