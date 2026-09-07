# doc-html

An HTML parser for the documents this repository holds. It reads a page
and produces the blocks and inline runs the Markdown parser produces, so
that everything downstream — the layout, the page breaking, the outline —
is the same code for both. It renders nothing and knows nothing about
pages.

It is not a browser engine. There is no stylesheet here, no script, no
layout, and no error recovery beyond what a document needs to survive its
own author. What it has is a tokenizer, a tree builder with the implied
end tags real documents rely on, and one table saying what each element
is.

## The rule that carries it

An element the tables do not name is treated as though it were not there:
its children take its place. A page made of custom elements therefore
comes out as a document rather than as a list of tags nobody wrote a case
for, and the tables stay short enough to read.

The tables say four things. An element is dropped with everything in it
(`script`, `style`, `head`, and the other places prose is not); it stands
on its own line (`p`, `li`, `table`, `pre`, and the rest); it is set in a
face of its own inside a line (`code` and `var` and their kind); or it is
none of those, and it disappears in favour of its children.

## Headings count from where they stand

A page written as one section after another gives every section an `h1`,
and the number in the tag then says nothing about how deep the heading
is. The number of sections around it does. So a heading's level is its own
number plus the sections it sits in, clamped to six, which is what makes
an outline out of a document that has only one kind of heading in it. A
page that nests no sections keeps the numbers it wrote.

## Whitespace

A newline and eight spaces of indentation between two tags are one space,
and the same space may be written on either side of the tag that separates
two words. Text is therefore collected word by word with a space *owed*
rather than written, and the debt is paid when the next word or the next
mark arrives — never at the beginning of a line and never at its end.
Inside a `pre` none of that happens: the text is what it was.

## What ecmarkup gets, and why

The `emu-` elements are named in the tables. They are ecmarkup's, the
language the ECMAScript specification under `docs/ecma/` is written in,
and a custom element has no display of its own once its stylesheet is
gone. Without those names the grammar of that document — a production, its
`::`, and each right-hand side — would come out as one paragraph with
everything in it, and two thousand algorithm steps would run together the
same way. With them, a production is a block of fixed-pitch lines with one
right-hand side per line, and `emu-alg` is the ordered list it already
was.

## What it does not do

Pictures. An `img` becomes an image run carrying the file it names and
what it says it shows, and what becomes of that is the renderer's to
decide: `docpdf` draws a picture that stands alone on its line and says
`[image: …]` for one inside a sentence. Fragment links, because a PDF has
no name for the place a `#` points at inside itself, so the text of the
link is kept and the link is dropped. Tables of merged cells, which are read as the cells they are
written as. Forms, frames, and anything that needs a script to mean
something.

Character references are decoded from a table of the punctuation and
symbols documents are written with, plus every numeric reference. A name
that is not in the table stays the text it is, which is what a reader
would rather see than a character the parser guessed at.
