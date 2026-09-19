# doc-markdown, doc-html, doc-pdf, doc-svg, docpdf audit findings

Repository: AuDHSOS/AuDHSOS. Audit of doc-markdown, doc-html, doc-pdf, doc-svg, docpdf at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #515
Title: doc-html: a body start tag does not close an open head, so a document without </head> yields no blocks
Labels: bug, part::tools
Body:
`crates/tools/html/src/tree.rs:119-128` lists the start tags that end an open element; `head` is not among them. `crates/tools/html/src/element.rs:172-175` names `head` in `DROPPED`, and `crates/tools/html/src/build.rs:84-86` returns before reading anything inside a dropped element. A `<body>` start tag in a document that omits `</head>` therefore nests inside `head`, and the whole body is dropped. The HTML living standard, section 13.2.6.4.4 ("in head" insertion mode), pops `head` on any start tag it does not handle, and section 13.1.2.4 lets an author omit `</head>`.

`docs/w3c/compositing-1.html` and `docs/w3c/css-fonts-4.html` contain no `</head>` (Bikeshed output). `doc_html::parse` returns zero blocks for both, and `xtask pdf` writes `spec/compositing-1.pdf` and `spec/css-fonts-4.pdf` as one page holding only the title and the page number (1 KiB each). Inserting `</head>` before `<body` in a copy of each file gives 505 and 1511 blocks.

Fix: in `implied_end` at `crates/tools/html/src/tree.rs:119-128`, add an arm `"head" => starting == "body"` (or `!matches!(starting, "meta" | "link" | "title" | "style" | "script" | "base")`); dropping `head` from `DROPPED` instead would put `title` and `style` text into the document.

---

## F02 — issue #516
Title: doc-svg: a marker whose content asks for a marker recurses without bound and aborts the process
Labels: bug, part::tools
Body:
`crates/tools/svg/src/render.rs:53-55` documents `depth` as the guard against a drawing that refers to itself; `crates/tools/svg/src/render.rs:276-277` checks it for `use` only. `crates/tools/svg/src/render.rs:254-260` increments `depth` in `marker` and calls `children` without checking it. `crates/tools/svg/src/render.rs:141-143` applies stylesheet rules to every element, including the paths inside a `marker`, so a rule `path { marker-end: url(#a) }` reaches the marker's own path, and `crates/tools/svg/src/render.rs:212-214` calls `marker` again from inside `marker`.

`<svg viewBox='0 0 100 100'><style>path{marker-end:url(#a)}</style><defs><marker id='a'><path d='M0 0L1 1' stroke='black'/></marker></defs><path d='M0 0 L10 10' stroke='black'/></svg>` overflows the stack; the process aborts with exit 134. A figure file with this content aborts the whole `xtask pdf` run, not only the document that names it (`crates/tools/docpdf/src/pool.rs:442-461` runs the work on scoped threads, and a stack overflow aborts the process).

Fix: test `self.depth <= 16` in `marker` before `self.children(marker, ...)` at `crates/tools/svg/src/render.rs:254-259`, as `reuse` does; clearing `marker_end` in the inherited state instead would still recurse through a `use` inside the marker.

---

## F03 — issue #517
Title: doc-pdf: the info title and the outline titles are written in WinAnsiEncoding, not PDFDocEncoding
Labels: bug, part::tools
Body:
`crates/tools/pdf/src/document.rs:435-441` encodes every literal string of a dictionary with `crate::font::encode`, which maps to `WinAnsiEncoding` (`crates/tools/pdf/src/font.rs:183-189`, `265-295`). `crates/tools/pdf/src/document.rs:199-200` writes the `/Title` of the information dictionary and `crates/tools/pdf/src/document.rs:307-308` the `/Title` of every outline item through it. ISO 32000-1:2008 section 7.9.2.2 requires a text string to be PDFDocEncoding or UTF-16BE with a byte order mark. PDFDocEncoding and WinAnsiEncoding differ in the range 0x80 to 0x9F: 0x97 is the em dash in WinAnsi and `Scaron` in PDFDocEncoding, 0x96 is the en dash and `OE`, 0x92 is the right quotation mark and `trademark`.

