// SPDX-License-Identifier: Apache-2.0
//! Outlook `.msg` converter — a minimal MAPI property layer over the OLE
//! Compound File container, rebuilt into RFC 822 MIME bytes and rendered
//! through the shared eml pipeline (Filee's MsgReader architecture: same
//! road, one renderer).
//!
//! Property coverage is deliberately small: subject, sender, recipients,
//! creation time, text body, HTML body, message codepage, and the
//! attachment inventory (long filename, size, data). Everything else is
//! skipped; decode failures degrade lossily and say so via metadata.

#![cfg(feature = "msg")]

use std::borrow::Cow;
use std::io::{Cursor, Read as _};

use base64::Engine as _;

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::ConvertResult;

const PROPS_STREAM: &str = "__properties_version1.0";
const ATTACH_PREFIX: &str = "__attach_version1.0_";
/// Standard base64 (headers RFC 2047 + attachment transfer encoding).
const BASE64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Property tags used by the minimal layer: `(id, description)`.
const SUBJECT: u32 = 0x0037;
/// PidTagSenderName (modern); 0x0042 OriginalSenderName is the legacy fallback.
const SENDER_NAME: u32 = 0x0C1A;
const SENDER_NAME_LEGACY: u32 = 0x0042;
/// PidTagSenderEmailAddress (modern); 0x0044 OriginalSenderEmailAddress fallback.
const SENDER_EMAIL: u32 = 0x0044;
const SENDER_SMTP: u32 = 0x0C1F;
/// PidTagSenderSmtpAddress — real SMTP for Exchange-internal senders whose
/// 0x0C1F carries an X.500 DN.
const SENDER_SMTP_ADDR: u32 = 0x5D0A;
const DISPLAY_TO: u32 = 0x0E04;
const CREATION_TIME: u32 = 0x3007;
const BODY_TEXT: u32 = 0x1000;
const BODY_HTML: u32 = 0x1013;
const MESSAGE_CODEPAGE: u32 = 0x3FDE;
const ATTACH_DATA: u32 = 0x3701;
const ATTACH_FILENAME: u32 = 0x3707;
const ATTACH_SIZE: u32 = 0x3712;

/// Property value types (low 16 bits of a tag).
const PT_STRING8: u32 = 0x001E;
const PT_UNICODE: u32 = 0x001F;
const PT_BINARY: u32 = 0x0102;
const PT_I4: u32 = 0x0003;
const PT_FILETIME: u32 = 0x0040;

/// One `__properties_version1.0` record: tag + flags + 8-byte value slot.
struct PropRecord {
    id: u32,
    prop_type: u32,
    /// Inline payload for fixed-size types; `(length, reserved)` for the
    /// variable-size types whose bytes live in `__substg1.0_*` streams.
    inline: [u8; 8],
}

/// Outlook `.msg` converter: MAPI → MIME → the eml renderer.
pub struct MsgConverter;

impl MsgConverter {
    /// Create a new MSG converter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for MsgConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Converter for MsgConverter {
    fn name(&self) -> &'static str {
        "msg"
    }

    async fn convert(&self, input: Input) -> Result<ConvertResult, AlchemizeError> {
        let bytes = input
            .to_bytes()
            .await
            .map_err(|e| AlchemizeError::other(format!("failed to read MSG input: {e}")))?;

        // CFB 解析 + MIME 重建是 CPU 密集段，与 pdf/chm 一致包 spawn_blocking。
        tokio::task::spawn_blocking(move || convert_sync(bytes))
            .await
            .map_err(|e| AlchemizeError::other(format!("MSG extraction task failed: {e}")))?
    }

    fn supported_extensions(&self) -> &[&str] {
        &["msg"]
    }

    fn priority(&self) -> u8 {
        10
    }

    fn availability(&self) -> Availability {
        Availability::Available
    }
}

