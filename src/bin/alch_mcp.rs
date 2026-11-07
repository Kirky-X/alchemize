// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! `alch-mcp` — sdforge MCP stdio 服务（feature `mcp`）。
//!
//! 工具面与 `alch` CLI 共享同一份 `#[forge]` 端点（`forge_app.rs`）：
//! convert / doctor / formats。stdio 传输，无网络出口。

#[path = "../forge_app.rs"]
mod forge_app;

#[tokio::main]
async fn main() {
    sdforge::init_all_plugins();
    sdforge::mcp::serve_stdio(sdforge::mcp::build())
        .await
        .expect("alch-mcp stdio server");
}
