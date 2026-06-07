//! 集中执行 [`Effect`]。
//!
//! 同步副作用直接改 [`App`] 状态；异步副作用（网络/进程）spawn 到 tokio，
//! 完成后以一个**具体的 [`AppEvent`]**（而非闭包）回灌总线——
//! 从而保持 render-no-mutate 且可在 headless 单测中断言。

use std::{collections::HashMap, time::Duration};

use clashtui_core_api::{Mode, StreamKind, StreamMsg};

use crate::app::App;
use crate::context::AppContext;
use crate::event::{AppEvent, Effect, ProgressUpdate, StreamId};

/// 执行一批 Effect。返回是否需要重绘。
pub fn apply_all(app: &mut App, effects: impl IntoIterator<Item = Effect>) -> bool {
    let mut redraw = false;
    for e in effects {
        redraw |= apply(app, e);
    }
    redraw
}

/// 执行单个 Effect。返回 true 表示需要重绘。
pub fn apply(app: &mut App, effect: Effect) -> bool {
    match effect {
        // ---- UI 同步 ----
        Effect::SwitchTab(tab) => {
            app.switch_tab(tab);
            true
        }
        Effect::ToggleHelp => {
            app.help_open = !app.help_open;
            true
        }
        Effect::Toast(msg) => {
            app.set_toast(msg);
            true
        }
        Effect::Quit => {
            app.should_quit = true;
            false
        }

        // ---- 数据刷新（异步） ----
        Effect::RefreshStatus => {
            spawn_refresh_status(app.ctx());
            false
        }
        Effect::RefreshProxies => {
            spawn_refresh_proxies(app.ctx());
            false
        }
        Effect::RefreshProfiles => {
            spawn_refresh_profiles(app.ctx());
            false
        }

        // ---- 内核生命周期 ----
        Effect::StartCore => {
            spawn_core_action(app.ctx(), CoreAction::Start);
            false
        }
        Effect::StopCore => {
            spawn_core_action(app.ctx(), CoreAction::Stop);
            false
        }
        Effect::RestartCore => {
            spawn_core_action(app.ctx(), CoreAction::Restart);
            false
        }

        // ---- 配置 ----
        Effect::SwitchMode(mode) => {
            spawn_switch_mode(app.ctx(), mode);
            false
        }
        Effect::ToggleTun(enable) => {
            spawn_toggle_tun(app.ctx(), enable);
            false
        }
        Effect::SetProxyPorts {
            http_port,
            socks_port,
            mixed_port,
        } => {
            spawn_set_proxy_ports(app.ctx(), http_port, socks_port, mixed_port);
            false
        }
        Effect::SetKeepCoreRunning(enable) => {
            spawn_set_keep_core_running(app.ctx(), enable);
            false
        }

        // ---- 代理 ----
        Effect::SelectNode { group, node } => {
            spawn_select_node(app.ctx(), group, node);
            false
        }
        Effect::UnfixGroup(group) => {
            spawn_unfix(app.ctx(), group);
            false
        }
        Effect::TestNode(node) => {
            spawn_test_node(app.ctx(), node);
            false
        }
        Effect::TestGroup(group) => {
            spawn_test_group(app.ctx(), group);
            false
        }

        // ---- 流 ----
        Effect::StartStream(id) => {
            app.start_stream(to_kind(id));
            false
        }
        Effect::StopStream(id) => {
            app.stop_stream(to_kind(id));
            false
        }
        Effect::ReconnectStreams => {
            app.reconnect_streams();
            false
        }

        // ---- 连接 ----
        Effect::CloseConn(id) => {
            spawn_close_conn(app.ctx(), Some(id));
            false
        }
        Effect::CloseAllConns => {
            spawn_close_conn(app.ctx(), None);
            false
        }

        // ---- Profile（M2） ----
        Effect::AddProfile {
            name,
            source,
            is_url,
        } => {
            spawn_add_profile(app.ctx(), name, source, is_url);
            false
        }
        Effect::AddProfileFromUrl(url) => {
            spawn_add_profile_from_url(app.ctx(), url);
            false
        }
        Effect::SwitchProfile(name) => {
            spawn_switch_profile(app.ctx(), name);
            false
        }
        Effect::DeleteProfile(name) => {
            spawn_delete_profile(app.ctx(), name);
            false
        }
        Effect::UpdateProfile(name) => {
            spawn_update_profile(app.ctx(), name);
            false
        }
        Effect::UpdateAllProfiles => {
            spawn_update_all(app.ctx());
            false
        }

        // ---- 系统代理（M4） ----
        Effect::ToggleSysProxy => {
            spawn_toggle_sysproxy(app.ctx());
            false
        }

        // ---- 升级（M11） ----
        Effect::UpgradeKernel => {
            spawn_upgrade(app.ctx());
            false
        }

        // ---- Mixin 编辑（M8）：置标志，由主循环挂起 TUI 后执行 ----
        Effect::EditMixin => {
            app.request_edit_mixin();
            false
        }
    }
}