`crates/jrs/README.md:1` is `# jrs — JavaScript Rust`. `Document::new("jrs \u{2014} JavaScript Rust")` writes `/Title (jrs \227 JavaScript Rust)`, and a viewer shows the document title and its first outline entry as `jrs Š JavaScript Rust`. The seven headings of the form `### F0 — …` in `docs/jrs-firefox-integration.md:237-319` show the same `Š` in the outline. A title with a character outside both encodings becomes `?` (`crates/tools/pdf/src/font.rs:171`, `255-257`).

Fix: write dictionary strings as UTF-16BE with the prefix `FE FF` in `File::string` at `crates/tools/pdf/src/document.rs:435-441`, escaping each byte as now; a PDFDocEncoding table instead would still lose every character outside Latin-1.

---

## F04 — issue #518
Title: doc-html: a table row longer than its header keeps its extra cells, which the layout draws at width zero beyond the table
Labels: bug, part::tools
Body:
`crates/tools/html/src/build.rs:220-226` pushes every body row with the cell count it was written with, and `crates/tools/html/src/build.rs:234-241` sizes `alignments` from the header only. `crates/tools/markdown/src/block.rs:58` documents `alignments` as "the alignment of every column", and `crates/tools/markdown/src/parse.rs:449-450` enforces it by padding and truncating every row. `crates/tools/docpdf/src/layout.rs:762-783` sizes columns from the header's cells; `crates/tools/docpdf/src/layout.rs:455-459` gives a cell without a column the width `0`, so `inner` is negative, and `crates/tools/docpdf/src/text.rs:397` then puts every word of the cell on a line of its own.

`docs/w3c/png-3.html` yields 58 body rows longer than their header, and `docs/w3c/woff2.html` 207 (a header row of one `th` over rows of two and three `td`). In `spec/woff2.pdf` those cells are set one word per line at the right edge of a one-column table, past the measure, and every such row is as tall as its word count. The input `<table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td><td>3</td></tr></table>` produces a `Block::Table` whose row has three cells and whose `alignments` has two.

Fix: in `Builder::table` at `crates/tools/html/src/build.rs:220-226`, resize every row to `head.len()` after the header is known, as `crates/tools/markdown/src/parse.rs:449-450` does; widening the header to the longest row instead would give the extra columns no name and no width.

---

## F05 — issue #519
Title: docpdf: a table row taller than the page is drawn below the bottom margin
Labels: bug, part::tools
Body:
`crates/tools/docpdf/src/layout.rs:405-412` starts a new page when a row does not fit, then draws the whole row on that page. `crates/tools/docpdf/src/layout.rs:460-476` sets every line of every cell downward from `top` without a page check, and `crates/tools/docpdf/src/layout.rs:479` sets `self.y` to `top - height`, which saturates below zero.

A two-column Markdown table whose second cell holds 1200 words gives a row of about 120 lines; the page holds 49. The layout writes 73 of those lines below the bottom margin, the lowest at a baseline of `-965` points, and the following row starts a page. The 1200-word cell reads as a cut-off block in every viewer.

Fix: in `Layout::row`, break a row whose `height` exceeds the page's text height across pages by setting the cells' lines page by page (the header repeated) instead of from one `top`; clamping the row to the page instead would drop the lines that do not fit.

---

## F06 — issue #520
Title: doc-html: an abruptly closed comment swallows the rest of the document
Labels: bug, part::tools
Body:
`crates/tools/html/src/token.rs:181-185` strips `<!--` and searches the remainder for `-->`. For `<!-->` the remainder is `>` and for `<!--->` it is `->`, neither contains `-->`, and the comment is taken to run to the end of the input. The HTML living standard, section 13.2.5.43 (comment start state), emits an empty comment on `>`, and section 13.2.5.44 (comment start dash state) does the same for `->`. Section 13.2.5.51 (comment end bang state) also closes a comment on `--!>`, which the search does not find.

