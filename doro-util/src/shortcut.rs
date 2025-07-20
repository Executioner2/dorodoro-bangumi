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
        if $cond { $if_true } else { $if_false }
    };
}

#[macro_export]
macro_rules! hashmap {
    () => { ::std::collections::HashMap::new() };
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut hashmap = ::std::collections::HashMap::new();
        $( hashmap.insert($key, $value); )*
        hashmap
    }};
}

#[macro_export]
macro_rules! linked_hashmap {
    () => { ::hashlink::LinkedHashMap::new() };
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut hashmap = ::hashlink::LinkedHashMap::new();
        $( hashmap.insert($key, $value); )*
        hashmap
    }};
}

/// 命令宏
#[macro_export]
macro_rules! command_system {
    (
        ctx: $ctx:ty,
        Command { $($variant:ident),+ $(,)? }
    ) => {
        use crate::core::command::CommandHandler;
        use crate::emitter::transfer::{CommandEnum, TransferPtr};
        
        #[derive(Debug)]
        pub enum Command {
            $(
                $variant($variant),
            )+
        }

        impl CommandEnum for Command {}

        $(
            impl From<$variant> for Command {
                #[inline]
                fn from(cmd: $variant) -> Self {
                    Self::$variant(cmd)
                }
            }

            impl Into<TransferPtr> for $variant {
                #[inline]
                fn into(self) -> TransferPtr {
                    Command::$variant(self).into()
                }
            }
        )+

        impl<'a> CommandHandler<'a, Result<()>> for Command {
            type Target = &'a mut $ctx;

            async fn handle(self, ctx: Self::Target) -> Result<()> {
                match self {
                    $(
                        Self::$variant(cmd) => {
                            cmd.handle(ctx).await
                        }
                    )+
                }
            }
        }
    };
}

// ===========================================================================
// anyhow 宏扩展
// ===========================================================================

/// 检查两个表达式是否相等，如果不相等则返回错误信息
///
/// # Arguments
///
/// * `left` - 左值表达式
/// * `right` - 右值表达式
/// * `msg` - 错误信息 
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::anyhow_eq;
///
/// let a = 1;
/// let b = 2;
/// let c = 1;
///
/// anyhow_eq!(a, b, "a should be equal to b"); // Err(a should be equal to b)
/// anyhow_eq!(a, c, "a should be equal to c"); // Ok(())
/// ```
#[macro_export]
macro_rules! anyhow_eq {
    ($left:expr, $right:expr, $($arg:tt)*) => {
        if $left != $right {
            return Err(anyhow::anyhow!($($arg)*))
        }
    }
}

/// 检查两个表达式是否不相等，如果相等则返回错误信息
///
/// # Arguments
///
/// * `left` - 左值表达式
/// * `right` - 右值表达式
/// * `msg` - 错误信息 
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::anyhow_ne;
///
/// let a = 1;
/// let b = 2;
/// let c = 1;
///
/// anyhow_ne!(a, b, "a should not be equal to b"); // Ok(())
/// anyhow_ne!(a, c, "a should not be equal to c"); // Err(a should not be equal to c)
#[macro_export]
macro_rules! anyhow_ne {
    ($left:expr, $right:expr, $($arg:tt)*) => {
        if $left == $right {
            return Err(anyhow::anyhow!($($arg)*))
        }
    }
}

/// 检查左值表达式是否小于等于右值表达式，如果不小于等于则返回错误信息
/// 
/// # Arguments
/// 
/// * `left` - 左值表达式
/// * `right` - 右值表达式
/// * `msg` - 错误信息 
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::anyhow_le;
///
/// let a = 1;
/// let b = 2;
/// let c = 1;
///
/// anyhow_le!(a, b, "a should be less than or equal to b"); // Ok(())
/// anyhow_le!(a, c, "a should be less than or equal to c"); // Ok(())
/// ```
#[macro_export]
macro_rules! anyhow_le {    
    ($left:expr, $right:expr, $($arg:tt)*) => {
        if $left > $right {
            return Err(anyhow::anyhow!($($arg)*))
        }
    }
}

/// 检查左值表达式是否小于右值表达式，如果不小于则返回错误信息
///
/// # Arguments
///
/// * `left` - 左值表达式
/// * `right` - 右值表达式
/// * `msg` - 错误信息 
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::anyhow_lt;
///
/// let a = 1;
/// let b = 2;
/// let c = 3;
///
/// anyhow_lt!(a, b, "a should be less than b"); // Ok(())
/// anyhow_lt!(a, c, "a should be less than c"); // Err(a should be less than c)
/// ```
#[macro_export]
macro_rules! anyhow_lt {
    ($left:expr, $right:expr, $($arg:tt)*) => {
        if $left >= $right {
            return Err(anyhow::anyhow!($($arg)*));
        }
    }
}

/// 检查左值表达式是否大于等于右值表达式，如果不大于等于则返回错误信息
///
/// # Arguments
///
/// * `left` - 左值表达式
/// * `right` - 右值表达式
/// * `msg` - 错误信息 
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::anyhow_ge;
///
/// let a = 1;
/// let b = 2;
/// let c = 1;
///
/// anyhow_ge!(a, b, "a should be greater than or equal to b"); // Ok(())
/// anyhow_ge!(a, c, "a should be greater than or equal to c"); // Ok(())
/// ```
#[macro_export]
macro_rules! anyhow_ge {
    ($left:expr, $right:expr, $($arg:tt)*) => {
        if $left < $right {
            return Err(anyhow::anyhow!($($arg)*))
        }
    }
}

/// 检查左值表达式是否大于右值表达式，如果不大于则返回错误信息
///
/// # Arguments
///
/// * `left` - 左值表达式
/// * `right` - 右值表达式
/// * `msg` - 错误信息 
///
/// # Examples
///
/// ```
/// use dorodoro_bangumi::anyhow_gt;
///
/// let a = 3;
/// let b = 2;
/// let c = 1;
///
/// anyhow_gt!(a, b, "a should be greater than b"); // Ok(())
/// anyhow_gt!(a, c, "a should be greater than c"); // Err(a should be greater than c)
/// ```
#[macro_export]
macro_rules! anyhow_gt {
    ($left:expr, $right:expr, $($arg:tt)*) => {
        if $left <= $right {
            return Err(anyhow::anyhow!($($arg)*))
        }
    }
}