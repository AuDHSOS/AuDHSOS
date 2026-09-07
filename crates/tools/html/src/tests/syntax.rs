// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Character references, the tokenizer, and the tree it builds.

use crate::entity::decode;
use crate::token::{Token, tokens};
use crate::tree::{Element, Node, parse};

fn start(name: &str, attributes: &[(&str, &str)]) -> Token {
    Token::Start {
        name: name.to_owned(),
        attributes: attributes
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        closed: false,
    }
}

fn text(text: &str) -> Token {
    Token::Text {
        text: text.to_owned(),
    }
}

fn end(name: &str) -> Token {
    Token::End {
        name: name.to_owned(),
    }
}

fn element(node: &Node) -> Option<&Element> {
    match node {
        Node::Element(element) => Some(element),
        Node::Text(_) => None,
    }
}

fn names(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(element)
        .map(|element| element.name.clone())
        .collect()
}

#[test]
fn text_without_a_reference_is_itself() {
    assert_eq!(decode("plain text"), "plain text");
}

#[test]
fn a_named_reference_becomes_its_character() {
    assert_eq!(decode("a &amp; b &mdash; c"), "a & b — c");
}

#[test]
fn a_reference_this_table_does_not_know_stays_as_it_was() {
    assert_eq!(decode("&curlyvee; and &nothing"), "&curlyvee; and &nothing");
}

#[test]
fn a_numeric_reference_is_read_in_both_bases() {
    assert_eq!(decode("&#65;&#x42;&#X43;"), "ABC");
}

#[test]
fn a_numeric_reference_that_names_no_character_stays_as_it_was() {
    assert_eq!(
        decode("&#xD800; &#; &#99999999;"),
        "&#xD800; &#; &#99999999;"
    );
}

#[test]
fn the_legacy_names_need_no_semicolon() {
    assert_eq!(decode("&ampquot; &lt x"), "&quot; < x");
}

#[test]
fn an_ampersand_at_the_end_is_text() {
    assert_eq!(decode("end &"), "end &");
}

#[test]
fn text_between_tags_comes_out_as_text() {
    assert_eq!(
        tokens("<p>one</p>"),
        vec![start("p", &[]), text("one"), end("p")]
    );
}

#[test]
fn an_attribute_is_read_quoted_bare_or_alone() {
    assert_eq!(
        tokens("<a href=\"x y\" id=z hidden>"),
        vec![start("a", &[("href", "x y"), ("id", "z"), ("hidden", "")])]
    );
}

#[test]
fn an_attribute_may_be_written_in_single_quotes() {
    assert_eq!(tokens("<a id='q'>"), vec![start("a", &[("id", "q")])]);
}

#[test]
fn a_name_is_lower_cased_and_a_value_is_not() {
    assert_eq!(
        tokens("<A HREF=Value>"),
        vec![start("a", &[("href", "Value")])]
    );
}

#[test]
fn a_tag_that_closes_itself_says_so() {
    assert_eq!(
        tokens("<br />"),
        vec![Token::Start {
            name: "br".to_owned(),
            attributes: Vec::new(),
            closed: true,
        }]
    );
}

#[test]
fn a_comment_a_doctype_and_an_instruction_are_read_and_dropped() {
    assert_eq!(
        tokens("<!doctype html><!-- note --><?pi ?><p>x"),
        vec![start("p", &[]), text("x")]
    );
}

#[test]
fn an_unterminated_comment_ends_the_document() {
    assert_eq!(tokens("<p><!-- forever"), vec![start("p", &[])]);
}

#[test]
fn the_content_of_a_script_or_a_style_is_text_and_not_markup() {
    assert_eq!(
        tokens("<script>if (a<b) {}</script><style>p{}</style>after"),
        vec![
            start("script", &[]),
            text("if (a<b) {}"),
            end("script"),
            start("style", &[]),
            text("p{}"),
            end("style"),
            text("after"),
        ]
    );
}

