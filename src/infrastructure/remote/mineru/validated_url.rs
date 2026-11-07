// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! SSRF-safe URL wrapper for MinerU file URLs .

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::domain::error::{AlchemizeError, SecurityViolationType};

/// A URL that has passed SSRF validation.
///
/// This type ensures that URLs passed to external services (like MinerU)
/// are safe and won't leak internal network access.
///
/// # Validation Rules
///
/// - Must use HTTPS scheme
/// - Must not point to loopback addresses (127.0.0.0/8, ::1, localhost)
/// - Must not point to private IPv4 ranges (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
/// - Must not point to link-local addresses (169.254.0.0/16)
/// - Hostname must be in the allowlist (or allowlist must be empty = allow all public)
#[derive(Debug, Clone)]
pub struct ValidatedFileUrl(String);

impl ValidatedFileUrl {
    /// Get the validated URL as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the validated URL as a String.
    pub fn into_inner(self) -> String {
        self.0
    }
}

/// Default hostname allowlist for MinerU URLs.
/// These are the official MinerU CDN domains.
pub const DEFAULT_MINERU_ALLOWLIST: &[&str] = &["mineru.net", "cdn.mineru.net", "api.mineru.net"];

/// Classify an IPv4 address (given as octets) for SSRF safety.
///
/// Returns `Ok()` if the address is safe, or `Err(AlchemizeError::SecurityViolation)`
/// with the appropriate `kind`. `label` is inserted into the error message to
/// distinguish plain IPv4 from IPv4-mapped/compatible IPv6 addresses
/// (e.g. `""` for plain IPv4, `"IPv4-mapped IPv6 "` for embedded IPv4).
///
/// Covers:
/// - `127.0.0.0/8` (loopback)
/// - `0.0.0.0/8` ("this network", RFC 1122)
/// - `10.0.0.0/8` (RFC1918 Class A private)
/// - `172.16.0.0/12` (RFC1918 Class B private)
/// - `192.168.0.0/16` (RFC1918 Class C private)
/// - `100.64.0.0/10` (CGNAT, RFC 6598 — includes Alibaba Cloud metadata `100.100.100.200`)
/// - `169.254.0.0/16` (link-local)
/// - `255.255.255.255` (broadcast — defense in depth)
///
/// Used for both plain IPv4 addresses and IPv4 addresses embedded in
/// IPv4-mapped/compatible IPv6 addresses (`::ffff:a.b.c.d`, `::a.b.c.d`).
fn check_ipv4_ssrf(octets: [u8; 4], host: &str, label: &str) -> Result<(), AlchemizeError> {
    // Loopback (127.0.0.0/8)
    if octets[0] == 127 {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LoopbackAddress,
            message: format!("URL points to {label}loopback address '{host}'"),
        });
    }
    // Private ranges (RFC1918), 0.0.0.0/8, and CGNAT (RFC 6598)
    if octets[0] == 0
        || octets[0] == 10
        || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
        || (octets[0] == 192 && octets[1] == 168)
        || (octets[0] == 100 && octets[1] >= 64 && octets[1] <= 127)
    {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::PrivateAddress,
            message: format!("URL points to {label}private address '{host}'"),
        });
    }
    // Link-local (169.254/16)
    if octets[0] == 169 && octets[1] == 254 {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LinkLocalAddress,
            message: format!("URL points to {label}link-local address '{host}'"),
        });
    }
    // Broadcast (255.255.255.255) — defense in depth
    if octets == [255, 255, 255, 255] {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::PrivateAddress,
            message: format!("URL points to {label}broadcast address '{host}'"),
        });
    }
    Ok(())
}

/// Validate a file URL for SSRF safety .
///
/// # Arguments
///
/// - `raw`: The raw URL string to validate
/// - `allowlist`: List of allowed hostnames. If empty, any non-private hostname is allowed.
///
/// # Returns
///
/// - `Ok(ValidatedFileUrl)` if the URL passes all checks
/// - `Err(AlchemizeError::SecurityViolation)` if any check fails
///
/// # SSRF Protections
///
/// 1. **Scheme**: Only HTTPS allowed
/// 2. **Loopback**: 127.0.0.0/8, ::1, and "localhost" rejected
/// 3. **Private IPv4**: RFC1918 ranges (10/8, 172.16/12, 192.168/16) rejected
/// 4. **Link-local**: 169.254/16 rejected
/// 5. **Hostname allowlist**: Optional, defaults to MinerU official domains
pub fn validate_file_url(
    raw: &str,
    allowlist: &[&str],
) -> Result<ValidatedFileUrl, AlchemizeError> {
    let parsed = reqwest::Url::parse(raw).map_err(|e| AlchemizeError::SecurityViolation {
        kind: SecurityViolationType::InvalidScheme,
        message: format!("invalid URL '{raw}': {e}"),
    })?;

    // Check scheme: only HTTPS allowed
    if parsed.scheme() != "https" {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::InvalidScheme,
            message: format!("URL must use HTTPS scheme, got '{}'", parsed.scheme()),
        });
    }

    // Extract host
    let host = parsed
        .host_str()
        .ok_or_else(|| AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::HostnameNotAllowed,
            message: format!("URL '{raw}' has no host"),
        })?;

    // Reject loopback addresses
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LoopbackAddress,
            message: format!("URL points to loopback address '{host}'"),
        });
    }

    if let Ok(ipv4) = host.parse::<Ipv4Addr>() {
        check_ipv4_ssrf(ipv4.octets(), host, "")?;
    }

    // Try parsing as IPv6 address (strip brackets for host_str output)
    let ipv6_str = if host.starts_with('[') && host.ends_with(']') {
        &host[1..host.len() - 1]
    } else {
        host
    };
    if let Ok(ipv6) = ipv6_str.parse::<Ipv6Addr>() {
        check_ipv6_ssrf(ipv6, host)?;
    }

    // Check hostname allowlist (if provided)
    check_hostname_allowlist(host, allowlist)?;

    Ok(ValidatedFileUrl(raw.to_string()))
}