fn to_kind(id: StreamId) -> StreamKind {
    match id {
        StreamId::Traffic => StreamKind::Traffic,
        StreamId::Logs => StreamKind::Logs,
        StreamId::Connections => StreamKind::Connections,
        StreamId::Memory => StreamKind::Memory,
    }
}

// ---------- 异步任务：完成后回灌具体 AppEvent ----------

fn emit_progress(
    ctx: &AppContext,
    id: &str,
    label: impl Into<String>,
    current: u64,
    total: Option<u64>,
) {
    ctx.emit(AppEvent::Progress(ProgressUpdate {
        id: id.to_string(),
        label: label.into(),
        current,
        total,
        done: false,
    }));
}

fn finish_progress(ctx: &AppContext, id: &str) {
    ctx.emit(AppEvent::Progress(ProgressUpdate {
        id: id.to_string(),
        label: String::new(),
        current: 0,
        total: Some(0),
        done: true,
    }));
}

fn finish_task(ctx: &AppContext, key: &str) {
    ctx.emit(AppEvent::TaskDone(key.to_string()));
}

fn load_config_snapshot(ctx: &AppContext) -> clashtui_domain::AppConfig {
    clashtui_domain::AppConfig::load(&ctx.paths.config_file())
        .unwrap_or_else(|_| (*ctx.config).clone())
}

fn spawn_refresh_status(ctx: AppContext) {
    tokio::spawn(async move {
        let task_key = "refresh_status";
        ctx.emit(AppEvent::AppConfigLoaded(Box::new(load_config_snapshot(
            &ctx,
        ))));
        let status = ctx.core.probe().await;
        ctx.emit(AppEvent::CoreStatus(status.clone()));
        if status.is_running() {
            let ver = ctx.client.version().await.ok();
            ctx.emit(AppEvent::Version(ver));
            if let Ok(cfg) = ctx.client.configs().await {
                ctx.emit(AppEvent::ConfigLoaded(Box::new(cfg)));
            }
        } else {
            ctx.emit(AppEvent::Version(None));
        }
        finish_task(&ctx, task_key);
    });
}

fn spawn_refresh_proxies(ctx: AppContext) {
    spawn_refresh_proxies_inner(ctx, true);
}

fn spawn_refresh_proxies_background(ctx: AppContext) {
    spawn_refresh_proxies_inner(ctx, false);
}

fn spawn_refresh_proxies_inner(ctx: AppContext, finish_inflight: bool) {
    tokio::spawn(async move {
        let task_key = "refresh_proxies";
        refresh_proxies(&ctx).await;
        if finish_inflight {
            finish_task(&ctx, task_key);
        }
    });
}

async fn refresh_proxies(ctx: &AppContext) {
    match (ctx.client.groups().await, ctx.client.proxies().await) {
        (Ok(groups), Ok(all)) => {
            ctx.emit(AppEvent::ProxiesLoaded {
                groups,
                all: all.proxies,
            });
        }
        (Err(e), _) | (_, Err(e)) => {
            ctx.emit(AppEvent::Error(format!("加载代理失败: {e}")));
        }
    }
}

enum CoreAction {
    Start,
    Stop,
    Restart,
}

fn spawn_core_action(ctx: AppContext, action: CoreAction) {
    tokio::spawn(async move {
        let task_key = "core_action";
        let (progress_id, label) = match action {
            CoreAction::Start => ("core_action", "启动内核"),
            CoreAction::Stop => ("core_action", "停止内核"),
            CoreAction::Restart => ("core_action", "重启内核"),
        };
        emit_progress(&ctx, progress_id, label, 0, None);
        let result = match action {
            CoreAction::Start => ctx.core.start().await,
            CoreAction::Stop => ctx.core.stop().await,
            CoreAction::Restart => ctx.core.restart().await,
        };
        finish_progress(&ctx, progress_id);
        match result {
            Ok(status) => {
                ctx.emit(AppEvent::CoreStatus(status));
                // 重启/启动后重连流。
                if matches!(action, CoreAction::Restart | CoreAction::Start) {
                    ctx.emit(AppEvent::Toast("内核已操作，重连数据流".into()));
                }
            }
            Err(e) => ctx.emit(AppEvent::Error(e.to_string())),
        }
        finish_task(&ctx, task_key);
    });
}

