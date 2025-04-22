/// 这个标记实现者是一个持续运行时
pub trait Runnable {
    fn run(self) -> impl Future<Output = ()> + Send;
}
