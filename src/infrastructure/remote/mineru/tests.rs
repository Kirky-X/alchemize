// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Tests for MinerU module — SSRF validation, option builder, factory selection.

use super::audit::test_support::capture_guard;
use super::client::MineruClient;
use super::factory::create_mineru_client;
use super::precision::MineruPrecisionClient;
use super::types::{ParseOptions, TaskId, TaskStatus};
use super::validated_url::{DEFAULT_MINERU_ALLOWLIST, validate_file_url};
use crate::domain::error::{AlchemizeError, SecurityViolationType};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[test]
fn validate_https_url_succeeds() {
    let url = "https://mineru.net/file/example.pdf";
    let result = validate_file_url(url, DEFAULT_MINERU_ALLOWLIST);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().as_str(), url);
}

#[test]
fn validate_http_url_fails() {
    let url = "http://mineru.net/file/example.pdf";
    let result = validate_file_url(url, DEFAULT_MINERU_ALLOWLIST);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::InvalidScheme,
            ..
        }
    ));
}

#[test]
fn validate_loopback_ipv4_fails() {
    let url = "https://127.0.0.1/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LoopbackAddress,
            ..
        }
    ));
}

#[test]
fn validate_localhost_fails() {
    let url = "https://localhost/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LoopbackAddress,
            ..
        }
    ));
}

#[test]
fn validate_private_ipv4_10_fails() {
    let url = "https://10.0.0.1/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::PrivateAddress,
            ..
        }
    ));
}

#[test]
fn validate_private_ipv4_172_fails() {
    let url = "https://172.16.0.1/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::PrivateAddress,
            ..
        }
    ));
}

#[test]
fn validate_private_ipv4_192_fails() {
    let url = "https://192.168.1.1/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::PrivateAddress,
            ..
        }
    ));
}

#[test]
fn validate_link_local_fails() {
    let url = "https://169.254.1.1/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LinkLocalAddress,
            ..
        }
    ));
}

#[test]
fn validate_hostname_not_in_allowlist_fails() {
    let url = "https://evil.com/file.pdf";
    let result = validate_file_url(url, DEFAULT_MINERU_ALLOWLIST);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::HostnameNotAllowed,
            ..
        }
    ));
}

#[test]
fn validate_public_ip_with_empty_allowlist_succeeds() {
    let url = "https://8.8.8.8/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_ok());
}

#[test]
fn validate_ipv6_loopback_fails() {
    let url = "https://[::1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(
        err,
        AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LoopbackAddress,
            ..
        }
    ));
}

#[test]
fn validate_ipv4_mapped_ipv6_loopback_fails() {
    // ::ffff:127.0.0.1 — IPv4-mapped IPv6 must be blocked to prevent SSRF bypass
    let url = "https://[::ffff:127.0.0.1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::LoopbackAddress,
                ..
            }
        ),
        "expected LoopbackAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_mapped_ipv6_private_10_fails() {
    let url = "https://[::ffff:10.0.0.1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_mapped_ipv6_private_192_fails() {
    let url = "https://[::ffff:192.168.1.1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_mapped_ipv6_private_172_fails() {
    // 172.16/12 — RFC1918 Class B private range via IPv4-mapped IPv6
    let url = "https://[::ffff:172.16.0.1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_mapped_ipv6_cgnat_fails() {
    // 100.64/10 — RFC 6598 CGNAT range (Alibaba Cloud metadata 100.100.100.200)
    let url = "https://[::ffff:100.100.100.200]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_mapped_ipv6_link_local_fails() {
    // AWS metadata endpoint via IPv4-mapped IPv6
    let url = "https://[::ffff:169.254.169.254]/latest/meta-data/";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::LinkLocalAddress,
                ..
            }
        ),
        "expected LinkLocalAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_compatible_ipv6_loopback_fails() {
    // ::127.0.0.1 — IPv4-compatible IPv6 (RFC 4291 deprecated) must be blocked.
    // to_ipv4 covers this form; to_ipv4_mapped does NOT.
    let url = "https://[::127.0.0.1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::LoopbackAddress,
                ..
            }
        ),
        "expected LoopbackAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_compatible_ipv6_private_10_fails() {
    // ::10.0.0.1 — IPv4-compatible IPv6 private
    let url = "https://[::10.0.0.1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_compatible_ipv6_link_local_fails() {
    // ::169.254.169.254 — IPv4-compatible IPv6 link-local (AWS metadata)
    let url = "https://[::169.254.169.254]/latest/meta-data/";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::LinkLocalAddress,
                ..
            }
        ),
        "expected LinkLocalAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_compatible_ipv6_cgnat_fails() {
    // ::100.100.100.200 — IPv4-compatible IPv6 CGNAT (Alibaba metadata)
    let url = "https://[::100.100.100.200]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_cgnat_fails() {
    // 100.100.100.200 — Alibaba Cloud metadata endpoint (RFC 6598 CGNAT)
    let url = "https://100.100.100.200/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv4_broadcast_fails() {
    // 255.255.255.255 — broadcast address (defense in depth)
    let url = "https://255.255.255.255/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv6_unspecified_fails() {
    // :: — IPv6 unspecified address (equivalent to 0.0.0.0).
    // `to_ipv4` returns `Some(0.0.0.0)` (IPv4-compatible form), which is
    // rejected as a private address by the 0.0.0.0/8 rule.
    let url = "https://[::]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(
        result.is_err(),
        "expected error for IPv6 unspecified address"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress for IPv6 unspecified, got {:?}",
        err
    );
}

