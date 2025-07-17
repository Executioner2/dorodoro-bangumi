use crate::command_system;
use crate::core::control::{Dispatcher, TranId};
use anyhow::Result;
use bytes::Bytes;
use crate::router::Code;

command_system! {
    ctx: Dispatcher,
    Command {
        Request,
        Response,
    }
}

#[derive(Debug)]
pub struct Request {
    pub code: Code,
    pub tran_id: TranId,
    pub data: Option<Bytes>,
}
impl<'a> CommandHandler<'a, Result<()>> for Request {
    type Target = &'a mut Dispatcher;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.dispatch(self.code, self.tran_id, self.data);
        Ok(())
    }
}

#[derive(Debug)]
pub struct Response {
    pub data: Vec<u8>,
}
impl<'a> CommandHandler<'a, Result<()>> for Response {
    type Target = &'a mut Dispatcher;

    async fn handle(self, ctx: Self::Target) -> Result<()> {
        ctx.send(&self.data).await
    }
}