// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What comes out of a page: the blocks, the runs inside them, and the
//! title.

use doc_markdown::{Align, Block, Inline, plain};

use crate::{parse, title};

fn said(block: Option<&Block>) -> String {
    match block {
        Some(Block::Paragraph(content) | Block::Heading { content, .. }) => plain(content),
        _ => String::new(),
    }
}

fn first(html: &str) -> Vec<Block> {
    parse(html)
}

fn levels(html: &str) -> Vec<usize> {
    parse(html)
        .iter()
        .filter_map(|block| match block {
            Block::Heading { level, .. } => Some(*level),
            _ => None,
        })
        .collect()
}

fn runs(html: &str) -> Vec<Inline> {
    match parse(html).into_iter().next() {
        Some(Block::Paragraph(content)) => content,
        _ => Vec::new(),
    }
}

#[test]
fn a_page_of_nothing_has_no_block() {
    assert!(parse("").is_empty());
    assert!(parse("   \n  ").is_empty());
}

#[test]
fn text_outside_any_element_is_still_a_paragraph() {
    assert_eq!(said(first("hello").first()), "hello");
}

#[test]
fn the_whitespace_of_the_markup_becomes_one_space() {
    let blocks = first("<p>one\n   <em>two</em>\n\n three  </p>");
    assert_eq!(said(blocks.first()), "one two three");
}

#[test]
fn a_paragraph_never_begins_or_ends_with_a_space() {
    let blocks = first("<p>\n  <em>one</em> two\n</p>");
    assert_eq!(said(blocks.first()), "one two");
}

#[test]
fn a_heading_keeps_its_own_number_where_no_section_moves_it() {
    assert_eq!(levels("<h1>a</h1><h3>b</h3>"), vec![1, 3]);
}

#[test]
fn a_heading_counts_the_sections_around_it() {
    let html =
        "<section><h1>a</h1><section><h1>b</h1><section><h1>c</h1></section></section></section>";
    assert_eq!(levels(html), vec![1, 2, 3]);
}

#[test]
fn a_heading_never_goes_deeper_than_six() {
    let html = "<section><section><section><section><section><section><h3>deep</h3></section></section></section></section></section></section>";
    assert_eq!(levels(html), vec![6]);
}

#[test]
fn a_heading_with_nothing_in_it_is_not_a_heading() {
    assert!(levels("<h1>  </h1>").is_empty());
}