`<p>a<!-->b</p><p>c</p>` yields one paragraph `a`; `b` and `c` are lost. `<p>a<!--->b</p><p>c</p>` yields the same.

Fix: in `skipped`, after stripping `<!--`, return `5` when the body starts with `>` and `6` when it starts with `->`, and search for `-->` or `--!>` otherwise; treating `<!-->` as a bogus comment instead would still drop `b`.

---

## F07 — issue #521
Title: doc-markdown: a heading's closing-hash rule strips a hash that is part of the text
Labels: bug, part::tools
Body:
`crates/tools/markdown/src/parse.rs:273` removes every trailing `#` from the heading text with `trim_end_matches('#')`. CommonMark 0.31.2 section 4.2 requires the closing sequence to be preceded by a space or tab; `# foo#` is the heading `foo#` (example 43).

`# C#` yields `Heading { level: 1, content: [Text("C")] }`. The heading text, the outline entry, and the document title (`crates/tools/docpdf/src/sources.rs:244-250`) lose the `#`.

Fix: strip the trailing hashes only when the text before them ends in a space or is empty, then trim; keeping the hashes always instead would print `foo ##` for `## foo ##`.

---

## F08 — issue #522
Title: doc-markdown: a table is recognized inside an indented code block
Labels: bug, part::tools
Body:
`crates/tools/markdown/src/parse.rs:158-164` tries `table` before `indented_code`, and `table` at `crates/tools/markdown/src/parse.rs:425-431` has no indentation check, unlike `fenced_code` (`187`), `heading_of` (`261`), `thematic_break` (`279`), `quote` (`298`), and `list` (`362`). CommonMark section 4.4 makes any line indented four or more spaces part of an indented code block.

`    | a | b |\n    |---|---|\n    | 1 | 2 |` yields a `Block::Table`, not a `Block::Code`. An ASCII table inside an indented code block is set as a proportional-font table with a header ground.

Fix: return `None` from `table` when `indent(header_line) >= TAB`, as the other block starters do.

---

## F09 — issue #523
Title: doc-markdown: indentation is counted in bytes, so a non-ASCII space drops a list item
Labels: bug, part::tools
Body:
`crates/tools/markdown/src/parse.rs:178-180` computes the indent as `line.len() - line.trim_start().len()`, which is a byte count, and `trim_start` removes every Unicode whitespace character. `crates/tools/markdown/src/parse.rs:332` and `343` count the spaces after a marker the same way and cap them at 3. `crates/tools/markdown/src/parse.rs:391` then slices the line with `line.get(width..)`, and a `width` that falls inside a two-byte character yields `None` and an empty item line.

`- \u{a0}\u{a0}text` gives `spaces = 3` (two NBSP are four bytes, capped at 3), `width = 5`, `line.get(5..)` is `None`, the item is empty, `crates/tools/markdown/src/parse.rs:409-412` drops it, and the line is parsed as `Paragraph([Text("- \u{a0}\u{a0}text")])`. The item's text is set with its bullet as a literal dash.

Fix: count columns with `chars().take_while(|c| *c == ' ')` in `indent`, `marker`, and `strip_indent`, and slice by the byte offset of that many characters; leaving `trim_start` and slicing by chars instead would still miscount a line that starts with a tab after `expand_tabs`.

---

## F10 — issue #524
Title: doc-markdown: a blank line inside a nested list makes the outer list loose
Labels: bug, part::tools
Body:
`crates/tools/markdown/src/parse.rs:399-405` sets `tight = false` for the list being parsed whenever a blank line is followed by any line that continues the list, including a line that belongs to a nested item. CommonMark section 5.3 makes a list loose only when its own items are separated by blank lines or an item directly contains two blocks with a blank line between them; the blank line between two nested items belongs to the nested list.

