// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Points the linker at the kernel's own script, for the image and for
//! every test kernel.

fn main() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("kernel.ld");
    println!("cargo:rustc-link-arg-bins=-T{}", script.display());
    println!("cargo:rustc-link-arg-tests=-T{}", script.display());
    println!("cargo:rerun-if-changed=kernel.ld");
}
