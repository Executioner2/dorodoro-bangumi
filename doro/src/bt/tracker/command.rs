use doro_util::command_system;
use crate::tracker::Tracker;
use super::Result;

command_system! {
    ctx: Tracker,
    Command {
        Exit,
        _Other,
    }
}

/// 退出 tacker。什么都不用做，调用方会退出 loop。
#[derive(Debug)]
pub struct Exit;
impl<'a> CommandHandler<'a, Result<()>> for Exit {
    type Target = &'a mut Tracker;

    async fn handle(self, _ctx: Self::Target) -> Result<()> {
        unimplemented!()
    }
}

/// 这个没什么用，只是去掉黄线警告。
#[derive(Debug)]
pub struct _Other;
impl<'a> CommandHandler<'a, Result<()>> for _Other {
    type Target = &'a mut Tracker;

    async fn handle(self, _ctx: Self::Target) -> Result<()> {
        unimplemented!()
    }
}