`- a\n  - b\n\n  - c\n- d` yields the outer list with `tight: false` and the inner list with `tight: false`; the outer list must be tight. `crates/tools/docpdf/src/layout.rs:361-364` then sets the outer items `a` and `d` with the paragraph space instead of the tight space.

Fix: in `list`, set `tight = false` only when the blank line is followed by a line of this list's own item at column `< width` (a new marker or a lazy continuation) or by a block that stays at the item's indent, and leave lines at indent `>= width` that a nested list claims to the recursive parse; recomputing tightness from the parsed items instead would need the blank lines the recursion has already consumed.

---

## F11 — issue #525
Title: doc-markdown: a backslash before a line ending is not a hard line break
Labels: bug, part::tools
Body:
`crates/tools/markdown/src/inline.rs:80-89` escapes a backslash only before a punctuation character and otherwise pushes the backslash as text; `crates/tools/markdown/src/inline.rs:90-102` turns a newline into a hard break only after two spaces. CommonMark section 6.7 makes a backslash at the end of a line a hard line break.

`a\\\nb` yields `Paragraph([Text("a\\ b")])`: a literal backslash is set and the line is not broken.

Fix: in the `'\\'` arm, when the next character is `'\n'`, flush the buffer, push `Inline::Break`, and skip the newline.

---

## F12 — issue #526
Title: doc-html: a legacy character reference in an attribute value is decoded before = or a letter
Labels: bug, part::tools
Body:
`crates/tools/html/src/token.rs:293` and `300` decode attribute values with the same `entity::decode` as text, and `crates/tools/html/src/entity.rs:471-476` decodes a name in `LEGACY` without its semicolon whatever follows it. The HTML living standard, section 13.2.5.73 (named character reference state), leaves the reference undecoded inside an attribute when the match has no semicolon and the next character is `=` or ASCII alphanumeric.

`<a href="?x=1&copy=2&lt=3">t</a>` yields a link whose `href` is `?x=1©=2<=3`; the standard keeps `?x=1&copy=2&lt=3`. The rewritten link points at an address that does not exist.

Fix: give `decode` a flag for attribute context and, in `named`, refuse a legacy match when the flag is set and the following character is `=` or alphanumeric.

---

## F13 — issue #527
Title: doc-html: numeric character references in the C1 range, zero, and out-of-range values are not mapped as the standard says
Labels: bug, part::tools
Body:
`crates/tools/html/src/entity.rs:445-446` turns every numeric value that `char::from_u32` accepts into that character and leaves every other value as text. The HTML living standard, section 13.2.5.80 (numeric character reference end state), maps 0x80 to 0x9F through a table (0x96 to U+2013, 0x97 to U+2014, 0x91 to U+2018, and so on), and maps 0x00, surrogates, and values above 0x10FFFF to U+FFFD.

`<p>&#150; &#0; &#x110000;</p>` yields `Text("\u{96} \0 &#x110000;")`; the standard yields `– \u{fffd} \u{fffd}`. The control character U+0096 has no glyph and becomes `?` on the page (`crates/tools/pdf/src/font.rs:221`), and the NUL reaches the content stream as `\000`.

Fix: in `numeric`, apply the C1 table, and return U+FFFD for zero, surrogates, and values that overflow or exceed 0x10FFFF.

---

## F14 — issue #528
Title: doc-html: a no-break space is collapsed and broken like an ordinary space
Labels: bug, part::tools
Body:
`crates/tools/html/src/runs.rs:497-507` collapses text with `char::is_whitespace` and `split_whitespace`, both of which treat U+00A0 as whitespace; `crates/tools/html/src/build.rs:401-411` does the same for non-`pre` code. The HTML living standard defines the collapsible characters as ASCII whitespace (section 2.3.5, and the `white-space` processing it refers to); `&nbsp;` is written to keep two words together.

