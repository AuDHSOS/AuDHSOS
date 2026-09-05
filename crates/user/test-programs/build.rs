// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Points the linker at the script that puts a program where the test
//! kernels map it.

fn main() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("user.ld");
    println!("cargo:rustc-link-arg-bins=-T{}", script.display());
    println!("cargo:rerun-if-changed=user.ld");
}
