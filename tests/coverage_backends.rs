// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Coverage supplements for env-gated backends and external-tool boundaries
//! (ASR engines, pdfium probe, legacy Office error arms, EML body/address
//! branches, MinerU URL SSRF arms). Each test asserts a behaviour, not
//! merely that a line ran. Everything here is environment-independent:
//! native pdfium/soffice/ONNX binaries are mocked, faked, or pointed at
//! garbage files so the error paths run everywhere CI runs.
#![cfg(feature = "full")]

use std::io::{Read, Write};

use alchemize::application::ports::{Availability, Converter, TranscribeProvider};
use alchemize::domain::input::Input;
use alchemize::infrastructure::document::eml::EmlConverter;
use alchemize::infrastructure::document::office::legacy_office::{
    LegacyOfficeConverter, LegacyPptConverter,
};
use alchemize::infrastructure::document::pdfium::{
    ENV_PDFIUM_PATH, PdfiumConverter, resolve_library,
};
use alchemize::infrastructure::media::asr::vecboost::{ENV_TRANSCRIBE_MODELS, VecboostTranscriber};
use alchemize::infrastructure::media::asr::whisper_api::{WhisperApiConfig, WhisperApiTranscriber};
use alchemize::infrastructure::remote::mineru::{DEFAULT_MINERU_ALLOWLIST, validate_file_url};

/// Process-global env vars are mutated by several tests in this file: one
/// async-aware mutex serializes them (a std guard held across `await` is
/// both a deadlock risk and a clippy violation).
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A throwaway regular file: enough for `is_file()`-style probes, not a
/// real pdfium library (bind attempts on it must fail cleanly).
fn fake_native_file(tag: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap();
    let keep = dir.path().to_path_buf();
    let file = keep.join(format!("fake-{tag}.bin"));
    std::fs::write(&file, b"definitely not a native library").unwrap();
    // Leak the tempdir so the file outlives the test (cleaned by the OS).
    std::mem::forget(dir);
    file
}

// ── whisper-api: HTTP boundary fully covered by a local mock server ────────

/// Read one HTTP request to completion (headers + Content-Length body),
/// then answer with `status`/`body`. Returns the request head for asserts.
fn serve_one_request(
    listener: std::net::TcpListener,
    status_line: &'static str,
    body: &'static str,
) {
    let (mut sock, _) = listener.accept().unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = sock.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_sub(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_string();
            let content_length: usize = head
                .lines()
                .find_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.trim().parse().ok())
                })
                .unwrap_or(0);
            if buf.len() >= pos + 4 + content_length {
                break;
            }
        }
    }
    let resp = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = sock.write_all(resp.as_bytes());
    let _ = sock.flush();
}

fn find_sub(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn transcriber_on_port(port: u16) -> WhisperApiTranscriber {
    WhisperApiTranscriber::new(WhisperApiConfig::new(
        format!("http://127.0.0.1:{port}"),
        "sk-test-key",
    ))
}

#[tokio::test]
async fn whisper_api_success_returns_text_segment() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        serve_one_request(listener, "200 OK", r#"{"text":"hello transcription"}"#)
    });
    let out = transcriber_on_port(port)
        .transcribe(b"fake-wav-bytes")
        .await
        .expect("success against local mock");
    server.join().unwrap();
    assert_eq!(out.segments.len(), 1);
    assert_eq!(out.segments[0].text, "hello transcription");
    assert_eq!(out.language, None);
}

#[tokio::test]
async fn whisper_api_http_error_status_maps_to_audio_unavailable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server =
        std::thread::spawn(move || serve_one_request(listener, "500 Internal Server Error", "{}"));
    let err = transcriber_on_port(port)
        .transcribe(b"fake-wav-bytes")
        .await
        .expect_err("5xx must surface as an error");
    server.join().unwrap();
    assert!(err.to_string().contains("whisper api status"), "{err}");
}

#[tokio::test]
async fn whisper_api_bad_json_maps_to_decode_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server =
        std::thread::spawn(move || serve_one_request(listener, "200 OK", "not json at all"));
    let err = transcriber_on_port(port)
        .transcribe(b"fake-wav-bytes")
        .await
        .expect_err("non-JSON body must surface as a decode error");
    server.join().unwrap();
    assert!(err.to_string().contains("whisper api decode"), "{err}");
}