`<p>a&nbsp;b</p>` yields `Text("a b")` with a breakable U+0020. `docs/w3c/woff2.html` writes `&nbsp;` 18 times and `docs/w3c/png-3.html` 5 times; each becomes a place the layout may break the line.

Fix: collapse only `[' ', '\t', '\n', '\r', '\x0c']` in `Runs::text` and `push_collapsed`, and keep U+00A0 as a character of the word.

---

## F15 — issue #529
Title: docpdf: the line breaker breaks at a no-break space
Labels: bug, part::tools
Body:
`crates/tools/docpdf/src/text.rs:346` starts a new word at every character for which `char::is_whitespace` is true, which includes U+00A0.

`Inline::Text("a\u{a0}b")` becomes the tokens `Word(a) Space Word(b)`, and `wrap` at `crates/tools/docpdf/src/text.rs:392-413` may put `b` on the next line. The space is also set with the width of U+0020 instead of the width of U+00A0 in the font's table.

Fix: split words at ASCII whitespace only in `push_words`, so that U+00A0 stays inside the word and is measured through `WinAnsiEncoding` byte 0xA0.

---

## F16 — issue #530
Title: doc-svg: arc flags written without a separator misread the rest of the path
Labels: bug, part::tools
Body:
`crates/tools/svg/src/path.rs:156-168` reads the seven arguments of `A` with `number::read`, which consumes as many digits as follow. The SVG 1.1 path grammar (section 8.3.9) defines `flag` as a single `0` or `1`, so `01` is two flags, and the compact form `a10 10 0 0110 10` is what minifiers write.

`<path d='M0 0 a10 10 0 0110 10 L50 50'/>` reads `110` as the fourth argument and `10` as the fifth, finds no sixth, and `crates/tools/svg/src/path.rs:127-129` ends the path: the drawing holds only the `Move`. The arc and the `L50 50` after it are lost.

Fix: in `read`, when the command is `A` or `a` and the index is 3 or 4, take exactly one character `0` or `1` instead of a number.

---

## F17 — issue #531
Title: doc-svg: a smooth curve after a curve of the other kind mirrors the wrong control point
Labels: bug, part::tools
Body:
`crates/tools/svg/src/path.rs:83-91` mirrors whatever `control` holds, and `crates/tools/svg/src/path.rs:70` and `214` set `control` after both a cubic and a quadratic. SVG 1.1 section 8.3.6 makes the first control point of `S` the current point unless the previous command was `C`, `c`, `S`, or `s`; section 8.3.7 makes the control point of `T` the current point unless the previous command was `Q`, `q`, `T`, or `t`.

`<path d='M0 0 Q10 20 30 0 S60 40 80 0'/>` yields a second curve whose first control point is `(50, -20)`, the mirror of the quadratic's control; the standard places it at `(30, 0)`.

Fix: record which kind of command set `control` and let `mirrored` answer the current point when the kind does not match the command asking.

---

## F18 — issue #532
Title: doc-svg: an exponent is applied after the mantissa has been cut to three decimals
Labels: bug, part::tools
Body:
`crates/tools/svg/src/number.rs:71` computes `scale(fixed(mantissa), exponent)`; `fixed` at `crates/tools/svg/src/number.rs:121` keeps three fractional digits, and `scale` multiplies afterward.

`1.2345e2` is read as `123400` thousandths (123.4) instead of `123450`; `0.0001e4` is read as `0` instead of `1000`. `<rect x='1.2345e2' y='0.0001e4' …/>` is placed at `(123.4, 0)` in a drawing that says `(123.45, 1)`.

Fix: fold the exponent into `fixed` by shifting the decimal point of the digit string before the three-place cut.

---