fn spawn_switch_mode(ctx: AppContext, mode: Mode) {
    tokio::spawn(async move {
        use clashtui_core_api::ConfigPatch;
        let task_key = "switch_mode";
        let progress_id = "switch_mode";
        emit_progress(
            &ctx,
            progress_id,
            format!("切换模式为 {}", mode.as_str()),
            0,
            Some(1),
        );
        match ctx.client.patch_configs(&ConfigPatch::mode(mode)).await {
            Ok(()) => {
                emit_progress(
                    &ctx,
                    progress_id,
                    format!("切换模式为 {}", mode.as_str()),
                    1,
                    Some(1),
                );
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Toast(format!("模式已切换为 {}", mode.as_str())));
                if let Ok(cfg) = ctx.client.configs().await {
                    ctx.emit(AppEvent::ConfigLoaded(Box::new(cfg)));
                }
            }
            Err(e) => {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(format!("切换模式失败: {e}")));
            }
        }
        finish_task(&ctx, task_key);
    });
}

fn spawn_toggle_tun(ctx: AppContext, enable: bool) {
    tokio::spawn(async move {
        use clashtui_core_api::ConfigPatch;
        let task_key = "toggle_tun";
        let progress_id = "toggle_tun";
        emit_progress(
            &ctx,
            progress_id,
            if enable { "开启 TUN" } else { "关闭 TUN" },
            0,
            Some(1),
        );
        // 开启 TUN 但内核（由本进程托管）未提权时，给出警告——TUN 需内核持有
        // CAP_NET_ADMIN/root，本 TUI 绝不自动提权。
        if enable && ctx.core.status().await.is_managed() && !clashtui_domain::util::is_elevated() {
            ctx.emit(AppEvent::Toast(
                "警告：内核未以 root 运行，TUN 可能无法生效（需用提权的内核）".into(),
            ));
        }
        match ctx.client.patch_configs(&ConfigPatch::tun(enable)).await {
            Ok(()) => {
                emit_progress(
                    &ctx,
                    progress_id,
                    if enable { "开启 TUN" } else { "关闭 TUN" },
                    1,
                    Some(1),
                );
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Toast(format!(
                    "TUN 已{}",
                    if enable { "开启" } else { "关闭" }
                )));
                if let Ok(cfg) = ctx.client.configs().await {
                    ctx.emit(AppEvent::ConfigLoaded(Box::new(cfg)));
                }
            }
            Err(e) => {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(format!("切换 TUN 失败: {e}")));
            }
        }
        finish_task(&ctx, task_key);
    });
}

fn spawn_set_proxy_ports(ctx: AppContext, http_port: u16, socks_port: u16, mixed_port: u16) {
    tokio::spawn(async move {
        let task_key = "set_proxy_ports";
        let progress_id = "set_proxy_ports";
        emit_progress(&ctx, progress_id, "保存代理端口", 0, Some(3));

        let mut app_config = load_config_snapshot(&ctx);
        app_config.system_proxy.http_port = http_port;
        app_config.system_proxy.socks_port = socks_port;
        app_config.system_proxy.mixed_port = mixed_port;
        if let Err(e) = app_config.save(&ctx.paths.config_file()) {
            finish_progress(&ctx, progress_id);
            ctx.emit(AppEvent::Error(format!("保存端口失败: {e}")));
            finish_task(&ctx, task_key);
            return;
        }

        emit_progress(&ctx, progress_id, "重建运行时配置", 1, Some(3));
        if ctx.profiles.lock().await.current().is_some() {
            if let Err(e) = apply_current_profile(&ctx, Some((progress_id, 1, 3))).await {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(format!("端口已保存，重载配置失败: {e}")));
                finish_task(&ctx, task_key);
                return;
            }
        } else {
            ctx.emit(AppEvent::Toast(
                "端口已保存；当前没有 profile，加载订阅后生效".into(),
            ));
        }

        emit_progress(&ctx, progress_id, "刷新配置", 3, Some(3));
        finish_progress(&ctx, progress_id);
        if let Ok(cfg) = ctx.client.configs().await {
            ctx.emit(AppEvent::ConfigLoaded(Box::new(cfg)));
        }
        ctx.emit(AppEvent::Toast(format!(
            "代理端口已保存：http {http_port} · socks {socks_port} · mixed {mixed_port}"
        )));

        ctx.emit(AppEvent::AppConfigLoaded(Box::new(load_config_snapshot(
            &ctx,
        ))));
        finish_task(&ctx, task_key);
    });
}