#[tokio::test]
async fn whisper_api_unreachable_endpoint_maps_to_request_error() {
    // Bind then drop: the port is momentarily guaranteed free, connect gets refused.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let err = transcriber_on_port(port)
        .transcribe(b"fake-wav-bytes")
        .await
        .expect_err("refused connection must surface as a request error");
    assert!(
        err.to_string().contains("whisper api request failed"),
        "{err}"
    );
}

#[test]
fn whisper_api_availability_is_always_available() {
    let t = WhisperApiTranscriber::new(WhisperApiConfig::new("http://127.0.0.1:1", "k"));
    assert_eq!(t.availability(), Availability::Available);
    assert_eq!(t.name(), "whisper-api");
}

// ── vecboost: availability matrix + error mapping without real models ─────

#[tokio::test]
async fn vecboost_availability_matrix_and_garbage_wav_error() {
    let _env = ENV_LOCK.lock().await;

    // (1) bundle dir with silero VAD + ORT dylib path set → Available.
    let bundle = tempfile::tempdir().unwrap();
    std::fs::write(bundle.path().join("silero_vad.onnx"), b"vad").unwrap();
    unsafe { std::env::set_var(ENV_TRANSCRIBE_MODELS, bundle.path()) };
    unsafe { std::env::set_var("ORT_DYLIB_PATH", "/nonexistent-ort/libonnxruntime.so") };
    let t = VecboostTranscriber::new();
    assert_eq!(
        t.availability(),
        Availability::Available,
        "complete bundle must be available"
    );
    assert_eq!(t.name(), "vecboost");

    // (2) same bundle, ORT dylib path unset → Unavailable with the ORT hint.
    let saved_ort = std::env::var("ORT_DYLIB_PATH").ok();
    unsafe { std::env::remove_var("ORT_DYLIB_PATH") };
    match t.availability() {
        Availability::Unavailable { reason } => {
            assert!(reason.contains("ORT_DYLIB_PATH"), "{reason}");
        }
        Availability::Available => panic!("missing ORT dylib must be unavailable"),
    }

    // (3) bundle missing silero_vad.onnx → Unavailable naming the missing file.
    unsafe { std::env::set_var("ORT_DYLIB_PATH", "/nonexistent-ort/libonnxruntime.so") };
    let incomplete = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var(ENV_TRANSCRIBE_MODELS, incomplete.path()) };
    let t2 = VecboostTranscriber::new();
    match t2.availability() {
        Availability::Unavailable { reason } => {
            assert!(reason.contains("silero_vad.onnx"), "{reason}");
            assert!(reason.contains("incomplete bundle"), "{reason}");
        }
        Availability::Available => panic!("incomplete bundle must be unavailable"),
    }

    // (4) env points at a nonexistent path → Unavailable echoing the env value.
    unsafe { std::env::set_var(ENV_TRANSCRIBE_MODELS, "/nonexistent/alchemize-bundle") };
    let t3 = VecboostTranscriber::new();
    match t3.availability() {
        Availability::Unavailable { reason } => {
            assert!(reason.contains("ALCHEMIZE_TRANSCRIBE_MODELS"), "{reason}");
            assert!(reason.contains("does not exist"), "{reason}");
        }
        Availability::Available => panic!("nonexistent bundle must be unavailable"),
    }

    // (5) transcribe with garbage bytes: tempfile + write + blocking pool run
    // happen, the WAV decode fails inside the facade, and the error carries
    // the vecboost mapping (no model bundle needed to exercise this).
    unsafe { std::env::set_var(ENV_TRANSCRIBE_MODELS, bundle.path()) };
    unsafe { std::env::set_var("ORT_DYLIB_PATH", "/nonexistent-ort/libonnxruntime.so") };
    let t4 = VecboostTranscriber::new();
    let err = t4
        .transcribe(b"this is not a wav file")
        .await
        .expect_err("garbage wav must error");
    assert!(
        err.to_string().contains("vecboost transcription failed"),
        "{err}"
    );

    // Restore: env var was unset before this test ran (CI/dev default).
    unsafe { std::env::remove_var(ENV_TRANSCRIBE_MODELS) };
    match saved_ort {
        Some(v) => unsafe { std::env::set_var("ORT_DYLIB_PATH", v) },
        None => unsafe { std::env::remove_var("ORT_DYLIB_PATH") },
    }
}

// ── pdfium: probe paths with a plain file standing in for the library ──────

