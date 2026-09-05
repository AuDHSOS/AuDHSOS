// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The block parser.

use test_support::generators::{one_of, vec as gen_vec};
use test_support::property::check;

use crate::block::{Align, Block};
use crate::inline::{Inline, plain};
use crate::parse::document;

fn text(block: &Block) -> String {
    match block {
        Block::Paragraph(content) | Block::Heading { content, .. } => plain(content),
        Block::Code { lines, .. } => lines.join("\n"),
        _ => String::new(),
    }
}

/// The items of a list, or nothing when the block is not one. A test that
/// asks for the wrong kind of block gets an empty answer and fails on the
/// assertion it made, rather than on a panic of its own.
fn items(block: Option<&Block>) -> Vec<Vec<Block>> {
    match block {
        Some(Block::List { items, .. }) => items.clone(),
        _ => Vec::new(),
    }
}

/// The blocks inside a quotation.
fn quoted(block: Option<&Block>) -> Vec<Block> {
    match block {
        Some(Block::Quote(inner)) => inner.clone(),
        _ => Vec::new(),
    }
}

#[test]
fn an_empty_document_has_no_block() {
    assert!(document("").is_empty());
    assert!(document("\n\n   \n").is_empty());
}

#[test]
fn an_atx_heading_carries_its_level() {
    let blocks = document("# One\n\n### Three\n");
    assert_eq!(blocks.len(), 2);
    assert!(matches!(
        blocks.first(),
        Some(Block::Heading { level: 1, .. })
    ));
    assert!(matches!(
        blocks.get(1),
        Some(Block::Heading { level: 3, .. })
    ));
    assert_eq!(
        blocks.first().and_then(Block::heading_text),
        Some("One".to_owned())
    );
}

#[test]
fn a_hash_without_a_space_is_not_a_heading() {
    assert!(matches!(
        document("#hashtag").first(),
        Some(Block::Paragraph(_))
    ));
    assert!(matches!(
        document("####### seven").first(),
        Some(Block::Paragraph(_))
    ));
}

#[test]
fn closing_hashes_are_dropped() {
    assert_eq!(
        document("## Middle ##")
            .first()
            .and_then(Block::heading_text),
        Some("Middle".to_owned())
    );
}

#[test]
fn a_setext_underline_makes_the_line_above_a_heading() {
    let blocks = document("Title\n=====\n\nOther\n-----\n");
    assert!(matches!(
        blocks.first(),
        Some(Block::Heading { level: 1, .. })
    ));
    assert!(matches!(
        blocks.get(1),
        Some(Block::Heading { level: 2, .. })
    ));
}

#[test]
fn a_paragraph_joins_its_lines() {
    let blocks = document("one\ntwo\n\nthree\n");
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        text(&blocks.first().cloned().unwrap_or(Block::Rule)),
        "one two"
    );
}

#[test]
fn a_fence_keeps_its_lines_exactly() {
    let blocks = document("```rust\nlet x = 1;\n\n    indented\n```\n");
    assert_eq!(
        blocks.first().cloned(),
        Some(Block::Code {
            language: Some("rust".to_owned()),
            lines: vec![
                "let x = 1;".to_owned(),
                String::new(),
                "    indented".to_owned()
            ],
        })
    );
}

#[test]
fn a_fence_may_contain_the_other_fence_character() {
    let blocks = document("~~~\n```\n~~~\n");
    assert_eq!(text(&blocks.first().cloned().unwrap_or(Block::Rule)), "```");
}

#[test]
fn a_fence_that_never_closes_ends_with_the_document() {
    let blocks = document("```\nstill code\n");
    assert!(matches!(blocks.first(), Some(Block::Code { .. })));
    assert_eq!(blocks.len(), 1);
}

#[test]
fn four_spaces_make_a_code_block() {
    let blocks = document("text\n\n    code here\n    more\n\ntext\n");
    assert!(matches!(
        blocks.get(1),
        Some(Block::Code { language: None, .. })
    ));
    assert_eq!(blocks.len(), 3);
}

#[test]
fn a_rule_is_not_a_list_and_not_a_heading() {
    let blocks = document("---\n\n***\n\n___\n");
    assert_eq!(blocks, vec![Block::Rule, Block::Rule, Block::Rule]);
}

#[test]
fn a_quotation_holds_blocks_of_its_own() {
    let blocks = document("> # inside\n> and a paragraph\n> that runs on\n");
    let inner = quoted(blocks.first());
    assert!(matches!(
        inner.first(),
        Some(Block::Heading { level: 1, .. })
    ));
    assert_eq!(
        text(&inner.get(1).cloned().unwrap_or(Block::Rule)),
        "and a paragraph that runs on"
    );
}

