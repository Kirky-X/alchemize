// SPDX-License-Identifier: Apache-2.0
//! Number-format rendering for XLSX cell values — a practical subset of the
//! Excel format-code language.
//!
//! This module is pure: no I/O, no state. [`apply_format`] returns `None`
//! when the code is outside the supported subset; the caller then falls back
//! to the raw value and counts the fallback. Nothing here silently mangles a
//! value.
//!
//! Supported shape per section: literal prefixes/suffixes (quoted strings),
//! `0`/`#` digit placeholders, thousands separator `,`, decimal point `.`,
//! percent `%`, currency symbols ($, ¥), date/time tokens
//! (`yyyy`, `mm`, `dd`, `yy`, `h`, `m`, `s`, `AM/PM`), scientific notation
//! (`0.00E+00`), and the two/three-section layout
//! (`positive;negative;zero`). Bracketed conditions/colors (`[Red]`, `[<0]`)
//! are tolerated but ignored. Text sections (`@` only) and unsupported codes
//! yield `None`.

/// Excel built-in format-code table (subset actually used by the parser).
/// Returns the format string for a `numFmtId`, or `None` when the id is not
/// in the table.
pub fn builtin_format(id: u16) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        14 => "mm-dd-yy",
        37 => "#,##0 ;(#,##0)",
        38 => "#,##0 ;[Red](#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;[Red](#,##0.00)",
        45 => "h:mm",
        46 => "h:mm:ss",
        _ => return None,
    })
}

/// A parsed format-code section: digit layout plus literal decorations.
#[derive(Debug, Clone, PartialEq)]
struct Section {
    /// Literal text emitted before the digits (from quoted strings or bare
    /// currency symbols).
    prefix: String,
    /// Literal text emitted after the digits.
    suffix: String,
    /// Digit placeholders: `(integer_placeholders, decimal_placeholders)`.
    /// `#` positions that would be empty render as nothing.
    int_digits: u32,
    dec_digits: u32,
    /// Thousands separator requested (`#,##0` family).
    thousands: bool,
    /// Trailing `%` (multiply by 100 and append the symbol).
    percent: bool,
    /// Scientific notation (`0.00E+00` shape only).
    scientific: bool,
    /// Date/time rendering (`yyyy-mm-dd`, `mm-dd-yy`, `h:mm`, `h:mm:ss`).
    datetime: Option<DateTimeKind>,
    /// Render negatives wrapped in parentheses.
    parens_negative: bool,
}

/// Date/time token shapes recognized in a section.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DateTimeKind {
    /// `yyyy-mm-dd`
    IsoDate,
    /// `mm-dd-yy`
    SlashDate,
    /// `h:mm`
    TimeHm,
    /// `h:mm:ss`
    TimeHms,
}