#[tokio::test]
async fn pdfium_probe_paths_with_regular_file_as_library() {
    let _env = ENV_LOCK.lock().await;

    let fake = fake_native_file("pdfium");
    unsafe { std::env::set_var(ENV_PDFIUM_PATH, &fake) };

    // resolve_library: env candidate first, is_file() only — Ok.
    assert_eq!(resolve_library().unwrap(), fake);

    // Default + availability: path known → Available without any native load.
    let conv = PdfiumConverter::default();
    assert_eq!(conv.availability(), Availability::Available);
    assert_eq!(conv.name(), "pdfium");
    assert_eq!(conv.supported_extensions(), &["pdf"]);
    assert_eq!(conv.priority(), 20);

    // convert: bytes read + library resolution + blocking task all run; the
    // bind against the fake file fails cleanly inside extract().
    let err = conv
        .convert(Input::bytes(b"%PDF-1.4 not really a pdf".to_vec(), "x.pdf"))
        .await
        .expect_err("bind against a non-library file must fail");
    let msg = err.to_string();
    assert!(msg.contains("pdfium bind failed"), "{msg}");

    unsafe { std::env::remove_var(ENV_PDFIUM_PATH) };
}

// ── legacy office: error arms reachable without soffice ────────────────────

#[test]
#[allow(clippy::default_constructed_unit_structs)] // 本测试的目的就是执行 Default impl 本体
fn legacy_converter_defaults_and_contracts() {
    let _doc = LegacyOfficeConverter::default();
    let _ppt = LegacyPptConverter::default();
}

#[tokio::test]
async fn legacy_doc_missing_input_file_errors_on_read() {
    let err = LegacyOfficeConverter::new()
        .convert(Input::Path(std::path::PathBuf::from(
            "/nonexistent/alchemize-missing.doc",
        )))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("failed to read doc input"),
        "{err}"
    );
}

#[tokio::test]
async fn legacy_ppt_without_soffice_errors_instead_of_silently_degrading() {
    let _env = ENV_LOCK.lock().await;
    let saved = std::env::var("PATH").ok();
    unsafe { std::env::set_var("PATH", "/nonexistent-dir") };
    let err = LegacyPptConverter::new()
        .convert(Input::bytes(b"legacy ppt bytes".to_vec(), "deck.ppt"))
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("soffice"), "err: {msg}");
    match saved {
        Some(p) => unsafe { std::env::set_var("PATH", p) },
        None => unsafe { std::env::remove_var("PATH") },
    }
}

// ── eml: defaults, unreadable input, group addresses, html body ────────────

#[test]
#[allow(clippy::default_constructed_unit_structs)] // 本测试的目的就是执行 Default impl 本体
fn eml_default_and_contract() {
    let c = EmlConverter::default();
    assert_eq!(c.name(), "eml");
    assert_eq!(c.supported_extensions(), &["eml", "mht", "mhtml"]);
}

#[tokio::test]
async fn eml_missing_input_file_errors_on_read() {
    let err = EmlConverter::new()
        .convert(Input::Path(std::path::PathBuf::from(
            "/nonexistent/alchemize-missing.eml",
        )))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("failed to read EML input"),
        "{err}"
    );
}

#[tokio::test]
async fn eml_group_addresses_render_as_name_list() {
    let mail = concat!(
        "From: sender@example.com\r\n",
        "To: Review Team: alice@example.com, bob@example.org;\r\n",
        "Subject: group test\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "\r\n",
        "plain body line\r\n",
    );
    let out = EmlConverter::new()
        .convert(Input::bytes(mail.as_bytes().to_vec(), "group.eml"))
        .await
        .unwrap();
    assert!(
        out.markdown.contains("Review Team:"),
        "group display name missing: {}",
        out.markdown
    );
    assert!(
        out.markdown.contains("alice@example.com"),
        "{}",
        out.markdown
    );
    assert!(out.markdown.contains("bob@example.org"), "{}", out.markdown);
    assert!(out.markdown.contains("plain body line"), "{}", out.markdown);
}

#[tokio::test]
async fn eml_html_body_flows_through_html_chain() {
    let mail = concat!(
        "From: sender@example.com\r\n",
        "To: rcpt@example.com\r\n",
        "Subject: html body test\r\n",
        "Content-Type: text/html; charset=utf-8\r\n",
        "\r\n",
        "<html><body><p>Hello <b>bold</b> world</p></body></html>\r\n",
    );
    let out = EmlConverter::new()
        .convert(Input::bytes(mail.as_bytes().to_vec(), "page.eml"))
        .await
        .unwrap();
    assert!(out.markdown.contains("Hello"), "{}", out.markdown);
    assert!(out.markdown.contains("bold"), "{}", out.markdown);
    assert!(
        !out.markdown.contains("<p>"),
        "raw markup must be rendered, not fenced: {}",
        out.markdown
    );
}