fn spawn_set_keep_core_running(ctx: AppContext, enable: bool) {
    tokio::spawn(async move {
        let task_key = "set_keep_core_running";
        let mut app_config = load_config_snapshot(&ctx);
        app_config.keep_core_running = enable;
        match app_config.save(&ctx.paths.config_file()) {
            Ok(()) => {
                let was_managed = ctx.core.status().await.is_managed();
                ctx.core.set_keep_running_on_drop(enable);
                if was_managed && enable {
                    match ctx.core.restart().await {
                        Ok(status) => ctx.emit(AppEvent::CoreStatus(status)),
                        Err(e) => {
                            ctx.emit(AppEvent::Error(format!(
                                "设置已保存，重启内核以保留运行失败: {e}"
                            )));
                            finish_task(&ctx, task_key);
                            return;
                        }
                    }
                }
                ctx.emit(AppEvent::AppConfigLoaded(Box::new(load_config_snapshot(
                    &ctx,
                ))));
                let suffix = if !enable && was_managed {
                    "；当前托管内核重启后按退出即停生效"
                } else {
                    ""
                };
                ctx.emit(AppEvent::Toast(format!(
                    "退出后保留内核已{}{}",
                    if enable { "开启" } else { "关闭" },
                    suffix
                )));
            }
            Err(e) => ctx.emit(AppEvent::Error(format!("保存退出保留内核设置失败: {e}"))),
        }
        finish_task(&ctx, task_key);
    });
}

fn spawn_select_node(ctx: AppContext, group: String, node: String) {
    tokio::spawn(async move {
        let task_key = format!("select_node:{group}");
        let progress_id = "select_node";
        emit_progress(&ctx, progress_id, format!("{group} -> {node}"), 0, Some(1));
        match ctx.client.select_node(&group, &node).await {
            Ok(()) => {
                emit_progress(&ctx, progress_id, format!("{group} -> {node}"), 1, Some(1));
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Toast(format!("{group} → {node}")));
                spawn_refresh_proxies_background(ctx.clone());
            }
            Err(e) => {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(e.to_string()));
            }
        }
        finish_task(&ctx, &task_key);
    });
}

fn spawn_unfix(ctx: AppContext, group: String) {
    tokio::spawn(async move {
        let task_key = format!("unfix_group:{group}");
        let progress_id = "unfix_group";
        emit_progress(&ctx, progress_id, format!("解除固定 {group}"), 0, Some(1));
        match ctx.client.unfix(&group).await {
            Ok(()) => {
                emit_progress(&ctx, progress_id, format!("解除固定 {group}"), 1, Some(1));
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Toast(format!("已解除固定: {group}")));
                spawn_refresh_proxies_background(ctx.clone());
            }
            Err(e) => {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(e.to_string()));
            }
        }
        finish_task(&ctx, &task_key);
    });
}

fn spawn_test_node(ctx: AppContext, node: String) {
    let url = ctx.config.test_url.clone();
    let timeout = ctx.config.test_timeout_ms;
    tokio::spawn(async move {
        let task_key = format!("test_node:{node}");
        let progress_id = "test_node";
        emit_progress(&ctx, progress_id, format!("测速 {node}"), 0, Some(1));
        match ctx.client.proxy_delay(&node, &url, timeout).await {
            Ok(delay) => {
                emit_progress(&ctx, progress_id, format!("测速 {node}"), 1, Some(1));
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::DelayResult { node, delay });
            }
            Err(e) => {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(format!("{node} 测速失败: {e}")));
            }
        }
        finish_task(&ctx, &task_key);
    });
}

fn spawn_test_group(ctx: AppContext, group: String) {
    let url = ctx.config.test_url.clone();
    let timeout = ctx.config.test_timeout_ms;
    tokio::spawn(async move {
        let task_key = format!("test_group:{group}");
        let progress_id = "test_group";
        emit_progress(&ctx, progress_id, format!("整组测速 {group}"), 0, Some(1));
        match ctx.client.group_delay(&group, &url, timeout).await {
            Ok(map) => {
                emit_progress(&ctx, progress_id, format!("整组测速 {group}"), 1, Some(1));
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::GroupDelayResult(map));
            }
            Err(e) => {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(format!("{group} 整组测速失败: {e}")));
            }
        }
        finish_task(&ctx, &task_key);
    });
}

