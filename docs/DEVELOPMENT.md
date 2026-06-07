# ClashTUI 开发文档

本文档面向后续维护者和贡献者，说明 ClashTUI 当前代码的真实结构、运行流程、模块边界、扩展方式与验证方法。

> 代码基准：当前 workspace 下的四个 crate：`clashtui-core-api`、`clashtui-domain`、`clashtui-tui`、`clashtui-bin`。README 中列出的部分 CLI 子命令仍是规划骨架，本文会区分“已实现”和“预留”。

## 1. 项目定位

ClashTUI 是一个纯终端 TUI 的 [mihomo](https://github.com/MetaCubeX/mihomo) 管理器，目标平台是 macOS，Linux 为次要平台。它负责：

- 连接或托管 mihomo 内核。
- 管理订阅/Profile，并生成运行时 `config.yaml`。
- 通过 mihomo external-controller HTTP API 修改配置、选节点、测速、关闭连接。
- 通过 mihomo WebSocket API 展示日志、连接、流量、内存等实时数据。
- 提供系统代理开关、mixin 编辑、内核升级等操作入口。

项目当前是 Rust Cargo workspace，整体设计原则是：

- `clashtui-core-api` 只做 mihomo HTTP/WebSocket 协议客户端，不碰文件系统、终端和 OS 命令。
- `clashtui-domain` 处理业务逻辑、配置文件、profile、mixin、生命周期、系统代理、升级等，不依赖 TUI 类型。
- `clashtui-tui` 只做 ratatui/crossterm 前端，UI 通过声明式 `Effect` 请求副作用，通过 `AppEvent` 接收结果。
- `clashtui-bin` 负责 CLI 入口和命令分派。

## 2. 环境要求

### 2.1 Rust 与系统依赖

- Rust edition：`2021`
- workspace `rust-version`：`1.94`
- 主要第三方库：
  - 异步运行时：`tokio`
  - TUI：`ratatui 0.30`、`crossterm 0.29`
  - HTTP/WebSocket：`reqwest 0.13`、`tokio-tungstenite 0.29`
  - 序列化：`serde`、`serde_json`、`toml`、`serde_yaml_ng`
  - CLI：`clap`
  - 错误：`color-eyre`、`thiserror`

### 2.2 平台工具

macOS：

- 系统代理通过 `networksetup` 修改。
- 活跃网络服务探测会调用 `ifconfig` 和 `networksetup -listnetworkserviceorder`。
- 启动/升级 mihomo 后会尽力删除 `com.apple.quarantine` xattr。

Linux：

- service 级系统代理当前使用 GNOME `gsettings`。
- 非 GNOME 桌面环境通常只能使用 env 级代理片段。

通用：

- `$EDITOR` 或 `$VISUAL` 用于编辑 `mixin.yaml`，未设置时默认 `vi`。
- 内核托管模式需要可执行的 mihomo 二进制。
- TUN 是否能生效取决于 mihomo 进程权限，本项目不会自动提权。

## 3. 常用命令

```sh
cargo build --workspace
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --check
```

运行 TUI：

```sh
cargo run -p clashtui-bin
```

已实现的 CLI：

```sh
cargo run -p clashtui-bin -- version
cargo run -p clashtui-bin -- sysproxy on
cargo run -p clashtui-bin -- sysproxy off
cargo run -p clashtui-bin -- sysproxy env
cargo run -p clashtui-bin -- sysproxy env --off
```

预留但当前未实现具体逻辑的 CLI：

- `profile`
- `mode`
- `proxy`
- `service`
- `upgrade`

这些命令在 `crates/clashtui-bin/src/cli.rs` 中已声明，在 `main.rs` 中会打印“尚未实现”。

## 4. 目录结构

```text
.
├── Cargo.toml
├── README.md
├── docs/
│   └── DEVELOPMENT.md
└── crates/
    ├── clashtui-bin/
    ├── clashtui-core-api/
    ├── clashtui-domain/
    └── clashtui-tui/
```

workspace 成员：

| crate | 职责 |
| --- | --- |
| `clashtui-core-api` | mihomo external-controller 的纯异步 HTTP + WebSocket 客户端 |
| `clashtui-domain` | 配置、profile、mixin、内核生命周期、系统代理、升级、调度 |
| `clashtui-tui` | ratatui/crossterm 前端、事件循环、组件、tab、widgets |
| `clashtui-bin` | `clashtui` 二进制入口、CLI 解析和分派 |

## 5. 运行时文件布局

路径由 `clashtui-domain/src/paths.rs` 的 `Paths::resolve()` 决定。

macOS 默认：

```text
~/Library/Application Support/ClashTUI/
```

Linux 默认：

```text
~/.config/clashtui/
```

无法定位标准目录时回退：

```text
.clashtui/
```

主要文件：

| 路径 | 用途 |
| --- | --- |
| `config.toml` | 应用主配置 |
| `profiles.toml` | profile 元数据 DB，含当前 profile 指针 |
| `profiles/{name}.yaml` | 原始订阅 YAML，按字节保存，业务逻辑不直接改写 |
| `mixin.yaml` | 用户 mixin，走精细合并语义 |
| `override.yaml` | blunt override，深合并覆盖 |
| `core/config.yaml` | mihomo 运行时配置，由当前 profile 生成 |
| `core/` | mihomo 工作目录 |
| `bin/mihomo` | 默认 mihomo 二进制 |
| `proxy.sh` | env 级代理片段 |

启动 TUI 时会调用 `paths.ensure_dirs()` 创建必要目录。

## 6. 应用配置

应用配置定义在 `clashtui-domain/src/config.rs`。

`AppConfig` 字段：

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `external_controller` | `127.0.0.1:9090` | mihomo external-controller 地址 |
| `secret` | 空 | external-controller secret |
| `mihomo_binary` | 空 | 空表示使用 `bin/mihomo` |
| `keep_core_running` | `false` | 退出 TUI 后是否保留由 ClashTUI 启动的 mihomo 内核 |
| `test_url` | `http://www.gstatic.com/generate_204` | 节点测速 URL |
| `test_timeout_ms` | `5000` | 节点测速超时 |
| `log_level` | `info` | 预留/配置展示用日志级别 |
| `manual_network_service` | 空 | macOS 手动指定网络服务名 |
| `system_proxy` | 见下 | 系统代理配置 |
| `auto_update` | 见下 | 自动更新配置 |

`SystemProxyConfig`：

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `service_enabled` | `false` | 当前未作为可靠状态源，真实状态以 OS 查询为准 |
| `env_enabled` | `false` | 当前未作为可靠状态源 |
| `http_port` | `7890` | HTTP/HTTPS 代理端口，对应 mihomo `port` |
| `socks_port` | `7891` | SOCKS 端口 |
| `mixed_port` | `7892` | Mixed 端口，对应 mihomo `mixed-port` |
| `bypass` | 本地地址和私网段 | no_proxy / bypass 列表 |

`AutoUpdateConfig`：

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `enabled` | `false` | 是否开启进程内自动更新调度 |
| `interval_hours` | `24` | 自动更新间隔，调度层下限为 1 小时 |

加载行为：

- `AppConfig::load()` 读取 `config.toml`。
- 文件不存在时返回默认配置，不主动写盘。
- TOML 缺字段时由 serde 默认值补齐。
- `save()` 使用 `atomic_write()` 原子写入。

## 7. 配置生成管线

配置生成在 `clashtui-domain/src/mixin.rs` 中。

当切换 profile 或更新当前 profile 时，TUI 会调用 `apply_current_profile()`：

```text
profiles/{current}.yaml
  -> optional mixin.yaml
  -> optional override.yaml
  -> force inject external-controller/secret/port/socks-port/mixed-port
  -> core/config.yaml
  -> PUT /configs?force=true reload
```

关键约束：

- 原始订阅文件只保存，不直接修改。
- `mixin.yaml` 用精细语义修改配置。
- `override.yaml` 用普通深合并覆盖。
- 最后强制注入 `external-controller`、非空 `secret`、`port`、`socks-port`、`mixed-port`，确保 TUI 仍能控制 mihomo。
- 如果 mihomo 在线，写完 `core/config.yaml` 后调用 `CoreManager::reload()`。
- reload 会断开 WebSocket，调用方应执行 `Effect::ReconnectStreams` 或依赖流自身重连。

### 7.1 mixin 语义

支持三个数组段：

- `rules`
- `proxies`
- `proxy-groups`

支持操作：

| 键 | 行为 |
| --- | --- |
| `prepend-rules` | 把数组插到 `rules` 前面 |
| `append-rules` | 把数组追加到 `rules` 后面 |
| `override-rules` | 直接替换整段 `rules` |
| `prepend-proxies` | 插到 `proxies` 前面 |
| `append-proxies` | 追加到 `proxies` 后面 |
| `override-proxies` | 按 `name` 替换同名项，新名字追加 |
| `prepend-proxy-groups` | 插到 `proxy-groups` 前面 |
| `append-proxy-groups` | 追加到 `proxy-groups` 后面 |
| `override-proxy-groups` | 按 `name` 替换同名项，新名字追加 |

其它普通键执行深合并，mixin 优先。

示例：

```yaml
append-rules:
  - DOMAIN-SUFFIX,example.com,DIRECT

override-proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - DIRECT

log-level: info
```

### 7.2 YAML 抽象

YAML 处理在 `clashtui-domain/src/yaml.rs`。

- 默认 feature：`yaml-ng`，使用 `serde_yaml_ng`。
- 可选 feature：`yaml-noya`。
- `Value` 和 `Mapping` 统一由该模块导出。
- 设计重点是保序，避免代理组和规则顺序被破坏。

## 8. Profile 管理

Profile 代码在 `clashtui-domain/src/profile/`。

持久化模型：

- `profiles.toml` 保存 `ProfileDb`：
  - `current: Option<String>`
  - `profiles: Vec<ProfileMeta>`
- `profiles/{name}.yaml` 保存原始订阅内容。

`ProfileMeta`：

| 字段 | 说明 |
| --- | --- |
| `name` | profile 名，也是 YAML 文件名基础 |
| `kind` | `file` 或 `url` |
| `url` | URL 或源文件路径 |
| `last_updated` | 最近更新 Unix 秒 |
| `subscription_info` | 订阅流量/到期信息 |
| `mixin_enabled` | 当前保留字段，默认 true |

URL 订阅下载在 `profile/download.rs`：

- 使用 `User-Agent: clash.meta`。
- 解析 `subscription-userinfo` 响应头。
- 下载后先校验 YAML 可解析，再写入 store。

本地文件导入：

- 读取文件内容。
- 校验 YAML。
- 保存为 profile 原始 YAML。

当前 TUI 已支持：

- 添加 URL 或本地 profile。
- 列表展示。
- 切换当前 profile。
- 删除 profile。
- 更新单个 URL profile。
- 更新全部 URL profile。

更新当前 profile 成功后，会重新生成运行时配置并尝试 reload mihomo。

## 9. mihomo API 客户端

API 客户端在 `clashtui-core-api`。

### 9.1 MihomoClient

`MihomoClient` 在 `src/client.rs`。

构造：

```rust
MihomoClient::new("127.0.0.1:9090", secret)
```

行为：

- 自动补 `http://` scheme。
- 去掉 base URL 尾斜杠。
- secret 非空时加 `Authorization: Bearer <secret>`。
- path 段使用 percent encode，避免节点名含空格、CJK 或符号时请求失败。
- 不同请求设置不同 timeout。

已封装 REST API：

| 方法 | mihomo API | 用途 |
| --- | --- | --- |
| `version()` | `GET /version` | 获取版本 |
| `ping()` | `GET /version` | 在线探测，401 也视为内核在线 |
| `configs()` | `GET /configs` | 获取通用配置 |
| `patch_configs()` | `PATCH /configs` | 切换模式、TUN 等 |
| `reload_config()` | `PUT /configs?force=` | 重载配置文件 |
| `proxies()` | `GET /proxies` | 获取所有代理，返回包裹结构 |
| `proxy()` | `GET /proxies/{name}` | 获取单个 proxy，未包裹 |
| `select_node()` | `PUT /proxies/{group}` | Selector 组选节点 |
| `unfix()` | `DELETE /proxies/{name}` | 解除非 Selector 固定 |
| `proxy_delay()` | `GET /proxies/{name}/delay` | 单节点测速 |
| `groups()` | `GET /group` | 获取代理组 |
| `group_delay()` | `GET /group/{name}/delay` | 整组测速 |
| `connections()` | `GET /connections` | 连接快照 |
| `close_connection()` | `DELETE /connections/{id}` | 关闭单条 |
| `close_all_connections()` | `DELETE /connections` | 关闭全部 |
| `restart()` | `POST /restart` | 重启内核 |
| `upgrade_core()` | `POST /upgrade?force=` | mihomo 自升级 API |
| `flush_fakeip()` | `POST /cache/fakeip/flush` | 刷 fakeip 缓存 |

错误映射：

- `401` -> `ApiError::Auth`
- `404` -> `ApiError::NotFound`
- `400` 且 body 包含 `Proxy can't update` -> `ApiError::ProxyCantUpdate`
- 其它非 2xx -> `ApiError::Status`
- JSON 解析失败 -> `ApiError::Decode`

### 9.2 数据模型

模型在 `src/models.rs`。

需要维护的协议不变量：

- `Mode` 序列化为小写：`rule`、`global`、`direct`。
- 延迟 `Delay(0)` 表示超时/不可达，渲染时不能显示为 `0ms`。
- `GET /proxies` 返回 `{ "proxies": { ... } }`。
- `GET /proxies/{name}` 返回未包裹单个 proxy。
- 代理组字段有 camelCase，例如 `testUrl`、`expectedStatus`。
- 部分配置字段使用 kebab-case，例如 `mixed-port`、`socks-port`、`log-level`；HTTP 端口字段是普通 `port`。

### 9.3 WebSocket StreamHub

`StreamHub` 在 `src/stream.rs`。

支持四路流：

| StreamKind | API | AppEvent |
| --- | --- | --- |
| `Traffic` | `/traffic` | `WsTraffic` |
| `Logs` | `/logs` | `WsLog` |
| `Connections` | `/connections?interval=1000` | `WsConnections` |
| `Memory` | `/memory` | `WsMemory` |

行为：

- 每个流一个 tokio task。
- 多路流扇入单个 `mpsc::UnboundedSender<StreamMsg>`。
- secret 通过 `?token=` 放入 WS URL。
- traffic 在源头合并到约 10fps，保留最新帧。
- 超过 15 秒无帧判定为 stale 并重连。
- 重连使用指数退避：200ms 到 30s。
- `PUT /configs` 和 `POST /restart` 可能断开 WS，调用方需要重连或等待流自动恢复。

## 10. 内核生命周期

代码在 `clashtui-domain/src/lifecycle.rs`。

状态：

| 状态 | 说明 |
| --- | --- |
| `AttachedExternal` | external-controller 可访问，但不是本进程托管 |
| `ManagedRunning(pid)` | 本进程启动并托管 mihomo 子进程 |
| `Stopped` | 未运行 |
| `Crashed(String)` | 托管进程异常退出 |

启动策略：

```text
probe GET /version
  -> 成功或 401: AttachedExternal
  -> 失败: Stopped，后续可由用户 StartCore spawn
```

托管启动命令：

```text
mihomo -d <core_dir> -f <runtime_config>
```

行为：

- 找不到二进制时返回 `DomainError::Core`。
- macOS 启动前尽力删除 quarantine xattr。
- stdout/stderr 按行转发到 TUI 日志，level 为 `core`。
- stop 托管内核时，Unix 下先 SIGTERM，等待 1.5s，再 SIGKILL 兜底。
- 外部 attached 内核不能由本进程 stop。
- restart：
  - 托管内核：`stop + start`
  - 外部内核：`POST /restart`
  - 其它状态：尝试 `start`
- reload：调用 `PUT /configs?force=true`。

## 11. 系统代理

代码在 `clashtui-domain/src/sysproxy/`。

系统代理分两层：

| 类型 | 实现 | 说明 |
| --- | --- | --- |
| service 级 | macOS `networksetup`，Linux `gsettings` | 直接修改 OS 设置 |
| env 级 | 输出 shell 片段 | 不能修改父 shell，只能 `eval` 或 `source` |

### 11.1 service 级

统一 trait：

```rust
trait ServiceProxy {
    fn enable(&self, s: &ProxySettings) -> DomainResult<()>;
    fn disable(&self) -> DomainResult<()>;
    fn is_enabled(&self) -> DomainResult<bool>;
}
```

macOS：

- 优先使用 `manual_network_service`。
- 未配置时探测活跃网络服务：
  - UDP socket 连接 `1.1.1.1:80`，获取出口 IP。
  - 通过 `ifconfig` 找设备名。
  - 通过 `networksetup -listnetworkserviceorder` 找服务名。
- 兜底使用第一个未禁用网络服务。

Linux：

- 使用 GNOME `gsettings`：
  - `org.gnome.system.proxy mode manual`
  - 设置 http/https/socks host 与 port
  - 设置 ignore-hosts
- 非 GNOME 环境可能失败。

### 11.2 env 级

生成方式：

```sh
clashtui sysproxy env
clashtui sysproxy env --off
```

启用片段包含：

- `http_proxy`
- `https_proxy`
- `all_proxy`
- 大写版本
- `no_proxy`
- `NO_PROXY`

典型用法：

```sh
eval "$(clashtui sysproxy env)"
eval "$(clashtui sysproxy env --off)"
```

TUI 中 `Effect::ToggleSysProxy` 会：

- 查询当前 OS 状态。
- 翻转 service 级代理。
- 同步写出 `proxy.sh` env 片段。

## 12. 内核升级

代码在 `clashtui-domain/src/upgrade.rs`。

流程：

```text
GET https://api.github.com/repos/MetaCubeX/mihomo/releases/latest
  -> 按当前平台选择 mihomo-{os}-{arch}-v*.gz
  -> 下载 .gz
  -> gunzip
  -> chmod +x
  -> 备份旧二进制为 .bak
  -> 原子替换
  -> 失败时尝试回滚 .bak
```

平台资产 infix：

- `darwin-arm64`
- `darwin-amd64`
- `linux-arm64`
- `linux-amd64`

限制：

- 当前只支持 macOS/Linux 的 arm64/amd64。
- GitHub API 403 会返回限流错误。
- 选择资产时排除 `compatible` 变体。

TUI 中 `Effect::UpgradeKernel` 会：

- 发进度 toast。
- 下载并安装。
- 如果当前为托管内核，升级后尝试重启。

## 13. TUI 架构

TUI 在 `clashtui-tui`。

入口：

```text
clashtui-bin main()
  -> clashtui_tui::run()
  -> Paths::resolve + ensure_dirs
  -> AppConfig::load
  -> MihomoClient::new
  -> CoreManager::new
  -> ProfileStore::load
  -> AppContext
  -> tui::init
  -> App::new
  -> App::run
```

### 13.1 AppContext

`AppContext` 是传给异步副作用的共享句柄集合：

- `client: MihomoClient`
- `core: Arc<CoreManager>`
- `config: Arc<AppConfig>`
- `paths: Arc<Paths>`
- `profiles: Arc<Mutex<ProfileStore>>`
- `theme: Theme`
- `event_tx: mpsc::UnboundedSender<AppEvent>`

它不是全局状态。后台任务完成后通过 `ctx.emit(AppEvent::...)` 回灌主循环。

### 13.2 主事件循环

`App::run()` 中只有三个 `tokio::select!` arm：

1. 中央 `AppEvent` mpsc 接收。
2. `TuiEventStream` 输入/重绘请求。
3. 16ms tick interval。

数据流：

```text
keyboard/resize/draw request
  -> AppEvent
  -> App::on_app_event
  -> router or tab component
  -> Vec<Effect>
  -> effect_runner::apply
  -> async task
  -> AppEvent
  -> tab.apply_event
  -> draw
```

WebSocket：

```text
StreamHub tasks
  -> StreamMsg
  -> forward_stream_msg()
  -> AppEvent::Ws*
  -> tab.apply_event
```

### 13.3 AppEvent 与 Effect

定义在 `clashtui-tui/src/event.rs`。

`AppEvent` 是“引擎 -> UI”：

- 输入和渲染：`Key`、`Mouse`、`Resize`、`Draw`、`Tick`
- 提示：`Toast`、`Error`
- 数据加载：`CoreStatus`、`Version`、`ConfigLoaded`、`ProxiesLoaded`、`ProfilesChanged` 等
- WS 流：`WsTraffic`、`WsLog`、`WsConnections`、`WsMemory`
- 升级：`UpgradeProgress`
- 退出：`Quit`

`Effect` 是“UI -> 引擎”：

- 同步 UI：切 tab、help、toast、quit
- 数据刷新：status/proxies/profiles
- 内核生命周期：start/stop/restart
- 配置：mode/TUN
- 代理：选节点、unfix、测速
- Profile：add/switch/delete/update/update all
- 系统代理、流、连接、升级、编辑 mixin

规则：

- Component 只返回 `Effect`，不要直接做 I/O。
- `effect_runner` 集中执行副作用。
- 异步副作用完成后只能发送具体 `AppEvent`，不要把闭包发回 UI。

### 13.4 Component 约定

trait 在 `clashtui-tui/src/component.rs`。

每个 tab 实现：

- `id()`
- `handle_key() -> (Handled, Vec<Effect>)`
- `apply_event() -> Vec<Effect>`
- `on_focus() -> Vec<Effect>`
- `on_blur() -> Vec<Effect>`
- `tick() -> bool`
- `capturing() -> bool`
- `draw(&self, ...)`
- `footer_hints()`

重要纪律：

- `draw()` 必须是只读渲染，不修改状态、不 await、不持锁。
- 按键处理只更新轻量 UI 状态或返回 `Effect`。
- 网络、文件、进程、OS 命令都放到 `effect_runner` 或 domain/core-api。
- `capturing()` 为 true 时，App 会把所有键直送组件，绕过全局快捷键，适合文本输入或确认弹窗。

### 13.5 按键路由

路由器在 `clashtui-tui/src/router.rs`，是纯函数：

```rust
route(focus, key) -> Routed
```

优先级：

```text
Popup -> GlobalChord -> Help -> ReservedGlobal -> ActiveTab -> GlobalFallback
```

全局保留键：

- `1` - `7`：直跳 tab
- `q`：退出
- `Ctrl+C`：退出
- `?`：帮助

全局 chord：

- `Ctrl+R`：重启内核
- `Ctrl+P`：切换系统代理

ActiveTab 键：

- 方向键
- `Enter`
- `Esc`
- 普通字符动作键

GlobalFallback：

- `Tab` / `Shift-Tab`：切 tab
- `F5`：刷新
- 裸 `r`：组件未消费时作为刷新兜底

### 13.6 终端封装

`tui.rs` 负责：

- 进入 raw mode。
- 进入 alternate screen。
- panic hook 中恢复终端。
- 把 crossterm `EventStream` 翻译成 `AppEvent`。
- 过滤 Key release，避免重复触发。
- 提供 `FrameRequester` 请求重绘。

## 14. Tab 行为清单

Tab 顺序由 `TabId::ORDER` 决定：

1. `Status`
2. `Proxies`
3. `Profiles`
4. `Connections`
5. `Logs`
6. `Traffic`
7. `Settings`

### 14.1 Status

文件：`tabs/status.rs`

焦点行为：

- `RefreshStatus`
- `StartStream(Traffic)`
- `StartStream(Memory)`

消费事件：

- `CoreStatus`
- `Version`
- `ConfigLoaded`
- `WsTraffic`
- `WsMemory`
- `WsConnected` / `WsDisconnected`
- `ProfilesChanged`

按键：

- `s`：启动内核
- `S`：停止内核
- `R`：重启内核

### 14.2 Proxies

文件：`tabs/proxies.rs`

焦点行为：

- `RefreshProxies`

布局：

- 左栏代理组。
- 右栏当前组节点。

按键：

- `←/→`：切换栏。
- `↑/↓`：移动选择。
- `Enter`：Selector 组选节点。
- `t`：测试当前节点。
- `T`：测试当前组。
- `u`：解除当前组固定选择。

消费事件：

- `ProxiesLoaded`
- `DelayResult`
- `GroupDelayResult`

注意：

- 只有 `Proxy::is_selector()` 为 true 的组可以手动选节点。
- 非 Selector 组 Enter 会 toast，而不是调用 API。

### 14.3 Profiles

文件：`tabs/profiles.rs`

焦点行为：

- `RefreshProfiles`

按键：

- `↑/↓`：选择。
- `Enter`：切换当前 profile。
- `a`：添加 profile。
- `d`：删除，弹确认。
- `u`：更新选中 profile。
- `U`：更新全部 profile。

添加流程：

1. 输入名称。
2. 输入 URL 或文件路径。
3. 根据 `http://` 或 `https://` 判断 URL / 本地文件。
4. 发 `Effect::AddProfile`。

确认删除：

- `y` / `Enter`：确认。
- `n` / `Esc`：取消。

`capturing()`：

- 添加流程或删除确认打开时返回 true。

### 14.4 Connections

文件：`tabs/connections.rs`

焦点行为：

- `on_focus`: `StartStream(Connections)`
- `on_blur`: `StopStream(Connections)`

按键：

- `↑/↓`：选择连接。
- `dd`：关闭单条连接。
- `a`：关闭全部连接，弹确认。
- `p`：暂停/继续更新视图。

消费事件：

- `WsConnections`

### 14.5 Logs

文件：`tabs/logs.rs`

焦点行为：

- `StartStream(Logs)`

当前代码没有在失焦时停止 logs 流；如果要改成只在聚焦时消费，可给 `LogsTab` 增加 `on_blur() -> StopStream(Logs)`，但要同时评估切 tab 后是否仍希望保留后台日志。

按键：

- `↑/↓`：滚动。
- `f`：循环过滤级别：all -> info -> warning -> error -> core -> all。
- `p`：暂停/继续。
- `c`：清空。

日志环：

- 最大 5000 条。
- 超出后从前面丢弃。

注意：

- `paused` 当前只影响 UI 状态和提示，不阻止新日志进入环。

### 14.6 Traffic

文件：`tabs/traffic.rs`

焦点行为：

- `StartStream(Traffic)`
- `StartStream(Memory)`

展示：

- 上行速率 sparkline。
- 下行速率 sparkline。
- 当前内存。

历史点：

- `HISTORY = 120`

### 14.7 Settings

文件：`tabs/settings.rs`

焦点行为：

- `RefreshStatus`

设置项：

- 代理模式：Rule -> Global -> Direct -> Rule。
- TUN 模式：开关。
- 系统代理：切换 service 级代理。
- 编辑 Mixin：挂起 TUI 并启动 `$EDITOR`。
- 升级内核。
- 重启内核。

按键：

- `↑/↓`：选择。
- `Enter` / `→` / 空格：执行或切换。

注意：

- `sysproxy_on` 当前是根据 toast 文本推断的展示状态，不是持久可靠状态。

## 15. 添加新功能的推荐路径

### 15.1 添加新的 mihomo HTTP API

1. 在 `clashtui-core-api/src/models.rs` 添加请求/响应类型。
2. 在 `clashtui-core-api/src/client.rs` 添加 `MihomoClient` 方法。
3. 在 `crates/clashtui-core-api/tests/mock_server.rs` 添加 wire 行为测试。
4. 如果 TUI 需要调用：
   - 在 `event.rs` 添加 `Effect` 和必要的 `AppEvent`。
   - 在 `effect_runner.rs` 实现异步任务。
   - 在对应 tab 的 `handle_key()` 或 `on_focus()` 返回该 `Effect`。
   - 在 tab 的 `apply_event()` 消费结果。

不要让 TUI 直接构造 URL 或直接调用 `reqwest`。

### 15.2 添加新的业务能力

1. 优先判断是否属于 domain：
   - 文件读写
   - profile/config/mixin 规则
   - OS 命令
   - 内核二进制管理
2. 在 `clashtui-domain` 添加纯业务 API。
3. 补 domain 层单元测试。
4. TUI 只通过 `Effect` 触发。

### 15.3 添加新的 tab

1. 在 `event.rs` 的 `TabId` 添加枚举值，并更新 `ORDER`、`title()`。
2. 在 `tabs/` 新建文件，实现 `Component`。
3. 在 `tabs/mod.rs` 导出并在 `build_tab()` 注册。
4. 如有副作用，添加 `Effect`、`AppEvent` 和 `effect_runner` 逻辑。
5. 增加路由测试或组件 headless 测试。

### 15.4 添加新快捷键

按键有三类：

- 必须全局生效：改 `router.rs` 的 reserved global 或 chord。
- 只属于当前 tab：在该 tab 的 `handle_key()` 中处理。
- 组件不消费时兜底：改 `global_fallback()`。

不要在 `App::on_key()` 里零散堆条件，除非是跨组件路由机制本身。

### 15.5 添加配置字段

1. 在 `AppConfig` 或子配置结构中添加字段。
2. 给字段加 `#[serde(default)]` 或默认函数，保证老配置兼容。
3. 更新 `Default` 实现。
4. 增加 roundtrip / missing fields 测试。
5. 如需要展示或修改，在 Settings 或对应 tab 中通过 `Effect` 接入。

### 15.6 修改运行时配置生成

优先在 `mixin.rs` 中修改，注意：

- 保持 raw profile 不变。
- 保持 YAML 顺序。
- `force_inject()` 的 external-controller 不能被用户配置覆盖掉。
- 涉及数组顺序必须增加测试。

### 15.7 添加 WebSocket 流

1. 在 `clashtui-core-api/src/stream.rs` 添加 `StreamKind` 和解析逻辑。
2. 添加 `StreamMsg` 变体。
3. 在 `clashtui-tui/src/event.rs` 添加 `AppEvent` 和 `StreamId`。
4. 在 `effect_runner::to_kind()` 和 `forward_stream_msg()` 映射。
5. 在目标 tab 的 `on_focus()` / `on_blur()` 管理流生命周期。

## 16. 并发与状态规则

项目当前并发模型相对简单：

- UI 主循环单任务持有 `App`。
- 后台副作用使用 `tokio::spawn`。
- 后台任务不能直接改 UI 状态，只能发 `AppEvent`。
- `ProfileStore` 用 `Arc<Mutex<_>>` 共享。
- `CoreManager` 内部用 `Arc<Mutex<Option<Child>>>` 和 `Arc<Mutex<CoreStatus>>`。
- `MihomoClient` 可 cheap clone。
- `StreamHub` 管理 WS task，drop 时 abort。

开发注意：

- 不要在 `draw()` 中加锁或 await。
- 不要持有 `MutexGuard` 跨不必要的 await。
- 涉及 profile store 时，读完需要的内容后尽快 `drop(store)`。
- `AppEvent` 和 `Effect` 尽量保持具体、可调试，不使用闭包。
- 需要重绘时返回 true 或发送 `AppEvent::Draw`。

## 17. 错误处理约定

crate 分层错误：

- `clashtui-core-api::ApiError`
- `clashtui-domain::DomainError`
- `clashtui-bin` / TUI 顶层使用 `color-eyre`

TUI 中：

- 非致命错误转为 `AppEvent::Error(String)`，最终显示为 toast。
- 后台任务不 panic，不直接打印到终端。
- TUI 模式下 stdout/stderr 输出会污染画面，避免在运行中 `println!`。

API 错误可恢复性：

- `ApiError::Http` 和 `ApiError::Ws` 被认为可恢复。
- `Auth` 通常需要用户改 secret。
- `ProxyCantUpdate` 是协议层可预期错误，不应当作为崩溃处理。

## 18. 测试策略

当前测试覆盖重点：

- core-api：
  - base URL 标准化。
  - path encode。
  - 状态码映射。
  - mode/delay/proxy/config 序列化不变量。
  - mock server 验证 wire 行为。
- domain：
  - config 默认值和持久化。
  - paths 组合。
  - profile store 增删改查和订阅信息计算。
  - mixin 合并语义。
  - YAML 深合并和保序。
  - schedule staleness。
  - upgrade 安装/备份。
  - sysproxy 片段和 macOS 解析纯函数。
- tui：
  - router 纯函数优先级。
  - widgets 状态逻辑。

推荐新增测试：

- 对纯函数优先写单元测试。
- 对 API 行为用 `tests/mock_server.rs`。
- 对 TUI 按键行为可直接构造组件，断言 `handle_key() -> Effect`。
- 对 effect runner 中的异步行为，如涉及外部网络/OS 命令，优先把可测逻辑下沉到 domain/core-api。

验证命令：

```sh
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets
```

## 19. 发布与版本

版本来源：

- crate 版本在 workspace `Cargo.toml` 的 `[workspace.package]`。
- `clashtui-bin/build.rs` 会在编译期设置 `CLASHTUI_VERSION`。
- 如果处于 git 仓库，会追加短 hash 和 dirty 标记。
- 非 git 检出时只使用 package version。

release 构建配置：

```toml
[profile.release]
lto = "thin"
strip = true
```

当前仓库目录如果没有 `.git`，`build.rs` 会正常降级，不影响构建。

## 20. 当前实现边界

已实现：

- TUI 主循环、tab、路由、组件模型。
- Status/Proxies/Profiles/Connections/Logs/Traffic/Settings。
- mihomo HTTP API 基础操作。
- 四路 WebSocket 流。
- profile 添加/切换/删除/更新。
- mixin/override 生成运行时配置。
- 内核 attach/spawn/stop/restart/reload。
- service/env 系统代理。
- 内核升级下载与安装。
- 自动更新调度入口。

当前限制或待完善：

- README 中的多个 CLI 子命令仍是预留骨架。
- TUI 中 AppConfig 加载后被 `Arc` 固定，运行时修改 `config.toml` 不会自动热加载。
- `system_proxy.service_enabled` / `env_enabled` 当前不是可靠状态源。
- Logs tab 目前聚焦后启动日志流，失焦不停止。
- 自动更新调度通过特殊 `SubUpdated("__auto_update_all__")` 哨兵触发，后续可改成专门 `AppEvent`。
- Windows 没有系统代理和内核升级平台支持。
- TUI 日志初始化当前仍写 stderr，注释中提到后续改文件 appender。
- 升级使用 GitHub latest API，可能被限流。

## 21. 排障指南

### TUI 打开后无数据

检查：

- `config.toml` 的 `external_controller` 是否正确。
- mihomo 是否已启动。
- secret 是否匹配。
- `GET /version` 是否可访问。

如果 API 返回 401，`ping()` 会认为内核在线，但 `version()` 和其它 API 会报 `Auth`。

### 启动内核失败

检查：

- `mihomo_binary` 是否配置正确。
- 默认 `bin/mihomo` 是否存在。
- 文件是否有执行权限。
- macOS 是否仍被 Gatekeeper 拦截。
- `core/config.yaml` 是否已由 profile 生成。

### 切换 profile 后无效

检查：

- 当前 profile 是否有原始 YAML。
- `mixin.yaml` / `override.yaml` 是否是合法 YAML。
- `core/config.yaml` 是否生成。
- mihomo 是否在线，reload 是否成功。

### 系统代理失败

macOS：

- 确认 `networksetup` 可用。
- 如自动探测网络服务失败，设置 `manual_network_service`。

Linux：

- 确认桌面环境支持 GNOME `gsettings`。
- 非 GNOME 环境使用 `clashtui sysproxy env`。

### TUN 开启后不生效

TUN 需要 mihomo 进程具备 root/CAP_NET_ADMIN 等权限。ClashTUI 只发 `PATCH /configs`，不会自动提权。

### 节点显示 timeout

`Delay(0)` 表示超时/不可达，不是 0ms。检查测速 URL、代理节点、网络环境和 timeout 设置。

## 22. 维护清单

提交改动前建议至少确认：

- `cargo fmt --check`
- `cargo test --workspace`
- 涉及 API 协议时补 mock server 测试。
- 涉及 mixin/profile/config 时补 domain 单元测试。
- 涉及按键路由时补 router 单元测试。
- 涉及 tab 行为时确认 `footer_hints()` 与实际按键一致。
- 涉及 WS reload/restart 时确认是否需要 `ReconnectStreams`。

文档维护原则：

- README 保持用户视角。
- 本文档保持开发者视角。
- 新增模块时同步更新“目录结构”“扩展路径”“当前实现边界”。
