use std::fmt::Display;
use std::ops::Deref;
use std::sync::OnceLock;
use std::sync::atomic::Ordering;

#[cfg(target_has_atomic = "64")]
use std::sync::atomic::AtomicU64;
#[cfg(not(target_has_atomic = "64"))]
use portable_atomic::AtomicU64;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Default)]
pub struct Id(u64);

impl Deref for Id {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 全局 id
#[derive(Default)]
pub struct GlobalId {
    /// id 计数器
    id_counter: AtomicU64,
}

impl GlobalId {
    pub fn global() -> &'static Self {
        static GLOBAL_ID: OnceLock<GlobalId> = OnceLock::new();
        GLOBAL_ID.get_or_init(|| GlobalId {
            id_counter: AtomicU64::new(1),
        })
    }

    pub fn next_id() -> Id {
        // 正常使用，几乎是不会出触发 id 用尽的情况。
        // todo - 后续再看看有没有更好的方案
        let this = GlobalId::global();
        let id = this.id_counter.fetch_add(1, Ordering::Acquire);
        if id == 0 {
            panic!("id counter overflow")
        }

        Id(id)
    }
}
