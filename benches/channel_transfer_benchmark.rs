//! 通道不同传输方式的测试
//!
//! 测试对象：
//!  - enum transfer                直接使用枚举传输
//!  - unsafe ptr transfer          传输裸指针
//!  - dynamic dispatch transfer    动态分发
//!
//! 测试结果：
//!     Finished `bench` profile [optimized] target(s) in 0.10s
//!     Running benches/channel_transfer_benchmark.rs (target/release/deps/channel_transfer_benchmark-18302fcba3578faf)
//!     Gnuplot not found, using plotters backend
//!     enum transfer           time:   [7.6708 ms 7.7135 ms 7.7592 ms]
//!     change: [-2.2676% -1.5632% -0.9107%] (p = 0.00 < 0.05)
//!     Change within noise threshold.
//!     Found 2 outliers among 100 measurements (2.00%)
//!     2 (2.00%) high mild
//!     
//!     unsafe ptr transfer     time:   [7.8800 ms 7.9106 ms 7.9427 ms]
//!     change: [-3.6556% -3.1498% -2.6494%] (p = 0.00 < 0.05)
//!     Performance has improved.
//!     Found 4 outliers among 100 measurements (4.00%)
//!     4 (4.00%) high mild
//!     
//!     dynamic dispatch transfer
//!     time:   [8.0531 ms 8.0833 ms 8.1144 ms]
//!     change: [-2.4624% -1.9685% -1.4719%] (p = 0.00 < 0.05)
//!     Performance has improved.
//!     Found 1 outliers among 100 measurements (1.00%)
//!     1 (1.00%) high mild
//!
//! 结论：
//! 这里的测试去掉了发送端准备数据的过程，相当于是只测试了接收端转换数据的性能。结果而言，原生枚举会快
//! 一点。但是综合项目可维护性，可扩展性，裸指针和动态分发必然是优先选择项。动态分发不支持原始数据还原
//! ，也不支持消耗原始数据。handle 接口的定义是需要指令的处理者消耗掉这个指令的。所以 emiiter 采用
//! 裸指针实现。

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use transfer_data::*;

pub mod transfer_data {
    pub trait CommandHandler {
        fn handle(&self) -> u64;
    }

    #[derive(Eq, PartialEq, Debug, Clone)]
    pub struct A(pub i64, pub i8, pub bool);

    #[derive(Eq, PartialEq, Debug, Clone)]
    pub enum Scheduler {
        Shoutown(Shoutown),
        AddTorrent(AddTorrent),
        OtherData(OtherData),
    }

    impl CommandHandler for Scheduler {
        fn handle(&self) -> u64 {
            match self {
                Scheduler::Shoutown(cmd) => cmd.handle(),
                Scheduler::AddTorrent(cmd) => cmd.handle(),
                Scheduler::OtherData(cmd) => cmd.handle(),
            }
        }
    }

    #[derive(Eq, PartialEq, Debug, Clone)]
    pub struct Shoutown;
    impl From<Shoutown> for Scheduler {
        fn from(value: Shoutown) -> Self {
            Scheduler::Shoutown(value)
        }
    }
    impl CommandHandler for Shoutown {
        fn handle(&self) -> u64 {
            0
        }
    }

    #[derive(Eq, PartialEq, Debug, Clone)]
    pub struct AddTorrent(pub &'static str);
    impl From<AddTorrent> for Scheduler {
        fn from(value: AddTorrent) -> Self {
            Scheduler::AddTorrent(value)
        }
    }
    impl CommandHandler for AddTorrent {
        fn handle(&self) -> u64 {
            1
        }
    }

    #[derive(Eq, PartialEq, Debug, Clone)]
    pub struct OtherData(pub A);
    impl From<OtherData> for Scheduler {
        fn from(value: OtherData) -> Self {
            Scheduler::OtherData(value)
        }
    }
    impl CommandHandler for OtherData {
        fn handle(&self) -> u64 {
            2
        }
    }

    #[derive(Eq, PartialEq, Debug, Clone)]
    pub enum PeerManager {
        NewDonwload(NewDownload),
    }
    impl CommandHandler for PeerManager {
        fn handle(&self) -> u64 {
            match self {
                PeerManager::NewDonwload(cmd) => cmd.handle(),
            }
        }
    }

    #[derive(Eq, PartialEq, Debug, Clone)]
    pub struct NewDownload;
    impl From<NewDownload> for PeerManager {
        fn from(value: NewDownload) -> Self {
            PeerManager::NewDonwload(value)
        }
    }
    impl CommandHandler for NewDownload {
        fn handle(&self) -> u64 {
            3
        }
    }

    #[derive(Eq, PartialEq, Hash)]
    pub enum EmitterType {
        Scheduler,
        PeerManager,
    }

    #[derive(Eq, PartialEq, Debug)]
    pub struct TransferPtr {
        pub inner: *const (),
    }
    impl TransferPtr {
        pub fn new(inner: *const ()) -> Self {
            Self { inner }
        }
    }

    unsafe impl Send for TransferPtr {}
    unsafe impl Sync for TransferPtr {}

    pub struct TransferDynBox {
        pub inner: Box<dyn CommandHandler>,
    }
    impl TransferDynBox {
        pub fn new(inner: Box<dyn CommandHandler>) -> Self {
            Self { inner }
        }
    }

    unsafe impl Send for TransferDynBox {}
    unsafe impl Sync for TransferDynBox {}
}

// ===========================================================================
// 传输
// ===========================================================================

pub mod transfer1 {
    use std::sync::Arc;
    use std::thread;