fn spawn_close_conn(ctx: AppContext, id: Option<String>) {
    tokio::spawn(async move {
        let task_key = id
            .as_ref()
            .map(|i| format!("close_conn:{i}"))
            .unwrap_or_else(|| "close_all_conns".into());
        let progress_id = "close_conn";
        let label = id
            .as_ref()
            .map(|i| format!("关闭连接 {i}"))
            .unwrap_or_else(|| "关闭全部连接".into());
        emit_progress(&ctx, progress_id, label.clone(), 0, Some(1));
        let r = match &id {
            Some(id) => ctx.client.close_connection(id).await,
            None => ctx.client.close_all_connections().await,
        };
        match r {
            Ok(()) => {
                emit_progress(&ctx, progress_id, label, 1, Some(1));
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Toast(
                    id.map(|i| format!("已关闭连接 {i}"))
                        .unwrap_or_else(|| "已关闭全部连接".into()),
                ));
            }
            Err(e) => {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(e.to_string()));
            }
        }
        finish_task(&ctx, &task_key);
    });
}

// ---------- 内核升级 ----------

fn spawn_upgrade(ctx: AppContext) {
    tokio::spawn(async move {
        use clashtui_domain::upgrade;
        let task_key = "upgrade_kernel";
        ctx.emit(AppEvent::UpgradeProgress("查询最新版本…".into()));
        let info = match upgrade::fetch_latest().await {
            Ok(i) => i,
            Err(e) => {
                ctx.emit(AppEvent::Error(format!("升级失败: {e}")));
                finish_task(&ctx, task_key);
                return;
            }
        };
        // 目标二进制路径。
        let target = if ctx.config.mihomo_binary.is_empty() {
            ctx.paths.default_binary()
        } else {
            std::path::PathBuf::from(&ctx.config.mihomo_binary)
        };
        ctx.emit(AppEvent::UpgradeProgress(format!(
            "下载 {} …",
            info.asset_name
        )));
        let progress_ctx = ctx.clone();
        let progress_label = format!("下载 {}", info.asset_name);
        let progress_id = "upgrade_kernel";
        let result =
            upgrade::download_and_install_with_progress(&info, &target, move |done, total| {
                emit_progress(
                    &progress_ctx,
                    progress_id,
                    progress_label.clone(),
                    done,
                    total,
                );
            })
            .await;
        finish_progress(&ctx, progress_id);
        match result {
            Ok(()) => {
                ctx.emit(AppEvent::Toast(format!("内核已升级到 {}", info.tag)));
                // 若为托管内核，重启使新二进制生效。
                if ctx.core.status().await.is_managed() {
                    let _ = ctx.core.restart().await;
                    ctx.emit(AppEvent::Toast("已重启内核".into()));
                }
            }
            Err(e) => ctx.emit(AppEvent::Error(format!("升级失败: {e}"))),
        }
        finish_task(&ctx, task_key);
    });
}

// ---------- 系统代理 ----------

fn spawn_toggle_sysproxy(ctx: AppContext) {
    tokio::spawn(async move {
        use clashtui_domain::sysproxy::{env_snippet, service_proxy, ProxySettings};
        let task_key = "toggle_sysproxy";
        let progress_id = "toggle_sysproxy";
        emit_progress(&ctx, progress_id, "切换系统代理", 0, None);
        let config = load_config_snapshot(&ctx);
        let sp = &config.system_proxy;
        let settings = ProxySettings::new(sp.http_port, sp.socks_port, sp.bypass.clone());
        let manual = if config.manual_network_service.is_empty() {
            None
        } else {
            Some(config.manual_network_service.clone())
        };
        let proxy = service_proxy(manual);

        // 读当前状态并翻转。
        let enabled = proxy.is_enabled().unwrap_or(false);
        let result = if enabled {
            proxy.disable()
        } else {
            proxy.enable(&settings)
        };
        match result {
            Ok(()) => {
                emit_progress(&ctx, progress_id, "写入 env 代理片段", 1, Some(2));
                // 同步写出 env 片段（两维度）。
                let snippet = env_snippet(&settings, !enabled);
                let _ = std::fs::write(ctx.paths.proxy_env_file(), snippet);
                emit_progress(&ctx, progress_id, "切换系统代理", 2, Some(2));
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Toast(format!(
                    "系统代理已{}",
                    if enabled { "关闭" } else { "开启" }
                )));
            }
            Err(e) => {
                finish_progress(&ctx, progress_id);
                ctx.emit(AppEvent::Error(format!("系统代理操作失败: {e}")));
            }
        }
        finish_task(&ctx, task_key);
    });
}

