use std::io::Error;
use std::path::Path;
use std::thread::spawn;
use tracing::{Level, error, info_span, error_span, debug_span};
use crate::log::{register_logger, SizeBasedWriter};

const PATH_STR: &str = "logs";
const LOG_FILE_NAME: &str = "dorodoro-bangumi.log";
const LOG_FILE_SIZE: u64 = 10 << 20;
const LOG_FILE_CHUNKS: usize = 2;
const LOG_FILE_LEVEL: Level = Level::INFO;

/// 测试是否能够正常清理旧的日志文件
///
/// 需要添加 --nocapture 参数运行测试，否则无法看到输出信息
#[test]
#[cfg_attr(miri, ignore)]
fn test_cleanup_old_files() {
    let writer = SizeBasedWriter::new(
        Path::new(PATH_STR),
        LOG_FILE_NAME,
        LOG_FILE_SIZE,
        LOG_FILE_CHUNKS,
    )
    .unwrap();
    assert!(writer.cleanup_old_files().is_ok());
}

/// 测试多线程下的日志写入
///
/// 观察测试结果（测试判断条件懒得写了）
///
/// 检查多线程下，输出是否完整，输出的时间是否有序。注意是检查日志文件，而不是控制台输出。
///
/// 观察输出结果，日志条数正确，但是时间顺序会有细微的混乱，亚级秒保留到4位（到毫秒）基本上无差别。
#[test]
#[cfg_attr(miri, ignore)]
fn test_thread_write() {
    let _guard = register_logger(
        PATH_STR,
        LOG_FILE_NAME,
        LOG_FILE_SIZE,
        LOG_FILE_CHUNKS,
        LOG_FILE_LEVEL,
    )
    .unwrap();
    let mut list = vec![];
    (0..2).for_each(|_| {
        let handle = spawn(|| {
            for i in 0..10 {
                tracing::info!("{i}");
            }
        });
        list.push(handle);
    });

    for handle in list {
        handle.join().unwrap();
    }
}

/// 测试错误日志打印
#[test]
#[cfg_attr(miri, ignore)]
fn test_log_err_print() {
    let _guard = register_logger(
        PATH_STR,
        LOG_FILE_NAME,
        LOG_FILE_SIZE,
        LOG_FILE_CHUNKS,
        LOG_FILE_LEVEL,
    )
    .unwrap();
    let err = Error::new(std::io::ErrorKind::Other, "测试一下错误打印");
    error!("test error log {}", err)
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_log_span() {
    let _guard = register_logger(
        PATH_STR,
        LOG_FILE_NAME,
        LOG_FILE_SIZE,
        LOG_FILE_CHUNKS,
        LOG_FILE_LEVEL,
    ).unwrap();
    let span = info_span!("test_info_span");
    span.in_scope(|| {
        println!("这里执行 info span 里的内容")
    });

    let span = error_span!("test_error_span");
    span.in_scope(|| {
        println!("这里执行 error span 里的内容")
    });

    let span = debug_span!("test_debug_span");
    span.in_scope(|| {
        println!("这里执行 debug span 里的内容")
    });
}