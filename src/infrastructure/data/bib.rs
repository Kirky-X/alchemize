#![cfg(feature = "bibtex")]

//! BibTeX / BibLaTeX bibliography converter (T026).

use async_trait::async_trait;
use biblatex::{Bibliography, ChunksExt, Date, DateValue, Entry, PermissiveType, Person};

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Converts BibTeX / BibLaTeX `.bib` files into a markdown entry list.
///
/// Each entry renders as
/// `- **{key}** — {title} ({author}, {year}). {journal|booktitle}.`
/// with missing fields skipped rather than rendered as placeholders.
pub struct BibConverter;

impl BibConverter {
    /// Create a new bib converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for BibConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for BibConverter {
    fn name(&self) -> &'static str {
        "bib"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read bib input: {e}")))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| AlchemizeError::other("bib file is not valid UTF-8 text"))?;

        let bibliography = Bibliography::parse(&text)
            .map_err(|e| AlchemizeError::other(format!("failed to parse BibTeX: {e}")))?;

        let lines: Vec<String> = bibliography.iter().map(render_entry).collect();
        let entry_count = lines.len();
        let markdown = if lines.is_empty() {
            "*Bibliography contains no entries.*\n".to_string()
        } else {
            format!("{}\n", lines.join("\n"))
        };

        let metadata = serde_json::json!({
            "source_format": "bib",
            "entry_count": entry_count,
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["bib"]
    }
}

/// Render one bibliography entry as a markdown list item.
fn render_entry(entry: &Entry) -> String {
    let mut line = format!("- **{}**", entry.key);

    if let Ok(title) = entry.title() {
        let title = title.format_verbatim();
        if !title.trim().is_empty() {
            line.push_str(&format!(" — {title}"));
        }
    }

    let authors = entry
        .author()
        .ok()
        .map(|people| format_people(&people))
        .unwrap_or_default();
    let year = entry.date().ok().and_then(year_of);
    match (authors.is_empty(), year) {
        (false, Some(year)) => line.push_str(&format!(" ({authors}, {year}).")),
        (false, None) => line.push_str(&format!(" ({authors}).")),
        (true, Some(year)) => line.push_str(&format!(" ({year}).")),
        (true, None) => {}
    }

    let venue = entry
        .journal()
        .ok()
        .map(|v| v.format_verbatim())
        .or_else(|| entry.book_title().ok().map(|v| v.format_verbatim()));
    if let Some(venue) = venue
        && !venue.trim().is_empty()
    {
        line.push_str(&format!(" {venue}."));
    }

    line
}

/// Join people as `A`, `A & B`, `A, B & C`.
fn format_people(people: &[Person]) -> String {
    let names: Vec<String> = people
        .iter()
        .map(|p| {
            if p.given_name.is_empty() {
                p.name.clone()
            } else {
                format!("{} {}", p.given_name, p.name)
            }
            .trim()
            .to_string()
        })
        .filter(|n| !n.is_empty())
        .collect();
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        2 => format!("{} & {}", names[0], names[1]),
        n => format!("{} & {}", names[..n - 1].join(", "), names[n - 1]),
    }
}

/// Extract the starting year from a parsed date field.
///
/// `entry.date()` returns `PermissiveType<Date>`: a parsed `Date` (including
/// the `year`/`month`/`day` legacy-field fallback) or the raw chunks when the
/// `date` field is present but unparseable — those carry the year as text.
fn year_of(value: PermissiveType<Date>) -> Option<i32> {
    match value {
        PermissiveType::Typed(date) => match date.value {
            DateValue::At(dt) | DateValue::After(dt) | DateValue::Before(dt) => Some(dt.year),
            DateValue::Between(start, _) => Some(start.year),
        },
        PermissiveType::Chunks(chunks) => first_year_in(&chunks.format_verbatim()),
    }
}