/// The whole (synchronous) MSG conversion: MAPI read → MIME rebuild → the
/// shared eml renderer. Runs on the blocking pool.
fn convert_sync(bytes: Vec<u8>) -> Result<ConvertResult, AlchemizeError> {
    let mut cf = cfb::CompoundFile::open(Cursor::new(bytes))
        .map_err(|e| AlchemizeError::other(format!("MSG compound file open failed: {e}")))?;

    let props = read_prop_records(&mut cf, "/")?;
    let codepage = inline_u32(&props, MESSAGE_CODEPAGE, PT_I4).unwrap_or(65001);
    let mut lossy = false;

    let subject = read_string(&mut cf, "/", &props, SUBJECT, codepage, &mut lossy);
    // Modern Outlook writes SenderName/SMTP as 0x0C1A/0x0C1F; older
    // writers (and forward originals) fall back to 0x0042/0x0044.
    let sender_name =
        read_string(&mut cf, "/", &props, SENDER_NAME, codepage, &mut lossy).or_else(|| {
            read_string(
                &mut cf,
                "/",
                &props,
                SENDER_NAME_LEGACY,
                codepage,
                &mut lossy,
            )
        });
    // Prefer the address form that actually looks like an e-mail: the
    // SMTP-specific properties first, then the generic address tags
    // (0x0C1F may hold an X.500 DN for Exchange-internal senders).
    let addr_candidates = [SENDER_SMTP_ADDR, SENDER_SMTP, SENDER_EMAIL];
    let sender_addr = addr_candidates
        .iter()
        .filter_map(|id| read_string(&mut cf, "/", &props, *id, codepage, &mut lossy))
        .find(|addr| addr.contains('@'))
        .or_else(|| {
            addr_candidates
                .iter()
                .filter_map(|id| read_string(&mut cf, "/", &props, *id, codepage, &mut lossy))
                .find(|addr| !addr.is_empty())
        });
    let from = match (sender_name, sender_addr) {
        (Some(name), Some(addr)) => Some(format!("{name} <{addr}>")),
        (None, Some(addr)) => Some(addr),
        (Some(name), None) => Some(name),
        _ => None,
    };
    let to = read_string(&mut cf, "/", &props, DISPLAY_TO, codepage, &mut lossy);
    let text_body = read_string(&mut cf, "/", &props, BODY_TEXT, codepage, &mut lossy);
    let html_body = read_bytes(&mut cf, "/", &props, BODY_HTML);
    let date = inline_u64(&props, CREATION_TIME, PT_FILETIME).map(filetime_to_rfc2822);

    // Attachment storages, in their numbered order.
    let mut attach_paths: Vec<String> = cf
        .walk()
        .filter(|e| {
            e.is_storage()
                && e.path()
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(ATTACH_PREFIX))
        })
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    attach_paths.sort();
    let mut attachments = Vec::new();
    for path in &attach_paths {
        let records = read_prop_records(&mut cf, path)?;
        let name = read_string(
            &mut cf,
            path,
            &records,
            ATTACH_FILENAME,
            codepage,
            &mut lossy,
        )
        .unwrap_or_else(|| format!("attachment-{}", attachments.len()));
        let size = inline_u32(&records, ATTACH_SIZE, PT_I4).unwrap_or(0) as usize;
        attachments.push((name, size, read_bytes(&mut cf, path, &records, ATTACH_DATA)));
    }

    let html_body_text = html_body.as_deref().map(String::from_utf8_lossy);
    let attach_refs: Vec<(&str, &[u8])> = attachments
        .iter()
        .map(|(name, _size, data)| (name.as_str(), data.as_deref().unwrap_or(&[])))
        .collect();
    let mime = build_mime(MimeInput {
        subject: subject.as_deref(),
        from: from.as_deref(),
        to: to.as_deref(),
        date: date.as_deref(),
        text_body: text_body.as_deref(),
        html_body: html_body_text.as_deref(),
        attachments: &attach_refs,
    });

    let mut result = crate::infrastructure::document::eml::convert_eml_bytes(&mime, "msg")?;
    result.metadata["msg_codepage"] = serde_json::json!(codepage);
    if lossy {
        result.metadata["lossy_decode"] = serde_json::json!(true);
    }
    Ok(result)
}

struct MimeInput<'a> {
    subject: Option<&'a str>,
    from: Option<&'a str>,
    to: Option<&'a str>,
    date: Option<&'a str>,
    text_body: Option<&'a str>,
    html_body: Option<&'a str>,
    attachments: &'a [(&'a str, &'a [u8])],
}

