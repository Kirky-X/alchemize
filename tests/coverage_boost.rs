// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Coverage boosters: branches not exercised by the golden corpus.

use std::sync::Arc;

use alchemize::application::builder::{Alchemize, AlchemizeBuilder, AssetMode, MineruConfig};
use alchemize::application::ports::{
    Availability, Transcript, TranscriptSegment, TranscribeProvider, VisionProvider,
};
use alchemize::domain::error::AlchemizeError;
use alchemize::domain::input::Input;
use alchemize::application::ports::Converter as _;
use alchemize::domain::result::ConvertResult;

// -- builder ----------------------------------------------------------------

#[test]
fn builder_engines_and_counts() {
    let alch = AlchemizeBuilder::new().build();
    let engines = alch.engines();
    assert!(engines.contains(&"text"));
    assert!(engines.contains(&"pdf"));
    assert!(engines.contains(&"subtitle"));
    assert_eq!(alch.engine_count(), engines.len());
    assert!(!alch.is_no_remote());
}

#[test]
fn builder_mineru_engine_registered_when_configured() {
    // The mineru engine itself is feature-gated; without the feature the
    // engine list must simply not contain it.
    let alch = AlchemizeBuilder::new()
        .with_mineru(MineruConfig::default())
        .build();
    let has_mineru = alch.engines().iter().any(|e| e.contains("mineru"));
    #[cfg(feature = "mineru")]
    assert!(has_mineru);
    #[cfg(not(feature = "mineru"))]
    assert!(!has_mineru);
}

#[test]
fn builder_no_remote_flag_and_availability_rows() {
    let alch = AlchemizeBuilder::new().with_no_remote().build();
    assert!(alch.is_no_remote());
    let rows = alch.availability();
    assert!(rows.iter().any(|(name, _, _)| name == "zip"));
    assert!(rows.iter().any(|(name, _, _)| name == "text"));
}

#[test]
fn builder_inline_asset_sink_mode() {
    let alch = AlchemizeBuilder::new()
        .with_asset_mode(AssetMode::Inline)
        .build();
    let mut sink = alch.asset_sink_for(std::path::Path::new("/tmp/out"));
    let url = sink
        .put(alchemize::domain::asset::Asset::new("a.png", vec![1]))
        .unwrap();
    assert!(url.starts_with("data:image/png;base64,"));
}

#[test]
fn builder_max_file_size_round_trip() {
    // Building with a custom size must not change engine count semantics.
    let a = AlchemizeBuilder::new().build();
    let b = AlchemizeBuilder::new()
        .with_max_file_size(1024)
        .with_video_mode(alchemize::domain::result::VideoProcessingMode::Disabled)
        .build();
    assert_eq!(a.engine_count(), b.engine_count());
}

// -- ipynb output branches ----------------------------------------------------

fn ipynb_json(cells: &str) -> String {
    format!(
        r#"{{"cells":[{cells}],"metadata":{{"kernelspec":{{"display_name":"Python 3","name":"python3"}},"language_info":{{"name":"python"}}}},"nbformat":4}}"#
    )
}

#[tokio::test]
async fn ipynb_stream_output_rendered() {
    let cells = r#"{"cell_type":"code","source":["print('hi')"],"outputs":[{"output_type":"stream","name":"stdout","text":["hi\n"]}]}"#;
    let converter = alchemize::infrastructure::document::IpynbConverter::new();
    let input = Input::bytes(ipynb_json(cells).into_bytes(), "n.ipynb");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("print('hi')"));
    assert!(result.markdown.contains("**Output:**"));
    assert!(result.markdown.contains("hi"));
    assert!(result.metadata["code_cell_count"] == 1);
}

#[tokio::test]
async fn ipynb_execute_result_text_plain() {
    let cells = r#"{"cell_type":"code","source":["1+1"],"outputs":[{"output_type":"execute_result","data":{"text/plain":"2"}}]}"#;
    let converter = alchemize::infrastructure::document::IpynbConverter::new();
    let input = Input::bytes(ipynb_json(cells).into_bytes(), "n.ipynb");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("2"));
}

