// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Points the linker at the script each program needs.
//!
//! The root task is linked at the address the kernel maps it to, with
//! every section on a page of its own; everything else is an executable in
//! the archive, linked where the loader is happy to put it.

fn main() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = here.join("root.ld");
    let program = here.join("program.ld");
    println!("cargo:rustc-link-arg-bin=server-init=-T{}", root.display());
    for name in [
        "server-memory",
        "server-name",
        "server-console",
        "app-hello",
    ] {
        println!("cargo:rustc-link-arg-bin={name}=-T{}", program.display());
    }
    println!("cargo:rerun-if-changed=root.ld");
    println!("cargo:rerun-if-changed=program.ld");
}
