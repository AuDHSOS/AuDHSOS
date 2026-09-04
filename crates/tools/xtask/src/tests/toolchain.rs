// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::toolchain`.

use crate::toolchain::{channel_of, check_active};

#[test]
fn channel_is_extracted_from_the_toolchain_file() {
    let manifest =
        "# header\n[toolchain]\nchannel = \"nightly-2026-08-25\"\nprofile = \"minimal\"\n";
    assert_eq!(channel_of(manifest).as_deref(), Some("nightly-2026-08-25"));
    assert_eq!(
        channel_of("  channel=\"stable\"  "),
        Some("stable".to_owned())
    );
    assert_eq!(channel_of("[toolchain]\nprofile = \"minimal\"\n"), None);
    assert_eq!(channel_of("channel = stable"), None);
    assert_eq!(channel_of("channel = \"unterminated"), None);
}

#[test]
fn active_toolchain_must_start_with_the_channel() {
    assert!(
        check_active(
            "nightly-2026-08-25",
            Some("nightly-2026-08-25-aarch64-apple-darwin")
        )
        .is_ok()
    );
    assert!(check_active("nightly-2026-08-25", Some("stable-aarch64-apple-darwin")).is_err());
    assert!(check_active("nightly-2026-08-25", None).is_err());
}
