// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Connect-time SSRF revalidation for the MinerU HTTP clients.
//!
//! [`SsrfSafeResolver`] wraps DNS resolution: every address the inner
//! resolver returns is revalidated against the same address policy as
//! parse-time validation ([`validate_resolved_addr`]). Violating addresses
//! are dropped; if no address survives, resolution fails (deny-by-default,
//! so Happy Eyeballs can never fall back onto a violating stack). This
//! closes the DNS-rebinding window between "URL validated at parse time"
//! and "socket connected at request time".

use std::net::SocketAddr;

use reqwest::dns::{Addrs, Name, Resolve};

use super::validated_url::validate_resolved_addr;
use crate::domain::error::AlchemizeError;

/// DNS resolver that filters resolved addresses through the SSRF policy.
///
/// `inner` is the address-producing strategy: the system resolver by
/// default (tokio `lookup_host` — reqwest does not expose its own system
/// resolver publicly, and going through tokio adds no new dependency), a
/// stub in tests.
pub(crate) struct SsrfSafeResolver {
    inner: Box<dyn Resolve>,
}

impl SsrfSafeResolver {
    /// Build with a custom inner resolver (test injection point).
    #[cfg(test)]
    pub(crate) fn with_inner(inner: Box<dyn Resolve>) -> Self {
        Self { inner }
    }
}

impl Default for SsrfSafeResolver {
    fn default() -> Self {
        Self {
            inner: Box::new(SystemResolver),
        }
    }
}

/// System resolver via tokio's getaddrinfo wrapper (port 0 — the connector
/// fills in the request's port).
struct SystemResolver;

impl Resolve for SystemResolver {
    fn resolve(&self, name: Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
                .collect();
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

impl Resolve for SsrfSafeResolver {
    fn resolve(&self, name: Name) -> reqwest::dns::Resolving {
        let resolving = self.inner.resolve(name);
        Box::pin(async move {
            let all: Vec<SocketAddr> = resolving.await?.collect();
            let mut first_violation: Option<AlchemizeError> = None;
            let safe: Vec<SocketAddr> = all
                .into_iter()
                .filter(|addr| match validate_resolved_addr(addr.ip()) {
                    Ok(()) => true,
                    Err(e) => {
                        first_violation.get_or_insert(e);
                        false
                    }
                })
                .collect();
            if safe.is_empty() {
                let err = first_violation.unwrap_or_else(|| AlchemizeError::SecurityViolation {
                    kind: crate::domain::error::SecurityViolationType::PrivateAddress,
                    message: "resolver returned no SSRF-safe addresses".to_string(),
                });
                return Err(Box::new(err) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(safe.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubResolver(Vec<SocketAddr>);
    impl Resolve for StubResolver {
        fn resolve(&self, _name: Name) -> reqwest::dns::Resolving {
            let addrs = self.0.clone();
            Box::pin(async move { Ok(Box::new(addrs.into_iter()) as Addrs) })
        }
    }

    fn sock(host: &str, port: u16) -> SocketAddr {
        format!("{host}:{port}").parse().unwrap()
    }

    /// All-violating resolution → resolve() errors (deny-by-default).
    #[tokio::test]
    async fn loopback_only_resolution_is_rejected() {
        let resolver =
            SsrfSafeResolver::with_inner(Box::new(StubResolver(vec![sock("127.0.0.1", 443)])));
        let result = resolver.resolve("mineru.example".parse().unwrap()).await;
        assert!(result.is_err(), "must reject loopback-only resolution");
    }

    /// Mixed resolution → only public addresses survive the filter.
    #[tokio::test]
    async fn mixed_resolution_keeps_only_public_addresses() {
        let resolver = SsrfSafeResolver::with_inner(Box::new(StubResolver(vec![
            sock("93.184.216.34", 443),
            sock("10.0.0.5", 443),
        ])));
        let addrs: Vec<SocketAddr> = resolver
            .resolve("mineru.example".parse().unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(addrs, vec![sock("93.184.216.34", 443)]);
    }

    /// All-public resolution passes through unchanged (address-family
    /// diversity preserved).
    #[tokio::test]
    async fn public_resolution_passes_through() {
        let public = vec![
            sock("93.184.216.34", 443),
            sock("[2606:2800:220:1::1]", 443),
        ];
        let resolver = SsrfSafeResolver::with_inner(Box::new(StubResolver(public.clone())));
        let addrs: Vec<SocketAddr> = resolver
            .resolve("mineru.example".parse().unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(addrs, public);
    }
}