#[test]
fn nothing_in_a_script_or_a_style_is_a_character_reference() {
    assert_eq!(
        tokens("<style>a{content:'&amp;'}</style>"),
        vec![
            start("style", &[]),
            text("a{content:'&amp;'}"),
            end("style"),
        ]
    );
}

#[test]
fn a_script_that_is_never_closed_ends_the_document() {
    assert_eq!(
        tokens("<script>x"),
        vec![start("script", &[]), text("x"), end("script")]
    );
}

#[test]
fn a_less_than_that_begins_nothing_is_text() {
    assert_eq!(tokens("a < b"), vec![text("a "), text("<"), text(" b")]);
}

#[test]
fn an_unterminated_tag_is_still_a_tag() {
    assert_eq!(tokens("<p class=x"), vec![start("p", &[("class", "x")])]);
    assert_eq!(tokens("</p"), vec![end("p")]);
}

#[test]
fn a_reference_inside_an_attribute_is_decoded() {
    assert_eq!(
        tokens("<a title=\"a &amp; b\">"),
        vec![start("a", &[("title", "a & b")])]
    );
}

#[test]
fn an_element_holds_what_is_inside_it() {
    let nodes = parse("<div><p>one</p></div>");
    let div = nodes.first().and_then(element);
    assert_eq!(names(&nodes), vec!["div"]);
    assert_eq!(
        div.map(|div| names(&div.children)).unwrap_or_default(),
        vec!["p"]
    );
}

#[test]
fn an_element_left_open_is_closed_at_the_end() {
    let nodes = parse("<div><p>text");
    assert_eq!(names(&nodes), vec!["div"]);
}

#[test]
fn an_end_tag_that_names_nothing_open_is_dropped() {
    let nodes = parse("</span><p>one</p>");
    assert_eq!(names(&nodes), vec!["p"]);
}

#[test]
fn an_end_tag_closes_what_is_still_open_inside_it() {
    let nodes = parse("<div><em>text</div><p>next</p>");
    assert_eq!(names(&nodes), vec!["div", "p"]);
}

#[test]
fn a_block_ends_an_open_paragraph() {
    let nodes = parse("<p>one<p>two<div>three</div>");
    assert_eq!(names(&nodes), vec!["p", "p", "div"]);
}

#[test]
fn an_item_ends_the_item_before_it_but_not_the_one_around_it() {
    let nodes = parse("<ul><li>one<li>two<ul><li>inner</ul></ul>");
    let outer = nodes.first().and_then(element);
    let items = outer.map(|list| names(&list.children)).unwrap_or_default();
    assert_eq!(items, vec!["li", "li"]);
}

#[test]
fn a_cell_and_a_row_end_the_ones_before_them() {
    let nodes = parse("<table><tr><td>a<td>b<tr><th>c</table>");
    let rows = nodes
        .first()
        .and_then(element)
        .map(|table| names(&table.children))
        .unwrap_or_default();
    assert_eq!(rows, vec!["tr", "tr"]);
}

#[test]
fn a_term_and_its_description_end_each_other() {
    let nodes = parse("<dl><dt>one<dd>first<dt>two<dd>second</dl>");
    let list = nodes.first().and_then(element);
    assert_eq!(
        list.map(|list| names(&list.children)).unwrap_or_default(),
        vec!["dt", "dd", "dt", "dd"]
    );
}

#[test]
fn a_void_element_never_holds_anything() {
    let nodes = parse("<p>a<br>b</p>");
    let paragraph = nodes.first().and_then(element);
    assert_eq!(
        paragraph
            .map(|node| names(&node.children))
            .unwrap_or_default(),
        vec!["br"]
    );
}

#[test]
fn an_attribute_is_found_by_name_and_a_child_by_its_own() {
    let nodes = parse("<ul start=3><li>one</li><p>no</p></ul>");
    let list = nodes.first().and_then(element);
    assert_eq!(list.and_then(|list| list.attribute("start")), Some("3"));
    assert_eq!(list.and_then(|list| list.attribute("end")), None);
    assert_eq!(list.map(|list| list.children_named("li").count()), Some(1));
}
