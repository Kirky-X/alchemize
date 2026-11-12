// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! MCP 工具注册面测试（feature `mcp`）：三个 `#[forge]` 端点经
//! `init_all_plugins` 固化后，必须以 `tool_name` 注册为 MCP 工具。
//! 端点层由 `alch` CLI 与其 `alch mcp` 子命令共享同一份 `forge_app.rs`
//! （原独立 `alch-mcp` 二进制已折叠为主 CLI 子命令）。

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

/// `alch mcp` 子命令端到端：真实 MCP 握手（initialize → initialized →
/// tools/list）后以 EOF 干净收尾（rmcp `waiting()` 在对端断开后返回），
/// 退出码 0，stdout 承载 JSON-RPC 且工具面齐全。
#[test]
fn mcp_subcommand_serves_handshake_and_tools_list() {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_alch"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn alch mcp");
    let requests = concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"alch-test","version":"0"}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n",
    );
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(requests.as_bytes())
        .expect("write MCP requests");
    // stdin drop 触发 EOF：serve 收尾，进程退出。
    let out = child.wait_with_output().expect("wait alch mcp");
    assert!(
        out.status.success(),
        "alch mcp must exit 0 after handshake+EOF; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"serverInfo\""), "no initialize result");
    for tool in ["\"convert\"", "\"doctor\"", "\"formats\""] {
        assert!(stdout.contains(tool), "tools/list missing {tool}: {stdout}");
    }
}

/// 无握手的立即 EOF 是异常关闭：handler 必须显性报错（stderr 人读明细）
/// 并以退出码 1 结束——折叠前 `alch-mcp` 在此路径直接 panic（exit 101）。
#[test]
fn mcp_subcommand_reports_honest_error_on_premature_eof() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_alch"))
        .arg("mcp")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("spawn alch mcp");
    assert_eq!(
        out.status.code(),
        Some(1),
        "premature EOF must exit 1, not panic; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mcp"),
        "stderr must name the failing surface: {stderr}"
    );
}