#[test]
fn validate_ipv6_multicast_fails() {
    // ff00::/8 — IPv6 multicast (defense in depth)
    let url = "https://[ff02::1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv6_ula_fails() {
    // fc00::/7 — IPv6 Unique Local Address (RFC 4193), equivalent to IPv4 RFC1918
    let url = "https://[fc00::1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::PrivateAddress,
                ..
            }
        ),
        "expected PrivateAddress, got {:?}",
        err
    );
}

#[test]
fn validate_ipv6_link_local_fails() {
    // fe80::/10 — IPv6 link-local (RFC 4291), equivalent to IPv4 169.254/16
    let url = "https://[fe80::1]/file.pdf";
    let result = validate_file_url(url, &[]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            AlchemizeError::SecurityViolation {
                kind: SecurityViolationType::LinkLocalAddress,
                ..
            }
        ),
        "expected LinkLocalAddress, got {:?}",
        err
    );
}

#[test]
fn validate_subdomain_in_allowlist_succeeds() {
    let url = "https://sub.mineru.net/file.pdf";
    let result = validate_file_url(url, DEFAULT_MINERU_ALLOWLIST);
    assert!(result.is_ok());
}

#[test]
fn validate_partial_label_not_matched_fails() {
    // `evilmineru.net` ends with `mineru.net` as a substring but is not a
    // subdomain of `mineru.net` — `check_hostname_allowlist` uses a leading
    // dot (`.{domain}`) to prevent this partial-label match.
    let url = "https://evilmineru.net/file.pdf";
    let result = validate_file_url(url, DEFAULT_MINERU_ALLOWLIST);
    assert!(
        result.is_err(),
        "evilmineru.net must NOT match allowlist entry mineru.net (partial-label bypass)"
    );
}

#[test]
fn parse_options_default() {
    let opts = ParseOptions::default();
    assert!(!opts.enable_formula);
    assert!(!opts.enable_table);
    assert_eq!(opts.model_version, "");
}

#[test]
fn parse_options_builder() {
    let opts = ParseOptions::new()
        .with_formula(true)
        .with_table(true)
        .with_model("vlm");
    assert!(opts.enable_formula);
    assert!(opts.enable_table);
    assert_eq!(opts.model_version, "vlm");
}

#[test]
fn create_mineru_client_small_file_returns_agent() {
    let client = create_mineru_client(5 * 1024 * 1024).unwrap(); // 5MB
    assert_eq!(client.max_file_size(), 10 * 1024 * 1024);
}

#[test]
fn create_mineru_client_large_file_returns_precision() {
    // Without MINERU_API_TOKEN env var, precision client construction fails.
    // With token set, it succeeds. Either is acceptable in test env.
    let result = create_mineru_client(50 * 1024 * 1024); // 50MB
    if std::env::var("MINERU_API_TOKEN").is_ok() {
        assert!(
            result.is_ok(),
            "with MINERU_API_TOKEN set, precision client should construct"
        );
    } else {
        assert!(
            result.is_err(),
            "without MINERU_API_TOKEN, precision client should fail"
        );
    }
}