## F19 — issue #533
Title: doc-svg: an all-zero or negative dash array reaches the content stream
Labels: bug, part::tools
Body:
`crates/tools/svg/src/paint.rs:555-561` takes every number of `stroke-dasharray`, and `crates/tools/svg/src/render.rs:198-202` scales them; `crates/tools/pdf/src/page.rs:336-337` writes them as `[…] 0 d`. ISO 32000-1:2008 section 8.4.3.6 forbids a dash array whose elements are all zero, and SVG 1.1 section 11.4 makes a list with a negative value an error and a list of all zeros a solid line.

`stroke-dasharray='0'` writes `[0] 0 d`, and `stroke-dasharray='-5 2'` writes `[-5 2] 0 d`; a viewer rejects the operator or the page.

Fix: in `State::apply`, drop the list when any value is negative and clear it when every value is zero.

---

## F20 — issue #534
Title: doc-svg: a percentage inside rgb() is read as a byte value
Labels: bug, part::tools
Body:
`crates/tools/svg/src/paint.rs:650-662` reads the three numbers of `rgb(…)` and clamps them to 0 to 255; a `%` after a number is not read. CSS Color Level 3 section 4.2.1 defines `rgb(100%, 0%, 0%)` as `rgb(255, 0, 0)`.

`fill='rgb(100%,0%,0%)'` yields `Color { red: 100, green: 0, blue: 0 }`, a dark red instead of red.

Fix: when the component text ends in `%`, scale the value by 255/100 before clamping.

---

## F21 — issue #535
Title: doc-svg: a marker is turned along the path whether or not it asks for it
Labels: bug, part::tools
Body:
`crates/tools/svg/src/render.rs:250-253` composes the direction matrix from `ending` into every marker's placement; the `orient` attribute is not read. SVG 1.1 section 14.3.2 defaults `orient` to `0`, which draws the marker unrotated, and turns it along the path only for `orient="auto"`.

A marker without `orient` on a path ending in a vertical stroke is drawn rotated a quarter turn. The seven figures under `docs/ecma/img/` all write `orient="auto"` and are unaffected.

Fix: read `orient` on the marker element and use `Matrix::IDENTITY` in place of `direction` unless it is `auto` or `auto-start-reverse`.

---

## F22 — issue #536
Title: doc-pdf: annotation dictionaries are written directly in the Annots array
Labels: bug, part::tools
Body:
`crates/tools/pdf/src/document.rs:252-281` writes each link annotation as a direct dictionary inside `/Annots`. ISO 32000-1:2008 Table 30 defines `Annots` as "an array of annotation dictionaries that shall contain indirect references to all annotations associated with the page".

Every page with a link (`crates/tools/docpdf/src/layout.rs:570-576`, `crates/tools/docpdf/src/index.rs:591-597`) carries such an array. A conformance checker reports the deviation, and a tool that edits annotations by reference has none to name.

Fix: hand out one object number per link after the page objects in `Document::finish` and write each annotation as its own object; a `/P` entry can then name the page.

---

## F23 — issue #537
Title: docpdf: a list marker is left at the foot of a page when the item's first block starts a new one
Labels: bug, part::tools
Body:
`crates/tools/docpdf/src/layout.rs:374-381` reserves one body line, sets the marker, and then calls `self.blocks(item, inner)`. A first block that reserves more, a code block (`crates/tools/docpdf/src/layout.rs:301`), a table (`403`), a heading (`171-175`), or a figure (`221`), starts a page in `ensure`, and the marker stays on the previous page.

A Markdown document of 26 one-line paragraphs followed by `- item` whose item body is a fenced code block writes the bullet `(\225) Tj` on page 1 and the code on page 2 (probed with `docpdf --root` over such a file; 27 paragraphs give the same).

Fix: reserve the height the first block reserves before setting the marker, or set the marker inside the first block's `draw` at its first line; moving the check after `blocks` instead would leave the marker where it is.

---