#[test]
fn a_bullet_list_collects_its_items() {
    let blocks = document("- one\n- two\n- three\n");
    assert!(matches!(
        blocks.first(),
        Some(Block::List { ordered: false, .. })
    ));
    let items = items(blocks.first());
    assert_eq!(items.len(), 3);
    assert_eq!(
        items.get(2).and_then(|item| item.first()).map(text),
        Some("three".to_owned())
    );
}

#[test]
fn a_numbered_list_keeps_the_number_it_starts_at() {
    let blocks = document("3. third\n4. fourth\n");
    assert!(matches!(
        blocks.first(),
        Some(Block::List {
            ordered: true,
            start: 3,
            ..
        })
    ));
    assert_eq!(items(blocks.first()).len(), 2);
}

#[test]
fn an_item_may_hold_a_list_of_its_own() {
    let blocks = document("- outer\n  - inner one\n  - inner two\n- second\n");
    let items = items(blocks.first());
    assert_eq!(items.len(), 2);
    let first = items.first().cloned().unwrap_or_default();
    assert!(matches!(first.get(1), Some(Block::List { .. })));
}

#[test]
fn an_item_may_hold_a_code_block() {
    let blocks = document("- item\n\n  ```\n  code\n  ```\n");
    let first = items(blocks.first()).first().cloned().unwrap_or_default();
    assert!(
        first
            .iter()
            .any(|block| matches!(block, Block::Code { .. }))
    );
}

#[test]
fn a_blank_line_between_items_does_not_end_the_list() {
    assert_eq!(items(document("- one\n\n- two\n").first()).len(), 2);
}

#[test]
fn a_list_written_without_blank_lines_is_a_tight_one() {
    assert!(matches!(
        document("- one\n- two\n").first(),
        Some(Block::List { tight: true, .. })
    ));
}

#[test]
fn a_blank_line_inside_a_list_makes_it_loose() {
    assert!(matches!(
        document("- one\n\n- two\n").first(),
        Some(Block::List { tight: false, .. })
    ));
    assert!(matches!(
        document("- item\n\n  more of the item\n").first(),
        Some(Block::List { tight: false, .. })
    ));
}

#[test]
fn a_blank_line_after_a_list_does_not_make_it_loose() {
    assert!(matches!(
        document("- one\n- two\n\nA paragraph.\n").first(),
        Some(Block::List { tight: true, .. })
    ));
}

#[test]
fn a_table_reads_its_alignments_and_its_rows() {
    let blocks = document("| a | b | c |\n| :-- | :-: | --: |\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n");
    let (alignments, header, rows) = match blocks.first() {
        Some(Block::Table {
            alignments,
            header,
            rows,
        }) => (alignments.clone(), header.clone(), rows.clone()),
        _ => (Vec::new(), Vec::new(), Vec::new()),
    };
    assert_eq!(alignments, [Align::Left, Align::Center, Align::Right]);
    assert_eq!(header.len(), 3);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.first()
            .and_then(|row| row.get(2))
            .map(|cell| plain(cell)),
        Some("3".to_owned())
    );
}

#[test]
fn a_row_shorter_than_the_header_is_padded() {
    let blocks = document("| a | b |\n| --- | --- |\n| 1 |\n");
    let rows = match blocks.first() {
        Some(Block::Table { rows, .. }) => rows.clone(),
        _ => Vec::new(),
    };
    assert_eq!(rows.first().map(Vec::len), Some(2));
}

#[test]
fn a_line_with_a_pipe_but_no_delimiter_row_stays_a_paragraph() {
    assert!(matches!(
        document("a | b\nc | d\n").first(),
        Some(Block::Paragraph(_))
    ));
}

#[test]
fn a_heading_ends_the_paragraph_before_it() {
    let blocks = document("paragraph\n# heading\n");
    assert_eq!(blocks.len(), 2);
    assert!(matches!(blocks.first(), Some(Block::Paragraph(_))));
    assert!(matches!(blocks.get(1), Some(Block::Heading { .. })));
}

#[test]
fn a_tab_indents_like_four_spaces() {
    assert!(matches!(
        document("text\n\n\tcode\n").get(1),
        Some(Block::Code { .. })
    ));
}

#[test]
fn an_inline_run_survives_the_block_that_holds_it() {
    let blocks = document("A **strong** word.");
    let content = match blocks.first() {
        Some(Block::Paragraph(content)) => content.clone(),
        _ => Vec::new(),
    };
    assert!(content.iter().any(|item| matches!(item, Inline::Strong(_))));
}