#[test]
fn a_list_holds_its_items() {
    let blocks = first("<ul><li>one</li><li>two</li></ul>");
    match blocks.first() {
        Some(Block::List {
            ordered,
            start,
            tight,
            items,
        }) => {
            assert!(!ordered);
            assert_eq!(*start, 1);
            assert!(tight);
            assert_eq!(items.len(), 2);
            assert_eq!(said(items.first().and_then(|item| item.first())), "one");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_ordered_list_begins_where_it_says() {
    let blocks = first("<ol start=\"3\"><li>three</li></ol>");
    match blocks.first() {
        Some(Block::List { ordered, start, .. }) => {
            assert!(ordered);
            assert_eq!(*start, 3);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_item_written_as_a_paragraph_makes_the_list_a_loose_one() {
    let blocks = first("<ul><li><p>one</p></li><li><p>two</p></li></ul>");
    match blocks.first() {
        Some(Block::List { tight, .. }) => assert!(!tight),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_item_holds_whatever_was_written_in_it() {
    let blocks = first("<ol><li>step<ol><li>substep</li></ol></li></ol>");
    let Some(Block::List { items, .. }) = blocks.first() else {
        panic!("no list");
    };
    let inner = items.first().and_then(|item| item.get(1));
    assert!(matches!(inner, Some(Block::List { .. })), "{inner:?}");
}

#[test]
fn a_list_of_no_items_is_no_list() {
    assert!(parse("<ul>  </ul>").is_empty());
}

#[test]
fn a_table_takes_its_header_from_the_head() {
    let html = "<table><thead><tr><td>one</td><td>two</td></tr></thead><tbody><tr><td>a</td><td>b</td></tr></tbody></table>";
    match first(html).first() {
        Some(Block::Table {
            alignments,
            header,
            rows,
        }) => {
            assert_eq!(alignments, &vec![Align::Left, Align::Left]);
            assert_eq!(
                plain(header.first().map(Vec::as_slice).unwrap_or_default()),
                "one"
            );
            assert_eq!(rows.len(), 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_row_of_headings_is_a_header_wherever_it_stands() {
    let html = "<table><tr><th>one</th></tr><tr><td>a</td></tr></table>";
    match first(html).first() {
        Some(Block::Table { header, rows, .. }) => {
            assert_eq!(
                plain(header.first().map(Vec::as_slice).unwrap_or_default()),
                "one"
            );
            assert_eq!(rows.len(), 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_table_that_names_no_header_gives_up_its_first_row() {
    let html = "<table><tr><td>one</td></tr><tr><td>two</td></tr></table>";
    match first(html).first() {
        Some(Block::Table { header, rows, .. }) => {
            assert_eq!(
                plain(header.first().map(Vec::as_slice).unwrap_or_default()),
                "one"
            );
            assert_eq!(rows.len(), 1);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_table_of_no_rows_is_no_table() {
    assert!(parse("<table><tbody></tbody></table>").is_empty());
}

#[test]
fn a_header_cell_says_how_its_column_is_set() {
    let html = "<table><tr><th align=right>a</th><th style=\"text-align: center\">b</th><th align=\"left\">c</th><th style=\"color:red\">d</th></tr><tr><td>1</td></tr></table>";
    match first(html).first() {
        Some(Block::Table { alignments, .. }) => assert_eq!(
            alignments,
            &vec![Align::Right, Align::Center, Align::Left, Align::Left]
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_cell_is_read_for_its_text_whatever_is_written_in_it() {
    let html = "<table><tr><th>head</th></tr><tr><td><p>one</p><p>two</p></td></tr></table>";
    match first(html).first() {
        Some(Block::Table { rows, .. }) => {
            let cell = rows.first().and_then(|row| row.first());
            assert_eq!(
                plain(cell.map(Vec::as_slice).unwrap_or_default()),
                "one two"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn preformatted_text_is_kept_line_for_line() {
    let blocks =
        first("<pre><code class=\"language-rust\">fn main() {\n    ok();\n}\n</code></pre>");
    match blocks.first() {
        Some(Block::Code { language, lines }) => {
            assert_eq!(language.as_deref(), Some("rust"));
            assert_eq!(lines, &vec!["fn main() {", "    ok();", "}"]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_class_that_names_no_language_leaves_the_block_without_one() {
    let blocks = first("<pre class=\"quiet\">text</pre>");
    match blocks.first() {
        Some(Block::Code { language, .. }) => assert_eq!(language.as_deref(), None),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_empty_preformatted_block_is_dropped() {
    assert!(parse("<pre>\n\n</pre>").is_empty());
}

#[test]
fn a_grammar_gets_a_line_for_every_production_and_every_side() {
    let html = "<emu-grammar><emu-production><emu-nt>Digits</emu-nt> <emu-geq>::</emu-geq> <emu-rhs><emu-t>0</emu-t></emu-rhs><emu-rhs><emu-t>1</emu-t></emu-rhs></emu-production></emu-grammar>";
    match first(html).first() {
        Some(Block::Code { lines, .. }) => {
            assert_eq!(lines, &vec!["Digits ::", "  0", "  1"]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_break_inside_preformatted_text_begins_a_line() {
    match first("<pre>one<br>two</pre>").first() {
        Some(Block::Code { lines, .. }) => assert_eq!(lines, &vec!["one", "two"]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_quotation_holds_blocks_of_its_own() {
    let blocks = first("<blockquote><p>one</p><p>two</p></blockquote>");
    match blocks.first() {
        Some(Block::Quote(inner)) => assert_eq!(inner.len(), 2),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_note_is_read_as_a_quotation_and_an_empty_one_is_dropped() {
    assert!(matches!(
        first("<emu-note><p>note</p></emu-note>").first(),
        Some(Block::Quote(_))
    ));
    assert!(parse("<emu-note>  </emu-note>").is_empty());
}

#[test]
fn a_term_stands_in_bold_above_what_it_describes() {
    let blocks = first("<dl><dt>term</dt><dd>meaning</dd></dl>");
    assert!(
        matches!(blocks.first(), Some(Block::Paragraph(content)) if matches!(content.first(), Some(Inline::Strong(_)))),
        "{blocks:?}"
    );
    assert!(matches!(blocks.get(1), Some(Block::Quote(_))));
}

#[test]
fn a_term_of_nothing_is_no_term() {
    assert!(parse("<dl><dt> </dt></dl>").is_empty());
}

#[test]
fn a_caption_is_set_apart_from_the_prose() {
    let blocks = first("<figure><figcaption>Figure 1: a picture</figcaption></figure>");
    assert!(
        matches!(blocks.first(), Some(Block::Paragraph(content)) if matches!(content.first(), Some(Inline::Emphasis(_)))),
        "{blocks:?}"
    );
    assert!(parse("<figcaption> </figcaption>").is_empty());
}

#[test]
fn a_rule_is_a_rule() {
    assert!(matches!(first("<hr>").first(), Some(Block::Rule)));
}

#[test]
fn what_carries_no_prose_is_dropped_with_everything_in_it() {
    assert!(parse("<script>let a = '<p>x</p>';</script><style>p{}</style>").is_empty());
}

#[test]
fn an_element_nobody_named_is_read_for_its_children() {
    assert_eq!(said(first("<custom>text</custom>").first()), "text");
    let blocks = first("<custom><p>one</p><p>two</p></custom>");
    assert_eq!(blocks.len(), 2);
}

#[test]
fn a_face_is_kept_and_a_block_inside_a_line_is_not() {
    assert_eq!(
        runs("<p><em>a</em> <strong>b</strong> <code>c</code> <var>d</var></p>"),
        vec![
            Inline::Emphasis(vec![Inline::Text("a".to_owned())]),
            Inline::Text(" ".to_owned()),
            Inline::Strong(vec![Inline::Text("b".to_owned())]),
            Inline::Text(" ".to_owned()),
            Inline::Code("c".to_owned()),
            Inline::Text(" ".to_owned()),
            Inline::Emphasis(vec![Inline::Text("d".to_owned())]),
        ]
    );
    assert_eq!(said(first("<em><p>one</p> two</em>").first()), "one two");
}

#[test]
fn an_empty_face_says_nothing() {
    assert!(parse("<p><em> </em></p>").is_empty());
}

#[test]
fn a_line_break_is_kept() {
    assert_eq!(
        runs("<p>one<br>two</p>"),
        vec![
            Inline::Text("one".to_owned()),
            Inline::Break,
            Inline::Text("two".to_owned()),
        ]
    );
}

#[test]
fn a_link_out_of_the_document_keeps_its_address() {
    assert_eq!(
        runs("<p><a href=\"https://example.test/x\">there</a></p>"),
        vec![Inline::Link {
            content: vec![Inline::Text("there".to_owned())],
            href: "https://example.test/x".to_owned(),
        }]
    );
}

#[test]
fn a_link_into_the_document_keeps_only_its_text() {
    assert_eq!(
        runs("<p><a href=\"#sec-one\">6.1.4</a></p>"),
        vec![Inline::Text("6.1.4".to_owned())]
    );
    assert_eq!(
        runs("<p><a>bare</a> <a href=\"\">empty</a></p>"),
        vec![Inline::Text("bare empty".to_owned())]
    );
}

#[test]
fn an_image_carries_the_file_it_is_in_and_what_it_shows() {
    assert_eq!(
        runs("<p><img src=\"img/one.svg\" alt=\"A graph\"></p>"),
        vec![Inline::Image {
            source: "img/one.svg".to_owned(),
            alt: "A graph".to_owned(),
        }]
    );
    assert_eq!(
        runs("<p>before <img src=\"img/two.svg\"> after</p>")
            .first()
            .cloned(),
        Some(Inline::Text("before ".to_owned()))
    );
    assert_eq!(
        said(first("<p><img alt=\"nowhere\"></p>").first()),
        "[image: nowhere]"
    );
}

#[test]
fn the_title_is_the_one_the_page_gives_itself() {
    assert_eq!(
        title("<html><head><TITLE>A  page\n  title</TITLE></head></html>").as_deref(),
        Some("A page title")
    );
    assert_eq!(title("<html><head></head></html>"), None);
    assert_eq!(title("<title>  </title>"), None);
    assert_eq!(title("<titlebar>x</titlebar>"), None);
    assert_eq!(
        title("<title>unterminated").as_deref(),
        Some("unterminated")
    );
}

#[test]
fn an_element_that_says_it_belongs_in_a_line_is_read_in_one() {
    let blocks = first("<p>from <emu-eqn class=\"inline\">2^31</emu-eqn> to zero</p>");
    assert_eq!(blocks.len(), 1);
    assert_eq!(said(blocks.first()), "from 2^31 to zero");
}

#[test]
fn an_equation_of_its_own_stands_on_its_own() {
    assert!(matches!(
        first("<emu-eqn>x = y</emu-eqn>").first(),
        Some(Block::Code { .. })
    ));
}
