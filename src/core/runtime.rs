/// 这个标记实现者是一个持续运行时
pub trait Runnable {
    async fn run(self);
}