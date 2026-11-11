#![cfg(feature = "sqlite")]

//! SQLite database converter — schema and sample rows as markdown (T025).
//!
//! Databases are opened strictly read-only (`SQLITE_OPEN_READ_ONLY`, which
//! also implies "must already exist, never created"): the converter can never
//! write, create, or modify the database it converts.

use std::path::Path;

use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags};

use crate::application::ports::Converter;
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::{ContentType, ConversionQuality, ConvertResult};

/// Rows sampled per table.
const SAMPLE_ROWS: usize = 5;

/// Converts SQLite database files into markdown.
pub struct SqliteConverter;

impl SqliteConverter {
    /// Create a new sqlite converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SqliteConverter {
    fn default() -> Self {
        Self::new()
    }
}

/// Private temp copy of the database, deleted when dropped (failure paths
/// included). Staging keeps the attacker-controlled file away from its
/// sibling `-wal`/`-shm` (RUSTSEC-2026-0459 vector) and out of the shared
/// temp dir's default world-readable permissions: `tempfile` creates an
/// exclusively-owned 0600 file with an unpredictable name.
struct TempDbGuard {
    file: tempfile::NamedTempFile,
}

impl TempDbGuard {
    /// Materialize raw database bytes into a unique temp file.
    fn from_bytes(bytes: &[u8]) -> Result<Self, AlchemizeError> {
        use std::io::Write as _;
        let mut file = tempfile::Builder::new()
            .prefix("alchemize-sqlite-")
            .suffix(".db")
            .tempfile()
            .map_err(|e| AlchemizeError::other(format!("failed to stage SQLite temp file: {e}")))?;
        file.write_all(bytes)
            .and_then(|()| file.flush())
            .map_err(|e| AlchemizeError::other(format!("failed to stage SQLite temp file: {e}")))?;
        Ok(Self { file })
    }

    fn path(&self) -> &std::path::Path {
        self.file.path()
    }
}

#[async_trait]
impl Converter for SqliteConverter {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read sqlite input: {e}")))?;

        // All inputs are staged into a private temp copy before opening:
        // reading the attacker-supplied database in place would also let a
        // sibling `-wal`/`-shm` file inject uninitialized page-cache bytes
        // (RUSTSEC-2026-0459 vector; the pinned libsqlite3-sys cannot fix it).
        // The bytes are already in memory (`to_bytes` above), so staging
        // adds no extra read of the source.
        let _temp = TempDbGuard::from_bytes(&bytes)?;
        let path: &Path = _temp.path();

        let conn =
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| {
                AlchemizeError::other(format!("failed to open SQLite database read-only: {e}"))
            })?;

        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .map_err(|e| AlchemizeError::other(format!("failed to list SQLite tables: {e}")))?;
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| AlchemizeError::other(format!("failed to query SQLite tables: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AlchemizeError::other(format!("failed to read SQLite tables: {e}")))?;
        drop(stmt);

        let mut markdown = String::new();
        for table in &tables {
            render_table_section(&conn, table, &mut markdown)?;
        }
        if markdown.is_empty() {
            markdown.push_str("*SQLite database contains no user tables.*\n");
        }

        let metadata = serde_json::json!({
            "source_format": "sqlite",
            "table_count": tables.len(),
        });

        Ok(
            ConvertResult::with_metadata(markdown, ContentType::Document, metadata)
                .with_quality(ConversionQuality::LocalFull),
        )
    }

    fn supported_extensions(&self) -> &[&str] {
        &["sqlite", "db", "sqlite3"]
    }
}