/// Interpret a literal token (quoted string or bare currency char run).
/// Returns `None` when the section contains an unsupported directive.
fn parse_section(code: &str) -> Option<Section> {
    let mut s = Section {
        prefix: String::new(),
        suffix: String::new(),
        int_digits: 0,
        dec_digits: 0,
        thousands: false,
        percent: false,
        scientific: false,
        datetime: None,
        parens_negative: false,
    };

    // Detect date/time / scientific shapes first; they own the whole section.
    let code = code
        .split(&['[', ']'][..])
        .filter(|part| !is_bracket_color(part))
        .collect::<Vec<_>>()
        .join("");

    if code.contains('E') && code.contains('+') {
        // Only `0.00E+00`-shaped scientific is supported.
        let t = code.replace("E+00", "");
        return if t == "0.00" {
            s.scientific = true;
            Some(s)
        } else {
            None
        };
    }

    let lower = code.to_lowercase();
    if lower.contains('y') || lower.contains('d') {
        // Date section: recognize exact token sequences.
        s.datetime = Some(match lower.as_str() {
            "yyyy-mm-dd" => DateTimeKind::IsoDate,
            "mm-dd-yy" => DateTimeKind::SlashDate,
            _ => return None,
        });
        return Some(s);
    }
    if lower.contains('h') || lower.contains('m') && !lower.contains(['0', '#']) {
        // Time section: `h:mm` / `h:mm:ss`.
        s.datetime = Some(match lower.as_str() {
            "h:mm" => DateTimeKind::TimeHm,
            "h:mm:ss" => DateTimeKind::TimeHms,
            _ => return None,
        });
        return Some(s);
    }

    // General-text or text-placeholder sections are not numeric.
    if code.contains('@') || code == "General" {
        return None;
    }

    // Walk the remaining characters, classifying each token.
    let mut chars = code.chars().peekable();
    let mut in_quotes = false;
    let mut quote_buf = String::new();
    let mut past_point = false;
    let mut past_digits = false;
    let mut open_paren = false;

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes {
                    // Closing quote: flush literal into the correct side.
                    if past_digits || past_point {
                        s.suffix.push_str(&quote_buf);
                    } else {
                        s.prefix.push_str(&quote_buf);
                    }
                    quote_buf.clear();
                    in_quotes = false;
                } else {
                    in_quotes = true;
                }
            }
            '\\' => {
                // Escaped literal character (Excel shorthand for quoting).
                if let Some(esc) = chars.next() {
                    if past_digits || past_point {
                        s.suffix.push(esc);
                    } else {
                        s.prefix.push(esc);
                    }
                }
            }
            _ if in_quotes => quote_buf.push(c),
            '(' => {
                open_paren = true;
                s.prefix.push('(');
            }
            ')' => s.suffix.push(')'),
            '$' => {
                if past_digits || past_point {
                    s.suffix.push('$');
                } else {
                    s.prefix.push('$');
                }
            }
            '¥' => {
                if past_digits || past_point {
                    s.suffix.push('¥');
                } else {
                    s.prefix.push('¥');
                }
            }
            '%' => {
                s.percent = true;
                s.suffix.push('%');
            }
            ' ' => {
                // Spaces are literal padding (e.g. `#,##0 ;(#,##0)`).
                if past_digits || past_point {
                    s.suffix.push(' ');
                } else {
                    s.prefix.push(' ');
                }
            }
            '-' => {
                // Leading minus of a negative section (`-0.00`) is literal.
                if past_digits || past_point {
                    s.suffix.push('-');
                } else {
                    s.prefix.push('-');
                }
            }
            ',' => {
                // A comma followed by another digit placeholder is the
                // thousands separator; a trailing comma is a literal.
                if !past_point && chars.peek().is_some_and(|n| *n == '#' || *n == '0') {
                    s.thousands = true;
                } else {
                    s.suffix.push(',');
                }
            }
            '0' | '#' => {
                if past_point {
                    s.dec_digits += 1;
                } else {
                    s.int_digits += 1;
                    past_digits = true;
                }
            }
            '.' => past_point = true,
            _ => return None, // unsupported directive — never guess
        }
    }
    if in_quotes {
        // Unterminated literal: third-party generators emit codes like
        // `0.0"%`; Excel renders the rest as a literal, so flush the buffer
        // to the correct side instead of refusing the whole code.
        if past_digits || past_point {
            s.suffix.push_str(&quote_buf);
        } else {
            s.prefix.push_str(&quote_buf);
        }
    }
    s.parens_negative = open_paren;
    Some(s)
}

/// Whether a bracketed part is a color/condition directive (ignored).
fn is_bracket_color(part: &str) -> bool {
    // The split at '[' / ']' yields inner contents and outer runs. Inner
    // contents are exactly the bracket bodies; colors start with a letter.
    const COLORS: [&str; 9] = [
        "Black", "Blue", "Cyan", "Green", "Magenta", "Red", "White", "Yellow", "Color",
    ];
    COLORS.iter().any(|c| part.trim().starts_with(c))
}