## F24 — issue #538
Title: docpdf: the README says other/ is empty, and the run writes eighteen files into it
Labels: bug, part::tools
Body:
`crates/tools/docpdf/README.md:28-31` says of `other/`: "Today it is empty, which is the point". `crates/tools/docpdf/src/sources.rs:196-212` files every unclaimed Markdown document there, and `crates/tools/docpdf/src/sources.rs:91-107` claims only the files directly in `docs/`.

`xtask pdf` writes eighteen files into `other/`: `docs-acm-readme.pdf`, `docs-adobe-readme.pdf`, `docs-ecma-readme.pdf`, `docs-w3c-readme.pdf`, `anchors-readme.pdf`, `fonts-readme.pdf`, and the other reference-directory READMEs under `docs/`.

Fix: rewrite the sentence to say what lands there today (the README of every reference directory under `docs/` and the READMEs of `anchors/` and `fonts/`); adding a `reference/` category for `docs/*/README.md` instead would change the output tree the README documents.

---

## F25 — issue #539
Title: doc-markdown: nesting depth of lists, quotations, and links is bounded only by the stack
Labels: enhancement, part::tools
Body:
`crates/tools/markdown/src/parse.rs:316`, `387`, and `408` recurse into `blocks` once per nesting level of a quotation or list, and `crates/tools/markdown/src/inline.rs:249` and `315` recurse into `parse` once per nested emphasis or link. Each level of `list` also copies the remaining lines (`crates/tools/markdown/src/parse.rs:391-393`), so a line of `n` markers costs O(n²) bytes.

A line of 5000 `- ` (10 KB) overflows a 2 MiB stack, which is what the workers of `crates/tools/docpdf/src/pool.rs:442-461` run on; 20000 overflow the 8 MiB main thread. `> ` repeated and `[` repeated with matching `](u)` behave the same. The process aborts; no document of the repository nests deeper than a few levels.

Fix: pass a depth to `blocks` and `inline::parse` and treat a line beyond a fixed depth (32) as paragraph text.

---

## F26 — issue #540
Title: doc-html: element nesting depth is bounded only by the stack
Labels: enhancement, part::tools
Body:
`crates/tools/html/src/build.rs:58-65` and `82-128` recurse once per nested element, `crates/tools/html/src/build.rs:279-289` once per level in `holds_block`, and dropping `Vec<Node>` (`crates/tools/html/src/tree.rs:27-43`) recurses once per level. The tree builder itself keeps its stack on the heap (`crates/tools/html/src/tree.rs:68`).

`<div>` repeated 5000 times overflows a 2 MiB worker stack; 20000 overflow the main thread. `holds_block` is also called once per unknown element, and each call scans that element's subtree, so `d` nested unknown elements over `n` nodes cost O(n·d).

Fix: cap the builder's stack depth in `tree::parse` (an element beyond the cap is treated as closed) and build blocks with an explicit stack of builders.

---

## F27 — issue #541
Title: doc-svg: element nesting depth is bounded only by the stack
Labels: enhancement, part::tools
Body:
`crates/tools/svg/src/lib.rs:81-93`, `crates/tools/svg/src/render.rs:87-96`, and `crates/tools/svg/src/render.rs:99-129` each recurse once per nested `g`. `crates/tools/svg/src/render.rs:276-277` bounds `use` at sixteen levels, which still allows sixteen levels of fan-out: a `g` of ten `use` elements pointing at itself is drawn 10¹⁶ times.

`<g>` repeated 5000 times around a `rect` overflows a 2 MiB worker stack. No figure under `docs/` nests deeper than a few levels.

Fix: count the depth in `children` and stop at a fixed depth, and count the items drawn and stop at a fixed number.

---

