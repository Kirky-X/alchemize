// SPDX-License-Identifier: Apache-2.0
//! Plain text and code/config file converter with encoding detection.

use async_trait::async_trait;

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

use super::shared::detect_encoding;

/// Extensions routed to this converter, with markdown language tags where a
/// fenced code block applies. `None` = passthrough (no fence).
fn lang_tag(ext: &str) -> Option<&'static str> {
    match ext {
        // Plain text family: passthrough, no fence
        "txt" | "text" | "log" | "md" | "markdown" | "rst" | "org" => None,
        // Code / config family: fenced with a language tag
        "py" | "pyi" => Some("python"),
        "rs" => Some("rust"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("jsx"),
        "ts" | "tsx" => Some("typescript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "swift" => Some("swift"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" => Some("cpp"),
        "cs" => Some("csharp"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "lua" => Some("lua"),
        "r" => Some("r"),
        "sql" => Some("sql"),
        "sh" | "bash" | "zsh" => Some("bash"),
        "fish" => Some("fish"),
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "xml" => Some("xml"),
        "ini" | "cfg" | "conf" => Some("ini"),
        "proto" => Some("proto"),
        "graphql" | "gql" => Some("graphql"),
        "css" => Some("css"),
        "scss" => Some("scss"),
        "html" | "htm" => None, // routed to the HTML converter, never here
        _ => None,
    }
}

/// Whether an extension belongs to the code/config family (fenced output).
fn is_code_family(ext: &str) -> bool {
    lang_tag(ext).is_some()
}

/// Converts plain text and code/config files to markdown.
///
/// Plain text passes through as-is (encoding detected); code/config files
/// render as fenced code blocks with a language tag mapped from the
/// extension. Unknown code-ish extensions (fence but no tag) use `text`.
pub struct TextConverter;

impl TextConverter {
    /// Create a new text converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for TextConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Code/config extensions recognized by the converter.
pub const CODE_EXTENSIONS: &[&str] = &[
    "py", "pyi", "rs", "js", "mjs", "cjs", "jsx", "ts", "tsx", "go", "java", "kt", "kts",
    "swift", "c", "h", "cpp", "cc", "cxx", "hpp", "cs", "rb", "php", "lua", "r", "sql", "sh",
    "bash", "zsh", "fish", "json", "yaml", "yml", "toml", "xml", "ini", "cfg", "conf", "proto",
    "graphql", "gql", "css", "scss",
];

#[async_trait]
impl Converter for TextConverter {
    fn name(&self) -> &'static str {
        "text"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read text input: {e}")))?;

        let (text, encoding_name) = detect_encoding(&bytes);

        let ext = input.extension().unwrap_or_default();
        let line_count = text.lines().count();
        let word_count = text.split_whitespace().count();
        let markdown = if text.trim().is_empty() {
            "*Text file is empty.*\n".to_string()
        } else if ext == "md" || ext == "markdown" {
            // Markdown is the target format: passthrough unchanged.
            text
        } else if is_code_family(&ext) {
            let tag = lang_tag(&ext).unwrap_or("text");
            format!("```{tag}\n{text}\n```\n")
        } else {
            text
        };

        let metadata = serde_json::json!({
            "source_format": if is_code_family(&ext) { "code" } else { "text" },
            "encoding": encoding_name,
            "line_count": line_count,
            "word_count": word_count,
            "byte_count": bytes.len(),
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &SUPPORTED
    }
}

/// Supported extensions (text family + code family), built once.
static SUPPORTED: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
    let mut v = vec!["txt", "text", "log", "md", "markdown", "rst", "org"];
    v.extend_from_slice(CODE_EXTENSIONS);
    v.sort_unstable();
    v
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_include_code_family() {
        let converter = TextConverter::new();
        let exts = converter.supported_extensions();
        for expected in ["txt", "md", "py", "rs", "json", "yaml", "toml", "sh"] {
            assert!(exts.contains(&expected), "missing {expected}");
        }
    }

    #[tokio::test]
    async fn convert_utf8_text_passthrough() {
        let converter = TextConverter::new();
        let input = Input::bytes("Hello, world!\nThis is a test.".as_bytes().to_vec(), "t.txt");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Hello"));
        assert!(!result.markdown.contains("```"));
        assert_eq!(result.quality, ConversionQuality::LocalFull);
    }

    #[tokio::test]
    async fn convert_markdown_passthrough_unchanged() {
        let converter = TextConverter::new();
        let src = "# Title\n\nbody";
        let input = Input::bytes(src.as_bytes().to_vec(), "doc.md");
        let result = converter.convert(input).await.unwrap();
        assert_eq!(result.markdown, src);
    }

    #[tokio::test]
    async fn convert_python_gets_fenced_block() {
        let converter = TextConverter::new();
        let src = "def main():\n    print('hi')";
        let input = Input::bytes(src.as_bytes().to_vec(), "app.py");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.starts_with("```python\n"));
        assert!(result.markdown.ends_with("```\n"));
        assert!(result.markdown.contains("def main():"));
        assert_eq!(result.metadata["source_format"], "code");
    }

    #[tokio::test]
    async fn convert_yaml_gets_language_tag() {
        let converter = TextConverter::new();
        let input = Input::bytes(b"key: value".to_vec(), "conf.yaml");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.starts_with("```yaml\n"));
    }

    #[tokio::test]
    async fn convert_empty_text() {
        let converter = TextConverter::new();
        let input = Input::bytes(vec![], "empty.txt");
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("empty"));
    }

    #[test]
    fn lang_tag_mapping_samples() {
        assert_eq!(lang_tag("rs"), Some("rust"));
        assert_eq!(lang_tag("tsx"), Some("typescript"));
        assert_eq!(lang_tag("cpp"), Some("cpp"));
        assert_eq!(lang_tag("toml"), Some("toml"));
        assert_eq!(lang_tag("txt"), None);
        assert_eq!(lang_tag("md"), None);
    }
}
