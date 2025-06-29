use enum_dispatch::enum_dispatch;
use std::fmt::Display;
use std::marker::PhantomData;
use tracing::{info, Level};
use dorodoro_bangumi::default_logger;

default_logger!(Level::DEBUG);

#[enum_dispatch]
trait MyTrait<T: Display> {
    async fn my_method(&self, arg: T);
}

#[enum_dispatch(MyTrait<T>)] // fixme - 这里只能继续 MyTrait<T> 而不能 MyTrait<i32>
enum Enum<T: Display> {
    A(A),
    _Maker(_Maker<T>),
}

struct _Maker<T: Display>(PhantomData<T>);
impl<T: Display> MyTrait<T> for _Maker<T> {
    async fn my_method(&self, _arg: T) {
        unimplemented!()
    }
}

#[derive(Default)]
struct A;
impl<T: Display> MyTrait<T> for A {
    async fn my_method(&self, arg: T) {
        info!("A.my_method({})", arg);
    }
}

#[tokio::test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
async fn test_enum_dispatch() {
    let a: Enum<_> = A::default().into();
    a.my_method(123).await;
}
