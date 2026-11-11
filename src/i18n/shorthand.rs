// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Translation shorthand functions — crate-root re-exports
//! ([`tr`](crate::tr) / [`tr_args`](crate::tr_args))。

use super::catalog::translate;

/// Shorthand: [`translate`](super::catalog::translate) with no dynamic
/// arguments.
pub fn tr(key: &str) -> String {
    translate(key, &[])
}

/// Shorthand: [`translate`](super::catalog::translate) with arguments.
///
/// ```ignore
/// tr_args("err-file-read", &[("path", path.display().to_string()), ("source", e.to_string())])
/// ```
pub fn tr_args(key: &str, args: &[(&str, String)]) -> String {
    translate(key, args)
}