/// Double-quote escape for SQLite identifiers.
fn escape_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Render one table: heading, column list (PRAGMA table_info), sample rows.
fn render_table_section(
    conn: &Connection,
    table: &str,
    out: &mut String,
) -> Result<(), AlchemizeError> {
    use std::fmt::Write as _;

    let _ = writeln!(out, "### {table}\n");

    // Column list.
    let pragma = format!("PRAGMA table_info({})", escape_ident(table));
    let mut col_stmt = conn
        .prepare(&pragma)
        .map_err(|e| AlchemizeError::other(format!("failed to inspect table '{table}': {e}")))?;
    let columns: Vec<(String, String, i64, i64)> = col_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?, // notnull
                row.get::<_, i64>(5)?, // pk
            ))
        })
        .map_err(|e| AlchemizeError::other(format!("failed to inspect table '{table}': {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AlchemizeError::other(format!("failed to inspect table '{table}': {e}")))?;
    drop(col_stmt);

    out.push_str("Columns:\n\n");
    for (name, ty, notnull, pk) in &columns {
        let ty = if ty.is_empty() { "?" } else { ty };
        let _ = write!(out, "- {name} {ty}");
        if *notnull != 0 {
            out.push_str(" (not null)");
        }
        if *pk != 0 {
            out.push_str(" (pk)");
        }
        out.push('\n');
    }
    out.push('\n');

    // Sample rows.
    let query = format!("SELECT * FROM {} LIMIT {SAMPLE_ROWS}", escape_ident(table));
    let mut sample = conn
        .prepare(&query)
        .map_err(|e| AlchemizeError::other(format!("failed to sample table '{table}': {e}")))?;
    let headers: Vec<String> = sample
        .column_names()
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    let rows = sample
        .query_map([], |row| {
            let mut cells = Vec::with_capacity(headers.len());
            for i in 0..headers.len() {
                cells.push(render_value(row.get_ref(i)?));
            }
            Ok(cells)
        })
        .map_err(|e| AlchemizeError::other(format!("failed to sample table '{table}': {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AlchemizeError::other(format!("failed to sample table '{table}': {e}")))?;
    drop(sample);

    if rows.is_empty() {
        out.push_str("Sample rows: *(none)*\n\n");
        return Ok(());
    }

    out.push_str(&format!("Sample rows (first {SAMPLE_ROWS}):\n\n"));
    out.push_str("| ");
    out.push_str(
        &headers
            .iter()
            .map(|h| escape_cell(h))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    out.push_str(" |\n|");
    for _ in &headers {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in &rows {
        out.push_str("| ");
        out.push_str(&row.join(" | "));
        out.push_str(" |\n");
    }
    out.push('\n');
    Ok(())
}

/// Render one SQLite cell; blobs become a short hex preview.
fn render_value(value: rusqlite::types::ValueRef) -> String {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(b) => hex_preview(b),
    }
}

/// First 16 bytes of a blob as hex, with a length note when truncated.
fn hex_preview(bytes: &[u8]) -> String {
    let shown = bytes.len().min(16);
    let hex: String = bytes[..shown]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("");
    if bytes.len() > shown {
        format!("{hex}… ({} bytes)", bytes.len())
    } else {
        hex
    }
}

/// Escape a value for a markdown table cell.
fn escape_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Build a real database on disk (words + meta), return its bytes and
    /// clean up behind itself.
    struct DbFixture {
        guard: TempDbGuard,
        write_path: PathBuf,
    }

    impl Drop for DbFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.write_path);
        }
    }

    fn build_db() -> DbFixture {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir();
        let write_path = dir.join(format!(
            "alchemize-test-build-{}-{}.db",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let conn = Connection::open(&write_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE words (id INTEGER PRIMARY KEY, term TEXT, def TEXT);
            INSERT INTO words VALUES (1, 'serendipity', 'a happy accident');
            INSERT INTO words VALUES (2, 'algorithm', 'a step-by-step procedure');
            INSERT INTO words VALUES (3, 'entropy', 'a measure of disorder');
            INSERT INTO words VALUES (4, 'quark', 'an elementary particle');
            INSERT INTO words VALUES (5, 'tensor', 'a geometric object');
            INSERT INTO words VALUES (6, 'vector', 'a quantity with direction');
            INSERT INTO words VALUES (7, 'plasma', 'ionized gas state');
            CREATE TABLE meta (key TEXT, value TEXT);
            INSERT INTO meta VALUES ('schema_version', '3');
            INSERT INTO meta VALUES ('lang', 'en');
            "#,
        )
        .unwrap();
        drop(conn);
        let bytes = std::fs::read(&write_path).unwrap();
        let guard = TempDbGuard::from_bytes(&bytes).unwrap();
        DbFixture { guard, write_path }
    }

    #[tokio::test]
    async fn convert_sqlite_tables_and_samples() {
        let fixture = build_db();
        let bytes = std::fs::read(fixture.guard.path()).unwrap();
        let converter = SqliteConverter::new();
        let result = converter
            .convert(Input::bytes(bytes, "real.sqlite"))
            .await
            .unwrap();

        assert!(result.markdown.contains("### words"));
        assert!(result.markdown.contains("### meta"));
        assert!(result.markdown.contains("- id INTEGER (pk)"));
        assert!(result.markdown.contains("- term TEXT"));
        assert!(result.markdown.contains("- key TEXT"));
        assert!(result.markdown.contains("serendipity"));
        assert!(result.markdown.contains("schema_version"));
        // words has 7 rows; only the first 5 are sampled.
        assert!(result.markdown.contains("first 5"));
        assert!(
            !result.markdown.contains("plasma"),
            "row 7 must not be sampled"
        );

        assert_eq!(result.metadata["source_format"], "sqlite");
        assert_eq!(result.metadata["table_count"], 2);
    }

    #[tokio::test]
    async fn sqlite_open_is_read_only() {
        // Direct proof of the read-only contract: the same flags we use in
        // convert() reject INSERT.
        let fixture = build_db();
        let conn =
            Connection::open_with_flags(fixture.guard.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let err = conn
            .execute("INSERT INTO meta VALUES ('x', 'y')", [])
            .unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("readonly"),
            "expected readonly rejection, got: {err}"
        );
    }

    #[tokio::test]
    async fn convert_path_input_directly() {
        let fixture = build_db();
        let converter = SqliteConverter::new();
        let result = converter
            .convert(Input::Path(fixture.guard.path().to_path_buf()))
            .await
            .unwrap();
        assert!(result.markdown.contains("### words"));
        assert_eq!(result.metadata["table_count"], 2);
    }

    #[tokio::test]
    async fn convert_invalid_bytes_error() {
        let converter = SqliteConverter::new();
        let err = converter
            .convert(Input::bytes(b"definitely not sqlite".to_vec(), "bad.db"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("read-only") || err.to_string().contains("SQLite"),
            "expected open failure, got: {err}"
        );
    }

    #[test]
    fn supported_extensions_and_name() {
        let converter = SqliteConverter::new();
        assert_eq!(
            converter.supported_extensions(),
            &["sqlite", "db", "sqlite3"]
        );
        assert_eq!(converter.name(), "sqlite");
    }

    #[test]
    fn blob_and_null_rendering() {
        assert_eq!(render_value(rusqlite::types::ValueRef::Null), "");
        assert_eq!(render_value(rusqlite::types::ValueRef::Integer(-5)), "-5");
        assert_eq!(
            render_value(rusqlite::types::ValueRef::Blob(&[0xAB, 0xCD])),
            "abcd"
        );
        let long = vec![1u8; 20];
        let rendered = render_value(rusqlite::types::ValueRef::Blob(&long));
        assert!(rendered.contains("(20 bytes)"));
    }

    #[test]
    fn escape_cell_renders_pipes_and_newlines() {
        assert_eq!(escape_cell("a|b"), "a\\|b");
        assert_eq!(escape_cell("x\ny"), "x y");
    }
}
