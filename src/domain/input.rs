// SPDX-License-Identifier: Apache-2.0
//! Input types and format detection.

use std::path::PathBuf;

/// Input for a converter — file path or raw bytes with a filename hint.
#[derive(Debug, Clone)]
pub enum Input {
    /// Read the file at this path.
    Path(PathBuf),
    /// Raw bytes with a filename hint and optional mimetype/extension overrides.
    Bytes {
        data: Vec<u8>,
        filename: String,
        /// Explicit mimetype override (e.g. "application/pdf").
        /// If set, takes precedence over extension-based detection.
        mimetype: Option<String>,
        /// Explicit extension override (e.g. "pdf"), lowercase without dot.
        /// If set, takes precedence over filename-based detection.
        extension: Option<String>,
    },
}

impl Input {
    /// Create a Bytes input from data and filename.
    pub fn bytes(data: Vec<u8>, filename: impl Into<String>) -> Self {
        Input::Bytes {
            data,
            filename: filename.into(),
            mimetype: None,
            extension: None,
        }
    }

    /// Create a Bytes input with an explicit mimetype.
    pub fn bytes_with_mimetype(
        data: Vec<u8>,
        filename: impl Into<String>,
        mimetype: impl Into<String>,
    ) -> Self {
        Input::Bytes {
            data,
            filename: filename.into(),
            mimetype: Some(mimetype.into()),
            extension: None,
        }
    }

    /// Return the filename hint, if available.
    pub fn filename(&self) -> Option<&str> {
        match self {
            Input::Path(p) => p.file_name().and_then(|n| n.to_str()),
            Input::Bytes { filename, .. } => Some(filename),
        }
    }

    /// Return the mimetype, if explicitly set.
    pub fn mimetype(&self) -> Option<&str> {
        match self {
            Input::Path(_) => None,
            Input::Bytes { mimetype, .. } => mimetype.as_deref(),
        }
    }

    /// Return the file extension (lowercase, without dot).
    ///
    /// Priority: explicit extension field > extension from filename.
    pub fn extension(&self) -> Option<String> {
        match self {
            Input::Path(p) => {
                let name = p.file_name().and_then(|n| n.to_str())?;
                Self::extract_extension(name)
            }
            Input::Bytes {
                extension,
                filename,
                ..
            } => {
                if let Some(ext) = extension {
                    return Some(ext.clone());
                }
                Self::extract_extension(filename)
            }
        }
    }

    fn extract_extension(name: &str) -> Option<String> {
        let dot = name.rfind('.')?;
        if dot == 0 || dot == name.len() - 1 {
            return None;
        }
        Some(name[dot + 1..].to_lowercase())
    }

    /// Resolve the input to bytes, reading from disk if necessary.
    pub async fn to_bytes(&self) -> anyhow::Result<Vec<u8>> {
        match self {
            Input::Path(p) => Ok(tokio::fs::read(p).await?),
            Input::Bytes { data, .. } => Ok(data.clone()),
        }
    }
}

/// Supported input formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputFormat {
    /// PDF document.
    Pdf,
    /// PowerPoint Open XML.
    Pptx,
    /// Word Open XML.
    Docx,
    /// Excel Open XML.
    Xlsx,
    /// Legacy Excel (BIFF).
    Xls,
    /// OpenDocument family (odt/ods/odp; precise member from extension).
    Odt,
    /// HTML document.
    Html,
    /// EPUB e-book.
    Epub,
    /// CSV table.
    Csv,
    /// ZIP archive (recursive container).
    Zip,
    /// Plain text (txt/text/log/md and code/config family).
    Txt,
    /// Jupyter notebook.
    Ipynb,
    /// GIF image.
    Gif,
    /// PNG image.
    Png,
    /// JPEG image.
    Jpeg,
    /// WebP image.
    Webp,
    /// WAV audio.
    Wav,
    /// MP3 audio.
    Mp3,
    /// Ogg container (ogg/opus).
    Ogg,
    /// FLAC audio.
    Flac,
    /// MP4 video (ISOBMFF family: mp4/m4v/mov).
    Mp4,
    /// Unknown format.
    Unknown,
}

