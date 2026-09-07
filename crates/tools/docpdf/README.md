# docpdf

Turns every Markdown document, every RFC, and every HTML standard of this
repository into a PDF, several at a time, and writes an index beside them.

```sh
sh tools/xtask.sh pdf
```

The output lands under `target/pdf/`, and it is not a copy of the source
tree. A reader looking for the testing strategy should not have to know
that it lives beside the roadmap, and a reader looking for what `net-ip`
does should not have to remember that its README is three directories down.
So a document is filed by what it is:

```text
target/pdf/
├── index.pdf     everything below, with a link to each
├── project/      the files at the root of the repository
├── handbook/     docs/, the numbered chapters in their order
├── crates/       one file per workspace crate, named after the package
├── tools/        one file per tool that is not a workspace crate
├── rfc/          the requests for comments the network stack answers to
├── spec/         the standards kept as HTML rather than as plain text
└── other/        anything Markdown that none of the above claimed
```

The last directory is the one that matters most. A rule that silently drops
a document it has no category for is a rule that loses documents; whatever
is not claimed is still converted, under a name made from its path. Today
it is empty, which is the point.

Links survive the move. A chapter that links to `12-parallel-work.md` gets
a link to `12-parallel-work.pdf`; a crate README that climbs three
directories to reach `docs/04-safety-policy.md` gets a link to
`../handbook/04-safety-policy.pdf`. A link to anything that was not
converted — an address on the web, a source file, a directory — is left
exactly as it was.

## Doing several at once

Every document is independent: it is read, parsed, laid out, and written
without looking at any other. So the pool is the simple one — a shared
cursor into the list of jobs and one thread per core, each taking the next
job when it has finished the last. Two things are ordered. The jobs start
with the largest source first, so that no worker picks up RFC 1122 after
the others have run out of work; and the report comes back in the order the
documents were found rather than the order they happened to finish. Eighty
documents and nine hundred pages take about thirty milliseconds.

Nothing records the time of the run, so a rebuild that changes nothing
writes the files it wrote before, byte for byte.

## What it is made of

Five crates, none of which depends on anything outside this workspace:

- `doc-markdown` parses the Markdown documents.
- `doc-html` parses the HTML ones into the same blocks, so that everything
  below this line is the same code for both.
- `doc-svg` reads the figures a document points at into paths and text.
- `doc-pdf` writes the format: pages, the standard fourteen fonts, links,
  and an outline.
- `docpdf`, this crate, finds the documents, decides where they go, lays
  them out, and runs the pool.

An RFC skips the first two. It arrives as fixed-pitch text already
divided into pages by form feeds, so there is nothing to lay out: each of
its pages goes onto a page, in the face it was written for, with every
column where the author put it. What is added is the outline, taken from
the numbered section headings, so that section 3.2 of RFC 8200 is one click
away instead of ninety pages of scrolling.

## Figures

A picture standing alone on its line is drawn. `doc-svg` reads the file
the document points at into paths and lines of text, and the layout puts
it where it stands: as wide as the measure and never wider than the
picture was made, centred, with the space of a paragraph above and below
it. A picture inside a sentence is not drawn — a figure needs a line of
its own — and one whose file is missing, or holds something `doc-svg`
cannot draw, is said instead, in the `[image: …]` form the Markdown
parser writes for the same thing.

## What an HTML document costs

The ECMAScript specification under `docs/ecma/` is seven megabytes of
markup, and it comes out as a thousand pages in about a third of a second,
which is the same rate as everything else. Its seven figures are drawn from
the SVG files beside it, arrowheads and all, and of the 1656 characters
that Latin-1 has no place for, all but a handful are set in Symbol or
read as the letters they stand for: the double-struck `F` of the numeric
operations comes out as `F`, and the infinity sign comes out as one. Seven
characters of the whole document are left as question marks: two Hangul
syllables and a syllable block from the clause on canonical
normalization, and four Latin letters carrying marks Latin-1 has no place
for.

## Two sizes

By default a stream goes into the file as the operators it was built
from, which is what makes a page readable with `less` when something has
gone wrong with it. `--compress` deflates them instead:

```text
                       plain     compressed
ecma262.pdf            6.62 MB      2.50 MB
target/pdf/ altogether 13.1 MB      6.2 MB
```

Nothing else about the file changes — the same pages, the same links, the
same outline — and both are written without recording the time, so either
one rebuilds byte for byte.

## Options

```text
--root <dir>     the repository to read
--out <dir>      where to write (default: <root>/target/pdf)
--jobs <n>       how many documents at a time (default: the machine's)
--only <text>    convert only what matches, for instance --only rfc/
--list           say what would be converted and write nothing
--compress       deflate the content streams
--quiet          report only the summary line
```