/// Check an IPv6 address for SSRF safety.
///
/// Covers:
/// - `::1` (loopback)
/// - `fc00::/7` (ULA, RFC 4193 — equivalent to IPv4 RFC1918)
/// - `fe80::/10` (link-local, RFC 4291)
/// - `ff00::/8` (multicast — defense in depth)
/// - IPv4-mapped (`::ffff:a.b.c.d`) and IPv4-compatible (`::a.b.c.d`, deprecated)
///   — delegates embedded IPv4 to [`check_ipv4_ssrf`] to prevent bypasses like
///   `[::ffff:127.0.0.1]` or `[::ffff:169.254.169.254]`.
///
/// `to_ipv4` is used (not `to_ipv4_mapped`) to cover both mapped and
/// compatible forms.
fn check_ipv6_ssrf(ipv6: Ipv6Addr, host: &str) -> Result<(), AlchemizeError> {
    if ipv6.is_loopback() {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LoopbackAddress,
            message: format!("URL points to loopback address '{host}'"),
        });
    }

    let segments = ipv6.segments();

    // IPv6 ULA (Unique Local Addresses, fc00::/7, RFC 4193) — equivalent to IPv4 RFC1918
    if (segments[0] & 0xfe00) == 0xfc00 {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::PrivateAddress,
            message: format!("URL points to IPv6 ULA address '{host}'"),
        });
    }

    // IPv6 link-local (fe80::/10, RFC 4291) — equivalent to IPv4 169.254/16
    if (segments[0] & 0xffc0) == 0xfe80 {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::LinkLocalAddress,
            message: format!("URL points to IPv6 link-local address '{host}'"),
        });
    }

    // IPv6 multicast (ff00::/8) — defense in depth
    if (segments[0] & 0xff00) == 0xff00 {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::PrivateAddress,
            message: format!("URL points to IPv6 multicast address '{host}'"),
        });
    }

    // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d, RFC 4291 deprecated)
    // — must apply IPv4 SSRF checks to the embedded IPv4 to prevent bypasses like
    // [::ffff:127.0.0.1] / [::127.0.0.1] / [::ffff:10.0.0.1] / [::ffff:169.254.169.254]
    // `to_ipv4` covers both forms (mapped + compatible), unlike `to_ipv4_mapped`.
    if let Some(ipv4_embedded) = ipv6.to_ipv4() {
        check_ipv4_ssrf(ipv4_embedded.octets(), host, "IPv4-mapped IPv6 ")?;
    }

    Ok(())
}

/// Check hostname against an allowlist.
///
/// `host` may be a literal IP or a hostname. A hostname is allowed if it
/// exactly matches an entry or is a subdomain of one (suffix match with a
/// leading dot to prevent partial-label matches like `evilmineru.net`).
///
/// `reqwest::Url::host_str` returns the host without port, so no port
/// stripping is needed. Using `split(':')` would break IPv6 literals like
/// `[2001:db8::1]` by truncating at the first colon.
///
/// Does not include the allowlist contents in the error message — leaking
/// configuration details could aid an attacker in understanding the
/// deployment's trust boundaries.
fn check_hostname_allowlist(host: &str, allowlist: &[&str]) -> Result<(), AlchemizeError> {
    if allowlist.is_empty() {
        return Ok(());
    }

    let suffixes: Vec<String> = allowlist.iter().map(|a| format!(".{}", a)).collect();
    let is_allowed = allowlist.contains(&host)
        || suffixes
            .iter()
            .any(|suffix| host.ends_with(suffix.as_str()));
    if !is_allowed {
        return Err(AlchemizeError::SecurityViolation {
            kind: SecurityViolationType::HostnameNotAllowed,
            message: format!("hostname '{host}' is not in the configured allowlist"),
        });
    }
    Ok(())
}

/// Validate the base URL scheme (http or https).
///
/// Used by `MineruAgentClient` and `MineruPrecisionClient` to validate their
/// `base_url` configuration. Unlike [`validate_file_url`], this allows `http`
/// because MinerU can be self-hosted on plain HTTP (e.g. `http://localhost:8888`).
pub(crate) fn validate_base_url(url: &str) -> Result<(), AlchemizeError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| AlchemizeError::Other(format!("invalid MinerU URL '{url}': {e}")))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(AlchemizeError::Other(format!(
            "MinerU URL must use http or https scheme, got '{scheme}'"
        )));
    }

    Ok(())
}
