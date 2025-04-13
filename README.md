# 介绍
dorodoro-bangumi（暂定名）是一款 Rust 编写，独立部署的自动追番机。皆在为用户提供番剧订阅，自动下载，整理等一体化服务。dorodoro-bangumi 小巧灵活，并在往，低占用、持续稳定的运行在 openwrt 平台而努力。

哦对了，doro 是一只有趣的粉色小狗😊。

# 待办事项
- [x] 重构 Error 结构
- [ ] 重构 Bencoding 结构

# 开发过程的一些记录
## 选型和优化
- buffer 优化（通过unsafe ptr避免初始化，性能提升5倍，考量指标：性能）
- 日志系统选型（log、tracing 还是 tklog，考量指标：性能、内存占用）
- 系统骨架选择（是 runnable + channel 还是共享锁，考量指标：io 性能，维护成本，可扩展性）
- 分发选型（动态、静态、第三方静态的 enum_dispatch 还是手动编写一个静态的固定模式，考量指标：性能、灵活性、维护成本）
  - 在性能方面 enum_dispatch 表现是很不错的，但是在实际使用中，发现有两个致命的缺点：
    1. 无法区分命名空间：被连接的枚举或 trait，必须是唯一的，这就意味着，无法在不同 mod 中命名相同的枚举或 trait。
    2. trait 泛型无法指定具体类型实现：如果被 dispatch 的 trait 上定义了泛型，连接的枚举无法指定具体的类型。详见 `test/enum_dispatch.rs`