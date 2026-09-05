# doc-markdown

A Markdown parser for the documents this repository holds: the handbook
under `docs/`, the README of every crate, and the files at the root. It
turns text into blocks and inline runs and stops there. It does not render,
it does not know what a page is, and it produces no HTML.

It is a subset of CommonMark, chosen by reading what is actually written
here: ATX and setext headings, paragraphs, fenced and indented code,
quotations, ordered and unordered lists to any depth, tables with column
alignment, thematic breaks, and inline emphasis, strong emphasis, code
spans, links, autolinks, images, and hard line breaks. Reference links,
raw HTML, footnotes, and definition lists are not parsed; the text of them
survives as the text it is.

Two rules earn their place. An underscore opens or closes emphasis only at
a word boundary, so `saturating_add` and `check_crate_roots` stay
identifiers instead of turning half a paragraph into italics — in a
repository whose prose is full of Rust names, the CommonMark rule is not a
nicety but the difference between a readable page and a broken one. And a
code span is delimited by as many backticks as opened it, so a span can
contain a backtick, which the documents do when they quote a fence.

A list item is parsed by cutting away the indent its marker created and
running the whole parser over what remains. An item can therefore hold a
paragraph, a code block, a table, and another list, without a single case
for any of that in the item code.
