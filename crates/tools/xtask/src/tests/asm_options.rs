// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::asm_options`.
//!
//! Every block under test is built by [`block`], so that no line of this
//! file carries an `asm!` invocation and the word the rule looks for at
//! once, which would make this file a violation of its own rule.

use crate::asm_options::problems;

/// The option the rule looks for.
const NOMEM: &str = "nomem";

/// One assembly block as source.
fn block(template: &str, options: &str) -> String {
    format!("asm!(\"{template}\", options({options}));")
}

#[test]
fn a_cli_block_with_the_option_is_a_violation() {
    let found = problems(&block("cli", &format!("{NOMEM}, nostack")));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found.first().is_some_and(|line| line.contains("`cli`")),
        "{found:?}"
    );
}

#[test]
fn an_sti_block_with_the_option_is_a_violation() {
    let found = problems(&block("sti", &format!("{NOMEM}, nostack")));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found.first().is_some_and(|line| line.contains("`sti`")),
        "{found:?}"
    );
}

#[test]
fn a_cli_block_without_the_option_passes() {
    let found = problems(&block("cli", "nostack, preserves_flags"));
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn nostack_is_not_an_sti() {
    let found = problems(&block("hlt", &format!("{NOMEM}, nostack")));
    assert!(found.is_empty(), "`nostack` ends in the letters of `sti`");
}

#[test]
fn a_longer_word_that_starts_with_cli_passes() {
    let found = problems(&block("client", &format!("{NOMEM}, nostack")));
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn the_reported_line_is_the_one_of_the_block() {
    let source = format!("fn f() {{\n    {}\n}}\n", block("cli", NOMEM));
    let found = problems(&source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found.first().is_some_and(|line| line.contains("line 2")),
        "{found:?}"
    );
}

#[test]
fn a_naked_block_is_read_as_well() {
    let found = problems(&format!("naked_{}", block("cli", NOMEM)));
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn a_block_written_over_several_lines_is_read_whole() {
    let source = format!("asm!(\n    \"cli\",\n    options({NOMEM}, nostack),\n);\n");
    let found = problems(&source);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found.first().is_some_and(|line| line.contains("line 1")),
        "{found:?}"
    );
}

#[test]
fn a_parenthesis_inside_a_template_does_not_end_the_block() {
    let source = format!("asm!(\"cli /* ( */\", options({NOMEM}));\nasm!(\"nop\");\n");
    let found = problems(&source);
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn two_blocks_are_reported_once_each() {
    let source = format!("{}\n{}\n", block("cli", NOMEM), block("sti", NOMEM));
    let found = problems(&source);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn an_unterminated_block_does_not_hang() {
    let source = format!("asm!(\"cli\", options({NOMEM}");
    assert_eq!(problems(&source).len(), 1);
}

#[test]
fn the_option_of_a_later_block_does_not_reach_an_earlier_one() {
    let source = format!("asm!(\"cli\", options(nostack));\nasm!(\"nop\", options({NOMEM}));\n");
    assert!(problems(&source).is_empty(), "the bodies are separate");
}
