// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! External tool ports (runtime-probed binaries).

#[cfg(any(feature = "latex", feature = "pandoc"))]
pub mod pandoc;

#[cfg(feature = "legacy-office")]
pub mod libreoffice;
