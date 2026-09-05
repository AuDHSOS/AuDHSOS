# docpdf

Turns every Markdown document and every RFC of this repository into a PDF,
several at a time, and writes an index beside them.

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

Three crates, none of which depends on anything outside this workspace:

- `doc-markdown` parses the documents.
- `doc-pdf` writes the format: pages, the standard fourteen fonts, links,
  and an outline.
- `docpdf`, this crate, finds the documents, decides where they go, lays
  them out, and runs the pool.

An RFC skips the first of those. It arrives as fixed-pitch text already
divided into pages by form feeds, so there is nothing to lay out: each of
its pages goes onto a page, in the face it was written for, with every
column where the author put it. What is added is the outline, taken from
the numbered section headings, so that section 3.2 of RFC 8200 is one click
away instead of ninety pages of scrolling.

## Options

```text
--root <dir>     the repository to read
--out <dir>      where to write (default: <root>/target/pdf)
--jobs <n>       how many documents at a time (default: the machine's)
--only <text>    convert only what matches, for instance --only rfc/
--list           say what would be converted and write nothing
--quiet          report only the summary line
```