#[test]
fn task_id_display() {
    let id = TaskId::new("abc-123".to_string()).unwrap();
    assert_eq!(id.to_string(), "abc-123");
}

#[test]
fn task_id_rejects_empty() {
    let result = TaskId::new(String::new());
    assert!(result.is_err(), "empty task_id should be rejected");
}

#[test]
fn task_id_rejects_path_traversal() {
    let result = TaskId::new("../admin".to_string());
    assert!(
        result.is_err(),
        "path traversal characters should be rejected"
    );
}

#[test]
fn task_id_rejects_query_injection() {
    let result = TaskId::new("abc?evil=1".to_string());
    assert!(
        result.is_err(),
        "query string characters should be rejected"
    );
}

#[test]
fn task_id_accepts_underscores() {
    let id = TaskId::new("task_2026_01_01".to_string()).unwrap();
    assert_eq!(id.as_str(), "task_2026_01_01");
}

#[test]
fn task_status_values() {
    assert_eq!(TaskStatus::Pending, TaskStatus::Pending);
    assert_ne!(TaskStatus::Pending, TaskStatus::Completed);
}

#[test]
fn poll_delay_pins_legacy_backoff_curve() {
    // 2s initial, ×2 growth, capped at 30s — the curve the three hand-rolled
    // `interval_secs` accumulators used to produce, pinned before/after the
    // move to `limiteron::retry::delay_for_attempt`.
    let secs: Vec<u64> = (1..=7)
        .map(|attempt| super::backoff::poll_delay("test-host", attempt).as_secs())
        .collect();
    assert_eq!(secs, vec![2, 4, 8, 16, 30, 30, 30]);
}

#[test]
fn poll_delay_is_monotonic_and_capped() {
    let last = super::backoff::poll_delay("test-host", 64);
    assert_eq!(last, std::time::Duration::from_secs(30));
    for attempt in 1..64 {
        assert!(
            super::backoff::poll_delay("test-host", attempt)
                <= super::backoff::poll_delay("test-host", attempt + 1),
            "backoff must not shrink between attempts"
        );
    }
}

// ── R-redact-002: error-path credential stripping (T005/T006) ──────────────

/// Minimal sequential HTTP/1.1 responder: one canned response per connection.
async fn spawn_error_server(responses: Vec<(u16, &'static str)>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        for (status, body) in responses {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut total = Vec::new();
            let mut buf = [0u8; 4096];
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            total.extend_from_slice(&buf[..n]);
                            if total.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
            })
            .await;
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.flush().await;
            let _ = sock.shutdown().await;
        }
    });
    port
}

const ECHO_TOKEN: &str = "sk-echotoken-9f8e7d6c5b4a";

fn echo_client(port: u16) -> MineruPrecisionClient {
    MineruPrecisionClient::with_base_url_and_token(format!("http://127.0.0.1:{port}"), ECHO_TOKEN)
        .unwrap()
}

fn allowlisted_url() -> super::validated_url::ValidatedFileUrl {
    validate_file_url(
        "https://cdn.mineru.net/fixture.pdf",
        DEFAULT_MINERU_ALLOWLIST,
    )
    .unwrap()
}

/// 401 body echoes the Authorization token and a presigned URL: neither the
/// token nor the Signature value may reach `AlchemizeError::to_string()` (R-redact-002).
#[tokio::test]
async fn submit_error_strips_echoed_token_and_presigned_param() {
    let body: &'static str = Box::leak(
        format!(
            "unauthorized: Bearer {ECHO_TOKEN}; download https://cdn.example.com/o.zip?Expires=1719999999&Signature=abc123SECRET&x=1"
        )
        .into_boxed_str(),
    );
    let port = spawn_error_server(vec![(401, body)]).await;
    let err = echo_client(port)
        .parse(&allowlisted_url(), ParseOptions::default())
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(!msg.contains(ECHO_TOKEN), "token leaked: {msg}");
    assert!(
        !msg.contains("Signature=abc123SECRET"),
        "presigned value leaked: {msg}"
    );
    assert!(msg.contains("REDACTED"), "expected redaction marker: {msg}");
    assert!(msg.contains("401"), "status must survive redaction: {msg}");
}

