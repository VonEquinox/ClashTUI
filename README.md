# ClashTUI

纯终端（TUI）的 [Mihomo](https://github.com/MetaCubeX/mihomo)（Clash.Meta）管理器，
用 Rust 编写，仿 Claude Code 的方向键导航体验。主平台 macOS（arm64），次要 Linux。

## 特性

- **内核生命周期**：自动探测已运行内核（attach）或托管子进程（spawn），启动/停止/重启。
- **订阅 / Profile 管理**：URL / 本地文件导入、列表、切换、删除、手动 / 定时更新；解析
  `subscription-userinfo` 流量与到期信息。
- **代理组与节点**：TwoPane 组/节点浏览、选节点（Selector）、单节点 / 整组测速。
- **系统代理**：service 级（macOS `networksetup` + 活跃服务探测 / Linux `gsettings`）
  与 env 级（`eval $(clashtui sysproxy env)`）两维度。
- **实时面板**：日志（`/logs`）、连接监控（`/connections`，关单条/全部）、流量曲线
  （`/traffic` + `/memory`）。
- **模式 / TUN**：Rule / Global / Direct 切换、TUN 开关（含提权警告）。
- **配置分层**：原始订阅 → mixin（prepend/append/override-by-name + 深合并）→ override →
  强制注入 external-controller，生成运行时配置并 reload。`$EDITOR` 编辑 mixin。
- **内核升级**：从 GitHub releases 下载、gunzip、原子替换 + `.bak` 回滚。

## 架构

Cargo workspace，四个 crate：

| crate | 职责 |
|-------|------|
| `clashtui-core-api` | mihomo 外部控制器的纯异步客户端（HTTP + 4 路 WS），可脱离终端测试 |
| `clashtui-domain`   | config / profile / mixin / lifecycle / sysproxy / upgrade / schedule |
| `clashtui-tui`      | ratatui + crossterm 前端：App 壳 + Component + 声明式 Effect + 各 tab |
| `clashtui-bin`      | CLI 入口（`clashtui` 二进制） |

核心设计：单 `tokio::select!` 事件循环（3 arm）+ StreamHub 扇入 4 路 WS；UI→引擎走
声明式 `Effect` 枚举（可 headless 单测）；引擎→UI 走 `AppEvent`；状态经 `AppContext`
按引用传递，无全局可变状态。

## 快捷键

- `←/→`、`Tab/Shift-Tab`、`1`-`7`：切换 tab
- `↑/↓`：列表内移动；`Enter`：选择 / 确认；`Esc`：返回
- `Ctrl+R` 重启内核 · `Ctrl+P` 切系统代理 · `F5` 刷新 · `?` 帮助 · `q`/`Ctrl+C` 退出
- 各 tab 专属键见底部提示栏。

## 构建运行

```sh
cargo build --release
./target/release/clashtui            # 启动 TUI
./target/release/clashtui version    # 版本
./target/release/clashtui sysproxy env   # 打印可 source 的代理 env 片段
```

配置位于 `~/Library/Application Support/ClashTUI/`（macOS）或 `~/.config/clashtui/`（Linux）。

## 开发

完整开发文档见 [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)。

```sh
cargo test --workspace        # 全部单元 + 集成测试
cargo clippy --workspace --all-targets
cargo fmt --check
```

> 说明：依赖 `ratatui 0.30` / `crossterm 0.29` / `reqwest 0.13`（feature `rustls`）。
> YAML 经保序 `Mapping` 抽象，默认后端 `serde_yaml_ng`，可用 feature `yaml-noya` 切到 `noyalib`。
