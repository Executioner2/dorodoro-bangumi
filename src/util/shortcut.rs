//! 这里定义一些便于开发的快捷宏

/// 条件判断，类似于三元运算符
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::if_else;
///
/// if_else!(true, 1, 2); // 1
/// if_else!(false, 1, 2); // 2
/// ```
#[macro_export]
macro_rules! if_else {
    ($cond:expr, $if_true:expr, $if_false:expr) => {
        if $cond {
            $if_true
        } else {
            $if_false
        }
    };
}