/// Rebuild a plain, deterministic RFC 822 message the eml renderer accepts.
fn build_mime(input: MimeInput<'_>) -> Vec<u8> {
    let mut parts: Vec<(String, Cow<'_, [u8]>)> = Vec::new();
    match (input.text_body, input.html_body) {
        (Some(text), Some(html)) => {
            parts.push((
                "text/plain; charset=utf-8".into(),
                Cow::Borrowed(text.as_bytes()),
            ));
            parts.push((
                "text/html; charset=utf-8".into(),
                Cow::Borrowed(html.as_bytes()),
            ));
        }
        (Some(text), None) => parts.push((
            "text/plain; charset=utf-8".into(),
            Cow::Borrowed(text.as_bytes()),
        )),
        (None, Some(html)) => parts.push((
            "text/html; charset=utf-8".into(),
            Cow::Borrowed(html.as_bytes()),
        )),
        (None, None) => {}
    }
    // MAPI strings may carry quotes or CR/LF; both would let a body forge
    // parts or inject headers into the rebuilt message.
    for (name, data) in input.attachments {
        let safe_name: String = name
            .chars()
            .map(|c| match c {
                '"' => '\'',
                c if c.is_control() => ' ',
                c => c,
            })
            .collect();
        parts.push((
            format!("application/octet-stream; name=\"{safe_name}\""),
            Cow::Borrowed(data),
        ));
    }

    let boundary = boundary_for(&parts);
    // Sizes are known up front: reserve once instead of doubling a
    // multi-megabyte buffer through incremental growth. Base64 rows are
    // 78 output bytes per 57 input bytes (76 chars + CRLF); text bodies
    // write through with a trailing CRLF.
    let estimate: usize = 4096
        + parts
            .iter()
            .map(|(ct, body)| {
                ct.len()
                    + 128
                    + if ct.starts_with("application/octet-stream") {
                        body.len().div_ceil(57) * 78
                    } else {
                        body.len() + 2
                    }
            })
            .sum::<usize>();
    let mut out = String::with_capacity(estimate);

    let header = |out: &mut String, name: &str, value: Option<&str>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            out.push_str(name);
            out.push_str(": ");
            out.push_str(&encode_header_value(value));
            out.push_str("\r\n");
        }
    };
    header(&mut out, "From", input.from);
    header(&mut out, "To", input.to);
    header(&mut out, "Subject", input.subject);
    header(&mut out, "Date", input.date);
    out.push_str("MIME-Version: 1.0\r\n");

    if parts.len() == 1 && input.attachments.is_empty() {
        // Single body part: no multipart framing needed.
        out.push_str(&format!("Content-Type: {}\r\n\r\n", parts[0].0));
        out.push_str(&String::from_utf8_lossy(&parts[0].1));
        out.push_str("\r\n");
        return out.into_bytes();
    }

    out.push_str(&format!(
        "Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"
    ));
    for (content_type, body) in &parts {
        out.push_str(&format!("--{boundary}\r\nContent-Type: {content_type}\r\n"));
        let is_attachment = content_type.starts_with("application/octet-stream");
        if is_attachment {
            let name = content_type
                .rsplit("name=\"")
                .next()
                .and_then(|s| s.split('"').next())
                .unwrap_or("attachment");
            out.push_str(&format!(
                "Content-Disposition: attachment; filename=\"{name}\"\r\n"
            ));
            out.push_str("Content-Transfer-Encoding: base64\r\n");
        }
        out.push_str("\r\n");
        if is_attachment {
            // Stream the base64 straight into the output: the raw bytes
            // never get copied, and the encoded form never exists as a
            // separate allocation. 57 raw bytes = 76 encoded chars, so the
            // padding only ever lands on the final row.
            const ROW: usize = 57;
            let mut row = [0u8; 76];
            for chunk in body.chunks(ROW) {
                let written = BASE64
                    .encode_slice(chunk, &mut row)
                    .expect("76 bytes always fit 57 input bytes");
                out.push_str(std::str::from_utf8(&row[..written]).expect("base64 is ASCII"));
                out.push_str("\r\n");
            }
        } else {
            out.push_str(&String::from_utf8_lossy(body));
            out.push_str("\r\n");
        }
    }
    out.push_str(&format!("--{boundary}--\r\n"));
    out.into_bytes()
}

/// Derive the multipart boundary from the part contents: a body that
/// contains the fixed default boundary cannot collide with a hash of
/// itself.
fn boundary_for(parts: &[(String, Cow<'_, [u8]>)]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (content_type, body) in parts {
        content_type.hash(&mut hasher);
        body.hash(&mut hasher);
    }
    format!("alch_{:016x}", hasher.finish())
}

/// Read all `__properties_version1.0` records under `storage` ("/" for the
/// message root). A missing properties stream is an empty map.
fn read_prop_records(
    cf: &mut cfb::CompoundFile<Cursor<Vec<u8>>>,
    storage: &str,
) -> Result<Vec<PropRecord>, AlchemizeError> {
    let prop_path = format!("{}/{}", storage.trim_end_matches('/'), PROPS_STREAM);
    let Ok(mut stream) = cf.open_stream(&prop_path) else {
        return Ok(Vec::new());
    };
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|e| AlchemizeError::other(format!("MSG properties read failed: {e}")))?;
    let mut records = Vec::new();
    let (chunks, rest) = bytes.as_chunks::<16>();
    if !rest.is_empty() {
        // A truncated tail means the stream is not a whole number of
        // records — say so instead of silently dropping it.
        tracing::warn!(
            "msg: {} trailing bytes in properties stream ignored",
            rest.len()
        );
    }
    for chunk in chunks {
        let tag = u32::from_le_bytes(chunk[0..4].try_into().unwrap());
        records.push(PropRecord {
            id: tag >> 16,
            prop_type: tag & 0xffff,
            inline: chunk[8..16].try_into().unwrap(),
        });
    }
    Ok(records)
}

