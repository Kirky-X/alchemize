// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! HTTP response size limits to prevent OOM from malicious/abnormal responses .

use reqwest::Response;

use crate::domain::error::AlchemizeError;

/// Maximum size of a MinerU task-submit response (task_id only, very small).
pub(super) const MAX_SUBMIT_RESPONSE_BYTES: u64 = 64 * 1024; // 64KB

/// Maximum size of a MinerU error response (diagnostic body).
///
/// Kept small (8KB) to limit information leakage when the error body is
/// embedded into `AlchemizeError::MineruConversion(format!("... {body}"))` —
/// large bodies may carry server paths, stack traces, or echoed secrets.
pub(super) const MAX_ERROR_RESPONSE_BYTES: u64 = 8 * 1024; // 8KB

/// Maximum size of a MinerU poll response (may include inline markdown).
pub(super) const MAX_POLL_RESPONSE_BYTES: u64 = 50 * 1024 * 1024; // 50MB

/// Maximum size of a MinerU result fetch response (markdown content).
pub(super) const MAX_RESULT_RESPONSE_BYTES: u64 = 50 * 1024 * 1024; // 50MB

/// Upper bound on the initial `Vec` capacity hint derived from Content-Length.
///
/// Caps over-allocation when a malicious server reports a huge Content-Length
/// that still fits under `max_bytes` (e.g. 49MB against a 50MB limit).
const MAX_PREALLOC_BYTES: usize = 8 * 1024 * 1024; // 8MB

/// Read a response body as bytes with a hard size limit.
///
/// Uses streaming reads and aborts as soon as the limit is exceeded, so a
/// 10GB response cannot exhaust memory before the limit check fires.
///
/// Returns `Err(AlchemizeError::ResponseTooLarge)` if the body exceeds `max_bytes`.
pub(super) async fn read_limited_bytes(
    resp: Response,
    max_bytes: u64,
) -> Result<Vec<u8>, AlchemizeError> {
    use futures::StreamExt;

    // Content-Length preflight: reject oversized responses before downloading.
    let content_length = resp.content_length();
    if let Some(cl) = content_length {
        if cl > max_bytes {
            return Err(AlchemizeError::ResponseTooLarge {
                actual: cl,
                limit: max_bytes,
            });
        }
    }

    // Pre-allocate based on Content-Length (capped) to avoid ~23 reallocs on
    // 50MB responses. Falls back to 0 when Content-Length is absent.
    let initial_cap = content_length
        .filter(|&cl| cl <= max_bytes)
        .map(|cl| (cl as usize).min(MAX_PREALLOC_BYTES))
        .unwrap_or(0);

    // Stream the body, accumulating bytes and aborting on limit breach.
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(initial_cap);
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| {
            AlchemizeError::MineruConversion(format!("failed reading response stream: {e}"))
        })?;
        let new_len = buf.len().saturating_add(chunk.len());
        if new_len as u64 > max_bytes {
            return Err(AlchemizeError::ResponseTooLarge {
                actual: new_len as u64,
                limit: max_bytes,
            });
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(buf)
}

/// Cap a remote response body embedded in an error message.
///
/// Error text can reach CLI stderr / MCP responses; keep it short and free
/// of unbounded remote content.
pub(super) fn truncate_body(body: String) -> String {
    const MAX_CHARS: usize = 512;
    if body.chars().count() <= MAX_CHARS {
        body
    } else {
        let mut out: String = body.chars().take(MAX_CHARS).collect();
        out.push('…');
        out
    }
}

/// Read a response body as text with a hard size limit.
pub(super) async fn read_limited_text(
    resp: Response,
    max_bytes: u64,
) -> Result<String, AlchemizeError> {
    let bytes = read_limited_bytes(resp, max_bytes).await?;
    String::from_utf8(bytes).map_err(|e| {
        AlchemizeError::MineruConversion(format!("response body is not valid UTF-8: {e}"))
    })
}

/// Read and JSON-decode a response body with a hard size limit.
pub(super) async fn read_limited_json<T>(
    resp: Response,
    max_bytes: u64,
) -> Result<T, AlchemizeError>
where
    T: serde::de::DeserializeOwned,
{
    let bytes = read_limited_bytes(resp, max_bytes).await?;
    serde_json::from_slice(&bytes).map_err(|e| {
        AlchemizeError::MineruConversion(format!("failed to parse JSON response: {e}"))
    })
}

