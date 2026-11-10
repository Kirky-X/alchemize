// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! `alch-mcp` — sdforge MCP stdio 服务（feature `mcp`）。
//!
//! 工具面与 `alch` CLI 共享同一份 `#[forge]` 端点（`forge_app.rs`）：
//! convert / doctor / formats。stdio 传输，无网络出口。

#[path = "../forge_app.rs"]
mod forge_app;

#[path = "../logging.rs"]
mod logging;

#[tokio::main]
async fn main() {
    sdforge::init_all_plugins();

    // MCP stdio：stdout 承载 JSON-RPC，日志前端把全部级别路由 stderr
    //（logging.rs 契约）；默认 off 不装配。装配失败降级为无日志继续。
    let mut logger = match logging::init_from_env().await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("alch-mcp: logging init failed: {e}");
            None
        }
    };

    sdforge::mcp::serve_stdio(sdforge::mcp::build())
        .await
        .expect("alch-mcp stdio server");

    // serve 返回即退出：排水再走，尾部日志不丢
    if let Some(l) = logger.take()
        && let Err(e) = l.shutdown()
    {
        eprintln!("alch-mcp: log shutdown failed: {e}");
    }
}
