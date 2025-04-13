//! 动态分发和静态分发的性能测试
//!
//! 运行方式：
//!
//! ``` shell
//! cargo bench --bench dispatch_benchmark
//! ```
//!
//! 测试结果：
//!
//!     Gnuplot not found, using plotters backend
//!     dispatch1 - enum match  time:   [689.18 ns 689.89 ns 690.79 ns]
//!                         change: [+0.8441% +1.1300% +1.4576%] (p = 0.00 < 0.05)
//!                         Change within noise threshold.
//!     Found 2 outliers among 100 measurements (2.00%)
//!     1 (1.00%) high mild
//!     1 (1.00%) high severe
//!
//!     dynamic dispatch - enum_dispatch
//!                         time:   [688.01 ns 688.39 ns 688.81 ns]
//!                         change: [-69.402% -69.311% -69.218%] (p = 0.00 < 0.05)
//!                         Performance has improved.
//!     Found 5 outliers among 100 measurements (5.00%)
//!     2 (2.00%) high mild
//!     3 (3.00%) high severe
//!
//!     dynamic dispatch - default
//!                         time:   [2.2016 µs 2.2102 µs 2.2195 µs]
//!                         change: [-0.1842% +0.0717% +0.3402%] (p = 0.59 > 0.05)
//!                         No change in performance detected.
//!     Found 14 outliers among 100 measurements (14.00%)
//!     3 (3.00%) high mild
//!     11 (11.00%) high severe
//!
//!
//! 结论：
//!
//! 第三方库 enum_dispatch 的分发性能和原生 enum match 基本相同，因此直接用第三方库即可。
//! 
//! 补充：第三方库 enum_dispatch 有致命缺陷，如下：
//!  1. 无法区分命名空间：被连接的枚举或 trait，必须是唯一的，这就意味着，无法在不同 mod 中命名相同的枚举或 trait。
//!  2. trait 泛型无法指定具体类型实现：如果被 dispatch 的 trait 上定义了泛型，连接的枚举无法指定具体的类型。详见 `test/enum_dispatch.rs`
//! 
//! 因此还是用原生的静态分发，后续有时间再抽象 match 的手动匹配。

use crate::dispatch1::Torrent;
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

pub trait Dispatchable {
    fn dispatch(&self) -> f64;
}

pub struct Add {
    pub data: String,
}
impl Dispatchable for Add {
    fn dispatch(&self) -> f64 {
        1.0 + self.data.len() as f64
    }
}

pub struct Delete {
    pub data: String,
}
impl Dispatchable for Delete {
    fn dispatch(&self) -> f64 {
        2.0 + self.data.len() as f64
    }
}

pub struct Get {
    pub data: String,
}
impl Dispatchable for Get {
    fn dispatch(&self) -> f64 {
        3.0 + self.data.len() as f64
    }
}

pub struct Update {
    pub data: String,
}
impl Dispatchable for Update {
    fn dispatch(&self) -> f64 {
        4.0 + self.data.len() as f64
    }
}

pub mod dispatch1 {
    use super::*;

    pub enum Torrent {
        Add(Add),
        Delete(Delete),
        Get(Get),
        Update(Update),
    }

    pub fn static_dispatch(torrent: &Torrent) -> f64 {
        match torrent {
            Torrent::Add(value) => value.dispatch(),
            Torrent::Delete(value) => value.dispatch(),
            Torrent::Get(value) => value.dispatch(),
            Torrent::Update(value) => value.dispatch(),
        }
    }
}

pub mod dispatch2 {
    use enum_dispatch::enum_dispatch;

    #[enum_dispatch(Torrent)]
    pub trait Dispatchable {
        fn dispatch(&self) -> f64;
    }

    #[enum_dispatch]
    pub enum Torrent {
        Add,
        Delete,
        Get,
        Update,
    }

    pub struct Add {
        pub data: String,
    }
    impl Dispatchable for Add {
        fn dispatch(&self) -> f64 {
            1.0 + self.data.len() as f64
        }
    }