impl InputFormat {
    /// Return the primary file extension (lowercase, without dot).
    pub fn extension(self) -> &'static str {
        match self {
            InputFormat::Pdf => "pdf",
            InputFormat::Pptx => "pptx",
            InputFormat::Docx => "docx",
            InputFormat::Xlsx => "xlsx",
            InputFormat::Xls => "xls",
            InputFormat::Odt => "odt",
            InputFormat::Html => "html",
            InputFormat::Epub => "epub",
            InputFormat::Csv => "csv",
            InputFormat::Zip => "zip",
            InputFormat::Txt => "txt",
            InputFormat::Ipynb => "ipynb",
            InputFormat::Gif => "gif",
            InputFormat::Png => "png",
            InputFormat::Jpeg => "jpeg",
            InputFormat::Webp => "webp",
            InputFormat::Wav => "wav",
            InputFormat::Mp3 => "mp3",
            InputFormat::Ogg => "ogg",
            InputFormat::Flac => "flac",
            InputFormat::Mp4 => "mp4",
            InputFormat::Unknown => "unknown",
        }
    }

    /// Return all known extensions for this format (e.g. jpeg also matches jpg).
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            InputFormat::Pdf => &["pdf"],
            InputFormat::Pptx => &["pptx"],
            InputFormat::Docx => &["docx"],
            InputFormat::Xlsx => &["xlsx"],
            InputFormat::Xls => &["xls"],
            InputFormat::Odt => &["odt", "ods", "odp"],
            InputFormat::Html => &["html", "htm"],
            InputFormat::Epub => &["epub"],
            InputFormat::Csv => &["csv", "tsv"],
            InputFormat::Zip => &["zip"],
            InputFormat::Txt => &["txt", "text", "log", "md"],
            InputFormat::Ipynb => &["ipynb"],
            InputFormat::Gif => &["gif"],
            InputFormat::Png => &["png"],
            InputFormat::Jpeg => &["jpeg", "jpg"],
            InputFormat::Webp => &["webp"],
            InputFormat::Wav => &["wav"],
            InputFormat::Mp3 => &["mp3"],
            InputFormat::Ogg => &["ogg", "opus"],
            InputFormat::Flac => &["flac"],
            InputFormat::Mp4 => &["mp4", "m4v", "mov"],
            InputFormat::Unknown => &["unknown"],
        }
    }

    /// Return the content type category.
    pub fn content_type(self) -> crate::domain::result::ContentType {
        use crate::domain::result::ContentType;
        match self {
            InputFormat::Pdf
            | InputFormat::Pptx
            | InputFormat::Docx
            | InputFormat::Xlsx
            | InputFormat::Xls
            | InputFormat::Odt
            | InputFormat::Html
            | InputFormat::Epub
            | InputFormat::Csv
            | InputFormat::Txt
            | InputFormat::Ipynb
            | InputFormat::Zip => ContentType::Document,
            InputFormat::Gif | InputFormat::Png | InputFormat::Jpeg | InputFormat::Webp => {
                ContentType::Image
            }
            InputFormat::Wav | InputFormat::Mp3 | InputFormat::Ogg | InputFormat::Flac => {
                ContentType::Audio
            }
            InputFormat::Mp4 => ContentType::Video,
            InputFormat::Unknown => ContentType::Document,
        }
    }
}

/// Detect the format of input data using magic bytes, ZIP structure,
/// mimetype hints, and file extension fallback.
pub fn detect_format(input: &Input) -> Option<InputFormat> {
    if let Some(format) = detect_magic_bytes(input) {
        return Some(format);
    }
    let ext = input.extension()?;
    ext_to_format(&ext)
}

