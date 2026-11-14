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

use std::io::{Cursor, Read as _};

use async_trait::async_trait;

use crate::application::ports::{Availability, Converter};
use crate::domain::error::AlchemizeError;
use crate::domain::input::Input;
use crate::domain::result::ConvertResult;

const PROPS_STREAM: &str = "__properties_version1.0";
const ATTACH_PREFIX: &str = "__attach_version1.0_";
const BOUNDARY: &str = "ALCHMSGBOUND";

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

        let mut cf = cfb::CompoundFile::open(Cursor::new(bytes))
            .map_err(|e| AlchemizeError::other(format!("MSG compound file open failed: {e}")))?;

        let props = read_prop_records(&mut cf, "/")?;
        let codepage = inline_u32(&props, MESSAGE_CODEPAGE, PT_I4).unwrap_or(65001);
        let mut lossy = false;

        let subject = read_string(&mut cf, "/", &props, SUBJECT, codepage, &mut lossy);
        // Modern Outlook writes SenderName/SMTP as 0x0C1A/0x0C1F; older
        // writers (and forward originals) fall back to 0x0042/0x0044.
        let sender_name = read_string(&mut cf, "/", &props, SENDER_NAME, codepage, &mut lossy)
            .or_else(|| {
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

        let html_body_text = html_body
            .as_deref()
            .map(String::from_utf8_lossy)
            .map(|cow| cow.into_owned());
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
    let mut out = String::new();
    let header = |out: &mut String, name: &str, value: Option<&str>| {
        if let Some(value) = value
            && !value.is_empty()
        {
            out.push_str(&format!("{name}: {}\r\n", encode_header_value(value)));
        }
    };
    header(&mut out, "From", input.from);
    header(&mut out, "To", input.to);
    header(&mut out, "Subject", input.subject);
    header(&mut out, "Date", input.date);
    out.push_str("MIME-Version: 1.0\r\n");

    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    match (input.text_body, input.html_body) {
        (Some(text), Some(html)) => {
            parts.push((
                "text/plain; charset=utf-8".to_string(),
                text.as_bytes().to_vec(),
            ));
            parts.push((
                "text/html; charset=utf-8".to_string(),
                html.as_bytes().to_vec(),
            ));
        }
        (Some(text), None) => parts.push((
            "text/plain; charset=utf-8".to_string(),
            text.as_bytes().to_vec(),
        )),
        (None, Some(html)) => parts.push((
            "text/html; charset=utf-8".to_string(),
            html.as_bytes().to_vec(),
        )),
        (None, None) => {}
    }
    for (name, data) in input.attachments {
        parts.push((
            format!(
                "application/octet-stream; name=\"{}\"",
                name.replace('"', "'")
            ),
            data.to_vec(),
        ));
    }

    if parts.len() == 1 && input.attachments.is_empty() {
        // Single body part: no multipart framing needed.
        out.push_str(&format!("Content-Type: {}\r\n\r\n", parts[0].0));
        out.push_str(&String::from_utf8_lossy(&parts[0].1));
        out.push_str("\r\n");
        return out.into_bytes();
    }

    out.push_str(&format!(
        "Content-Type: multipart/mixed; boundary=\"{BOUNDARY}\"\r\n\r\n"
    ));
    for (content_type, data) in &parts {
        out.push_str(&format!("--{BOUNDARY}\r\nContent-Type: {content_type}\r\n"));
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
            for chunk in base64_encode(data).as_bytes().chunks(76) {
                out.push_str(&String::from_utf8_lossy(chunk));
                out.push_str("\r\n");
            }
        } else {
            out.push_str(&String::from_utf8_lossy(data));
            out.push_str("\r\n");
        }
    }
    out.push_str(&format!("--{BOUNDARY}--\r\n"));
    out.into_bytes()
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
    for chunk in bytes.as_chunks::<16>().0 {
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
        PT_UNICODE => String::from_utf16_lossy(
            &bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect::<Vec<_>>(),
        ),
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

/// Non-ASCII header values become RFC 2047 encoded-words (UTF-8/base64);
/// ASCII passes through untouched.
fn encode_header_value(value: &str) -> String {
    if value.is_ascii() {
        return value.to_string();
    }
    format!("=?utf-8?B?{}?=", base64_encode(value.as_bytes()))
}

/// Minimal standard base64 encoder (avoids a new dependency for ~30 lines).
fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.as_chunks::<3>().0 {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        for shift in [18u32, 12, 6, 0] {
            out.push(TABLE[(n >> shift) as usize & 0x3f] as char);
        }
    }
    match data.len() % 3 {
        1 => {
            let n = u32::from(data[data.len() - 1]) << 16;
            out.push(TABLE[(n >> 18) as usize & 0x3f] as char);
            out.push(TABLE[(n >> 12) as usize & 0x3f] as char);
            out.push_str("==");
        }
        2 => {
            let n =
                (u32::from(data[data.len() - 2]) << 16) | (u32::from(data[data.len() - 1]) << 8);
            for shift in [18u32, 12, 6] {
                out.push(TABLE[(n >> shift) as usize & 0x3f] as char);
            }
            out.push('=');
        }
        _ => {}
    }
    out
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
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b"hell"), "aGVsbA==");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }

    #[test]
    fn non_ascii_headers_become_encoded_words() {
        assert_eq!(encode_header_value("plain"), "plain");
        let encoded = encode_header_value("季度数字");
        assert!(encoded.starts_with("=?utf-8?B?"), "{encoded}");
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
}