/// Read a `__substg1.0_<id><type>` stream under `storage`.
fn read_substg(
    cf: &mut cfb::CompoundFile<Cursor<Vec<u8>>>,
    storage: &str,
    id: u32,
    prop_type: u32,
) -> Option<Vec<u8>> {
    let path = format!(
        "{}/__substg1.0_{:04X}{:04X}",
        storage.trim_end_matches('/'),
        id,
        prop_type
    );
    let mut stream = cf.open_stream(&path).ok()?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Find the record for `id` (any type) — Unicode before codepage bytes.
fn find_record<'a>(props: &'a [PropRecord], id: u32, prefer: &[u32]) -> Option<&'a PropRecord> {
    for prop_type in prefer {
        if let Some(record) = props
            .iter()
            .find(|r| r.id == id && r.prop_type == *prop_type)
        {
            return Some(record);
        }
    }
    props.iter().find(|r| r.id == id)
}

/// Inline u32 value (PT_I4 etc.) from the 8-byte value slot.
fn inline_u32(props: &[PropRecord], id: u32, prop_type: u32) -> Option<u32> {
    let record = props
        .iter()
        .find(|r| r.id == id && r.prop_type == prop_type)?;
    Some(u32::from_le_bytes(record.inline[0..4].try_into().unwrap()))
}

/// Inline u64 value (PT_FILETIME).
fn inline_u64(props: &[PropRecord], id: u32, prop_type: u32) -> Option<u64> {
    let record = props
        .iter()
        .find(|r| r.id == id && r.prop_type == prop_type)?;
    Some(u64::from_le_bytes(record.inline[0..8].try_into().unwrap()))
}

/// Read a string property: Unicode first, then codepage bytes, marking the
/// result lossy when neither decodes cleanly. `storage` scopes the
/// `__substg1.0_*` lookup (root "/" or an attachment storage path).
fn read_string(
    cf: &mut cfb::CompoundFile<Cursor<Vec<u8>>>,
    storage: &str,
    props: &[PropRecord],
    id: u32,
    codepage: u32,
    lossy: &mut bool,
) -> Option<String> {
    let record = find_record(props, id, &[PT_UNICODE, PT_STRING8])?;
    let bytes = match record.prop_type {
        PT_UNICODE | PT_STRING8 | PT_BINARY => read_substg(cf, storage, id, record.prop_type)?,
        _ => return None,
    };
    Some(match record.prop_type {
        // PT_UNICODE 语义固定为 UTF-16LE：decode_with_bom_removal 剥掉
        // 误当 BOM 的前两字节，但不允许流首字节改写解码器选择；坏 UTF-16
        // 置 lossy，而不是无声地丢 replacement char。
        PT_UNICODE => {
            let (text, had_errors) = encoding_rs::UTF_16LE.decode_with_bom_removal(&bytes);
            if had_errors {
                *lossy = true;
            }
            text.into_owned()
        }
        PT_STRING8 => decode_codepage(&bytes, codepage, lossy),
        _ => String::from_utf8_lossy(&bytes).into_owned(),
    })
}

/// Read a binary property's stream bytes from `storage`.
fn read_bytes(
    cf: &mut cfb::CompoundFile<Cursor<Vec<u8>>>,
    storage: &str,
    props: &[PropRecord],
    id: u32,
) -> Option<Vec<u8>> {
    let record = find_record(props, id, &[PT_BINARY])?;
    read_substg(cf, storage, id, record.prop_type)
}

/// Decode PT_STRING8 bytes with the message codepage (common Windows
/// codepages only; anything else degrades to lossy UTF-8 and says so).
fn decode_codepage(bytes: &[u8], codepage: u32, lossy: &mut bool) -> String {
    let label = match codepage {
        65001 => "utf-8",
        1252 => "windows-1252",
        1251 => "windows-1251",
        932 => "Shift_JIS",
        936 => "gb18030",
        949 => "EUC-KR",
        950 => "Big5",
        _ => {
            *lossy = true;
            "utf-8"
        }
    };
    let (text, _, had_errors) = encoding_rs::Encoding::for_label(label.as_bytes())
        .unwrap_or(encoding_rs::UTF_8)
        .decode(bytes);
    if had_errors {
        *lossy = true;
    }
    text.into_owned()
}

