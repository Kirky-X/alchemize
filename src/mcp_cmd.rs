// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! `alch mcp` — sdforge MCP stdio 服务子命令（feature `mcp`）。
//!
//! 原独立二进制 `alch-mcp`（`src/bin/alch_mcp.rs`）的
//! `sdforge::mcp::serve_stdio(build())` 逻辑迁入本处理器；工具面与
//! `alch` CLI 共享同一份 `#[forge]` 端点（`forge_app.rs`）：
//! convert / doctor / formats。stdio 传输，无网络出口。
//!
//! 日志契约不变：main 在 dispatch 前装配日志前端（stdout 承载 JSON-RPC，
//! 全部级别路由 stderr）；serve 返回后 main 统一排水再退出。

use sdforge::cli::{CliCommandRegistration, CliHandlerRegistration};
use sdforge::core::{HandlerArgs, HandlerFuture, HandlerState};

/// MCP stdio 服务处理器：阻塞服务直到 stdin EOF，返回 `Value::Null`
/// 哨兵（CLI runner 据此跳过 stdout 渲染——JSON-RPC 已由 stdio 面输出）。
///
/// 失败路径沿用 forge_app 的 map_error 范式：人读明细先落 stderr，
/// `ApiError` 变体只承载退出码语义（Internal → 1）——runner 的
/// `emit_error` 对 Internal 只打通用行，真实原因若不先打就不可见。
fn serve(_args: HandlerArgs, _state: HandlerState) -> HandlerFuture {
    Box::pin(async move {
        sdforge::mcp::serve_stdio(sdforge::mcp::build())
            .await
            .map_err(|e| {
                let message =
                    alchemize::tr_args("mcp-stdio-server-failed", &[("source", e.to_string())]);
                eprintln!("{message}");
                sdforge::core::ApiError::internal_error(message, "alch-mcp")
            })?;
        Ok(serde_json::Value::Null)
    })
}

// inventory 经 sdforge re-export（本仓不直依赖 inventory crate）。
sdforge::inventory::submit! {
    CliCommandRegistration::new(
        "mcp",
        "1.0",
        "Serve MCP tools (convert/doctor/formats) over stdio",
        "alch_mcp_serve",
    )
}
sdforge::inventory::submit! { CliHandlerRegistration { name: "mcp", handler: serve } }

#[cfg(test)]
mod tests {
    use sdforge::cli::{CliCommandRegistration, CliHandlerRegistration};

    /// `mcp` 子命令的元数据与处理器都经 inventory 注册且按名配对。
    #[test]
    fn mcp_subcommand_registered_and_paired() {
        let cmds: Vec<&CliCommandRegistration> =
            sdforge::inventory::iter::<CliCommandRegistration>()
                .filter(|c| c.name == "mcp")
                .collect();
        assert_eq!(cmds.len(), 1, "exactly one mcp command registration");
        let cmd = cmds[0];
        assert_eq!(cmd.handler_fn_name, "alch_mcp_serve");
        assert!(cmd.args.is_empty(), "mcp takes no CLI arguments");

        let handlers: Vec<&CliHandlerRegistration> =
            sdforge::inventory::iter::<CliHandlerRegistration>()
                .filter(|h| h.name == "mcp")
                .collect();
        assert_eq!(handlers.len(), 1, "exactly one mcp handler registration");
        assert_eq!(handlers[0].name, "mcp", "handler name pairs with command");
    }
}
