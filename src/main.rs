// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! `alch` — sdforge 封装的命令行入口（`#[forge]` 端点见 `forge_app`）。
//!
//! 契约（退出码 0/1/2/3、位置参数、stdout/stderr 形态）由
//! `specmark/specs/cli-output/spec.md` 钉死；`tests/cli.rs` 是机械证明
//! （含 `--` 透传用例）。
//!
//! runner 不用 `CliBuilder::execute()`（其硬编码 exit 0/1），改用公开的
//! `build()` + `dispatch()` + `OutputFormat` 自管退出码与输出；argv 预处理
//! 归一 sdforge 无法表达的三处契约：`-o` 短旗标、`--no-remote` 裸旗标
//! （sdforge 生成下划线 long 名且 bool 恒为取值式）、根 `--version` 的
//! 正确版本号（build() 的根 version 取自 sdforge crate）。

use std::io::Write as _;

use sdforge::cli::{CliBuilder, OutputFormat};
use sdforge::core::ApiError;

#[path = "forge_app.rs"]
mod forge_app;

#[path = "logging.rs"]
mod logging;

/// 契约恢复层：sdforge 生成的参数面与本仓 CLI 契约之间的差异在此归一。
///
/// 规则（按 token 序处理）：
/// - `--version` / `-V`：仅限子命令之前，直出本仓版本（build() 的根
///   version 取自 sdforge crate）
/// - `-o` → `--output`（sdforge 不为 per-command 参数生成短旗标）
/// - `convert` 语境内的 `--format` → `--input_format`（扩展名覆盖；根级
///   全局 `--format text|json` 同名撞车 + 默认值会污染取值，且旧契约中
///   convert 的 --format 本就是扩展名覆盖。根级输出格式旗标须放在子命令
///   之前，如 `alch --format json formats`）
/// - `--no-remote` → `--no_remote true`（裸旗标语义归一；sdforge long 名
///   取参数标识符的下划线形式且 bool 恒为取值式）
/// - `--base-url` → `--base_url`、`--extract-main` → `--extract_main true`
///   （同下划线归一；仅 convert 语境，html feature 未编译时端点侧诚实告警）
/// - `--` 之后原样透传（位置参数字面量，不重写）
fn normalize_argv(raw: Vec<String>) -> Vec<String> {
    use std::collections::VecDeque;
    let mut rest: VecDeque<_> = raw.into_iter().collect();
    let mut out = Vec::with_capacity(rest.len() + 2);
    let mut in_convert = false;
    let mut seen_subcommand = false;
    while let Some(arg) = rest.pop_front() {
        // `--` 之后的 token 是位置参数字面量（如文件名）：原样透传。
        if arg == "--" {
            out.push(arg);
            out.extend(rest);
            return out;
        }
        match arg.as_str() {
            "--version" | "-V" if !seen_subcommand => {
                println!("alch {}", alchemize::VERSION);
                std::process::exit(0);
            }
            "convert" | "doctor" | "formats" => {
                seen_subcommand = true;
                in_convert = arg == "convert";
                out.push(arg);
            }
            "-o" if in_convert => out.push("--output".into()),
            "--no-remote" if in_convert => {
                out.push("--no_remote".into());
                out.push("true".into());
            }
            s if in_convert && s.starts_with("--no-remote=") => {
                out.push(format!(
                    "--no_remote={}",
                    s.trim_start_matches("--no-remote=")
                ));
            }
            "--format" if in_convert => out.push("--input_format".into()),
            "--base-url" if in_convert => out.push("--base_url".into()),
            s if in_convert && s.starts_with("--base-url=") => {
                out.push(format!(
                    "--base_url={}",
                    s.trim_start_matches("--base-url=")
                ));
            }
            "--extract-main" if in_convert => {
                out.push("--extract_main".into());
                out.push("true".into());
            }
            s if in_convert && s.starts_with("--format=") => {
                out.push(format!(
                    "--input_format={}",
                    s.trim_start_matches("--format=")
                ));
            }
            other => out.push(other.into()),
        }
    }
    out
}

