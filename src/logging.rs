// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! CLI 日志前端 — inklog subscriber 的唯一装配点。
//!
//! 由 `alch` CLI（main.rs，含 `alch mcp` 子命令路径）经 `#[path]` 装配
//! 同一份实现；lib 不装配 subscriber（库惯例：前端归宿主装配，alchemize
//! 作为库被集成时由下游决定日志去向）。
//!
//! 契约（specmark/specs/cli-output/spec.md 钉死 stdout/stderr 形态）：
//! 默认（`ALCHEMIZE_LOG` 未设或 `off`）不装配，输出形态与既有契约逐字节
//! 一致；显式设级后装配 inklog 全局前端，并把全部级别强制路由 stderr
//! （严格于 inklog 默认的 error/warn 分流）——stdout 只承载转换产物，
//! info/debug 也不污染管道。

/// 全部级别名（inklog `GlobalConfig::level` 合法域；`off` 由返回 `None` 表达）。
const LEVELS: [&str; 6] = ["trace", "debug", "info", "warn", "error", "fatal"];

/// 读 `ALCHEMIZE_LOG` 并装配 inklog 全局 subscriber（tracing + log 双前端）。
///
/// 级别域：`off|error|warn|info|debug|trace`（大小写不敏感），未设/空/`off`
/// 返回 `None`（零开销静默）；非法值打一行 stderr 提示后按 `off` 处理，
/// 不静默吞错。装配失败（`InklogError`）交调用方降级：报 stderr 后继续，
/// 不因日志前端失败中断转换主流程。
///
/// 须在 tokio 多线程 runtime 内调用（inklog 构建路径含 `block_in_place`）；
/// 返回 `Some(manager)` 时调用方须在进程退出前 `shutdown()` 排水——
/// `std::process::exit` 不跑 Drop，不排水则尾部日志丢失。
pub async fn init_from_env() -> Result<Option<inklog::LoggerManager>, inklog::InklogError> {
    let Some(raw) = std::env::var_os("ALCHEMIZE_LOG") else {
        return Ok(None);
    };
    let raw = raw.to_string_lossy().trim().to_lowercase();
    if raw.is_empty() || raw == "off" {
        return Ok(None);
    }
    if !LEVELS.contains(&raw.as_str()) {
        eprintln!(
            "alch: invalid ALCHEMIZE_LOG '{raw}' (valid: off, error, warn, info, debug, trace); logging stays off"
        );
        return Ok(None);
    }

    let config = inklog::InklogConfig {
        global: inklog::GlobalConfig {
            level: raw,
            ..Default::default()
        },
        console_sink: Some(inklog::ConsoleSinkConfig {
            // 任何级别都不碰 stdout：info/debug 一并路由 stderr
            stderr_levels: LEVELS.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }),
        // inklog 自身内部时序告警（如 shutdown 排水提示）是机器噪音不是
        // 业务诊断：压到 error，用户设 warn 只见 alchemize 侧埋点
        target_levels: [("inklog".to_string(), "error".to_string())].into(),
        ..Default::default()
    };
    // with_config 内部对"全局 subscriber 已被宿主安装"按良性条件降级
    // （try_init 失败转 debug），库侧被先行装配时不会硬失败
    let manager = inklog::LoggerManager::with_config(config).await?;
    Ok(Some(manager))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Serializes the env-mutating tests: ALCHEMIZE_LOG is process-global.
    // Async-aware (`tokio::sync::Mutex`): guards are held across `await`
    // points in async tests (clippy::await_holding_lock).
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        LOCK.lock().await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unset_env_is_silent() {
        let _g = env_guard().await;
        unsafe {
            std::env::remove_var("ALCHEMIZE_LOG");
        }
        assert!(init_from_env().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn off_and_empty_are_silent() {
        let _g = env_guard().await;
        unsafe {
            std::env::set_var("ALCHEMIZE_LOG", "off");
        }
        assert!(init_from_env().await.unwrap().is_none());
        unsafe {
            std::env::set_var("ALCHEMIZE_LOG", "   ");
        }
        assert!(init_from_env().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_level_stays_off() {
        let _g = env_guard().await;
        unsafe {
            std::env::set_var("ALCHEMIZE_LOG", "loud");
        }
        assert!(init_from_env().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trimmed_lowercase_level_installs_and_shuts_down() {
        let _g = env_guard().await;
        unsafe {
            std::env::set_var("ALCHEMIZE_LOG", "  DEBUG  ");
        }
        let manager = init_from_env().await.unwrap();
        assert!(manager.is_some(), "valid level must install the subscriber");
        if let Some(manager) = manager {
            let _ = manager.shutdown();
        }
    }
}