/// Try to detect format from magic bytes.
fn detect_magic_bytes(input: &Input) -> Option<InputFormat> {
    let data = match input {
        Input::Bytes { data, .. } => data,
        // Path variant would need async IO; delegate to extension detection.
        Input::Path(_) => return None,
    };

    if data.len() < 4 {
        return None;
    }

    // PDF: %PDF
    if &data[0..4] == b"%PDF" {
        return Some(InputFormat::Pdf);
    }

    // ZIP-based formats (DOCX, PPTX, XLSX, EPUB, ODF): PK\x03\x04
    if &data[0..4] == b"PK\x03\x04" {
        if data.len() > 30 {
            // ODF: the mimetype entry is the first zip entry (stored), whose
            // content starts with "application/vnd.oasis" — check it before
            // the META-INF heuristic because ODF lists mimetype first.
            if data
                .windows(30)
                .any(|w| w.starts_with(b"mimetypeapplication/vnd.oasis"))
            {
                return Some(InputFormat::Odt);
            }
            if data.windows(5).any(|w| w == b"word/") {
                return Some(InputFormat::Docx);
            }
            if data.windows(4).any(|w| w == b"ppt/") {
                return Some(InputFormat::Pptx);
            }
            if data.windows(3).any(|w| w == b"xl/") {
                return Some(InputFormat::Xlsx);
            }
            if data.windows(9).any(|w| w == b"META-INF/") {
                return Some(InputFormat::Epub);
            }
            // ODF zips may list content.xml before/without META-INF nearby.
            if data.windows(11).any(|w| w == b"content.xml") {
                return Some(InputFormat::Odt);
            }
        }
        return None;
    }

    // GIF: GIF8
    if &data[0..4] == b"GIF8" {
        return Some(InputFormat::Gif);
    }

    // PNG: \x89PNG
    if data[0..4] == [0x89, b'P', b'N', b'G'] {
        return Some(InputFormat::Png);
    }

    // JPEG: \xFF\xD8\xFF
    if data.len() >= 3 && data[0..3] == [0xFF, 0xD8, 0xFF] {
        return Some(InputFormat::Jpeg);
    }

    // RIFF containers: WEBP / WAV / AVI
    if data.len() >= 12 && &data[0..4] == b"RIFF" {
        return match &data[8..12] {
            b"WEBP" => Some(InputFormat::Webp),
            b"WAVE" => Some(InputFormat::Wav),
            b"AVI " => Some(InputFormat::Mp4),
            _ => None,
        };
    }

    // Ogg: OggS
    if &data[0..4] == b"OggS" {
        return Some(InputFormat::Ogg);
    }

    // FLAC: fLaC
    if &data[0..4] == b"fLaC" {
        return Some(InputFormat::Flac);
    }

    // MP3: ID3 header or MPEG frame sync
    if &data[0..3] == b"ID3" {
        return Some(InputFormat::Mp3);
    }

    // ISOBMFF (mp4/mov): size(4) + "ftyp" + major brand
    if data.len() >= 8 && &data[4..8] == b"ftyp" {
        return Some(InputFormat::Mp4);
    }

    // MP3 raw frame sync (0xFF Ex/Fx)
    if data[0] == 0xFF && data[1] & 0xE0 == 0xE0 {
        return Some(InputFormat::Mp3);
    }

    None
}