// ---------- Profile 异步任务 ----------

fn spawn_refresh_profiles(ctx: AppContext) {
    tokio::spawn(async move {
        let task_key = "refresh_profiles";
        let list = ctx.profiles.lock().await.list();
        ctx.emit(AppEvent::ProfilesChanged(list));
        finish_task(&ctx, task_key);
    });
}

fn spawn_add_profile(ctx: AppContext, name: String, source: String, is_url: bool) {
    let task_key = format!("add_profile:{name}");
    spawn_add_profile_inner(ctx, name, source, is_url, false, task_key);
}

fn spawn_add_profile_from_url(ctx: AppContext, url: String) {
    let name = auto_profile_name();
    let task_key = format!("add_profile_url:{url}");
    spawn_add_profile_inner(ctx, name, url, true, true, task_key);
}

fn spawn_add_profile_inner(
    ctx: AppContext,
    name: String,
    source: String,
    is_url: bool,
    activate_after_add: bool,
    task_key: String,
) {
    tokio::spawn(async move {
        use clashtui_domain::profile::{download, ProfileKind, ProfileMeta};
        let progress_id = format!("add_profile:{name}");
        let dl = if is_url {
            let progress_ctx = ctx.clone();
            let progress_id_for_download = progress_id.clone();
            let label = format!("下载订阅 {name}");
            download::download_with_progress(&source, move |done, total| {
                emit_progress(
                    &progress_ctx,
                    &progress_id_for_download,
                    label.clone(),
                    done,
                    total,
                );
            })
            .await
        } else {
            emit_progress(&ctx, &progress_id, format!("读取订阅 {name}"), 0, Some(1));
            download::read_local(&source)
        };
        finish_progress(&ctx, &progress_id);
        match dl {
            Ok(res) => {
                emit_progress(&ctx, &progress_id, format!("保存订阅 {name}"), 1, Some(2));
                let meta = ProfileMeta {
                    name: name.clone(),
                    kind: if is_url {
                        ProfileKind::Url
                    } else {
                        ProfileKind::File
                    },
                    url: source.clone(),
                    last_updated: now_unix(),
                    subscription_info: res.info,
                    mixin_enabled: true,
                };
                let r = ctx.profiles.lock().await.upsert(meta, &res.body);
                finish_progress(&ctx, &progress_id);
                match r {
                    Ok(()) => {
                        if activate_after_add {
                            let switch = ctx.profiles.lock().await.set_current(&name);
                            if let Err(e) = switch {
                                ctx.emit(AppEvent::Error(e.to_string()));
                                finish_task(&ctx, &task_key);
                                return;
                            }
                            if let Err(e) = apply_current_profile(&ctx, None).await {
                                ctx.emit(AppEvent::Error(e.to_string()));
                                finish_task(&ctx, &task_key);
                                return;
                            }
                            refresh_proxies(&ctx).await;
                            ctx.emit(AppEvent::Toast(format!("已添加并加载订阅: {name}")));
                        } else {
                            ctx.emit(AppEvent::Toast(format!("已添加订阅: {name}")));
                        }
                        let list = ctx.profiles.lock().await.list();
                        ctx.emit(AppEvent::ProfilesChanged(list));
                    }
                    Err(e) => ctx.emit(AppEvent::Error(e.to_string())),
                }
            }
            Err(e) => ctx.emit(AppEvent::Error(format!("添加失败: {e}"))),
        }
        finish_task(&ctx, &task_key);
    });
}

