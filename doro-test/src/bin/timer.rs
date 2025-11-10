//! 定时器实现

use std::collections::{BinaryHeap, HashMap};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use doro_util::{datetime, default_logger};
use tokio::task::JoinSet;
use tracing::{Level, error, info};

default_logger!(Level::DEBUG);

type AsyncFunc = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Arc<String>> + Send>> + Send + Sync>;

/// 定时任务执行模式
#[derive(Debug, Clone, Copy)]
enum TimerExecMode {
    /// 同步执行，参数单位为毫秒。每间隔指定时间执行一次，如果上次
    /// 执行还没结束，则等待上次结束，然后再间隔指定时间后执行  
    FixedDelay(u64),

    /// 异步执行，参数单位为毫秒。每间隔指定时间后，就会以异步的方
    /// 式执行该定时任务
    Async(u64),
}

/// 定时任务 trait
/// 用于定义定时任务的接口
trait TimerTask: 'static + Sync + Send {
    /// 定时任务 ID。一般取实现者的函数名即可
    fn get_task_id(&self) -> Arc<String>;

    /// 定时任务初始延迟。单位为毫秒
    fn get_initial_delay(&self) -> u64;

    /// 返回定时任务执行函数
    fn run(&self) -> Pin<Box<dyn Future<Output = Arc<String>> + Send>>;

    /// 执行模式
    fn get_exec_mode(&self) -> TimerExecMode;

    /// 获取下一次任务执行时间
    fn get_next_time(&self) -> u64;

    /// 获取执行间隔时间
    fn get_interval_time(&self) -> u64;

    /// 设置最后执行时间
    fn flush_last_exec_time(&mut self);
}

/// dht 定时扫描
struct DHTScanTimer {
    /// 定时任务 ID
    id: Arc<String>,

    /// 定时任务初始延迟。单位为毫秒
    initial_delay: u64,

    /// 定时任务执行模式
    exec_mode: TimerExecMode,

    /// 上一次执行任务的时间
    last_exec_time: u64,

    /// 第一次运行标志
    first_run: Arc<AtomicBool>,

    /// 定时任务执行函数
    timer_func: AsyncFunc,
}

impl DHTScanTimer {
    fn new<F, Fut>(id: String, initial_delay: u64, exec_mode: TimerExecMode, timer_func: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let timer_func = Arc::new(timer_func);
        let first_run = Arc::new(AtomicBool::new(true));
        let id = Arc::new(id);
        Self {
            id: id.clone(),
            initial_delay,
            exec_mode,
            last_exec_time: datetime::now_millis() as u64,
            first_run: first_run.clone(),
            timer_func: Arc::new(move || {
                Self::gen_timer_func(timer_func.clone(), id.clone(), first_run.clone())
            }),
        }
    }

    /// 生成定时任务执行函数
    fn gen_timer_func<F, Fut>(
        timer_func: Arc<F>, id: Arc<String>, first_run: Arc<AtomicBool>,
    ) -> Pin<Box<dyn Future<Output = Arc<String>> + Send>>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Box::pin(async move {
            first_run.store(false, Ordering::SeqCst);
            timer_func().await;
            id
        })
    }
}

impl PartialEq for Box<dyn TimerTask> {
    fn eq(&self, other: &Self) -> bool {
        self.get_next_time() == other.get_next_time()
    }
}

impl Eq for Box<dyn TimerTask> {}

impl PartialOrd for Box<dyn TimerTask> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Box<dyn TimerTask> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let a = self.get_next_time();
        let b = other.get_next_time();
        if a > b {
            std::cmp::Ordering::Less
        } else if a < b {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }
}

impl TimerTask for DHTScanTimer {
    /// 定时任务 ID。一般取实现者的函数名即可
    fn get_task_id(&self) -> Arc<String> {
        self.id.clone()
    }

    /// 定时任务初始延迟。单位为毫秒
    fn get_initial_delay(&self) -> u64 {
        self.initial_delay
    }

    /// 定时任务执行函数
    fn run(&self) -> Pin<Box<dyn Future<Output = Arc<String>> + Send>> {
        (self.timer_func.clone())()
    }

    /// 执行模式
    fn get_exec_mode(&self) -> TimerExecMode {
        self.exec_mode
    }