#[cfg(test)]
mod tests {
    //! Coverage for `read_limited_bytes` / `read_limited_text` / `read_limited_json`.
    //!
    //! Tests use `http::Response::builder` + `reqwest::Body` to construct
    //! `reqwest::Response` instances without spinning up an HTTP server.

    use super::*;
    use bytes::Bytes;
    use futures::stream;
    use reqwest::Body;

    /// Build a response with a single-chunk body and an explicit Content-Length.
    ///
    /// `content_length` overrides the actual body length so we can simulate
    /// a server that lies about its Content-Length header.
    fn make_response(body: Vec<u8>, content_length: Option<u64>) -> Response {
        let mut builder = http::Response::builder().status(200);
        if let Some(cl) = content_length {
            builder = builder.header("content-length", cl.to_string());
        }
        let http_resp = builder.body(Body::from(body)).unwrap();
        Response::from(http_resp)
    }

    /// Build a chunked-streaming response with no Content-Length header.
    /// `bytes_stream` will iterate the chunks one by one.
    fn make_streamed_response(chunks: Vec<Vec<u8>>) -> Response {
        let stream = stream::iter(
            chunks
                .into_iter()
                .map(|c| Ok::<Bytes, std::io::Error>(Bytes::from(c))),
        );
        let http_resp = http::Response::builder()
            .status(200)
            .body(Body::wrap_stream(stream))
            .unwrap();
        Response::from(http_resp)
    }

    #[tokio::test]
    async fn read_limited_bytes_preflight_rejects_oversized_content_length() {
        // `Response::content_length` reads `Body::size_hint.exact`,
        // not the Content-Length header — so we must provide a real 100-byte
        // body to trigger the preflight. Preflight fires before streaming.
        let resp = make_response(vec![0u8; 100], None);
        let err = read_limited_bytes(resp, 50).await.unwrap_err();
        assert!(
            matches!(
                err,
                AlchemizeError::ResponseTooLarge {
                    actual: 100,
                    limit: 50
                }
            ),
            "expected ResponseTooLarge {{actual: 100, limit: 50}}, got {err:?}"
        );
    }

    #[tokio::test]
    async fn read_limited_bytes_streaming_overflow_aborts_mid_stream() {
        // No Content-Length — preflight cannot catch this. Two chunks of 30
        // bytes each; max=50 means the second chunk pushes total to 60 > 50.
        let chunks = vec![vec![0u8; 30], vec![0u8; 30]];
        let resp = make_streamed_response(chunks);
        let err = read_limited_bytes(resp, 50).await.unwrap_err();
        match err {
            AlchemizeError::ResponseTooLarge { actual, limit } => {
                assert_eq!(limit, 50);
                assert_eq!(actual, 60, "actual should be 30+30=60");
            }
            other => panic!("expected ResponseTooLarge, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_limited_bytes_at_limit_succeeds_no_off_by_one() {
        // Body size == max_bytes must succeed (strict > comparison, not >=).
        let body = vec![0xABu8; 50];
        let resp = make_response(body.clone(), None);
        let out = read_limited_bytes(resp, 50).await.unwrap();
        assert_eq!(out.len(), 50);
        assert_eq!(out, body);
    }

    #[tokio::test]
    async fn read_limited_bytes_empty_body_succeeds() {
        let resp = make_response(Vec::new(), Some(0));
        let out = read_limited_bytes(resp, 50).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn read_limited_text_non_utf8_returns_mineru_conversion_error() {
        // 0xFF is invalid as a leading UTF-8 byte.
        let resp = make_response(vec![0xFFu8, 0xFE, 0xFD], None);
        let err = read_limited_text(resp, 50).await.unwrap_err();
        assert!(
            matches!(err, AlchemizeError::MineruConversion(_)),
            "expected MineruConversion for invalid UTF-8, got {err:?}"
        );
    }

    #[tokio::test]
    async fn read_limited_json_invalid_json_returns_mineru_conversion_error() {
        let resp = make_response(b"not json".to_vec(), None);
        let err: AlchemizeError = read_limited_json::<serde_json::Value>(resp, 50)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AlchemizeError::MineruConversion(_)),
            "expected MineruConversion for invalid JSON, got {err:?}"
        );
    }

    #[tokio::test]
    async fn read_limited_json_valid_json_decodes_successfully() {
        let payload = br#"{"task_id":"abc-123"}"#.to_vec();
        let resp = make_response(payload, None);
        let value: serde_json::Value = read_limited_json(resp, 50).await.unwrap();
        assert_eq!(value["task_id"], "abc-123");
    }
}
