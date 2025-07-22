use super::*;
use crate::core::emitter::transfer::TransferPtr;
use std::{mem, thread};
use tokio::sync::mpsc::{Receiver, channel};
use tracing::info;

trait CommandHandler {
    fn handle(self);
}

#[derive(Eq, PartialEq, Debug)]
struct A(i64, i8, bool);

#[repr(u8)]
#[derive(Eq, PartialEq, Debug)]
enum MyOption<T> {
    None,
    Some(T),
}

#[derive(Eq, PartialEq, Debug)]
#[repr(u8)]
enum Scheduler {
    Shoutown(Shoutown),
    AddTorrent(AddTorrent),
    Ok,
    StackData(StackData),
}

impl CommandHandler for Scheduler {
    fn handle(self) {
        match self {
            Scheduler::Shoutown(cmd) => cmd.handle(),
            Scheduler::AddTorrent(cmd) => cmd.handle(),
            Scheduler::StackData(cmd) => cmd.handle(),
            _ => (),
        }
    }
}

#[derive(Eq, PartialEq, Debug)]
struct Shoutown;
impl From<Shoutown> for Scheduler {
    fn from(value: Shoutown) -> Self {
        Scheduler::Shoutown(value)
    }
}
impl CommandHandler for Shoutown {
    fn handle(self) {
        info!("执行关机指令");
    }
}
impl Drop for Shoutown {
    fn drop(&mut self) {
        info!("shoutown drop 了");
    }
}

