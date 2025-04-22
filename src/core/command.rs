/// 命令处理器
pub trait CommandHandler {
    type Target;

    fn handle(self, context: Self::Target) -> impl Future<Output = ()> + Send;
}
