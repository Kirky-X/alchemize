// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Structured MinerU failure/retry audit events (R-audit-001).
//!
//! Events go to the dedicated `alchemize::audit` tracing target and carry only
//! structured fields (host, status, attempt, delay, error kind). No token, URL
//! path, or query string is ever included — the payload stays safe for the
//! console sink and any downstream log aggregator.

/// Extract a safe audit host label from a configured base URL.
///
/// Returns `<host>:<port>` (port only when explicit) with no scheme, path,
/// query, or userinfo — so the value can never carry a credential. Falls back
/// to a placeholder when the URL cannot be parsed.
pub(crate) fn endpoint_host_label(base_url: &str) -> String {
    match reqwest::Url::parse(base_url) {
        Ok(url) => match url.host_str() {
            Some(host) => match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            },
            None => "<unknown-host>".to_string(),
        },
        Err(_) => "<unknown-host>".to_string(),
    }
}

/// Emit a structured MinerU failure event.
///
/// `endpoint_host` is the configured base URL host only (no scheme, path, or
/// query); `error_kind` is a fixed label such as `http_5xx` / `http_4xx`.
pub(crate) fn audit_mineru_failure(
    endpoint_host: &str,
    status: u16,
    retry_attempt: u32,
    error_kind: &str,
) {
    tracing::warn!(
        target: "alchemize::audit",
        event = "mineru_failure",
        endpoint_host = endpoint_host,
        status = status,
        retry_attempt = retry_attempt,
        error_kind = error_kind,
        "MinerU request failed"
    );
}

/// Emit a structured MinerU retry event.
pub(crate) fn audit_mineru_retry(endpoint_host: &str, retry_attempt: u32, delay_ms: u64) {
    tracing::warn!(
        target: "alchemize::audit",
        event = "mineru_retry",
        endpoint_host = endpoint_host,
        retry_attempt = retry_attempt,
        delay_ms = delay_ms,
        "MinerU poll retry scheduled"
    );
}

/// Test-only tracing capture utilities (hand-written, no new dependencies).
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Metadata, Subscriber};

    #[derive(Clone, Default)]
    pub(crate) struct Capture(pub(crate) Arc<Mutex<Vec<String>>>);

    impl Capture {
        pub(crate) fn output(&self) -> String {
            self.0.lock().unwrap().join("\n")
        }
    }

    struct FieldVisitor(Vec<String>);

    impl Visit for FieldVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.push(format!("{}={:?}", field.name(), value));
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            self.0.push(format!("{}={}", field.name(), value));
        }
        fn record_i64(&mut self, field: &Field, value: i64) {
            self.0.push(format!("{}={}", field.name(), value));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push(format!("{}={}", field.name(), value));
        }
        fn record_bool(&mut self, field: &Field, value: bool) {
            self.0.push(format!("{}={}", field.name(), value));
        }
    }

    impl Subscriber for Capture {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            metadata.target() == "alchemize::audit"
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, event: &Event<'_>) {
            let mut visitor = FieldVisitor(Vec::new());
            event.record(&mut visitor);
            let line = format!(
                "target={} {}",
                event.metadata().target(),
                visitor.0.join(" ")
            );
            self.0.lock().unwrap().push(line);
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    /// Install a capture subscriber as the thread default. The returned guard
    /// keeps it active until dropped — safe across `.await` in current-thread
    /// tokio tests.
    pub(crate) fn capture_guard() -> (Capture, tracing::subscriber::DefaultGuard) {
        let buf = Capture::default();
        let guard = tracing::subscriber::set_default(buf.clone());
        (buf, guard)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::Capture;
    use super::*;

    fn capture_events<F: FnOnce()>(f: F) -> String {
        let buf = Capture::default();
        tracing::subscriber::with_default(buf.clone(), f);
        buf.output()
    }

    #[test]
    fn endpoint_host_label_strips_scheme_path_query_and_credentials() {
        assert_eq!(
            endpoint_host_label("https://mineru.example.com:8443/api?Signature=secret"),
            "mineru.example.com:8443"
        );
        assert_eq!(
            endpoint_host_label("http://127.0.0.1:9000"),
            "127.0.0.1:9000"
        );
        assert!(!endpoint_host_label("not a url").contains("Signature"));
    }

    #[test]
    fn failure_event_has_audit_target_and_structured_fields() {
        let out = capture_events(|| {
            audit_mineru_failure("mineru.example.com", 503, 2, "http_5xx");
        });
        assert!(out.contains("target=alchemize::audit"), "{out}");
        assert!(out.contains("event=mineru_failure"), "{out}");
        assert!(out.contains("endpoint_host=mineru.example.com"), "{out}");
        assert!(out.contains("status=503"), "{out}");
        assert!(out.contains("error_kind=http_5xx"), "{out}");
        assert!(out.contains("retry_attempt=2"), "{out}");
        assert!(!out.contains("Bearer"), "{out}");
        assert!(!out.contains("Signature="), "{out}");
    }

    #[test]
    fn retry_event_has_audit_target_and_attempt_delay() {
        let out = capture_events(|| {
            audit_mineru_retry("mineru.example.com", 3, 8000);
        });
        assert!(out.contains("target=alchemize::audit"), "{out}");
        assert!(out.contains("event=mineru_retry"), "{out}");
        assert!(out.contains("delay_ms=8000"), "{out}");
        assert!(out.contains("retry_attempt=3"), "{out}");
        assert!(!out.contains("Bearer"), "{out}");
    }
}
