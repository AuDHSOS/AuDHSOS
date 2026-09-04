// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Verification that the pinned toolchain is in use.

use std::path::Path;

use crate::error::Error;
use crate::fs;

/// The environment variable rustup's proxies set for child processes.
const TOOLCHAIN_VARIABLE: &str = "RUSTUP_TOOLCHAIN";

/// Reads the pinned channel from `rust-toolchain.toml` and verifies that the
/// running Cargo is rustup's proxy for that channel. Returns the channel.
pub(crate) fn verify(root: &Path) -> Result<String, Error> {
    let manifest = fs::read(&root.join("rust-toolchain.toml"))?;
    let channel = channel_of(&manifest)
        .ok_or_else(|| Error::Toolchain("rust-toolchain.toml names no channel".to_owned()))?;
    let active = std::env::var(TOOLCHAIN_VARIABLE).ok();
    check_active(&channel, active.as_deref())?;
    Ok(channel)
}

/// The `channel = "..."` value of a toolchain file.
pub(crate) fn channel_of(manifest: &str) -> Option<String> {
    manifest.lines().map(str::trim).find_map(|line| {
        let rest = line
            .strip_prefix("channel")?
            .trim_start()
            .strip_prefix('=')?
            .trim();
        let inner = rest.strip_prefix('"')?;
        let end = inner.find('"')?;
        inner.get(..end).map(str::to_owned)
    })
}

/// Checks the active toolchain name against the pinned channel.
pub(crate) fn check_active(channel: &str, active: Option<&str>) -> Result<(), Error> {
    match active {
        Some(name) if name.starts_with(channel) => Ok(()),
        Some(name) => Err(Error::Toolchain(format!(
            "the active toolchain is `{name}` but the workspace pins `{channel}`; \
             run through rustup's Cargo proxy (`~/.cargo/bin/cargo xtask ...`)"
        ))),
        None => Err(Error::Toolchain(format!(
            "`{TOOLCHAIN_VARIABLE}` is not set, so Cargo was not started through rustup; \
             run `~/.cargo/bin/cargo xtask ...` so that the pin `{channel}` applies"
        ))),
    }
}
