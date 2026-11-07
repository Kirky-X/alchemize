// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! MCP 工具注册面测试（feature `mcp`）：三个 `#[forge]` 端点经
//! `init_all_plugins` 固化后，必须以 `tool_name` 注册为 MCP 工具。
//! 端点层与 `alch`/`alch-mcp` 两个二进制共享同一份 `forge_app.rs`。

#![cfg(feature = "mcp")]

#[path = "../src/forge_app.rs"]
mod forge_app;

#[test]
fn mcp_tools_registered_convert_doctor_formats() {
    sdforge::init_all_plugins();
    let tools = sdforge::mcp::get_mcp_tools();
    let names: Vec<String> = tools
        .iter()
        .map(|t| t.metadata().name().to_string())
        .collect();
    for expected in ["convert", "doctor", "formats"] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing MCP tool {expected}; registered: {names:?}"
        );
    }
}