/// Format `value` per the Excel format `code`.
///
/// Returns `None` when the code is outside the supported subset; the caller
/// must then fall back to the raw value and count the fallback.
pub fn apply_format(value: f64, code: &str) -> Option<String> {
    let code = code.trim();
    if code.is_empty() || code == "General" {
        return None; // General is rendered by the caller's default path
    }

    // Split into sections: positive;negative;zero
    let sections: Vec<&str> = code.split(';').collect();
    let num_code = pick_section(value, &sections);

    let mut sec = parse_section(num_code)?;

    // Percent scales the magnitude.
    let magnitude = if sec.percent { value.abs() * 100.0 } else { value.abs() };

    if let Some(kind) = sec.datetime {
        return format_datetime(value, kind);
    }
    if sec.scientific {
        return format_scientific(value);
    }

    // Zero magnitude with no decimal places renders as "0".
    // A section without any digit placeholder renders literals only.
    let literal_only = sec.int_digits == 0 && sec.dec_digits == 0;
    let formatted = if literal_only {
        String::new()
    } else if sec.thousands && sec.int_digits >= 4 {
        format_thousands(magnitude, sec.dec_digits)
    } else {
        format_plain(magnitude, sec.int_digits, sec.dec_digits)
    };

    let mut out = String::new();
    if value < 0.0 && sec.parens_negative {
        // The walking pass already collected '(' into the prefix and ')'
        // into the suffix; rebuild exactly one paren pair around the digits.
        out.push('(');
        out.push_str(&sec.prefix.replace('(', ""));
        out.push_str(&formatted);
        out.push_str(&sec.suffix.replace(')', ""));
        out.push(')');
        return Some(out);
    }
    out.push_str(&sec.prefix);
    out.push_str(&formatted);
    out.push_str(&sec.suffix);
    // Prepend the sign unless the section's own literal already carries it
    // (e.g. the `-0.00` negative section) or parens are the intended form.
    if value < 0.0
        && !sec.parens_negative
        && !sec.prefix.starts_with('-')
        && !sec.suffix.ends_with('-')
    {
        out.insert(0, '-');
    }
    Some(out)
}