/// FILETIME (100ns ticks since 1601-01-01 UTC) → RFC 2822 date string.
fn filetime_to_rfc2822(ticks: u64) -> String {
    const EPOCH_DELTA_SECS: u64 = 11_644_473_600;
    let unix_secs = (ticks / 10_000_000).saturating_sub(EPOCH_DELTA_SECS);
    let days = unix_secs / 86_400;
    let secs_of_day = unix_secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let weekday = ((days + 4) % 7) as usize;
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        WEEKDAYS[weekday],
        day,
        MONTHS[(month - 1) as usize],
        year,
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// Header values go through two sanity passes: CR/LF and other controls
/// are stripped (a MAPI string property must not inject or forge headers
/// in the rebuilt MIME), then non-ASCII text becomes an RFC 2047
/// encoded-word (UTF-8/base64).
fn encode_header_value(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| match c {
            '\r' | '\n' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    if cleaned.is_ascii() {
        return cleaned;
    }
    format!("=?utf-8?B?{}?=", BASE64.encode(cleaned.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// Write one properties record: tag, flags(0), 8-byte value slot.
    fn prop_record(tag: u32, value: [u8; 8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&value);
        out
    }

    /// Variable-size record value slot: (length, reserved).
    fn var_value(len: u32) -> [u8; 8] {
        let mut slot = [0u8; 8];
        slot[0..4].copy_from_slice(&len.to_le_bytes());
        slot
    }

    /// Inline u32 slot.
    fn u32_value(value: u32) -> [u8; 8] {
        let mut slot = [0u8; 8];
        slot[0..4].copy_from_slice(&value.to_le_bytes());
        slot
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    /// A deterministic minimal .msg (fixture generator — the cfb crate's
    /// write side stands in for Outlook here).
    fn sample_msg() -> Vec<u8> {
        let subject = utf16("Quarterly numbers");
        let body = utf16("Hello Bob,\r\nattached is the report.\r\n");
        let from_name = utf16("Alice");
        let from_addr = utf16("alice@example.com");
        let to = utf16("Bob <bob@example.com>");
        let attach_name = utf16("report.txt");
        let attach_data = b"col1,col2\r\n1,2\r\n".to_vec();

        // FILETIME for 2026-11-09T10:00:00Z: 100ns ticks since 1601.
        let ticks: u64 = (unix_secs(2026, 11, 9, 10, 0, 0) + 11_644_473_600) * 10_000_000;

        let mut root_props = Vec::new();
        root_props.extend(prop_record(
            (SUBJECT << 16) | PT_UNICODE,
            var_value(subject.len() as u32),
        ));
        root_props.extend(prop_record(
            (SENDER_NAME << 16) | PT_UNICODE,
            var_value(from_name.len() as u32),
        ));
        root_props.extend(prop_record(
            (SENDER_EMAIL << 16) | PT_UNICODE,
            var_value(from_addr.len() as u32),
        ));
        root_props.extend(prop_record(
            (DISPLAY_TO << 16) | PT_UNICODE,
            var_value(to.len() as u32),
        ));
        root_props.extend(prop_record(
            (CREATION_TIME << 16) | PT_FILETIME,
            ticks.to_le_bytes(),
        ));
        root_props.extend(prop_record(
            (MESSAGE_CODEPAGE << 16) | PT_I4,
            u32_value(65001),
        ));
        root_props.extend(prop_record(
            (BODY_TEXT << 16) | PT_UNICODE,
            var_value(body.len() as u32),
        ));

        let cursor = Cursor::new(Vec::new());
        let mut cf = cfb::CompoundFile::create(cursor).unwrap();
        cf.create_stream("/__properties_version1.0")
            .unwrap()
            .write_all(&root_props)
            .unwrap();
        for (id, bytes) in [
            (SUBJECT, &subject),
            (SENDER_NAME, &from_name),
            (SENDER_EMAIL, &from_addr),
            (DISPLAY_TO, &to),
            (BODY_TEXT, &body),
        ] {
            let path = format!("/__substg1.0_{id:04X}001F");
            cf.create_stream(&path).unwrap().write_all(bytes).unwrap();
        }

        let mut attach_props = Vec::new();
        attach_props.extend(prop_record(
            (ATTACH_FILENAME << 16) | PT_UNICODE,
            var_value(attach_name.len() as u32),
        ));
        attach_props.extend(prop_record(
            (ATTACH_SIZE << 16) | PT_I4,
            u32_value(attach_data.len() as u32),
        ));
        attach_props.extend(prop_record(
            (ATTACH_DATA << 16) | PT_BINARY,
            var_value(attach_data.len() as u32),
        ));
        let attach_root = "/__attach_version1.0_#00000001";
        cf.create_storage(attach_root).unwrap();
        cf.create_stream(format!("{attach_root}/{PROPS_STREAM}"))
            .unwrap()
            .write_all(&attach_props)
            .unwrap();
        cf.create_stream(format!(
            "{attach_root}/__substg1.0_{ATTACH_FILENAME:04X}001F"
        ))
        .unwrap()
        .write_all(&attach_name)
        .unwrap();
        cf.create_stream(format!("{attach_root}/__substg1.0_{ATTACH_DATA:04X}0102"))
            .unwrap()
            .write_all(&attach_data)
            .unwrap();
        cf.into_inner().into_inner()
    }

    /// Unix seconds for a UTC wall-clock time (test helper only).
    fn unix_secs(year: i64, month: i64, day: i64, hh: i64, mm: i64, ss: i64) -> u64 {
        let y = if month <= 2 { year - 1 } else { year };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = if month > 2 { month - 3 } else { month + 9 };
        let doy = (153 * mp + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        (days * 86_400 + hh * 3600 + mm * 60 + ss) as u64
    }

    #[tokio::test]
    async fn headers_body_and_attachment_render() {
        let result = MsgConverter::new()
            .convert(Input::bytes(sample_msg(), "sample.msg"))
            .await
            .unwrap();
        let md = &result.markdown;
        assert!(md.contains("Quarterly numbers"), "{md}");
        assert!(md.contains("alice@example.com"), "{md}");
        assert!(md.contains("bob@example.com"), "{md}");
        assert!(md.contains("Hello Bob"), "{md}");
        assert_eq!(result.metadata["source_format"].as_str(), Some("msg"));
        assert_eq!(result.metadata["msg_codepage"].as_u64(), Some(65001));
        assert_eq!(result.metadata["attachment_count"].as_u64(), Some(1));
        let attachments = result.metadata["attachments"].as_array().unwrap();
        assert!(
            attachments[0].as_str().unwrap().contains("report.txt"),
            "attachments: {attachments:?}"
        );
    }

    #[tokio::test]
    async fn output_shape_matches_eml_path() {
        let msg_result = MsgConverter::new()
            .convert(Input::bytes(sample_msg(), "s.msg"))
            .await
            .unwrap();
        let eml_result = crate::infrastructure::document::eml::convert_eml_bytes(
            &build_mime(MimeInput {
                subject: Some("Quarterly numbers"),
                from: Some("Alice <alice@example.com>"),
                to: Some("Bob <bob@example.com>"),
                date: None,
                text_body: Some("Hello Bob,\r\nattached is the report.\r\n"),
                html_body: None,
                attachments: &[("report.txt", b"col1,col2\r\n1,2\r\n")],
            }),
            "msg",
        )
        .unwrap();
        // Same keys, same values (modulo the msg-specific codepage key).
        let a = &msg_result.metadata;
        let b = &eml_result.metadata;
        for key in ["subject", "attachment_count", "attachments"] {
            assert_eq!(a[key], b[key], "metadata key {key} drifted");
        }
        assert!(a.get("msg_codepage").is_some() && b.get("msg_codepage").is_none());
    }

    #[tokio::test]
    async fn garbage_input_errors_without_panicking() {
        let err = MsgConverter::new()
            .convert(Input::bytes(b"not a compound file".to_vec(), "x.msg"))
            .await;
        assert!(err.is_err(), "garbage must not silently convert");
    }

    #[test]
    fn contract() {
        let c = MsgConverter::new();
        assert_eq!(c.name(), "msg");
        assert_eq!(c.supported_extensions(), &["msg"]);
        assert_eq!(c.availability(), Availability::Available);
    }

    #[test]
    fn filetime_conversion_is_stable() {
        let ticks = (unix_secs(2026, 11, 9, 10, 0, 0) + 11_644_473_600) * 10_000_000;
        assert_eq!(
            filetime_to_rfc2822(ticks),
            "Mon, 09 Nov 2026 10:00:00 +0000"
        );
    }

    #[test]
    fn non_ascii_headers_become_encoded_words() {
        assert_eq!(encode_header_value("plain"), "plain");
        let encoded = encode_header_value("季度数字");
        assert!(encoded.starts_with("=?utf-8?B?"), "{encoded}");
    }

    #[test]
    fn header_control_chars_are_stripped() {
        // Controls collapse to spaces: the value stays on ONE header line,
        // so no new header or part can be injected.
        let cleaned = encode_header_value("a\r\nX-Evil: injected\r\n");
        assert!(!cleaned.contains('\r'), "{cleaned}");
        assert!(!cleaned.contains('\n'), "{cleaned}");
    }

    #[test]
    fn attachment_name_cannot_forge_parts() {
        let forged = "a\".bin\r\nContent-Type: text/html\r\n\r\nFORGED-PART";
        let out = build_mime(MimeInput {
            subject: None,
            from: None,
            to: None,
            date: None,
            text_body: Some("body"),
            html_body: None,
            attachments: &[(forged, b"x")],
        });
        let text = String::from_utf8_lossy(&out);
        // Injection is structural: CR/LF are gone, so the only Content-Type
        // lines are the three real ones (multipart header + body part +
        // attachment part), and FORGED-PART never starts a body line of
        // its own — it survives only inside the quoted filename.
        assert_eq!(
            text.lines()
                .filter(|l| l.starts_with("Content-Type:"))
                .count(),
            3,
            "{text}"
        );
        assert!(!text.contains("\r\n\r\nFORGED-PART"), "{text}");
    }

    #[test]
    fn boundary_tracks_content() {
        let mk = |body: &[u8]| boundary_for(&[("text/plain".to_string(), Cow::Borrowed(body))]);
        assert_ne!(mk(b"one"), mk(b"two"));
    }

    #[test]
    fn reserve_covers_large_attachment_without_realloc() {
        // 1 MiB attachment: a correct single reserve keeps capacity within
        // the slack of the final length (no doubling growth happened).
        let data = vec![0x41u8; 1024 * 1024];
        let out = build_mime(MimeInput {
            subject: Some("big"),
            from: None,
            to: None,
            date: None,
            text_body: Some("body"),
            html_body: None,
            attachments: &[("big.bin", &data)],
        });
        assert!(
            out.capacity() <= out.len() + 8192,
            "capacity {} vs len {}",
            out.capacity(),
            out.len()
        );
    }

    /// A message carrying only the modern SMTP sender property (0x0C1F):
    /// no legacy 0x0044, so the From header must come from 0x0C1F.
    #[tokio::test]
    async fn sender_smtp_property_is_preferred_fallback() {
        let smtp = utf16("carol@example.net");
        let subject = utf16("SMTP only");
        let mut props = prop_record(
            (SENDER_SMTP << 16) | PT_UNICODE,
            var_value(smtp.len() as u32),
        );
        props.extend(prop_record(
            (SUBJECT << 16) | PT_UNICODE,
            var_value(subject.len() as u32),
        ));
        props.extend(prop_record(
            (MESSAGE_CODEPAGE << 16) | PT_I4,
            u32_value(65001),
        ));

        let mut cf = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        cf.create_stream("/__properties_version1.0")
            .unwrap()
            .write_all(&props)
            .unwrap();
        cf.create_stream("/__substg1.0_0C1F001F")
            .unwrap()
            .write_all(&smtp)
            .unwrap();
        cf.create_stream("/__substg1.0_0037001F")
            .unwrap()
            .write_all(&subject)
            .unwrap();
        let bytes = cf.into_inner().into_inner();

        let result = MsgConverter::new()
            .convert(Input::bytes(bytes, "smtp.msg"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("carol@example.net"),
            "{}",
            result.markdown
        );
        assert!(result.markdown.contains("SMTP only"), "{}", result.markdown);
    }

    /// Real-file regression (ascii.msg from msg-parser-rs): modern Outlook
    /// files carry sender as 0x0C1A/0x0C1F and may lack 0x0044 entirely —
    /// the legacy-only chain used to drop From entirely.
    #[tokio::test]
    async fn modern_sender_tags_render_from_without_legacy() {
        let name = b"from@domain.com".to_vec();
        let smtp = b"from@domain.com".to_vec();
        let subject = b"no legacy sender".to_vec();
        let mut props = prop_record(
            (SENDER_NAME << 16) | PT_STRING8,
            var_value(name.len() as u32),
        );
        props.extend(prop_record(
            (SENDER_SMTP << 16) | PT_STRING8,
            var_value(smtp.len() as u32),
        ));
        props.extend(prop_record(
            (SUBJECT << 16) | PT_STRING8,
            var_value(subject.len() as u32),
        ));
        props.extend(prop_record(
            (MESSAGE_CODEPAGE << 16) | PT_I4,
            u32_value(65001),
        ));

        let mut cf = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        cf.create_stream("/__properties_version1.0")
            .unwrap()
            .write_all(&props)
            .unwrap();
        cf.create_stream(format!("/__substg1.0_{SENDER_NAME:04X}001E"))
            .unwrap()
            .write_all(&name)
            .unwrap();
        cf.create_stream(format!("/__substg1.0_{SENDER_SMTP:04X}001E"))
            .unwrap()
            .write_all(&smtp)
            .unwrap();
        cf.create_stream(format!("/__substg1.0_{SUBJECT:04X}001E"))
            .unwrap()
            .write_all(&subject)
            .unwrap();
        let bytes = cf.into_inner().into_inner();

        let result = MsgConverter::new()
            .convert(Input::bytes(bytes, "modern.msg"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("from@domain.com"),
            "{}",
            result.markdown
        );
    }

    /// Exchange-internal senders carry an X.500 DN in 0x0C1F; the real SMTP
    /// address lives in 0x5D0A (PidTagSenderSmtpAddress) and must win.
    #[tokio::test]
    async fn smtp_address_property_wins_over_dn() {
        let name = utf16("Satya");
        let dn = utf16("/O=EXCHANGELABS/OU=GROUP/CN=RECIPIENTS/CN=SATYA");
        let smtp = utf16("Satya.Nagisetti@netapp.com");
        let subject = utf16("DN vs SMTP");
        let mut props = prop_record(
            (SENDER_NAME << 16) | PT_UNICODE,
            var_value(name.len() as u32),
        );
        props.extend(prop_record(
            (SENDER_SMTP << 16) | PT_UNICODE,
            var_value(dn.len() as u32),
        ));
        props.extend(prop_record(
            (SENDER_SMTP_ADDR << 16) | PT_UNICODE,
            var_value(smtp.len() as u32),
        ));
        props.extend(prop_record(
            (SUBJECT << 16) | PT_UNICODE,
            var_value(subject.len() as u32),
        ));
        props.extend(prop_record(
            (MESSAGE_CODEPAGE << 16) | PT_I4,
            u32_value(65001),
        ));

        let mut cf = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        cf.create_stream("/__properties_version1.0")
            .unwrap()
            .write_all(&props)
            .unwrap();
        cf.create_stream(format!("/__substg1.0_{SENDER_NAME:04X}001F"))
            .unwrap()
            .write_all(&name)
            .unwrap();
        cf.create_stream(format!("/__substg1.0_{SENDER_SMTP:04X}001F"))
            .unwrap()
            .write_all(&dn)
            .unwrap();
        cf.create_stream(format!("/__substg1.0_{SENDER_SMTP_ADDR:04X}001F"))
            .unwrap()
            .write_all(&smtp)
            .unwrap();
        cf.create_stream(format!("/__substg1.0_{SUBJECT:04X}001F"))
            .unwrap()
            .write_all(&subject)
            .unwrap();
        let bytes = cf.into_inner().into_inner();

        let result = MsgConverter::new()
            .convert(Input::bytes(bytes, "dn.msg"))
            .await
            .unwrap();
        assert!(
            result.markdown.contains("Satya.Nagisetti@netapp.com"),
            "{}",
            result.markdown
        );
        assert!(
            !result.markdown.contains("EXCHANGELABS"),
            "DN must not leak into From: {}",
            result.markdown
        );
    }

    // -- 95 攻坚：发件人回退与单部件 MIME 分型 ------------------------------

    /// Minimal fixture with caller-chosen root property streams (no
    /// attachments) — variants for the sender fallback and single-part
    /// MIME framing paths.
    fn minimal_msg(
        with_sender_name: bool,
        body_text: Option<&str>,
        body_html: Option<&str>,
    ) -> Vec<u8> {
        use std::io::Write as _;
        let mut props = Vec::new();
        props.extend(prop_record(
            (MESSAGE_CODEPAGE << 16) | PT_I4,
            u32_value(65001),
        ));
        let mut streams: Vec<(u32, Vec<u8>)> = Vec::new();
        if with_sender_name {
            let name = utf16("Alice");
            props.extend(prop_record(
                (SENDER_NAME << 16) | PT_UNICODE,
                var_value(name.len() as u32),
            ));
            streams.push((SENDER_NAME, name));
        }
        {
            let addr = utf16("alice@example.com");
            props.extend(prop_record(
                (SENDER_EMAIL << 16) | PT_UNICODE,
                var_value(addr.len() as u32),
            ));
            streams.push((SENDER_EMAIL, addr));
        }
        if let Some(text) = body_text {
            let body = utf16(text);
            props.extend(prop_record(
                (BODY_TEXT << 16) | PT_UNICODE,
                var_value(body.len() as u32),
            ));
            streams.push((BODY_TEXT, body));
        }
        if let Some(html) = body_html {
            props.extend(prop_record(
                (BODY_HTML << 16) | PT_BINARY,
                var_value(html.len() as u32),
            ));
            streams.push((BODY_HTML, html.as_bytes().to_vec()));
        }
        let cursor = Cursor::new(Vec::new());
        let mut cf = cfb::CompoundFile::create(cursor).unwrap();
        cf.create_stream(PROPS_STREAM)
            .unwrap()
            .write_all(&props)
            .unwrap();
        for (id, bytes) in streams {
            let suffix = if id == BODY_HTML { "0102" } else { "001F" };
            cf.create_stream(format!("/__substg1.0_{id:04X}{suffix}"))
                .unwrap()
                .write_all(&bytes)
                .unwrap();
        }
        cf.into_inner().into_inner()
    }

    #[tokio::test]
    async fn sender_addr_without_name_falls_back_to_addr() {
        let result = MsgConverter::new()
            .convert(Input::bytes(
                minimal_msg(false, Some("body"), None),
                "m.msg",
            ))
            .await
            .unwrap();
        let md = &result.markdown;
        assert!(md.contains("alice@example.com"), "{md}");
        assert!(!md.contains("Alice"), "no name: {md}");
    }

    #[tokio::test]
    async fn text_only_body_single_part_framing() {
        let result = MsgConverter::new()
            .convert(Input::bytes(
                minimal_msg(true, Some("plain body"), None),
                "m.msg",
            ))
            .await
            .unwrap();
        let md = &result.markdown;
        assert!(md.contains("plain body"), "{md}");
        // 单部件 wire 无 html 段（间接证据：转换产物无 html 专有标记）
        assert!(!md.contains("<p>"), "no html conversion: {md}");
    }

    #[tokio::test]
    async fn html_only_body_single_part_framing() {
        let result = MsgConverter::new()
            .convert(Input::bytes(
                minimal_msg(true, None, Some("<p>html body</p>")),
                "m.msg",
            ))
            .await
            .unwrap();
        let md = &result.markdown;
        // HTML 体经转换进 markdown；单部件 wire 下无 text/plain 段
        assert!(md.contains("html body"), "{md}");
        assert!(!md.contains("plain body"), "no text part: {md}");
    }
}