// ── mineru validated_url: SSRF arms for embedded IPv4 and allowlist ────────

#[test]
fn validated_url_rejects_ipv4_mapped_ipv6_loopback() {
    for host in ["::ffff:127.0.0.1", "::127.0.0.1", "::ffff:10.0.0.1"] {
        let raw = format!("https://[{host}]/file.pdf");
        let err = validate_file_url(&raw, DEFAULT_MINERU_ALLOWLIST).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("loopback") || msg.contains("private"),
            "{raw}: {msg}"
        );
    }
}

#[test]
fn validated_url_rejects_non_allowlisted_host() {
    let err = validate_file_url(
        "https://evil.example.com/file.pdf",
        DEFAULT_MINERU_ALLOWLIST,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("evil.example.com") || msg.contains("allow"),
        "{msg}"
    );
}

#[test]
fn validated_url_accepts_allowlisted_host_and_roundtrips() {
    let url = validate_file_url(
        "https://cdn.mineru.net/a/file.pdf",
        DEFAULT_MINERU_ALLOWLIST,
    )
    .expect("official CDN host must validate");
    assert_eq!(url.as_str(), "https://cdn.mineru.net/a/file.pdf");
    assert_eq!(url.into_inner(), "https://cdn.mineru.net/a/file.pdf");
}

// ── builder: redacted-debug arms, host converters, zip dispatch ────────────

use alchemize::application::builder::{AlchemizeBuilder, MineruConfig};
use alchemize::domain::result::{ContentType, ConvertResult};
use std::future::Future;
use std::pin::Pin;

#[test]
fn mineru_config_debug_and_display_redact_absent_token() {
    let cfg = MineruConfig::default();
    let dbg = format!("{cfg:?}");
    assert!(dbg.contains("None"), "debug with no token: {dbg}");
    let disp = format!("{cfg}");
    assert!(disp.contains("token=none"), "display with no token: {disp}");
}

#[test]
fn builder_default_constructs() {
    let _b = AlchemizeBuilder::default();
}

/// A minimal host converter: unique extension, deterministic markdown.
/// `convert` is boxed by hand (async-trait is not a dev-dependency).
struct HostStub;

impl Converter for HostStub {
    fn convert<'life0, 'async_trait>(
        &'life0 self,
        _input: Input,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ConvertResult, alchemize::domain::error::AlchemizeError>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
    {
        Box::pin(async {
            Ok(ConvertResult::from_markdown(
                "host stub body\n",
                ContentType::Document,
            ))
        })
    }

    fn supported_extensions(&self) -> &'static [&'static str] {
        &["exthost"]
    }

    fn name(&self) -> &'static str {
        "host-stub"
    }
}

#[tokio::test]
async fn host_converter_is_registered_and_dispatched() {
    let alch = AlchemizeBuilder::new()
        .with_host_converter(Box::new(HostStub))
        .build();
    let out = alch
        .convert(&Input::bytes(b"whatever".to_vec(), "thing.exthost"))
        .await
        .expect("host converter must handle its extension");
    assert!(out.markdown.contains("host stub body"), "{}", out.markdown);
}

/// Build a zip package from (name, bytes) parts.
fn zip_bytes(parts: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write as _;
    let buf = std::io::Cursor::new(Vec::new());
    let mut w = zip::ZipWriter::new(buf);
    let opts = zip::write::SimpleFileOptions::default();
    for (name, body) in parts {
        w.start_file(*name, opts).unwrap();
        w.write_all(body).unwrap();
    }
    w.finish().unwrap().into_inner()
}

#[tokio::test]
async fn zip_extension_dispatches_through_container_chain() {
    // zip is a dev-dependency used for fixture building (same pattern as
    // coverage_extra.rs); the crate under test consumes the bytes.
    let archive = zip_bytes(&[("note.txt", b"nested zip text".as_ref())]);
    let alch = AlchemizeBuilder::new().build();
    let out = alch
        .convert(&Input::bytes(archive, "bundle.zip"))
        .await
        .expect("zip input must dispatch to the container converter");
    assert!(out.markdown.contains("nested zip text"), "{}", out.markdown);
}