fn spawn_switch_profile(ctx: AppContext, name: String) {
    tokio::spawn(async move {
        let task_key = format!("switch_profile:{name}");
        let progress_id = "switch_profile";
        emit_progress(&ctx, progress_id, format!("切换配置 {name}"), 0, Some(4));
        // 切换当前指针 + 生成运行时配置 + reload。
        let set = ctx.profiles.lock().await.set_current(&name);
        if let Err(e) = set {
            finish_progress(&ctx, progress_id);
            ctx.emit(AppEvent::Error(e.to_string()));
            finish_task(&ctx, &task_key);
            return;
        }
        emit_progress(&ctx, progress_id, format!("生成配置 {name}"), 1, Some(4));
        if let Err(e) = apply_current_profile(&ctx, Some((progress_id, 2, 4))).await {
            finish_progress(&ctx, progress_id);
            ctx.emit(AppEvent::Error(e.to_string()));
            finish_task(&ctx, &task_key);
            return;
        }
        emit_progress(&ctx, progress_id, format!("切换配置 {name}"), 4, Some(4));
        finish_progress(&ctx, progress_id);
        refresh_proxies(&ctx).await;
        ctx.emit(AppEvent::Toast(format!("已切换到: {name}")));
        let list = ctx.profiles.lock().await.list();
        ctx.emit(AppEvent::ProfilesChanged(list));
        finish_task(&ctx, &task_key);
    });
}

fn spawn_delete_profile(ctx: AppContext, name: String) {
    tokio::spawn(async move {
        let task_key = format!("delete_profile:{name}");
        let r = ctx.profiles.lock().await.delete(&name);
        match r {
            Ok(()) => {
                ctx.emit(AppEvent::Toast(format!("已删除: {name}")));
                let list = ctx.profiles.lock().await.list();
                ctx.emit(AppEvent::ProfilesChanged(list));
            }
            Err(e) => ctx.emit(AppEvent::Error(e.to_string())),
        }
        finish_task(&ctx, &task_key);
    });
}

fn spawn_update_profile(ctx: AppContext, name: String) {
    tokio::spawn(async move {
        let task_key = format!("update_profile:{name}");
        update_one(&ctx, &name, None).await;
        let list = ctx.profiles.lock().await.list();
        ctx.emit(AppEvent::ProfilesChanged(list));
        finish_task(&ctx, &task_key);
    });
}

fn spawn_update_all(ctx: AppContext) {
    tokio::spawn(async move {
        let task_key = "update_all_profiles";
        let names: Vec<String> = ctx
            .profiles
            .lock()
            .await
            .list()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        let total = names.len() as u64;
        if total == 0 {
            ctx.emit(AppEvent::Toast("没有可更新的订阅".into()));
            finish_task(&ctx, task_key);
            return;
        }
        let batch_id = "update_all_profiles";
        emit_progress(&ctx, batch_id, "更新全部订阅", 0, Some(total));
        for (idx, n) in names.iter().enumerate() {
            emit_progress(
                &ctx,
                batch_id,
                format!("更新全部订阅：{n}"),
                idx as u64,
                Some(total),
            );
            update_one(
                &ctx,
                n,
                Some(BatchProgress {
                    id: batch_id,
                    total,
                    index: idx as u64,
                }),
            )
            .await;
            emit_progress(
                &ctx,
                batch_id,
                format!("更新全部订阅：{n}"),
                idx as u64 + 1,
                Some(total),
            );
        }
        finish_progress(&ctx, batch_id);
        ctx.emit(AppEvent::Toast("已更新全部订阅".into()));
        let list = ctx.profiles.lock().await.list();
        ctx.emit(AppEvent::ProfilesChanged(list));
        finish_task(&ctx, task_key);
    });
}

#[derive(Debug, Clone, Copy)]
struct BatchProgress {
    id: &'static str,
    total: u64,
    index: u64,
}

/// 更新单个订阅（仅 URL 类型重新下载）。
async fn update_one(ctx: &AppContext, name: &str, batch: Option<BatchProgress>) {
    use clashtui_domain::profile::{download, ProfileKind, ProfileMeta};
    let meta = ctx.profiles.lock().await.get(name).cloned();
    let Some(meta) = meta else { return };
    if meta.kind != ProfileKind::Url {
        return;
    }
    let progress_id = format!("update_profile:{name}");
    let progress_ctx = ctx.clone();
    let progress_id_for_download = progress_id.clone();
    let label = format!("更新订阅 {name}");
    match download::download_with_progress(&meta.url, move |done, total| {
        if batch.is_none() {
            emit_progress(
                &progress_ctx,
                &progress_id_for_download,
                label.clone(),
                done,
                total,
            );
        }
    })
    .await
    {
        Ok(res) => {
            if batch.is_none() {
                finish_progress(ctx, &progress_id);
            }
            let new_meta = ProfileMeta {
                last_updated: now_unix(),
                subscription_info: res.info,
                ..meta
            };
            let is_current = ctx.profiles.lock().await.current() == Some(name);
            if let Err(e) = ctx.profiles.lock().await.upsert(new_meta, &res.body) {
                ctx.emit(AppEvent::Error(e.to_string()));
                return;
            }
            ctx.emit(AppEvent::SubUpdated(name.to_string()));
            // 若更新的是当前配置，重新生成并 reload。
            if is_current {
                if let Err(e) = apply_current_profile(ctx, None).await {
                    ctx.emit(AppEvent::Error(e.to_string()));
                }
            }
            if let Some(batch) = batch {
                emit_progress(
                    ctx,
                    batch.id,
                    format!("更新全部订阅：{name}"),
                    batch.index + 1,
                    Some(batch.total),
                );
            }
        }
        Err(e) => {
            if batch.is_none() {
                finish_progress(ctx, &progress_id);
            }
            ctx.emit(AppEvent::Error(format!("{name} 更新失败: {e}")));
        }
    }
}