/// Map a file extension to an InputFormat.
pub fn ext_to_format(ext: &str) -> Option<InputFormat> {
    match ext {
        "pdf" => Some(InputFormat::Pdf),
        "pptx" => Some(InputFormat::Pptx),
        "docx" => Some(InputFormat::Docx),
        "xlsx" => Some(InputFormat::Xlsx),
        "xls" => Some(InputFormat::Xls),
        "odt" | "ods" | "odp" => Some(InputFormat::Odt),
        "html" | "htm" => Some(InputFormat::Html),
        "epub" => Some(InputFormat::Epub),
        "csv" | "tsv" => Some(InputFormat::Csv),
        "zip" => Some(InputFormat::Zip),
        "txt" | "text" | "log" | "md" => Some(InputFormat::Txt),
        "ipynb" => Some(InputFormat::Ipynb),
        "gif" => Some(InputFormat::Gif),
        "png" => Some(InputFormat::Png),
        "jpeg" | "jpg" => Some(InputFormat::Jpeg),
        "webp" => Some(InputFormat::Webp),
        "wav" => Some(InputFormat::Wav),
        "mp3" => Some(InputFormat::Mp3),
        "ogg" | "opus" => Some(InputFormat::Ogg),
        "flac" => Some(InputFormat::Flac),
        "mp4" | "m4v" | "mov" => Some(InputFormat::Mp4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn bytes_input(data: Vec<u8>, filename: &str) -> Input {
        Input::Bytes {
            data,
            filename: filename.to_string(),
            mimetype: None,
            extension: None,
        }
    }

    #[test]
    fn input_path_filename() {
        let input = Input::Path(PathBuf::from("/tmp/test.pdf"));
        assert_eq!(input.filename(), Some("test.pdf"));
    }

    #[test]
    fn input_path_filename_root_path() {
        let input = Input::Path(PathBuf::from("/"));
        assert_eq!(input.filename(), None);
    }

    #[test]
    fn input_bytes_filename() {
        let input = bytes_input(vec![], "report.docx");
        assert_eq!(input.filename(), Some("report.docx"));
    }

    #[test]
    fn input_extension_pdf() {
        let input = bytes_input(vec![], "doc.pdf");
        assert_eq!(input.extension(), Some("pdf".to_string()));
    }

    #[test]
    fn input_extension_uppercase() {
        let input = bytes_input(vec![], "doc.PDF");
        assert_eq!(input.extension(), Some("pdf".to_string()));
    }

    #[test]
    fn input_extension_double_dot() {
        let input = bytes_input(vec![], "archive.tar.gz");
        assert_eq!(input.extension(), Some("gz".to_string()));
    }

    #[test]
    fn input_extension_no_extension() {
        let input = bytes_input(vec![], "README");
        assert_eq!(input.extension(), None);
    }

    #[test]
    fn input_extension_dotfile() {
        let input = bytes_input(vec![], ".gitignore");
        assert_eq!(input.extension(), None);
    }

    #[test]
    fn input_extension_trailing_dot() {
        let input = bytes_input(vec![], "file.");
        assert_eq!(input.extension(), None);
    }

    #[test]
    fn input_extension_no_filename() {
        let input = Input::Path(PathBuf::from("/tmp/"));
        assert_eq!(input.extension(), None);
    }

    #[test]
    fn input_extension_explicit_override_takes_priority() {
        let input = Input::Bytes {
            data: vec![],
            filename: "data.docx".to_string(),
            mimetype: None,
            extension: Some("txt".to_string()),
        };
        assert_eq!(input.extension(), Some("txt".to_string()));
    }

    #[test]
    fn input_bytes_constructor() {
        let input = Input::bytes(vec![1, 2, 3], "data.bin");
        assert_eq!(input.filename(), Some("data.bin"));
        assert_eq!(input.mimetype(), None);
        assert_eq!(input.extension(), Some("bin".to_string()));
    }

    #[test]
    fn input_bytes_with_mimetype_constructor() {
        let input = Input::bytes_with_mimetype(vec![1, 2, 3], "data.pdf", "application/pdf");
        assert_eq!(input.mimetype(), Some("application/pdf"));
    }

    #[tokio::test]
    async fn input_bytes_to_bytes() {
        let data = vec![1, 2, 3, 4];
        let input = Input::bytes(data.clone(), "test.bin");
        assert_eq!(input.to_bytes().await.unwrap(), data);
    }

    #[tokio::test]
    async fn input_path_to_bytes_missing_file() {
        let input = Input::Path(PathBuf::from("/nonexistent/file.txt"));
        assert!(input.to_bytes().await.is_err());
    }

    #[test]
    fn detect_magic_bytes_pdf() {
        let input = bytes_input(b"%PDF-1.4 fake content".to_vec(), "doc.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Pdf));
    }

    #[test]
    fn detect_magic_bytes_png() {
        let mut data = vec![0x89, b'P', b'N', b'G'];
        data.extend_from_slice(b"\r\n\x1a\nfake");
        let input = bytes_input(data, "img.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Png));
    }

    #[test]
    fn detect_magic_bytes_jpeg() {
        let input = bytes_input(vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10], "photo.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Jpeg));
    }

    #[test]
    fn detect_magic_bytes_wav() {
        let mut data = b"RIFF".to_vec();
        data.extend_from_slice(&[0x24, 0x00, 0x00, 0x00]);
        data.extend_from_slice(b"WAVE");
        data.extend_from_slice(b"fmt ");
        let input = bytes_input(data, "audio.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Wav));
    }

    #[test]
    fn detect_magic_bytes_gif() {
        let input = bytes_input(b"GIF89a\x01\x00".to_vec(), "anim.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Gif));
    }

    #[test]
    fn detect_magic_bytes_webp() {
        let mut data = b"RIFF".to_vec();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        data.extend_from_slice(b"WEBPVP8 ");
        let input = bytes_input(data, "img.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Webp));
    }

    #[test]
    fn detect_magic_bytes_ogg() {
        let input = bytes_input(b"OggS\x00\x02".to_vec(), "audio.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Ogg));
    }

    #[test]
    fn detect_magic_bytes_flac() {
        let input = bytes_input(b"fLaC\x00\x00".to_vec(), "audio.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Flac));
    }

    #[test]
    fn detect_magic_bytes_mp3_id3() {
        let input = bytes_input(b"ID3\x04\x00".to_vec(), "audio.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Mp3));
    }

    #[test]
    fn detect_magic_bytes_mp4_ftyp() {
        let mut data = vec![0, 0, 0, 0x20];
        data.extend_from_slice(b"ftypisom");
        data.extend_from_slice(&[0x00, 0x02, 0x00, 0x06]);
        let input = bytes_input(data, "video.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Mp4));
    }

    #[test]
    fn detect_magic_bytes_avi() {
        let mut data = b"RIFF".to_vec();
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        data.extend_from_slice(b"AVI LIST");
        let input = bytes_input(data, "video.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Mp4));
    }

    #[test]
    fn detect_magic_bytes_zip_generic_returns_none() {
        let input = bytes_input(vec![0x50, 0x4B, 0x03, 0x04, 0x00, 0x00], "archive.bin");
        assert_eq!(detect_format(&input), None);
    }

    #[test]
    fn detect_magic_bytes_odf_zip() {
        // ODF zip: mimetype entry (stored, first) with application/vnd.oasis content
        let mut data = b"PK\x03\x04".to_vec();
        data.extend_from_slice(&[0x0A, 0x00, 0x00, 0x00, 0x00, 0x00]);
        data.extend_from_slice(b"mimetypeapplication/vnd.oasis.opendocument.text");
        data.extend_from_slice(&[0x00; 20]);
        let input = bytes_input(data, "doc.bin");
        assert_eq!(detect_format(&input), Some(InputFormat::Odt));
    }

    #[test]
    fn detect_magic_bytes_unknown_returns_none() {
        let input = bytes_input(vec![0xDE, 0xAD, 0xBE, 0xEF], "unknown.bin");
        assert_eq!(detect_format(&input), None);
    }

    #[test]
    fn detect_magic_bytes_too_short_returns_none() {
        let input = bytes_input(vec![0x00, 0x01], "short.bin");
        assert_eq!(detect_format(&input), None);
    }

    #[test]
    fn detect_format_fallback_to_extension() {
        let input = bytes_input(vec![0x00; 8], "doc.xlsx");
        assert_eq!(detect_format(&input), Some(InputFormat::Xlsx));
    }

    #[test]
    fn detect_format_magic_bytes_override_extension() {
        let input = bytes_input(b"%PDF-1.4 content".to_vec(), "fake.txt");
        assert_eq!(detect_format(&input), Some(InputFormat::Pdf));
    }

    #[test]
    fn ext_to_format_family_mappings() {
        assert_eq!(ext_to_format("odt"), Some(InputFormat::Odt));
        assert_eq!(ext_to_format("ods"), Some(InputFormat::Odt));
        assert_eq!(ext_to_format("odp"), Some(InputFormat::Odt));
        assert_eq!(ext_to_format("tsv"), Some(InputFormat::Csv));
        assert_eq!(ext_to_format("mp3"), Some(InputFormat::Mp3));
        assert_eq!(ext_to_format("flac"), Some(InputFormat::Flac));
        assert_eq!(ext_to_format("opus"), Some(InputFormat::Ogg));
        assert_eq!(ext_to_format("mov"), Some(InputFormat::Mp4));
        assert_eq!(ext_to_format("exe"), None);
    }

    #[test]
    fn content_type_families() {
        use crate::domain::result::ContentType;
        assert_eq!(InputFormat::Odt.content_type(), ContentType::Document);
        assert_eq!(InputFormat::Gif.content_type(), ContentType::Image);
        assert_eq!(InputFormat::Webp.content_type(), ContentType::Image);
        assert_eq!(InputFormat::Ogg.content_type(), ContentType::Audio);
        assert_eq!(InputFormat::Flac.content_type(), ContentType::Audio);
        assert_eq!(InputFormat::Mp4.content_type(), ContentType::Video);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn detect_never_panics(data in proptest::collection::vec(any::<u8>(), 0..64)) {
            let input = Input::Bytes {
                data,
                filename: "fuzz.bin".to_string(),
                mimetype: None,
                extension: None,
            };
            let _ = detect_format(&input);
        }
    }
}
