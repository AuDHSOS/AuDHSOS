// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::linker`.

use crate::linker::{KERNEL_SCRIPT, check, constant};

#[test]
fn a_constant_is_read_in_either_base() {
    let script = "ENTRY(kernel_entry)\nKERNEL_BASE = 0xFFFFFFFF80000000;\nSTACK = 4096;\n";
    assert_eq!(constant(script, "KERNEL_BASE"), Some(0xFFFF_FFFF_8000_0000));
    assert_eq!(constant(script, "STACK"), Some(4096));
    assert_eq!(constant(script, "MISSING"), None);
}

#[test]
fn a_line_that_is_not_an_assignment_is_skipped() {
    let script =
        "/* KERNEL_BASE in a comment */\nKERNEL_BASE\nKERNEL_BASE = ;\nKERNEL_BASE = 0x10;\n";
    assert_eq!(constant(script, "KERNEL_BASE"), Some(0x10));
    assert_eq!(constant("", "KERNEL_BASE"), None);
    assert_eq!(constant("KERNEL_BASE = zzz;", "KERNEL_BASE"), None);
}

#[test]
fn the_kernel_script_agrees_with_the_abi() {
    let root = crate::workspace_root().unwrap();
    assert!(root.join(KERNEL_SCRIPT).is_file());
    assert_eq!(check(&root).unwrap(), Vec::<String>::new());
}
