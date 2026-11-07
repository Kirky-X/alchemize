// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Jupyter Notebook (IPYNB) to Markdown converter.
//!
//! Extracts code cells and markdown cells from Jupyter notebooks,
//! preserving the notebook structure with cell type markers.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};


/// Converts Jupyter Notebook (.ipynb) files to markdown.
///
/// Each cell is rendered with appropriate markers:
/// - Markdown cells: rendered directly
/// - Code cells: wrapped in fenced code blocks
/// - Output cells: rendered as output blocks
pub struct IpynbConverter;

impl IpynbConverter {
    /// Create a new notebook converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for IpynbConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for IpynbConverter {
    fn name(&self) -> &'static str {
        "ipynb"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::Other(format!("failed to read notebook input: {e}")))?;

        let json = String::from_utf8(bytes)
            .map_err(|e| AlchemizeError::Other(format!("notebook content is not valid UTF-8: {e}")))?;

        let notebook: Notebook = serde_json::from_str(&json)
            .map_err(|e| AlchemizeError::Other(format!("failed to parse notebook JSON: {e}")))?;

        let mut markdown_parts = Vec::new();
        let mut code_cell_count = 0;
        let mut markdown_cell_count = 0;

        // Add notebook metadata header if available
        if let Some(ref metadata) = notebook.metadata {
            if let Some(ref kernel) = metadata.kernelspec {
                markdown_parts.push(format!("# Jupyter Notebook ({})\n\n", kernel.display_name));
            }
        }

        // Process each cell
        for cell in notebook.cells {
            match cell.cell_type.as_str() {
                "markdown" => {
                    let content = join_cell_source(&cell.source);
                    if !content.trim().is_empty() {
                        markdown_parts.push(content);
                        markdown_parts.push(String::new());
                        markdown_cell_count += 1;
                    }
                }
                "code" => {
                    let code = join_cell_source(&cell.source);
                    if !code.trim().is_empty() {
                        let language = notebook
                            .metadata
                            .as_ref()
                            .and_then(|m| m.language_info.as_ref())
                            .map(|l| l.name.clone())
                            .unwrap_or_else(|| "python".to_string());

                        markdown_parts.push(format!("```{language}\n{code}\n```\n"));

                        // Include outputs if present
                        if let Some(outputs) = cell.outputs {
                            for output in outputs {
                                if output.output_type == "stream" {
                                    if let Some(text) = output.text {
                                        let text_content = join_multiline(&text);
                                        markdown_parts.push(format!(
                                            "**Output:**\n```\n{text_content}\n```\n"
                                        ));
                                    }
                                } else if output.output_type == "execute_result"
                                    || output.output_type == "display_data"
                                {
                                    if let Some(data) = output.data {
                                        if let Some(text_plain) = data.get("text/plain") {
                                            let text_content = extract_json_string(text_plain);
                                            markdown_parts.push(format!(
                                                "**Output:**\n```\n{text_content}\n```\n"
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        markdown_parts.push(String::new());
                        code_cell_count += 1;
                    }
                }
                _ => {
                    // Skip unknown cell types (raw, etc.)
                }
            }
        }

        let markdown = if markdown_parts.is_empty() {
            "*Jupyter notebook contains no extractable content.*\n".to_string()
        } else {
            markdown_parts.join("\n")
        };

        let metadata = serde_json::json!({
            "source_format": "ipynb",
            "code_cell_count": code_cell_count,
            "markdown_cell_count": markdown_cell_count,
            "nbformat": notebook.nbformat,
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["ipynb"]
    }
}

/// Join cell source lines into a single string.
fn join_cell_source(source: &[serde_json::Value]) -> String {
    source
        .iter()
        .map(|v| {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else if let Some(arr) = v.as_array() {
                arr.iter()
                    .filter_map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join("")
            } else {
                String::new()
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Join multiline output text.
fn join_multiline(text: &serde_json::Value) -> String {
    if let Some(s) = text.as_str() {
        s.to_string()
    } else if let Some(arr) = text.as_array() {
        arr.iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("")
    } else {
        String::new()
    }
}

/// Extract string from JSON value.
fn extract_json_string(value: &serde_json::Value) -> String {
    if let Some(s) = value.as_str() {
        s.to_string()
    } else if let Some(arr) = value.as_array() {
        arr.iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("")
    } else {
        value.to_string()
    }
}

// Notebook JSON structures

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Notebook {
    cells: Vec<Cell>,
    metadata: Option<NotebookMetadata>,
    nbformat: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cell {
    cell_type: String,
    source: Vec<serde_json::Value>,
    outputs: Option<Vec<Output>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NotebookMetadata {
    kernelspec: Option<KernelSpec>,
    language_info: Option<LanguageInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KernelSpec {
    display_name: String,
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanguageInfo {
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Output {
    output_type: String,
    text: Option<serde_json::Value>,
    data: Option<serde_json::Map<String, serde_json::Value>>,
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions() {
        let converter = IpynbConverter::new();
        assert_eq!(converter.supported_extensions(), &["ipynb"]);
    }

    #[test]
    fn default_creates_instance() {
        let converter = IpynbConverter;
        assert_eq!(converter.supported_extensions(), &["ipynb"]);
    }

    #[tokio::test]
    async fn convert_simple_notebook() {
        let converter = IpynbConverter::new();
        // Use a simple JSON without # to avoid raw string parsing issues
        let notebook_json = "{\"cells\":[{\"cell_type\":\"markdown\",\"source\":[\"Title\"]},{\"cell_type\":\"code\",\"source\":[\"x = 1\"],\"outputs\":[]}],\"metadata\":{},\"nbformat\":4}";
        let input = Input::Bytes {
            data: notebook_json.as_bytes().to_vec(),
            filename: "test.ipynb".to_string(),
            mimetype: None,
            extension: None,
        };
        let result = converter.convert(input).await.unwrap();
        assert!(result.markdown.contains("Title"));
        assert!(result.markdown.contains("x = 1"));
    }
}