fn main() {
    // init_all_plugins 触碰 inventory 注册，防止链接器优化掉端点注册项。
    sdforge::init_all_plugins();

    // multi_thread 运行时：与 alch-mcp 的 #[tokio::main] 默认一致（MCP
    // 工具包装器内部使用 tokio::task::block_in_place，要求 multi-thread）。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    // 日志前端：ALCHEMIZE_LOG 显式设级才装配（默认 off，契约零扰动）。
    // 装配失败降级为无日志继续，不中断转换主流程。
    let mut logger = match runtime.block_on(logging::init_from_env()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("alch: logging init failed: {e}");
            None
        }
    };

    let argv = normalize_argv(std::env::args().collect());
    let cmd = CliBuilder::new().with_name("alch").build();
    let matches = cmd.try_get_matches_from(argv).unwrap_or_else(|e| e.exit());
    let format = OutputFormat::from_matches(&matches);

    let dispatched = runtime.block_on(sdforge::cli::dispatch(&matches, None));
    // exit 不跑 Drop：排水须在 process::exit 之前（尾部日志冲刷）
    if let Some(l) = logger.take()
        && let Err(e) = l.shutdown()
    {
        eprintln!("alch: log shutdown failed: {e}");
    }

    match dispatched {
        Ok((_, value)) => {
            // `Value::Null` 是「handler 已自行输出」哨兵（写文件路径，
            // 摘要走 stderr）；其余按 --format 契约渲染。stdout 写入对
            // EPIPE 容错（管道下游提前关闭不是错误，与旧实现一致）。
            if !value.is_null() {
                let rendered = format.render_success(&value);
                let _ = writeln!(std::io::stdout(), "{rendered}");
            }
            std::process::exit(0);
        }
        Err(e) => match e {
            // 领域 stderr 行已由 handler（map_error）输出：变体只承载
            // 退出码，不再补打错误行。
            ApiError::NotFound { resource, .. } if resource == "format" => std::process::exit(2),
            ApiError::ServiceUnavailable { .. } => std::process::exit(3),
            other => {
                format.emit_error(&other);
                std::process::exit(1);
            }
        },
    }
}

#[cfg(test)]
mod argv_tests {
    use super::normalize_argv;

    #[test]
    fn version_short_circuit_not_tested_here() {
        // `--version` exits the process; it must never appear in tests that
        // call normalize_argv directly.
    }

    #[test]
    fn passthrough_keeps_positional_tokens_after_double_dash() {
        let raw = vec![
            "alch".into(),
            "convert".into(),
            "--".into(),
            "--output".into(),
        ];
        assert_eq!(
            normalize_argv(raw),
            vec!["alch", "convert", "--", "--output"]
        );
    }

    #[test]
    fn convert_flags_translate_to_snake_case() {
        let raw = vec![
            "alch".into(),
            "convert".into(),
            "-o".into(),
            "out.md".into(),
            "--no-remote".into(),
            "--no-remote=true".into(),
            "--format".into(),
            "pdf".into(),
            "--format=docx".into(),
            "--base-url".into(),
            "https://x.test".into(),
            "--base-url=https://y.test".into(),
            "--extract-main".into(),
        ];
        let out = normalize_argv(raw);
        let joined: Vec<&str> = out.iter().map(|s| s.as_str()).collect();
        let expected = [
            "alch",
            "convert",
            "--output",
            "out.md",
            "--no_remote",
            "true",
            "--no_remote=true",
            "--input_format",
            "pdf",
            "--input_format=docx",
            "--base_url",
            "https://x.test",
            "--base_url=https://y.test",
            "--extract_main",
            "true",
        ];
        assert_eq!(joined, expected, "{joined:?}");
    }

    #[test]
    fn flags_outside_convert_stay_verbatim() {
        let raw = vec![
            "alch".into(),
            "doctor".into(),
            "--format".into(),
            "json".into(),
        ];
        assert_eq!(
            normalize_argv(raw),
            vec!["alch", "doctor", "--format", "json"]
        );
    }
}
