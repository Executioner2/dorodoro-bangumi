/// 命令处理器
pub trait CommandHandler<'a, Return = ()> {
    type Target;

    fn handle(self, ctx: Self::Target) -> impl Future<Output = Return> + Send;
}
