/// 命令处理器
pub trait CommandHandler<'a, Return = ()> {
    type Target;

    #[allow(async_fn_in_trait)]
    async fn handle(self, ctx: Self::Target) -> Return;
}