/// First standalone 4-digit year in a text blob.
fn first_year_in(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    let mut start: Option<usize> = None;
    for (i, b) in bytes.iter().enumerate() {
        if b.is_ascii_digit() {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start {
            if i - s == 4 {
                return text[s..i].parse().ok();
            }
            start = None;
        }
    }
    if let Some(s) = start
        && bytes.len() - s == 4
    {
        return text[s..].parse().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_BIB: &str = r#"
@article{knuth1974,
    author  = {Knuth, Donald E.},
    title   = {Computer Programming as an Art},
    journal = {ACM Transactions on Programming Languages and Systems},
    year    = {1974},
    volume  = {16},
    pages   = {667--685}
}

@inproceedings{leciu2014,
    author    = {LeCun, Yann and Bengio, Yoshua and Hinton, Geoffrey},
    title     = {Deep Learning},
    booktitle = {Nature Machine Intelligence Workshop},
    year      = {2015}
}

@article{turing1950,
    author  = {Turing, Alan M.},
    title   = {Computing Machinery and Intelligence},
    journal = {Mind},
    year    = {1950}
}

@inproceedings{minimal,
    author = {One, Author},
    booktitle = {Bare Proceedings}
}
"#;

    #[tokio::test]
    async fn convert_bib_entries() {
        let converter = BibConverter::new();
        let result = converter
            .convert(Input::bytes(SAMPLE_BIB.as_bytes().to_vec(), "refs.bib"))
            .await
            .unwrap();

        let md = &result.markdown;
        assert!(md.contains(
            "- **knuth1974** — Computer Programming as an Art (Donald E. Knuth, 1974). ACM Transactions on Programming Languages and Systems."
        ));
        assert!(md.contains(
            "- **leciu2014** — Deep Learning (Yann LeCun, Yoshua Bengio & Geoffrey Hinton, 2015). Nature Machine Intelligence Workshop."
        ));
        assert!(md.contains(
            "- **turing1950** — Computing Machinery and Intelligence (Alan M. Turing, 1950). Mind."
        ));
        // Missing title/year: both segments skipped, booktitle kept.
        assert!(md.contains("- **minimal** (Author One). Bare Proceedings."));
        assert!(!md.contains(" —  "), "no empty title placeholders");

        assert_eq!(result.metadata["source_format"], "bib");
        assert_eq!(result.metadata["entry_count"], 4);
    }

    #[tokio::test]
    async fn convert_empty_bib() {
        let converter = BibConverter::new();
        let result = converter
            .convert(Input::bytes(b"\n".to_vec(), "empty.bib"))
            .await
            .unwrap();
        assert!(result.markdown.contains("no entries"));
        assert_eq!(result.metadata["entry_count"], 0);
    }

    #[tokio::test]
    async fn convert_invalid_syntax_error() {
        let converter = BibConverter::new();
        let err = converter
            .convert(Input::bytes(b"@article{unterminated,".to_vec(), "bad.bib"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("BibTeX"),
            "expected parse error, got: {err}"
        );
    }

    #[tokio::test]
    async fn convert_non_utf8_error() {
        let converter = BibConverter::new();
        let err = converter
            .convert(Input::bytes(vec![0xFF, 0xFE, 0x00], "bin.bib"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("UTF-8"));
    }

    #[test]
    fn supported_extensions_and_name() {
        let converter = BibConverter::new();
        assert_eq!(converter.supported_extensions(), &["bib"]);
        assert_eq!(converter.name(), "bib");
    }

    #[test]
    fn format_people_variants() {
        let mk = |name: &str, given: &str| Person {
            name: name.to_string(),
            given_name: given.to_string(),
            prefix: String::new(),
            suffix: String::new(),
            id: None,
            prefix_initials: None,
            given_initials: None,
            use_prefix: None,
        };
        assert_eq!(
            format_people(&[mk("Einstein", "Albert")]),
            "Albert Einstein"
        );
        assert_eq!(
            format_people(&[mk("Curie", "Marie"), mk("Curie", "Pierre")]),
            "Marie Curie & Pierre Curie"
        );
        assert_eq!(
            format_people(&[mk("A", "Aa"), mk("B", "Bb"), mk("C", "Cc")]),
            "Aa A, Bb B & Cc C"
        );
    }

    #[test]
    fn first_year_in_scans_four_digit_groups() {
        assert_eq!(first_year_in("1990/1995"), Some(1990));
        assert_eq!(first_year_in("circa 1847 AD"), Some(1847));
        assert_eq!(first_year_in("no year here"), None);
        assert_eq!(first_year_in("12"), None);
    }
}