## F28 — issue #542
Title: doc-markdown: unmatched emphasis and link openers cost quadratic time and an allocation per scan step
Labels: enhancement, part::tools
Body:
`crates/tools/markdown/src/inline.rs:234-248` scans to the end of the text for each `*` or `_` that opens nothing, and `crates/tools/markdown/src/inline.rs:237` allocates `marker.to_string().repeat(width)` on every iteration of that scan. `crates/tools/markdown/src/inline.rs:266-278` scans to the end for each unmatched `[`. Over `n` such characters the parse is O(n²).

A paragraph of 20000 `*a ` (60 KB) parses in 10.5 s, 40000 in 41.7 s, 80000 in 166 s; `_a ` repeated behaves the same, `[` repeated costs 2.5 s at 80000. `` `a `` repeated costs 10 ms at 80000, because `code_span` allocates once.

Fix: build the closing pattern once before the loop, and remember for each marker the offset where the last failed scan ended so a later opener does not rescan it (the CommonMark delimiter stack does this in O(n)).

---

## F29 — issue #543
Title: doc-pdf: the outline tree is built in quadratic time
Labels: enhancement, part::tools
Body:
`crates/tools/pdf/src/outline.rs:515-529` computes for every entry its children by a scan of all entries, its previous and next sibling by scans that call `parent_of` per entry, and its descendants by a scan that climbs the parent chain per entry. For `n` entries of depth `d` the tree costs O(n²·d).

`docs/ecma/ecma262.html` has 2447 headings, so `tree` performs about six million parent lookups per run; `parents` at `crates/tools/pdf/src/outline.rs:536-547` already has every parent in one O(n) pass.

Fix: after `parents`, fill `first`, `last`, `previous`, and `next` in one forward pass over the entries with a per-parent "last child seen" table, and `descendants` in one backward pass adding each entry's count plus one to its parent.

---

## F30 — issue #544
Title: docpdf: cutting an over-wide word or code line measures the growing prefix from scratch per character
Labels: enhancement, part::tools
Body:
`crates/tools/docpdf/src/text.rs:436-443` clones the prefix and measures it with `style.width` for every character of a word wider than the measure, and `crates/tools/docpdf/src/layout.rs:738-745` does the same for every character of every code line. A word or line of `n` characters costs O(n²).

A 10000-character unbroken token (a base64 blob in a code span, a long URL) costs fifty million width lookups. `crates/tools/docpdf/src/index.rs:637-646` repeats the pattern for a clipped title.

Fix: keep a running width and add the width of each character (measured alone, in its font run) instead of re-measuring the prefix.

---

## F31 — issue #545
Title: doc-html: the tokenizer lower-cases the rest of the document for every script or style element
Labels: enhancement, part::tools
Body:
`crates/tools/html/src/token.rs:319` calls `rest.to_ascii_lowercase()` on the whole remaining input to find the end tag of a raw element. A document of `n` bytes with `k` `script` or `style` elements costs O(n·k) bytes of copying.

Two hundred `<style>` elements before 2 MB of text parse in 36 ms against 7 ms for one; `docs/w3c/css-fonts-4.html` carries its stylesheet in several `style` elements at the top of a 1.4 MB file.

Fix: search with a case-insensitive comparison at each `<` of `rest` (as `find_tag` at `crates/tools/html/src/lib.rs:45-49` does) instead of copying.

---

## F32 — issue #546
Title: doc-markdown: the setext check after the paragraph loop is dead code
Labels: enhancement, part::tools
Body:
`crates/tools/markdown/src/parse.rs:569-577` checks `lines.get(cursor)` for a setext underline after the loop at `crates/tools/markdown/src/parse.rs:547-568`. The loop exits at the end of the input (`get` is `None`), at a blank line (`setext` of a blank line is `None` by `crates/tools/markdown/src/parse.rs:584-586`), or at a line that `starts_block` matched after `setext` at `crates/tools/markdown/src/parse.rs:552` had already rejected it. The branch at 569 is never taken.

Removing the branch changes no output.

Fix: delete `crates/tools/markdown/src/parse.rs:569-577`.
