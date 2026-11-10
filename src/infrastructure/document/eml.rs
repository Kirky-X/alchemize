// SPDX-License-Identifier: Apache-2.0
//! EML (RFC 822 / MIME) converter — headers, the text body, and the
//! attachment inventory (names + sizes; attachment bytes are not extracted).
//!
//! Uses `mail-parser` rather than the `mail` crate: `mail` pulls
//! `mail-internals` with an unfixed use-after-free (RUSTSEC-2023-0054).

#![cfg(feature = "eml")]

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};
use crate::infrastructure::document::shared::fence_for;

/// EML converter (headers + text body + attachment list).
pub struct EmlConverter;

impl EmlConverter {
    /// Create a new EML converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for EmlConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for EmlConverter {
    fn name(&self) -> &'static str {
        "eml"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read EML input: {e}")))?;

        let parsed = mail_parser::MessageParser::default()
            .parse(&bytes[..])
            .ok_or_else(|| {
                AlchemizeError::other("EML parse failed: not a valid RFC 822 message")
            })?;

        let markdown = render_message(&parsed);
        let attachments = collect_attachments(&parsed);

        let metadata = serde_json::json!({
            "source_format": "eml",
            "subject": parsed.subject().unwrap_or_default(),
            "attachment_count": attachments.len(),
            "attachments": attachments,
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["eml"]
    }

    fn priority(&self) -> u8 {
        10
    }

    fn availability(&self) -> Availability {
        Availability::Available
    }
}

/// Render headers plus the best available body part.
fn render_message(parsed: &mail_parser::Message<'_>) -> String {
    let mut out = String::new();
    out.push_str("# ");
    out.push_str(
        &crate::infrastructure::document::shared::escape_markdown_label(
            parsed.subject().unwrap_or("(no subject)"),
        ),
    );
    out.push_str("\n\n");

    let field = |name: &str, value: Option<String>| {
        let value = value.unwrap_or_default();
        if value.is_empty() {
            String::new()
        } else {
            format!(
                "- **{name}**: {}\n",
                crate::infrastructure::document::shared::escape_markdown_label(&value)
            )
        }
    };
    out.push_str(&field(
        "From",
        parsed.from().map(|l| render_address_list(&l)),
    ));
    out.push_str(&field("To", parsed.to().map(|l| render_address_list(&l))));
    out.push_str(&field("Cc", parsed.cc().map(|l| render_address_list(&l))));
    out.push_str(&field("Date", parsed.date().map(|d| d.to_rfc3339())));
    if let Some(id) = parsed.message_id() {
        out.push_str(&field("Message-ID", Some(id.to_string())));
    }
    out.push('\n');

    // Prefer text/plain; fall back to HTML rendered as a code block (no HTML
    // engine here — the raw markup is honest and lossless).
    if let Some(text) = parsed.body_text(0) {
        let text = text.trim();
        if !text.is_empty() {
            out.push_str(text);
            out.push('\n');
            return out;
        }
    }
    if let Some(html) = parsed.body_html(0) {
        let html = html.trim();
        if !html.is_empty() {
            let fence = fence_for(html);
            out.push_str(&format!("{fence}html\n"));
            out.push_str(html);
            out.push_str(&format!("\n{fence}\n"));
        }
    }
    out
}

/// Render one address list as `Name <mail@host>, …`.
fn render_address_list(list: &mail_parser::AddressList<'_>) -> String {
    let mut parts = Vec::new();
    for addr in list.iter() {
        match addr {
            mail_parser::Address::Mailbox(mailbox) => {
                let address = mailbox.address().unwrap_or("");
                match mailbox.name() {
                    Some(name) if !name.is_empty() => {
                        parts.push(format!("{name} <{address}>"));
                    }
                    _ => parts.push(address.to_string()),
                }
            }
            mail_parser::Address::Group(group) => {
                let name = group.name().unwrap_or("");
                // AddressRun is an Iterator of Mailbox.
                let inner: Vec<String> = group
                    .mailboxes()
                    .filter_map(|mailbox| mailbox.address().map(|a| a.to_string()))
                    .collect();
                parts.push(format!("{name}: {};", inner.join(", ")));
            }
        }
    }
    parts.join(", ")
}

/// Attachment inventory: `name (N bytes)` per attachment.
fn collect_attachments(parsed: &mail_parser::Message<'_>) -> Vec<String> {
    parsed
        .attachments()
        .map(|part| {
            let name = part
                .attachment_name()
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("attachment-{:?}", part.id()));
            let size = part.raw_body().len();
            format!("{name} ({size} bytes)")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic multipart message with a text body and one attachment.
    fn sample_eml() -> Vec<u8> {
        let body = "\
From: Alice <alice@example.com>\r
To: Bob <bob@example.com>\r
Subject: Quarterly numbers\r
Date: Tue, 09 Nov 2026 10:00:00 +0000\r
Message-ID: <abc123@example.com>\r
MIME-Version: 1.0\r
Content-Type: multipart/mixed; boundary=\"BOUND\"\r
\r
--BOUND\r
Content-Type: text/plain; charset=utf-8\r
\r
Hello Bob,\r
attached is the report.\r
--BOUND\r
Content-Type: text/plain; name=\"report.txt\"\r
Content-Disposition: attachment; filename=\"report.txt\"\r
\r
col1,col2\r
1,2\r
--BOUND--\r
";
        body.as_bytes().to_vec()
    }

    #[tokio::test]
    async fn headers_and_body_render() {
        let result = EmlConverter::new()
            .convert(Input::bytes(sample_eml(), "mail.eml"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("Quarterly numbers"),
            "{}",
            result.markdown
        );
        assert!(result.markdown.contains("alice@example.com"));
        assert!(result.markdown.contains("bob@example.com"));
        assert!(result.markdown.contains("Hello Bob"), "{}", result.markdown);
        assert_eq!(result.metadata["attachment_count"].as_u64(), Some(1));
        let attachments = result.metadata["attachments"].as_array().unwrap();
        assert!(
            attachments[0].as_str().unwrap().contains("report.txt"),
            "attachments: {attachments:?}"
        );
    }

    #[tokio::test]
    async fn html_only_body_yields_text() {
        let html = "\
From: a@b.c\r
To: d@e.f\r
Subject: HTML only\r
MIME-Version: 1.0\r
Content-Type: text/html; charset=utf-8\r
\r
<p>Hello <b>world</b></p>\r
";
        let result = EmlConverter::new()
            .convert(Input::bytes(html.as_bytes().to_vec(), "h.eml"))
            .await
            .unwrap();
        // mail-parser's body_text strips markup from the HTML part: the
        // readable text is what a mail reader shows.
        assert!(result.markdown.contains("Hello"), "{}", result.markdown);
        assert!(result.markdown.contains("world"), "{}", result.markdown);
    }

    #[tokio::test]
    async fn invalid_message_errors_without_panicking() {
        let err = EmlConverter::new()
            .convert(Input::bytes(b"not a message".to_vec(), "x.eml"))
            .await;
        assert!(err.is_err(), "garbage must not silently convert");
    }

    #[test]
    fn contract() {
        let c = EmlConverter::new();
        assert_eq!(c.name(), "eml");
        assert_eq!(c.supported_extensions(), &["eml"]);
        assert_eq!(c.availability(), Availability::Available);
    }
}
