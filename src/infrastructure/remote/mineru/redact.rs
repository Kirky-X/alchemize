// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Credential stripping for MinerU error paths (R-redact-001).
//!
//! Upstream error bodies and reqwest error messages can echo back the
//! Authorization token or presigned-URL query parameters. These helpers
//! scrub such values before they enter [`AlchemizeError`], tracing, or any
//! Display/Debug output. Matching is hand-written (no regex crate): the
//! presigned parameter set is a fixed list, so a small scanner suffices
//! and the `pdf-cloud` feature gains zero new dependencies.

/// Replace every occurrence of each secret with `***`.
///
/// Empty secret strings are skipped (they would otherwise erase the whole
/// message, since every string contains the empty string).
pub(crate) fn redact_secrets(text: &str, secrets: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.is_empty() || !out.contains(secret) {
            continue;
        }
        out = out.replace(secret, "***");
    }
    out
}

/// Query parameters whose values are credentials (Aliyun OSS / AWS S3
/// presign families). Keys compare ASCII-case-insensitively.
const PRESIGNED_PARAM_KEYS: &[&str] = &[
    "Expires",
    "Signature",
    "OSSAccessKeyId",
    "SecurityToken",
    "X-Amz-Signature",
    "X-Amz-Credential",
    "X-Amz-Security-Token",
];

/// Key prefixes marking presigned credentials (Aliyun OSS extended params).
const PRESIGNED_PARAM_PREFIXES: &[&str] = &["X-Oss-"];

fn is_presigned_key(key: &str) -> bool {
    PRESIGNED_PARAM_KEYS
        .iter()
        .any(|k| k.eq_ignore_ascii_case(key))
        || PRESIGNED_PARAM_PREFIXES
            .iter()
            .any(|p| key.len() >= p.len() && key[..p.len()].eq_ignore_ascii_case(p))
}

/// Replace the values of presigned-URL query parameters with `REDACTED`.
///
/// Only the query string (between `?` and `#`) is rewritten; everything
/// else is preserved byte-for-byte. Text without a query string is
/// returned unchanged — non-URL error messages must not be mangled.
pub(crate) fn strip_presigned_params(url: &str) -> String {
    let Some(qmark) = url.find('?') else {
        return url.to_string();
    };
    let (before_query, rest) = url.split_at(qmark + 1);
    // A fragment (if any) is not part of the query string.
    let (query, fragment) = match rest.find('#') {
        Some(hash) => rest.split_at(hash),
        None => (rest, ""),
    };

    let mut out = String::with_capacity(url.len());
    out.push_str(before_query);
    for (i, pair) in query.split('&').enumerate() {
        if i > 0 {
            out.push('&');
        }
        match pair.find('=') {
            Some(eq) => {
                let (key, value) = pair.split_at(eq);
                out.push_str(key);
                out.push('=');
                if is_presigned_key(key) {
                    out.push_str("REDACTED");
                } else {
                    // Skip the '=' that split_at kept on the value side.
                    out.push_str(&value[1..]);
                }
            }
            None => out.push_str(pair),
        }
    }
    out.push_str(fragment);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    mod redact {
        use super::*;

        #[test]
        fn replaces_every_occurrence() {
            let got = redact_secrets("Bearer abc123 said abc123 twice, abc123", &["abc123"]);
            assert_eq!(got, "Bearer *** said *** twice, ***");
        }

        #[test]
        fn empty_secret_list_is_noop() {
            assert_eq!(redact_secrets("nothing to hide", &[]), "nothing to hide");
        }

        #[test]
        fn empty_secret_string_is_skipped() {
            // An empty secret must not erase the whole message.
            assert_eq!(redact_secrets("keep me", &[""]), "keep me");
        }

        #[test]
        fn no_match_returns_input_unchanged() {
            assert_eq!(
                redact_secrets("plain error text", &["tok"]),
                "plain error text"
            );
        }

        #[test]
        fn multiple_secrets_all_stripped() {
            let got = redact_secrets("a=X1; b=Y2", &["X1", "Y2"]);
            assert_eq!(got, "a=***; b=***");
        }
    }

    mod strip {
        use super::*;

        #[test]
        fn only_target_params_are_redacted() {
            let got = strip_presigned_params(
                "https://cdn.example.com/o.zip?Expires=1719999999&Signature=abc123&x=1&y=2",
            );
            assert_eq!(
                got,
                "https://cdn.example.com/o.zip?Expires=REDACTED&Signature=REDACTED&x=1&y=2"
            );
        }

        #[test]
        fn no_query_string_is_unchanged() {
            let text = "MinerU poll returned 500: plain body, not a URL";
            assert_eq!(strip_presigned_params(text), text);
        }

        #[test]
        fn repeated_params_each_replaced() {
            let got = strip_presigned_params("https://h/p?Signature=one&Signature=two&keep=yes");
            assert_eq!(
                got,
                "https://h/p?Signature=REDACTED&Signature=REDACTED&keep=yes"
            );
        }

        #[test]
        fn oss_and_amz_families_covered() {
            let got = strip_presigned_params(
                "https://h/p?OSSAccessKeyId=AK&SecurityToken=ST&X-Amz-Signature=S1&X-Amz-Credential=C1&X-Amz-Security-Token=T1&X-Oss-Meta-X=m",
            );
            assert_eq!(
                got,
                "https://h/p?OSSAccessKeyId=REDACTED&SecurityToken=REDACTED&X-Amz-Signature=REDACTED&X-Amz-Credential=REDACTED&X-Amz-Security-Token=REDACTED&X-Oss-Meta-X=REDACTED"
            );
        }

        #[test]
        fn non_url_text_with_question_mark_keeps_shape() {
            // A message containing '?' but no key/value pairs stays intact.
            let text = "what? no params here";
            assert_eq!(strip_presigned_params(text), text);
        }

        #[test]
        fn fragment_after_query_preserved_and_excluded() {
            let got = strip_presigned_params("https://h/p?Signature=s#frag?Signature=t");
            // The fragment (after '#') is not a query param and stays as-is.
            assert_eq!(got, "https://h/p?Signature=REDACTED#frag?Signature=t");
        }

        #[test]
        fn bare_key_without_value_preserved() {
            let got = strip_presigned_params("https://h/p?flag&Signature=s");
            assert_eq!(got, "https://h/p?flag&Signature=REDACTED");
        }
    }
}
