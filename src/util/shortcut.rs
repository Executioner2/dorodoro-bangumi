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

// #[macro_export]
// macro_rules! command_system {
//     (
//         ctx: $ctx:ty,
//         Command { $($variant:ident($cmd:ty)),+ $(,)? },
//         handlers { $($handler:ident => |$c:ident, $x:ident| $body:block),+ $(,)? }
//     ) => {
//         #[derive(Debug)]
//         pub enum Command {
//             $($variant($cmd)),+
//         }
//
//         impl CommandEnum for Command {}
//
//         $(
//             impl From<$cmd> for Command {
//                 #[inline]
//                 fn from(cmd: $cmd) -> Self {
//                     Self::$variant(cmd)
//                 }
//             }
//
//             impl Into<TransferPtr> for $cmd {
//                 #[inline]
//                 fn into(self) -> TransferPtr {
//                     Command::$variant(self).into()
//                 }
//             }
//         )+
//
//         impl<'a> CommandHandler<'a> for Command {
//             type Target = &'a $ctx;
//
//             async fn handle(self, context: Self::Target) {
//                 match self {
//                     $(
//                         Self::$variant($c) => {
//                             let $x = context;
//                             $body
//                         }
//                     )+
//                 }
//             }
//         }
//     };
// }

/// 命令宏
#[macro_export]
macro_rules! command_system {
    (
        ctx: $ctx:ty,
        Command { $($variant:ident),+ $(,)? }
    ) => {
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

        impl<'a> CommandHandler<'a> for Command {
            type Target = &'a mut $ctx;

            async fn handle(self, ctx: Self::Target) {
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
