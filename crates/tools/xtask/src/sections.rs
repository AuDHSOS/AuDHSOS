// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The sizes of `.data` and `.bss` of the kernel image, against two bounds.
//!
//! A `static` whose value is all zeros is `.bss`: the section holds no bytes
//! in the file and the loader clears it. One nonzero byte in that value
//! moves the whole cell into `.data`, which the file carries and the loader
//! copies. The pools of the machine are 1.4 MiB, so the bound on `.data` is
//! what D-66 and D-185 are checked by rather than stated in a comment.

use audhsos_elf::ElfError;

/// The bound on `.data` of the kernel image. The statics that are not all
/// zeros add up to 11,016 bytes today, of which
/// `kernel_core::memory::MEMORY` is 10,472, because `Global<T>` holds an
/// `Option<T>` and takes its value by move
/// (`crates/sync/src/lib.rs:51-53`). The bound leaves room for another such
/// cell.
pub(crate) const DATA_MAX: u64 = 64 * 1024;

/// The lower bound on `.bss` of the kernel image: the pools of the machine
/// are 1.4 MiB and belong there.
pub(crate) const BSS_MIN: u64 = 1024 * 1024;

/// What the section table says about the two sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sizes {
    /// Bytes of `.data`, zero when the section is absent.
    pub(crate) data: u64,
    /// Bytes of `.bss`, zero when the section is absent.
    pub(crate) bss: u64,
}

/// The sizes the section table of `bytes` gives.
///
/// # Errors
///
/// The errors of `audhsos_elf::sections()`.
pub(crate) fn sizes(bytes: &[u8]) -> Result<Sizes, ElfError> {
    let table = audhsos_elf::sections(bytes)?;
    let size_of = |name: &str| table.by_name(name).map_or(0, |section| section.size);
    Ok(Sizes {
        data: size_of(".data"),
        bss: size_of(".bss"),
    })
}

/// The bounds `sizes` violates.
pub(crate) fn violations(sizes: Sizes) -> Vec<String> {
    let mut violations = Vec::new();
    if sizes.data > DATA_MAX {
        violations.push(format!(
            ".data is {} bytes, above {DATA_MAX}: a `static` of the kernel is not all zeros",
            sizes.data
        ));
    }
    if sizes.bss < BSS_MIN {
        violations.push(format!(
            ".bss is {} bytes, below {BSS_MIN}: the pools of the machine are not there",
            sizes.bss
        ));
    }
    violations
}