#[derive(Eq, PartialEq, Debug)]
struct AddTorrent(&'static str);
impl From<AddTorrent> for Scheduler {
    fn from(value: AddTorrent) -> Self {
        Scheduler::AddTorrent(value)
    }
}
impl CommandHandler for AddTorrent {
    fn handle(self) {
        info!("执行添加种子的指令，携带的参数: {}", self.0);
    }
}
impl Drop for AddTorrent {
    fn drop(&mut self) {
        info!("add torrent drop 了");
    }
}

#[derive(Eq, PartialEq, Debug)]
struct StackData(A);
impl From<StackData> for Scheduler {
    fn from(value: StackData) -> Self {
        Scheduler::StackData(value)
    }
}
impl CommandHandler for StackData {
    fn handle(self) {
        info!("执行栈上数据处理: {:?}", self.0);
    }
}
impl Drop for StackData {
    fn drop(&mut self) {
        info!("stack data drop 了");
    }
}

#[repr(u8)]
#[derive(Eq, PartialEq, Debug)]
enum PeerManager {
    NewDonwload(NewDownload),
}
impl CommandHandler for PeerManager {
    fn handle(self) {
        match self {
            PeerManager::NewDonwload(cmd) => cmd.handle(),
        }
    }
}

#[derive(Eq, PartialEq, Debug)]
struct NewDownload;
impl From<NewDownload> for PeerManager {
    fn from(value: NewDownload) -> Self {
        PeerManager::NewDonwload(value)
    }
}
impl CommandHandler for NewDownload {
    fn handle(self) {
        info!("这里执行了 new download");
    }
}
impl Drop for NewDownload {
    fn drop(&mut self) {
        info!("new download drop 了");
    }
}

/// 测试从标准库的 Option 中取的 enum tag。
#[test]
#[should_panic] // 标准库的没有按 1 字节对齐，会被空指针优化
fn test_get_enum_tag_from_std_option() {
    let data = Some(A(1, 2, false));
    let ptr = &data as *const Option<A> as *const u8;
    let x = unsafe { ptr.read() };
    assert_eq!(x, 1u8);
    let ptr = ptr as *const Option<A>;
    let x = unsafe { ptr.read() };
    assert_eq!(x, data);

    let data = Some(Box::new(A(1, 2, false)));
    let ptr = &data as *const Option<Box<A>> as *const u8;
    let x = unsafe { ptr.read() };
    assert_eq!(x, 1u8);
    let ptr = ptr as *const Option<Box<A>>;
    let x = unsafe { ptr.read() };
    assert_eq!(x, data);
    mem::forget(x);
}

/// 测试从裸指针中获取枚举的 tag
///
/// 这个测试用例的在读取数据的操作上有些许问题，会导致 ub。但是这个测试的目的仅仅
/// 是验证读取 enum 类型的内存中第一个字节（enum tag），来区分枚举是否可行。（
/// 虽然在实际开发中，不需要我们去读取第一个字节，只需要把这个指针转换为对应枚举类
/// 型的指针，就像 [`scheduler_loop`] 和 [`peer_manager_loop`] 那样。）
#[test]
#[ignore]
fn test_get_enum_tag_from_unsafe_ptr() {
    let shoutown = Scheduler::Shoutown(Shoutown);
    let ptr = &shoutown as *const Scheduler as *const (); // 模拟我们 channel 的类型定义
    let ptr = ptr as *const u8;
    let x = unsafe { ptr.read() };
    assert_eq!(x, 0u8);
    let ptr = ptr as *const Scheduler;
    let x = unsafe { ptr.read() };
    assert_eq!(x, shoutown);

    let add_torrent = Scheduler::AddTorrent(AddTorrent("./test1.torrent"));
    let ptr = &add_torrent as *const Scheduler as *const ();
    let ptr = ptr as *const u8;
    let x = unsafe { ptr.read() };
    assert_eq!(x, 1u8);
    let ptr = ptr as *const Scheduler;
    let x = unsafe { ptr.read() };
    assert_eq!(x, add_torrent);
    mem::forget(x); // 因为字符串是堆分配，所以需要 forget 下

    let ok = Scheduler::Ok;
    let ptr = &ok as *const Scheduler as *const ();
    let ptr = ptr as *const u8;
    let x = unsafe { ptr.read() };
    assert_eq!(x, 2u8);
    let ptr = ptr as *const Scheduler;
    let x = unsafe { ptr.read() };
    assert_eq!(x, ok);

    let data = MyOption::Some(Box::new(A(1, 2, false)));
    let ptr = &data as *const MyOption<Box<A>>;
    let ptr = ptr as *const u8;
    let x = unsafe { ptr.read() };
    assert_eq!(x, 1u8); // Some 在 Option1 中是第二个
    // let ptr = ptr as *const MyOption<Box<A>>;
    // let x = unsafe { ptr.read() };
    // assert_eq!(x, data);
    // mem::forget(x); // 虽然加了这个可以过普通的单元测试，但还是过不了 miri 测试

    let data = MyOption::Some(A(1, 2, false));
    let ptr = &data as *const MyOption<A>;
    let ptr = ptr as *const u8;
    let x = unsafe { ptr.read() };
    assert_eq!(x, 1u8); // Some 在 Option1 中是第二个
    let ptr = ptr as *const MyOption<A>;
    let x = unsafe { ptr.read() };
    assert_eq!(x, data);

    let data = MyOption::None;
    let ptr = &data as *const MyOption<A>;
    let ptr = ptr as *const u8;
    let x = unsafe { ptr.read() };
    assert_eq!(x, 0u8); // None 在 Option 中是第一个
    let ptr = ptr as *const MyOption<A>;
    let x = unsafe { ptr.read() };
    assert_eq!(x, MyOption::None);
}

/// 测试在多任务中传输裸指针
#[tokio::test]
#[cfg_attr(miri, ignore)] // miri 不支持的操作，忽略掉
async fn test_transfer_unsafe_ptr() {
    let (send1, recv1) = channel(100);
    let (send2, recv2) = channel(100);

    let emitter = Emitter::global();
    emitter.register("scheduler", send1);
    emitter.register("task_handler", send2);

    let t1 = tokio::spawn(scheduler_loop(recv1));
    let t2 = tokio::spawn(peer_manager_loop(recv2));

    // 发送添加种子信号
    {
        let data: Scheduler = AddTorrent("./resource/test_channel.torrent").into();
        let cmd = Box::into_raw(Box::new(data)) as *const ();
        emitter
            .send("scheduler", TransferPtr::new(cmd))
            .await
            .unwrap();
    } // 测试是否会被提前释放，导致 ub

    // 发送下载信号
    {
        let data: PeerManager = NewDownload.into();
        let cmd = Box::into_raw(Box::new(data)) as *const ();
        emitter
            .send("task_handler", TransferPtr::new(cmd))
            .await
            .unwrap();
    }

    // 发送关机信号
    {
        let data: Scheduler = Shoutown.into();
        let cmd = Box::into_raw(Box::new(data)) as *const ();
        emitter
            .send("scheduler", TransferPtr::new(cmd))
            .await
            .unwrap();
    }

    t1.await.unwrap();
    t2.await.unwrap();
}

async fn scheduler_loop(mut recv: Receiver<TransferPtr>) {
    while let Some(data) = recv.recv().await {
        let data = unsafe { Box::from_raw(data.inner as *mut Scheduler) };
        data.handle();
    }
}

async fn peer_manager_loop(mut recv: Receiver<TransferPtr>) {
    while let Some(data) = recv.recv().await {
        let data = unsafe { Box::from_raw(data.inner as *mut PeerManager) };
        data.handle();
    }
}

/// 测试尝试发送一个栈上的数据（不出意外应该会存在一些意外）
///
/// 结果：miri 测试出现 ub 了！！！
#[test]
#[ignore]
#[cfg_attr(miri, ignore)]
fn test_send_stack_data() {
    let (send1, recv1) = std::sync::mpsc::channel();

    let t1 = thread::spawn(move || scheduler_loop_handle_on_stack(recv1));

    {
        let data: Scheduler = AddTorrent("./resource/test_channel.torrent").into();
        let cmd = &data as *const Scheduler as *const ();
        send1.send(TransferPtr::new(cmd)).unwrap();
        // mem::forget(data);
    }

    {
        let data: Scheduler = StackData(A(0, 1, false)).into();
        let cmd = &data as *const Scheduler as *const ();
        send1.send(TransferPtr::new(cmd)).unwrap();
        // mem::forget(data);
    }

    // 发完了，关闭发送端
    drop(send1);

    t1.join().unwrap();
}

fn scheduler_loop_handle_on_stack(recv: std::sync::mpsc::Receiver<TransferPtr>) {
    while let Ok(data) = recv.recv() {
        let data = unsafe { (data.inner as *mut Scheduler).read() };
        data.handle();
    }
}

/// 使用 miri 测试在多任务中传输裸指针
///
/// 结果：没有问题
#[test]
fn test_transfer_unsafe_ptr_miri() {
    let (send1, recv1) = std::sync::mpsc::channel();
    let (send2, recv2) = std::sync::mpsc::channel();

    let t1 = thread::spawn(move || scheduler_loop_miri(recv1));
    let t2 = thread::spawn(move || peer_manager_loop_miri(recv2));

    // 发送添加种子信号
    {
        let data: Scheduler = AddTorrent("./resource/test_channel.torrent").into();
        let cmd = Box::into_raw(Box::new(data)) as *const ();
        send1.send(TransferPtr::new(cmd)).unwrap();
    } // 测试是否会被提前释放，导致 ub

    // 发送下载信号
    {
        let data: PeerManager = NewDownload.into();
        let cmd = Box::into_raw(Box::new(data)) as *const ();
        send2.send(TransferPtr::new(cmd)).unwrap();
    }

    // 发送关机信号
    {
        let data: Scheduler = Shoutown.into();
        let cmd = Box::into_raw(Box::new(data)) as *const ();
        send1.send(TransferPtr::new(cmd)).unwrap();
    }

    // 发完了，关闭发送端
    drop(send1);
    drop(send2);

    t1.join().unwrap();
    t2.join().unwrap();
}

fn scheduler_loop_miri(recv: std::sync::mpsc::Receiver<TransferPtr>) {
    while let Ok(data) = recv.recv() {
        let data = unsafe { Box::from_raw(data.inner as *mut Scheduler) };
        data.handle();
    }
}

fn peer_manager_loop_miri(recv: std::sync::mpsc::Receiver<TransferPtr>) {
    while let Ok(data) = recv.recv() {
        let data = unsafe { Box::from_raw(data.inner as *mut PeerManager) };
        data.handle();
    }
}