#[test]
fn the_parser_never_invents_text() {
    // Every mark the parser understands either disappears into the
    // structure or stays the character it was, so the text of a parsed
    // document is never longer than the source it came from. The
    // exclamation mark is left out of the alphabet: an image is the one
    // construction that writes something the source did not say, namely
    // the description of what it could not draw.
    let alphabet = vec![
        '#', '*', '_', '`', '[', ']', '(', ')', '-', '|', '>', '.', ' ', '\n', 'a', 'b', '1',
    ];
    check(
        "the parser invents no text",
        &gen_vec(one_of(alphabet), 0..=64),
        |characters: &Vec<char>| {
            let source: String = characters.iter().collect();
            let parsed = document(&source);
            let produced = length(&parsed);
            if produced <= source.chars().count() {
                Ok(())
            } else {
                Err(format!(
                    "`{source}` became {produced} characters of text, from {}",
                    source.chars().count()
                ))
            }
        },
    );
}

/// The characters of every text a document holds.
fn length(blocks: &[Block]) -> usize {
    let mut total = 0usize;
    for block in blocks {
        let count = match block {
            Block::Paragraph(content) | Block::Heading { content, .. } => {
                plain(content).chars().count()
            }
            Block::Code { lines, .. } => lines.iter().map(|line| line.chars().count()).sum(),
            Block::Quote(inner) => length(inner),
            Block::List { items, .. } => items.iter().map(|item| length(item)).sum(),
            Block::Table { header, rows, .. } => {
                let head: usize = header.iter().map(|cell| plain(cell).chars().count()).sum();
                let body: usize = rows
                    .iter()
                    .flat_map(|row| row.iter())
                    .map(|cell| plain(cell).chars().count())
                    .sum();
                head.saturating_add(body)
            }
            Block::Rule => 0,
        };
        total = total.saturating_add(count);
    }
    total
}

#[test]
fn a_tab_is_expanded_in_the_indent_and_left_alone_in_the_text() {
    // Only the indent is measured in columns, so only there does a tab
    // have to become spaces. Inside a line it is a character of the text,
    // and whoever sets the text decides what it looks like.
    let blocks = document("a\tb\n");
    assert_eq!(
        text(&blocks.first().cloned().unwrap_or(Block::Rule)),
        "a\tb"
    );
}

#[test]
fn an_escaped_pipe_does_not_split_a_cell() {
    let blocks = document("| a | b |\n| --- | --- |\n| x \\| y | z |\n");
    let rows = match blocks.first() {
        Some(Block::Table { rows, .. }) => rows.clone(),
        _ => Vec::new(),
    };
    assert_eq!(rows.first().map(Vec::len), Some(2));
    assert_eq!(
        rows.first()
            .and_then(|row| row.first())
            .map(|cell| plain(cell)),
        Some("x | y".to_owned())
    );
}

#[test]
fn a_list_that_changes_its_kind_becomes_two_lists() {
    let blocks = document("- one\n- two\n1. first\n");
    assert_eq!(blocks.len(), 2);
    assert!(matches!(
        blocks.first(),
        Some(Block::List { ordered: false, .. })
    ));
    assert!(matches!(
        blocks.get(1),
        Some(Block::List { ordered: true, .. })
    ));
}

#[test]
fn two_blank_lines_end_a_list() {
    let blocks = document("- one\n\n\nnot an item\n");
    assert_eq!(items(blocks.first()).len(), 1);
    assert!(matches!(blocks.get(1), Some(Block::Paragraph(_))));
}

#[test]
fn a_rule_is_not_a_list_marker() {
    assert_eq!(document("- one\n---\n").len(), 2);
}

#[test]
fn a_quotation_ends_where_a_block_of_its_own_begins() {
    let blocks = document("> quoted\n# heading\n");
    assert!(matches!(blocks.first(), Some(Block::Quote(_))));
    assert!(matches!(blocks.get(1), Some(Block::Heading { .. })));
}

#[test]
fn a_fence_with_a_language_that_carries_braces_keeps_only_the_word() {
    let blocks = document("```{rust}\ncode\n```\n");
    assert!(matches!(
        blocks.first(),
        Some(Block::Code { language: Some(language), .. }) if language == "rust"
    ));
}

#[test]
fn a_line_of_backticks_inside_a_fence_does_not_close_it_early() {
    let blocks = document("````\n```\nstill inside\n````\n");
    assert_eq!(
        text(&blocks.first().cloned().unwrap_or(Block::Rule)),
        "```\nstill inside"
    );
}

#[test]
fn only_a_heading_has_the_text_of_a_heading() {
    assert_eq!(Block::Rule.heading_text(), None);
    assert_eq!(document("text").first().and_then(Block::heading_text), None);
}
