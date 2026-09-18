// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Golden images: the exact bytes of a surface, checked in as a test vector.
//!
//! D-177 makes them a gate. The crate has no floating point in any product
//! path, so one input gives one set of bytes on every host, in debug and in
//! release. A change to a checked-in image is a change to the rendering and is
//! reviewed as one.
//!
//! The files are binary PPM, which a person can look at, and hold three bytes
//! per pixel. `AUDHSOS_GOLDEN=1 cargo test -p text-raster golden` rewrites
//! them; without it a missing file fails.

use std::{env, fs, path::PathBuf};

/// Where the images live, relative to the crate root.
fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/tests/golden")
        .join(format!("{name}.ppm"))
}

/// One `Rgbx8888` surface as binary PPM.
fn portable(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in bytes.as_chunks::<4>().0 {
        out.extend_from_slice(pixel.get(..3).unwrap_or(&[0, 0, 0]));
    }
    out
}

/// Compare one drawing against its checked-in image, or rewrite it.
pub(super) fn check(name: &str, bytes: &[u8], width: u32, height: u32) {
    let image = portable(bytes, width, height);
    let file = path(name);
    if env::var_os("AUDHSOS_GOLDEN").is_some() {
        fs::write(&file, &image).expect("write the golden image");
        return;
    }
    let stored = fs::read(&file).unwrap_or_else(|error| {
        panic!(
            "{}: {error}; run with AUDHSOS_GOLDEN=1 to write it",
            file.display()
        )
    });
    assert!(
        stored == image,
        "{} differs: the rendering changed, which is reviewed as such",
        file.display()
    );
}