/// Choose the section for `value`: bracketed conditions aside, section 1 is
/// for positives, section 2 for negatives, section 3 for zero. Paren
/// rendering comes from the section's own `(` token (see `parse_section`),
/// never forced here.
fn pick_section<'a>(value: f64, sections: &'a [&'a str]) -> &'a str {
    // NB: f64::signum(0.0) == 1.0 (not 0), so zero is matched explicitly —
    // the signum-based arm would silently route zero to the positive branch.
    let sign: i32 = if value == 0.0 {
        0
    } else if value < 0.0 {
        -1
    } else {
        1
    };
    match (sign, sections.len()) {
        (_, 1) => sections[0],
        (0, _) if sections.len() >= 3 => sections[2],
        (-1, _) if sections.len() >= 2 => sections[1],
        _ => sections[0],
    }
}

/// Plain digit rendering with fixed decimal places and minimum integer width.
fn format_plain(magnitude: f64, int_digits: u32, dec_digits: u32) -> String {
    let formatted = format!("{magnitude:.0$}", dec_digits as usize);
    // Trim excess integer digits only when the source allows fewer (`#`).
    // `0`-placeholders keep at least that many digits; a lone `#` renders
    // the natural width. We approximate: keep all integer digits produced by
    // Rust's formatting, then pad to `int_digits` when needed.
    if let Some((int_part, dec_part)) = formatted.split_once('.') {
        let int_part = int_part.strip_prefix('-').unwrap_or(int_part);
        let padded = if (int_part.len() as u32) < int_digits {
            format!("{}{}", "0".repeat((int_digits - int_part.len() as u32) as usize), int_part)
        } else {
            int_part.to_string()
        };
        if dec_digits == 0 {
            padded
        } else {
            format!("{padded}.{dec_part}")
        }
    } else {
        let int_part = formatted.strip_prefix('-').unwrap_or(&formatted);
        let padded = if (int_part.len() as u32) < int_digits {
            format!("{}{}", "0".repeat((int_digits - int_part.len() as u32) as usize), int_part)
        } else {
            int_part.to_string()
        };
        padded
    }
}

/// Thousands-separated rendering at fixed decimal places, rounded-half-up to
/// match Excel's display rounding.
fn format_thousands(magnitude: f64, dec_digits: u32) -> String {
    let factor = 10f64.powi(dec_digits as i32);
    let rounded = (magnitude * factor + 0.5).floor() / factor;
    let formatted = format!("{rounded:.0$}", dec_digits as usize);
    let (int_part, dec_part) = formatted
        .split_once('.')
        .unwrap_or((&formatted, ""));
    let int_part = int_part.strip_prefix('-').unwrap_or(int_part);
    let grouped = group_thousands(int_part);
    if dec_digits == 0 {
        grouped
    } else {
        format!("{grouped}.{dec_part}")
    }
}

/// Insert commas every three digits from the right.
fn group_thousands(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// Scientific notation `d.dde±dd` (Excel shows at least 2 exponent digits).
fn format_scientific(value: f64) -> Option<String> {
    if value == 0.0 {
        return Some("0.00E+00".into());
    }
    let exp = value.abs().log10().floor() as i32;
    let mantissa = value / 10f64.powi(exp);
    Some(format!("{mantissa:.2}E{}{:02}", if exp >= 0 { "+" } else { "-" }, exp.abs()))
}

/// Excel serial datetime → formatted string.
///
/// Excel's 1900 date system (Lotus 1-2-3 compatible) counts a phantom leap
/// day, so no single linear base covers all serials. Verified mapping used
/// here:
/// - `serial 0` → `1899-12-30` (the epoch label pinned by the parser spec;
///   Excel itself displays serial 0 as "1900-01-00"),
/// - `serial 1..=59` → `1899-12-31 + serial` days (serial 1 = 1900-01-01,
///   serial 59 = 1900-02-28),
/// - `serial == 60` → the phantom `1900-02-29`,
/// - `serial >= 61` → `1899-12-30 + serial` days (serial 61 = 1900-03-01,
///   one real day past the phantom leap day).
fn format_datetime(value: f64, kind: DateTimeKind) -> Option<String> {
    let serial = value.trunc();
    if serial < 0.0 {
        return None;
    }
    // Two linear segments around Excel's phantom leap day (serial 60).
    let days = if serial <= 59.0 {
        days_from_ymd(1899, 12, 31) + serial as i64
    } else {
        days_from_ymd(1899, 12, 30) + serial as i64
    };
    let (mut y, mut m, mut d) = ymd_from_days(days);
    if serial == 0.0 {
        y = 1899;
        m = 12;
        d = 30;
    } else if serial == 60.0 {
        y = 1900;
        m = 2;
        d = 29;
    }

    // Time-of-day from the fractional part, rounded to the second.
    let frac = value - value.trunc();
    let total_seconds = (frac * 86_400.0).round() as i64;
    let total_seconds = total_seconds.clamp(0, 86_399);
    let h = total_seconds / 3600;
    let mi = (total_seconds % 3600) / 60;
    let s = total_seconds % 60;

    Some(match kind {
        DateTimeKind::IsoDate => format!("{y:04}-{m:02}-{d:02}"),
        DateTimeKind::SlashDate => format!("{m:02}-{d:02}-{:02}", y % 100),
        DateTimeKind::TimeHm => format!("{h}:{mi:02}"),
        DateTimeKind::TimeHms => format!("{h}:{mi:02}:{s:02}"),
    })
}

/// Days-since-epoch arithmetic (proleptic Gregorian, epoch 0001-01-01 = day 1).
fn days_from_ymd(y: i64, m: i64, d: i64) -> i64 {
    let (y, m) = if m <= 2 { (y - 1, m + 12) } else { (y, m) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468 + 719_162
}

/// Inverse of [`days_from_ymd`].
fn ymd_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468 - 719_162;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table-driven samples: (value, code, expected). `None` expected =
    /// fallback (function returns None).
    #[test]
    fn table_driven_samples() {
        let cases: &[(f64, &str, Option<&str>)] = &[
            // General / unknown codes → None (caller falls back)
            (1234.5, "General", None),
            (1234.5, "", None),
            (1234.5, "@", None),
            (1234.5, "0.00;(0.00);@", Some("1234.50")),
            (0.0, "0.00;(0.00);@", None), // text section "@" only serves zero/text — zero falls to section 2 and "@" itself is not numeric: fallback
            // Bracket colors are tolerated and ignored; the negative
            // section's leading `-` is a literal, and the magnitude is
            // rendered from the absolute value.
            (1234.5, "[Blue]0.00;[Red]-0.00;0.00E+00", Some("1234.50")),
            (-1234.5, "[Blue]0.00;[Red]-0.00;0.00E+00", Some("-1234.50")),
            // Integers
            (5.0, "0", Some("5")),
            (42.0, "000", Some("042")),
            (0.0, "0", Some("0")),
            // Decimals
            (1.5, "0.00", Some("1.50")),
            (3.14159, "0.00", Some("3.14")),
            (3.146, "0.00", Some("3.15")),
            (0.0, "0.00", Some("0.00")),
            (2.0, "0.0000", Some("2.0000")),
            // Thousands separator
            (1234.0, "#,##0", Some("1,234")),
            (1234567.0, "#,##0", Some("1,234,567")),
            (987654321.0, "#,##0", Some("987,654,321")),
            (1234.567, "#,##0.00", Some("1,234.57")),
            (1000.0, "#,##0.00", Some("1,000.00")),
            (0.5, "#,##0", Some("1")),
            // Negative numbers
            (-1234.0, "#,##0", Some("-1,234")),
            (-12.5, "0.00", Some("-12.50")),
            // Percent
            (0.25, "0%", Some("25%")),
            (0.1234, "0.00%", Some("12.34%")),
            (-0.05, "0%", Some("-5%")),
            (1.0, "0%", Some("100%")),
            // Currency
            (1234.5, "$#,##0.00", Some("$1,234.50")),
            (7.0, "$#,##0.00", Some("$7.00")),
            (12345.678, "$#,##0.00", Some("$12,345.68")),
            (999.0, "¥#,##0.00", Some("¥999.00")),
            (100000.0, "¥#,##0", Some("¥100,000")),
            // Quoted literal suffix
            (12.5, "0.00\"万元\"", Some("12.50万元")),
            (3.0, "0\"台\"", Some("3台")),
            (1.234, "0.0\"%", Some("1.2%")),
            // Date serials (Excel 1900 system; 0 → 1899-12-30)
            (0.0, "yyyy-mm-dd", Some("1899-12-30")),
            (1.0, "yyyy-mm-dd", Some("1900-01-01")),
            (59.0, "yyyy-mm-dd", Some("1900-02-28")),
            (60.0, "yyyy-mm-dd", Some("1900-02-29")),
            (61.0, "yyyy-mm-dd", Some("1900-03-01")),
            (45000.0, "yyyy-mm-dd", Some("2023-03-15")),
            (45000.0, "mm-dd-yy", Some("03-15-23")),
            // Two-section negative-with-parens
            (1234.0, "#,##0 ;(#,##0)", Some("1,234 ")),
            (-1234.0, "#,##0 ;(#,##0)", Some("(1,234)")),
            (-1234.56, "#,##0.00;(#,##0.00)", Some("(1,234.56)")),
            (1234.56, "#,##0.00;(#,##0.00)", Some("1,234.56")),
            (-5.0, "#,##0 ;[Red](#,##0)", Some("(5)")),
            // Scientific
            (12345.0, "0.00E+00", Some("1.23E+04")),
            (0.000123, "0.00E+00", Some("1.23E-04")),
            (0.0, "0.00E+00", Some("0.00E+00")),
            // Three sections: zero goes to section 3
            (0.0, "0.00;-0.00;\"nil\"", Some("nil")),
            (7.0, "0.00;-0.00;\"nil\"", Some("7.00")),
            (-7.0, "0.00;-0.00;\"nil\"", Some("-7.00")),
            // Times
            (0.5, "h:mm", Some("12:00")),
            (0.0, "h:mm", Some("0:00")),
            (45000.75, "h:mm:ss", Some("18:00:00")),
        ];
        for (value, code, expected) in cases {
            let got = apply_format(*value, code);
            assert_eq!(
                got.as_deref(),
                *expected,
                "apply_format({value}, {code:?}) → {got:?}, want {expected:?}"
            );
        }
    }

    #[test]
    fn builtin_format_table() {
        assert_eq!(builtin_format(0), Some("General"));
        assert_eq!(builtin_format(14), Some("mm-dd-yy"));
        assert_eq!(builtin_format(45), Some("h:mm"));
        assert_eq!(builtin_format(46), Some("h:mm:ss"));
        assert_eq!(builtin_format(5), None);
        assert_eq!(builtin_format(49), None);
    }

    #[test]
    fn builtin_format_roundtrip_through_apply() {
        // Every builtin numeric code must be renderable (dates included).
        for id in [1u16, 2, 3, 4, 9, 10, 11] {
            let code = builtin_format(id).unwrap();
            assert!(
                apply_format(1234.5, code).is_some(),
                "builtin {id} ({code}) must apply"
            );
        }
    }
}