    /// 获取下一次任务执行时间
    fn get_next_time(&self) -> u64 {
        let mut next_time = self.last_exec_time + self.get_interval_time();
        if self.first_run.load(Ordering::SeqCst) {
            next_time += self.get_initial_delay();
        }
        next_time
    }

    /// 获取执行的间隔时间
    fn get_interval_time(&self) -> u64 {
        match self.exec_mode {
            TimerExecMode::FixedDelay(delay) => delay,
            TimerExecMode::Async(delay) => delay,
        }
    }

    /// 设置最后执行时间
    fn flush_last_exec_time(&mut self) {
        self.last_exec_time = datetime::now_millis() as u64;
    }
}

/// 定时任务管理器
struct TimerTaskManager {
    /// 定时任务异步执行队列
    task_queue: JoinSet<Arc<String>>,

    /// 执行集合，存放 FixedDelay 模式的定时任务
    delay_map: HashMap<Arc<String>, Box<dyn TimerTask>>,

    /// 已注册的定时任务
    timer_tasks: BinaryHeap<Box<dyn TimerTask>>,
}

impl TimerTaskManager {
    /// 构造函数
    pub fn new() -> Self {
        Self {
            task_queue: JoinSet::new(),
            delay_map: HashMap::new(),
            timer_tasks: BinaryHeap::new(),
        }
    }

    /// 注册定时任务
    pub fn register_task(&mut self, task: Box<dyn TimerTask>) {
        self.timer_tasks.push(task);
    }

    /// 启动定时任务
    pub async fn run(mut self) {
        let mut delayer = 0;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(delayer)) => {
                    // 获取下一个执行的定时任务，不能在这里使用 while 之类的循环，
                    // 避免出现 Async(0) 这样的任务，导致 task_queue 当中已完成
                    // 的结果无法被读取到
                    if let Some(mut task) = self.timer_tasks.pop() {
                        let now = datetime::now_millis() as u64;
                        if task.get_next_time() > now {
                            delayer = task.get_next_time() - now;
                            self.register_task(task);
                            continue;
                        }

                        match task.get_exec_mode() {
                            TimerExecMode::Async(_) => {
                                // 异步执行，直接加入异步队列，并且设置执行时间
                                task.flush_last_exec_time();
                                self.task_queue.spawn(task.run());
                                self.timer_tasks.push(task);
                            },
                            TimerExecMode::FixedDelay(_) => {
                                // 固定间隔执行，设置下一次执行时间
                                let run = task.run();
                                self.delay_map.insert(task.get_task_id(), task);
                                self.task_queue.spawn(run);
                            }
                        }

                        delayer = 0;
                    } else {
                        delayer = 1000 * 10; // todo - 现在是 10s 后再次检查，应该改为永久阻塞，直到有新的任务注册
                    }
                }
                ret = self.task_queue.join_next(), if !self.task_queue.is_empty() => {                    
                    match ret {
                        Some(Ok(ret)) => {
                            if let Some(mut task) = self.delay_map.remove(&ret) {
                                task.flush_last_exec_time();
                                self.register_task(task);
                            }
                        }
                        Some(Err(e)) => {
                            error!("定时任务出错: {e}");
                        }
                        None => {
                            // 不会走到这个分支来的
                            info!("task queue 为空，没有 task 在执行中");
                        }
                    }
                }
            }
        }
    }
}

// ===========================================================================
// TEST
// ===========================================================================

/// 定时任务执行函数 1
async fn run_timer_task1() {
    info!("run_timer_task1 定时任务执行 start");
    tokio::time::sleep(std::time::Duration::from_millis(1000 * 3)).await;
    info!("run_timer_task1 定时任务执行 end");
}

/// 定时任务执行函数 2
async fn run_timer_task2() {
    info!("run_timer_task2 执行了");
}

/// 定时器 demo 启动入口
#[tokio::main]
async fn main() {
    let mut ttm = TimerTaskManager::new();

    let timer1 = DHTScanTimer::new(
        "dht_scan1".to_string(),
        1000 * 2,
        TimerExecMode::Async(1000 * 10),
        run_timer_task1,
    );

    let timer2 = DHTScanTimer::new(
        "dht_scan2".to_string(),
        0,
        TimerExecMode::FixedDelay(1000 * 2),
        run_timer_task2,
    );

    ttm.register_task(Box::new(timer1));
    ttm.register_task(Box::new(timer2));

    ttm.run().await;
}
