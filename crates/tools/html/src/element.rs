// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What an element is, as far as a page of prose is concerned.
//!
//! HTML says what an element means; a stylesheet says how it is set. This
//! crate has no stylesheet — the one the page was written against is not
//! in this repository — so the tables here take its place, and they are
//! short on purpose. An element is dropped, set as a block, set inside a
//! line, or unknown, and an unknown element is treated as though it were
//! not there at all: its children take its place. That is the rule that
//! lets a document made of custom elements come out as a document rather
//! than as a list of tags nobody wrote a case for.
//!
//! The `emu-` names are the exception that proves it. They are ecmarkup's,
//! the language the ECMAScript specification under `docs/ecma/` is written
//! in, and a custom element has no display of its own once its stylesheet
//! is gone. The ones named here are the ones that stand on their own line,
//! and without them the grammar and the algorithm steps of that document
//! would run together into one paragraph.

/// Elements that never have content.
const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Elements that carry no prose, and are dropped with everything in them.
const DROPPED: [&str; 14] = [
    "script", "style", "head", "title", "template", "noscript", "svg", "math", "iframe", "object",
    "canvas", "audio", "video", "select",
];

/// Elements that stand on their own line.
const BLOCK: [&str; 40] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dd",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "emu-alg",
    "emu-note",
];

/// The block-level elements of ecmarkup, which the tables above cannot
/// guess at because a custom element has no display without its
/// stylesheet.
const BLOCK_ECMARKUP: [&str; 12] = [
    "emu-annex",
    "emu-clause",
    "emu-eqn",
    "emu-figure",
    "emu-grammar",
    "emu-import",
    "emu-intro",
    "emu-normative-optional",
    "emu-production",
    "emu-rhs",
    "emu-see-also-para",
    "emu-table",
];

/// Elements that begin a section, so that a heading inside one of them
/// belongs a level deeper than its own number says.
const SECTIONING: [&str; 7] = [
    "article",
    "aside",
    "emu-annex",
    "emu-clause",
    "emu-intro",
    "nav",
    "section",
];

/// Elements whose text is kept line for line.
const PREFORMATTED: [&str; 3] = ["pre", "emu-eqn", "emu-grammar"];

/// Elements set in the fixed-pitch face.
const MONO: [&str; 8] = [
    "code",
    "emu-const",
    "emu-eqn",
    "emu-t",
    "emu-val",
    "kbd",
    "samp",
    "tt",
];

/// Elements set in italics.
const EMPHASIS: [&str; 6] = ["cite", "dfn", "em", "emu-nt", "i", "var"];

/// Elements set in bold.
const STRONG: [&str; 2] = ["b", "strong"];

/// Elements that are read as a quotation.
const QUOTED: [&str; 2] = ["blockquote", "emu-note"];

/// Whether the element has no content of its own.
pub(crate) fn is_void(name: &str) -> bool {
    VOID.contains(&name)
}

/// Whether the element and everything in it is dropped.
pub(crate) fn is_dropped(name: &str) -> bool {
    DROPPED.contains(&name)
}

/// Whether the element stands on its own line.
pub(crate) fn is_block(name: &str) -> bool {
    BLOCK.contains(&name) || BLOCK_ECMARKUP.contains(&name)
}

/// Whether a heading inside the element belongs one level deeper.
pub(crate) fn is_sectioning(name: &str) -> bool {
    SECTIONING.contains(&name)
}

/// Whether the text of the element is kept as it stands.
pub(crate) fn is_preformatted(name: &str) -> bool {
    PREFORMATTED.contains(&name)
}

/// Whether the element is read as a quotation.
pub(crate) fn is_quoted(name: &str) -> bool {
    QUOTED.contains(&name)
}

/// How the text inside an element is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Face {
    /// The fixed-pitch face.
    Mono,
    /// Italics.
    Emphasis,
    /// Bold.
    Strong,
}

/// The face the element asks for, if it asks for one.
pub(crate) fn face(name: &str) -> Option<Face> {
    if MONO.contains(&name) {
        return Some(Face::Mono);
    }
    if EMPHASIS.contains(&name) {
        return Some(Face::Emphasis);
    }
    if STRONG.contains(&name) {
        return Some(Face::Strong);
    }
    None
}

/// Whether a class list says the element belongs in a line. An element
/// that says so is set in one whatever its name would otherwise mean: an
/// equation written between two halves of a sentence is one, and set as a
/// block it would cut the sentence in three.
pub(crate) fn says_inline(classes: Option<&str>) -> bool {
    classes.is_some_and(|classes| {
        classes
            .split_whitespace()
            .any(|class| class.eq_ignore_ascii_case("inline"))
    })
}

/// The level of a heading element, one to six.
pub(crate) fn heading(name: &str) -> Option<usize> {
    let level = name.strip_prefix('h')?;
    match level {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        "5" => Some(5),
        "6" => Some(6),
        _ => None,
    }
}
