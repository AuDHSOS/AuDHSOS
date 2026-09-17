// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generate/check the checked-in Unicode tables from local UCD files.
use text_core as _;
#[path = "../generator/mod.rs"]
mod generator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let model = generator::load(&root.join("../../docs/unicode/ucd"))?;
    let output = generator::generate(&model)?;
    let path = root.join("src/unicode/generated.rs");
    match std::env::args().nth(1).as_deref() {
        Some("--check") => {
            if std::fs::read_to_string(path)? != output {
                return Err("Unicode tables differ; regenerate them".into());
            }
        }
        None => std::fs::write(path, output)?,
        _ => return Err("usage: unicode_gen [--check]".into()),
    }
    Ok(())
}