    use crate::transfer_data::{CommandHandler, Scheduler};

    pub fn enum_transfer(objs1: Arc<Vec<Arc<Scheduler>>>) -> u64 {
        let (send1, recv1) = std::sync::mpsc::channel::<Arc<Scheduler>>();
        thread::spawn(move || {
            for s in objs1.iter() {
                send1.send(s.clone()).unwrap()
            }
        });

        let mut sum = 0;
        while let Ok(handle) = recv1.recv() {
            sum += handle.handle();
        }

        sum
    }
}

pub mod transfer2 {
    use std::sync::Arc;
    use std::sync::mpsc::channel;
    use std::thread;

    use crate::transfer_data::{CommandHandler, Scheduler, TransferPtr};

    pub fn unsafe_ptr_transfer(objs: Arc<Vec<Arc<TransferPtr>>>) -> u64 {
        let (send1, recv1) = channel();

        thread::spawn(move || {
            for data in objs.iter() {
                send1.send(data.clone()).unwrap()
            }
        });

        let mut sum = 0;

        while let Ok(handle) = recv1.recv() {
            let data = unsafe { (handle.inner as *const Scheduler).as_ref().unwrap() };
            sum += data.handle();
        }

        sum
    }
}

pub mod transfer3 {
    use std::sync::Arc;
    use std::sync::mpsc::channel;
    use std::thread;

    use crate::transfer_data::TransferDynBox;

    pub fn dynamic_dispatch_transfer(objs: Arc<Vec<Arc<TransferDynBox>>>) -> u64 {
        let (send1, recv1) = channel();

        thread::spawn(move || {
            for data in objs.iter() {
                send1.send(data.clone()).unwrap()
            }
        });

        let mut sum = 0;
        while let Ok(handle) = recv1.recv() {
            sum += handle.inner.handle();
        }

        sum
    }
}

// ===========================================================================
// 生成测试数据
// ===========================================================================

fn generate_test_data1(len: usize) -> Vec<Arc<Scheduler>> {
    let mut list = Vec::with_capacity(len);
    for i in 0..len {
        match i % 4 {
            0 => {
                let data: Scheduler = Shoutown.into();
                list.push(Arc::new(data))
            }
            1 => {
                let data: Scheduler = AddTorrent("test").into();
                list.push(Arc::new(data))
            }
            2 => {
                let data: Scheduler = OtherData(A(1, 2, false)).into();
                list.push(Arc::new(data))
            }
            _ => (),
        }
    }
    list
}

fn generate_test_data2(len: usize) -> Vec<Arc<TransferPtr>> {
    let mut list = Vec::with_capacity(len);

    for i in 0..len {
        match i % 4 {
            0 => {
                let data: Scheduler = Shoutown.into();
                list.push(Arc::new(TransferPtr::new(
                    Box::into_raw(Box::new(data)) as *const ()
                )))
            }
            1 => {
                let data: Scheduler = AddTorrent("test").into();
                // list.push(TransferPtr::new(Box::into_raw(Box::new(data)) as *const ()))
                list.push(Arc::new(TransferPtr::new(
                    Box::into_raw(Box::new(data)) as *const ()
                )))
            }
            2 => {
                let data: Scheduler = OtherData(A(1, 2, false)).into();
                list.push(Arc::new(TransferPtr::new(
                    Box::into_raw(Box::new(data)) as *const ()
                )))
            }
            _ => (),
        }
    }

    list
}

fn generate_test_data3(len: usize) -> Vec<Arc<TransferDynBox>> {
    let mut list: Vec<Arc<TransferDynBox>> = Vec::with_capacity(len);

    for i in 0..len {
        match i % 4 {
            0 => {
                let data: Scheduler = Shoutown.into();
                // list.push(TransferDynBox::new(Box::new(data)))
                list.push(Arc::new(TransferDynBox::new(Box::new(data))))
            }
            1 => {
                let data: Scheduler = AddTorrent("test").into();
                // list.push(TransferDynBox::new(Box::new(data)))
                list.push(Arc::new(TransferDynBox::new(Box::new(data))))
            }
            2 => {
                let data: Scheduler = OtherData(A(1, 2, false)).into();
                // list.push(TransferDynBox::new(Box::new(data)))
                list.push(Arc::new(TransferDynBox::new(Box::new(data))))
            }
            _ => (),
        }
    }

    list
}

fn criterion_benchmark(c: &mut Criterion) {
    let len = 1024000;
    let list1 = Arc::new(generate_test_data1(len));
    let list2 = Arc::new(generate_test_data2(len));
    let list3 = Arc::new(generate_test_data3(len));

    // 原生枚举传输
    c.bench_function("enum transfer", |b| {
        b.iter(|| {
            let sum = transfer1::enum_transfer(list1.clone());
            black_box(sum)
        })
    });

    // 使用裸指针的方式
    c.bench_function("unsafe ptr transfer", |b| {
        b.iter(|| {
            let sum = transfer2::unsafe_ptr_transfer(list2.clone());
            black_box(sum)
        })
    });

    // 动态分发的方式
    c.bench_function("dynamic dispatch transfer", |b| {
        b.iter(|| {
            let sum = transfer3::dynamic_dispatch_transfer(list3.clone());
            black_box(sum)
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