/// 由当前 profile 生成运行时配置并 reload 内核。
async fn apply_current_profile(
    ctx: &AppContext,
    progress: Option<(&'static str, u64, u64)>,
) -> clashtui_domain::DomainResult<()> {
    use clashtui_domain::{mixin, DomainError};
    let store = ctx.profiles.lock().await;
    let Some(current) = store.current().map(|s| s.to_string()) else {
        return Err(DomainError::Profile("未选择当前 profile".into()));
    };
    let raw = store.read_raw(&current)?;
    drop(store);

    if let Some((id, step, total)) = progress {
        emit_progress(ctx, id, format!("读取配置 {current}"), step, Some(total));
    }
    let mixin_yaml = std::fs::read_to_string(ctx.paths.mixin_file()).ok();
    let override_yaml = std::fs::read_to_string(ctx.paths.override_file()).ok();
    let app_config = load_config_snapshot(ctx);

    if let Some((id, step, total)) = progress {
        emit_progress(
            ctx,
            id,
            format!("合成运行时配置 {current}"),
            step + 1,
            Some(total),
        );
    }
    let runtime = mixin::build_runtime(
        &raw,
        mixin_yaml.as_deref(),
        override_yaml.as_deref(),
        &app_config,
    )?;
    clashtui_domain::util::atomic_write(&ctx.paths.runtime_config(), runtime.as_bytes())?;

    // 内核在线则 reload；不在线则尝试启动托管内核，让“添加订阅后直接可用”。
    if !ctx.client.ping().await {
        if let Some((id, step, total)) = progress {
            emit_progress(ctx, id, "启动 mihomo 内核", step + 2, Some(total + 1));
        }
        let status = ctx.core.start().await?;
        ctx.emit(AppEvent::CoreStatus(status));
        wait_for_controller(ctx).await;
    }

    if ctx.client.ping().await {
        if let Some((id, step, total)) = progress {
            emit_progress(ctx, id, "重载 mihomo 配置", step + 3, Some(total + 1));
        }
        let _ = ctx.core.reload().await;
        ctx.emit(AppEvent::Toast("已重载运行时配置".into()));
        if let Ok(cfg) = ctx.client.configs().await {
            ctx.emit(AppEvent::ConfigLoaded(Box::new(cfg)));
        }
    }
    Ok(())
}

async fn wait_for_controller(ctx: &AppContext) -> bool {
    for _ in 0..20 {
        if ctx.client.ping().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// 当前 unix 时间（秒）。
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn auto_profile_name() -> String {
    format!("sub-{}", now_unix())
}

/// 把 StreamHub 的消息转换为 AppEvent 回灌（供 app 启动转发任务用）。
pub fn forward_stream_msg(ctx: &AppContext, msg: StreamMsg) {
    let ev = match msg {
        StreamMsg::Traffic(t) => AppEvent::WsTraffic(t),
        StreamMsg::Log(l) => AppEvent::WsLog {
            level: l.level,
            payload: l.payload,
        },
        StreamMsg::Connections(c) => AppEvent::WsConnections {
            download_total: c.download_total,
            upload_total: c.upload_total,
            connections: c.connections,
        },
        StreamMsg::Memory(m) => AppEvent::WsMemory(m),
        StreamMsg::Connected(k) => AppEvent::WsConnected(format!("{k:?}")),
        StreamMsg::Disconnected(k) => AppEvent::WsDisconnected(format!("{k:?}")),
    };
    ctx.emit(ev);
}

#[allow(unused)]
fn _type_check(_: &HashMap<String, u16>) {}