/// 500 body repeats the token three times interleaved with presigned URLs:
/// every occurrence must be gone and all target query values REDACTED (R-redact-002).
#[tokio::test]
async fn poll_error_strips_repeated_echo_without_mangling_body() {
    let body: &'static str = Box::leak(
        format!(
            "err {ECHO_TOKEN} | https://cdn.example.com/a?Signature=sigONE&Expires=1&keep=yes \
             | {ECHO_TOKEN} | {ECHO_TOKEN} end"
        )
        .into_boxed_str(),
    );
    // First response: submit 200 (task accepted); second: poll 500 echo body.
    let port = spawn_error_server(vec![(200, r#"{"task_id":"t-redact"}"#), (500, body)]).await;
    let err = echo_client(port)
        .parse(&allowlisted_url(), ParseOptions::default())
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert_eq!(
        msg.matches(ECHO_TOKEN).count(),
        0,
        "no token occurrence may survive: {msg}"
    );
    assert!(!msg.contains("sigONE"), "signature value leaked: {msg}");
    assert!(msg.contains("REDACTED"), "expected redaction marker: {msg}");
    assert!(
        msg.contains("keep=yes"),
        "non-secret param preserved: {msg}"
    );
    assert!(msg.contains("500"), "status must survive redaction: {msg}");
}

/// A normal 200 response must not be mangled by the redaction layer (R-redact-002).
#[tokio::test]
async fn success_path_markdown_is_not_redacted() {
    let body: &'static str =
        Box::leak("# Report\n\nDownload: https://cdn.example.com/a?Signature=public&x=1\n".into());
    let port = spawn_error_server(vec![
        (200, r#"{"task_id":"t-ok"}"#),
        (
            200,
            Box::leak(
                format!(
                    "{{\"status\":\"completed\",\"markdown\":{:?},\"result_url\":null,\"error\":null}}",
                    body
                )
                .into_boxed_str(),
            ) as &'static str,
        ),
    ])
    .await;
    let md = echo_client(port)
        .parse(&allowlisted_url(), ParseOptions::default())
        .await
        .unwrap();
    assert!(
        md.contains("Signature=public"),
        "success payload intact: {md}"
    );
    assert!(
        !md.contains("REDACTED"),
        "no redaction on success path: {md}"
    );
}

// ── R-audit-001: MinerU audit event emission (T008) ─────────────────────────

/// A non-2xx response on the poll path emits a structured failure event with
/// the `alchemize::audit` target, carrying only host/status/kind (R-audit-001).
#[tokio::test]
async fn http_failure_emits_audit_event() {
    let (capture, _guard) = capture_guard();
    let port = spawn_error_server(vec![
        (200, r#"{"task_id":"t-audit"}"#),
        (500, "upstream exploded"),
    ])
    .await;
    let err = echo_client(port)
        .parse(&allowlisted_url(), ParseOptions::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("500"), "{err}");
    let out = capture.output();
    assert!(out.contains("target=alchemize::audit"), "{out}");
    assert!(out.contains("event=mineru_failure"), "{out}");
    assert!(out.contains("status=500"), "{out}");
    assert!(out.contains("error_kind=http_5xx"), "{out}");
    assert!(!out.contains("Bearer"), "{out}");
    drop(_guard);
}

/// Each scheduled retry emits a retry event with an incrementing attempt
/// number (R-audit-001). Exercises the shared backoff instrumentation point directly.
#[test]
fn retry_events_carry_increasing_attempt_numbers() {
    let (capture, _guard) = capture_guard();
    super::backoff::poll_delay("mineru.example.com", 1);
    super::backoff::poll_delay("mineru.example.com", 2);
    let out = capture.output();
    assert!(out.contains("event=mineru_retry"), "{out}");
    assert!(out.contains("retry_attempt=1"), "{out}");
    assert!(out.contains("retry_attempt=2"), "{out}");
    assert!(out.matches("event=mineru_retry").count() >= 2, "{out}");
    drop(_guard);
}
