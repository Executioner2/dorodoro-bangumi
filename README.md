# 介绍
dorodoro-bangumi（暂定名）是一款 Rust 编写，独立部署的自动追番机。皆在为用户提供番剧订阅，自动下载，整理等一体化服务。dorodoro-bangumi 小巧灵活，并在往，低占用、持续稳定的运行在 openwrt 平台而努力。

哦对了，doro 是一只有趣的粉色小狗😊。

# 待办事项
- [ ] 基础功能接口完善
- [ ] 处理直接使用 rc4 加密传输，性能损失严重的问题
- [ ] 下载量计数可能存在 bug，需要排查确认
- [ ] 磁力链接解析速度慢，需要优化
- [ ] 优化运行的 lt_peer 数量低于阈值时，无法快速打满阈值的问题
- [ ] 处理 bendy 中，固定拒绝未排序 bencode 字符串的行为
- [ ] 速率控制写得不是很好，需要重构
- [ ] 定时功能抽离，封装通用逻辑，统一放到一个模块方便维护
- [ ] bt 上传实现
- [x] 完成 mikan 资源获取
- [x] 完成 rss 订阅
- [x] 实现磁力链接解析
- [x] 重构下载核心模块，ator 模型改为 mutex + channel 混合
- [x] 实现 springboot 风格的接口路由
- [x] 实现 dht 节点发现
- [x] 重构 Error 结构
- [x] 项目结构整改，改为 workspace
- [ ] ~~重构 Bencoding 结构~~ (用 bendy 库代替了)

# 开发过程的一些记录
## 选型和优化
- buffer 优化（通过unsafe ptr避免初始化，性能提升5倍，考量指标：性能）
- 日志系统选型（log、tracing 还是 tklog，考量指标：性能、内存占用）
- 系统骨架选择（是 runnable + channel 还是共享锁，考量指标：io 性能，维护成本，可扩展性）
- 分发选型（动态、静态、第三方静态的 enum_dispatch 还是手动编写一个静态的固定模式，考量指标：性能、灵活性、维护成本）
  - 在性能方面 enum_dispatch 表现是很不错的，但是在实际使用中，发现有两个致命的缺点：
    1. 无法区分命名空间：被连接的枚举或 trait，必须是唯一的，这就意味着，无法在不同 mod 中命名相同的枚举或 trait。
    2. trait 泛型无法指定具体类型实现：如果被 dispatch 的 trait 上定义了泛型，连接的枚举无法指定具体的类型。详见 `test/enum_dispatch.rs`

# 启动方式
## 直接启动
```shell
cargo run --package doro --bin doro
```

## 保留 symbols 的调试启动
```shell
cargo run --profile dev-with-symbols --package doro --bin doro
```

## dev 环境下以 tokio console 启动（linux or macos）
安装 tokio-console
```shell
cargo install tokio-console
```

运行 doro
```shell
RUSTFLAGS="--cfg tokio_unstable" TOKIO_CONSOLE=true RUST_LOG=trace cargo run --features dev --package doro --bin doro
```

打开监控面板
```shell
tokio-console http://127.0.0.1:9090
```

# 源码构建
## mips 平台
- 配置交叉编译环境
```shell
wget https://mirror-03.infra.openwrt.org/releases/24.10.2/targets/ramips/mt7621/openwrt-sdk-24.10.2-ramips-mt7621_gcc-13.3.0_musl.Linux-x86_64.tar.zst

zstd -d openwrt-sdk-24.10.2-ramips-mt7621_gcc-13.3.0_musl.Linux-x86_64.tar.zst

tar -xvf openwrt-sdk-24.10.2-ramips-mt7621_gcc-13.3.0_musl.Linux-x86_64.tar

mv openwrt-sdk-24.10.2-ramips-mt7621_gcc-13.3.0_musl.Linux-x86_64 /usr/local/bin/

cat >> /etc/profile << 'EOF'
export STAGING_DIR='/usr/local/bin/openwrt-sdk-24.10.2-ramips-mt7621_gcc-13.3.0_musl.Linux-x86_64/staging_dir'
export PATH=$PATH:$STAGING_DIR/'toolchain-mipsel_24kc_gcc-13.3.0_musl'/bin
EOF

source /etc/profile
```

- 安装 nightly 工具链
```shell
rustup toolchain install nightly
```

- 通过 nightly 构建
```shell
cargo +nightly build -r -Z build-std --target mipsel-unknown-linux-musl
```