    pub struct Delete {
        pub data: String,
    }
    impl Dispatchable for Delete {
        fn dispatch(&self) -> f64 {
            2.0 + self.data.len() as f64
        }
    }

    pub struct Get {
        pub data: String,
    }
    impl Dispatchable for Get {
        fn dispatch(&self) -> f64 {
            3.0 + self.data.len() as f64
        }
    }

    pub struct Update {
        pub data: String,
    }
    impl Dispatchable for Update {
        fn dispatch(&self) -> f64 {
            4.0 + self.data.len() as f64
        }
    }

    pub fn dynamic_dispatch(obj: &Torrent) -> f64 {
        obj.dispatch()
    }
}

pub mod dispatch3 {
    use super::*;

    pub enum Torrent {
        Add(Add),
        Delete(Delete),
        Get(Get),
        Update(Update),
    }

    impl Dispatchable for Torrent {
        fn dispatch(&self) -> f64 {
            match self {
                Torrent::Add(value) => value.dispatch(),
                Torrent::Delete(value) => value.dispatch(),
                Torrent::Get(value) => value.dispatch(),
                Torrent::Update(value) => value.dispatch(),
            }
        }
    }

    pub fn dynamic_dispatch(obj: &Box<dyn Dispatchable>) -> f64 {
        obj.dispatch()
    }
}

fn generate_test_data1(size: usize) -> Vec<Torrent> {
    let mut data = Vec::with_capacity(size);
    for i in 0..size {
        let value = format!("value_{}", i);
        match i % 4 {
            0 => data.push(Torrent::Add(Add { data: value })),
            1 => data.push(Torrent::Delete(Delete { data: value })),
            2 => data.push(Torrent::Get(Get { data: value })),
            3 => data.push(Torrent::Update(Update { data: value })),
            _ => unreachable!(),
        }
    }
    data
}

fn generate_test_data2(size: usize) -> Vec<dispatch2::Torrent> {
    let mut data: Vec<dispatch2::Torrent> = Vec::with_capacity(size);
    for i in 0..size {
        let value = format!("value_{}", i);
        match i % 4 {
            0 => data.push(dispatch2::Torrent::Add(dispatch2::Add { data: value })),
            1 => data.push(dispatch2::Torrent::Delete(dispatch2::Delete { data: value })),
            2 => data.push(dispatch2::Torrent::Get(dispatch2::Get { data: value })),
            3 => data.push(dispatch2::Torrent::Update(dispatch2::Update { data: value })),
            _ => unreachable!(),
        }
    }
    data
}

fn generate_test_data3(size: usize) -> Vec<Box<dyn Dispatchable>> {
    let mut data: Vec<Box<dyn Dispatchable>> = Vec::with_capacity(size);
    for i in 0..size {
        let value = format!("value_{}", i);
        match i % 4 {
            0 => data.push(Box::new(Add { data: value })),
            1 => data.push(Box::new(Delete { data: value })),
            2 => data.push(Box::new(Get { data: value })),
            3 => data.push(Box::new(Update { data: value })),
            _ => unreachable!(),
        }
    }
    data
}

fn criterion_benchmark(c: &mut Criterion) {
    let data1 = generate_test_data1(1024);
    let data2 = generate_test_data2(1024);
    let data3 = generate_test_data3(1024);

    // 原生 enum match
    c.bench_function("static dispatch - enum match", |b| {
        b.iter(|| {
            let mut sum = 0.0;
            for obj in &data1 {
                let x = dispatch1::static_dispatch(obj);
                sum += x;
            }
            black_box(sum)
        })
    });

    // 使用第三方库 enum_dispatch
    c.bench_function("enum_dispatch", |b| {
        b.iter(|| {
            let mut sum = 0.0;
            for obj in &data2 {
                let x = dispatch2::dynamic_dispatch(obj);
                sum += x;
            }
            black_box(sum)
        })
    });

    // 动态分发
    c.bench_function("dynamic dispatch - default", |b| {
        b.iter(|| {
            let mut sum = 0.0;
            for obj in &data3 {
                let x = dispatch3::dynamic_dispatch(obj);
                sum += x;
            }
            black_box(sum)
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