#[tokio::test]
async fn ipynb_kernel_header() {
    let converter = alchemize::infrastructure::document::IpynbConverter::new();
    let input = Input::bytes(ipynb_json(r#"{"cell_type":"raw","source":["meta"]}"#).into_bytes(), "n.ipynb");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("Jupyter Notebook (Python 3)"));
}

#[tokio::test]
async fn ipynb_markdown_cell_passthrough_and_raw_skip() {
    let cells = "{\"cell_type\":\"markdown\",\"source\":[\"## Notes\"]},{\"cell_type\":\"raw\",\"source\":[\"raw meta\"]}";
    let converter = alchemize::infrastructure::document::IpynbConverter::new();
    let input = Input::bytes(ipynb_json(cells).into_bytes(), "n.ipynb");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("## Notes"));
    assert!(!result.markdown.contains("raw meta"), "raw cells skipped");
    assert!(result.metadata["markdown_cell_count"] == 1);
}

#[tokio::test]
async fn ipynb_empty_notebook_message() {
    let converter = alchemize::infrastructure::document::IpynbConverter::new();
    let input = Input::bytes(br#"{"cells":[],"metadata":{},"nbformat":4}"#.to_vec(), "e.ipynb");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("no extractable content"));
}

#[tokio::test]
async fn ipynb_array_form_outputs_and_values() {
    // stream text as array; execute_result text/plain as array; a numeric data value
    let cells = r#"{"cell_type":"code","source":["x"],"outputs":[{"output_type":"stream","text":["line1\n","line2\n"]},{"output_type":"execute_result","data":{"text/plain":["a","b"]}}]}"#;
    let converter = alchemize::infrastructure::document::IpynbConverter::new();
    let input = Input::bytes(ipynb_json(cells).into_bytes(), "n.ipynb");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("line1"), "md: {}", result.markdown);
    assert!(result.markdown.contains("ab"));
}

// -- xlsx branches (python-generated fixtures; see tests/fixtures) ---------

#[tokio::test]
async fn xlsx_multi_sheet_markdown_tables() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xlsx_multi/sample.xlsx");
    let converter = alchemize::infrastructure::document::XlsxConverter::new();
    let result = converter
        .convert(Input::Path(path.clone()))
        .await
        .unwrap();
    assert!(result.markdown.contains("## Alpha"), "md: {}", result.markdown);
    assert!(result.markdown.contains("## Beta"));
}

#[tokio::test]
async fn xlsx_hidden_sheet_skipped() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xlsx_hidden/sample.xlsx");
    let converter = alchemize::infrastructure::document::XlsxConverter::new();
    let result = converter.convert(Input::Path(path.clone())).await.unwrap();
    assert!(result.markdown.contains("no extractable data"), "md: {}", result.markdown);
}

// -- video branches (no ffmpeg required) ----------------------------------------

#[tokio::test]
async fn video_disabled_mode_no_subprocess() {
    let converter = alchemize::infrastructure::media::VideoConverter::new(
        alchemize::domain::result::VideoProcessingMode::Disabled,
    );
    let input = Input::bytes(vec![0, 0, 0, 24, b'f', b't', b'y', b'p'], "v.mp4");
    let result = converter.convert(input).await.unwrap();
    assert!(result.quality == alchemize::domain::result::ConversionQuality::LocalFallback);
    assert!(!result.markdown.is_empty());
}

#[tokio::test]
async fn video_audioonly_without_ffmpeg_degrades() {
    // ffmpeg absent on this host: AudioOnly must degrade, not error.
    let converter = alchemize::infrastructure::media::VideoConverter::new(
        alchemize::domain::result::VideoProcessingMode::AudioOnly,
    );
    let input = Input::bytes(vec![0, 0, 0, 24, b'f', b't', b'y', b'p'], "v.mp4");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("v.mp4") || !result.markdown.is_empty());
}

// -- vision provider plumbing (image describe path) ------------------------------

struct EchoVision;

#[async_trait::async_trait]
impl VisionProvider for EchoVision {
    async fn describe(&self, _image: &[u8], _detail: alchemize::domain::result::DetailLevel) -> anyhow::Result<String> {
        Ok("a chart".to_string())
    }
    async fn ocr(&self, _image: &[u8]) -> anyhow::Result<String> {
        Ok("chart text".to_string())
    }
    fn name(&self) -> &'static str {
        "echo"
    }
}

#[tokio::test]
async fn image_with_vision_describe_and_ocr() {
    let converter = alchemize::infrastructure::media::ImageConverter::with_vision(Arc::new(EchoVision));
    let input = Input::bytes(vec![0x89, b'P', b'N', b'G', 0, 0, 0, 0], "i.png");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("a chart"), "md: {}", result.markdown);
    assert!(result.markdown.contains("chart text"));
    assert_eq!(result.quality, alchemize::domain::result::ConversionQuality::LocalFull);
}

// -- transcribe plumbing (audio provider path) ------------------------------------

struct EchoTranscriber;

#[async_trait::async_trait]
impl TranscribeProvider for EchoTranscriber {
    async fn transcribe(&self, _audio: &[u8]) -> Result<Transcript, AlchemizeError> {
        Ok(Transcript {
            language: Some("zh".into()),
            duration: Some(2.0),
            segments: vec![TranscriptSegment {
                start_sec: 0.0,
                end_sec: 2.0,
                text: "你好世界".into(),
            }],
        })
    }
    fn availability(&self) -> Availability {
        Availability::Available
    }
    fn name(&self) -> &'static str {
        "echo"
    }
}

#[tokio::test]
async fn audio_with_transcriber_full_pipeline() {
    let converter =
        alchemize::infrastructure::media::AudioConverter::with_transcriber(Arc::new(EchoTranscriber));
    // Minimal WAV header (zero data) is enough: the stub ignores bytes.
    let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    let input = Input::bytes(wav, "a.wav");
    let result = converter.convert(input).await.unwrap();
    assert!(result.markdown.contains("你好世界"), "md: {}", result.markdown);
    assert!(result.markdown.contains("Transcript"));
}

// -- facade convert (non-zip path through convert()) --------------------------------

#[tokio::test]
async fn facade_convert_routes_text() {
    let alch = AlchemizeBuilder::new().build();
    let input = Input::bytes(b"facade route".to_vec(), "f.txt");
    let result: ConvertResult = alch.convert(&input).await.unwrap();
    assert!(result.markdown.contains("facade route"));
}
