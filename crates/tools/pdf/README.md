# doc-pdf

A writer for PDF 1.7. It knows pages, text in the five faces of the
standard fourteen fonts it uses, filled rectangles, links, and an outline.
It knows nothing about Markdown, about documents, or about files.

The format is written by hand, as everything in this repository is. There
is no compression: a content stream goes into the file as the plain ASCII
operators it was built from, which costs perhaps three times the bytes of a
deflated stream and buys a document that can be read in a text editor when
a page comes out wrong. There is no embedded font either. The standard
fourteen are the fonts every viewer carries, so a document needs no font
file and no font parser, and the width tables here — the Adobe metrics, in
thousandths of the point size — exist only so that the line breaker can
measure a line before it is written.

Lengths are whole numbers of thousandths of a point, the unit the font
metrics already count in. Nothing in the layout or in the writer uses a
floating-point number, so two runs on two machines produce the same page
breaks and the same bytes. Nothing records the time either: a rebuild that
changes nothing writes the file it wrote before, and a document that
differs, differs because a source did.

The coordinate system is the one PDF uses, with the origin at the bottom
left corner and `y` growing upwards. A layout that counts downwards from
the top of the page converts once, where it asks for a page, rather than at
every line.

## What it does not do

Images, transparency, patterns, and anything that needs a graphics state
stack. Character sets beyond `WinAnsiEncoding`, which covers Latin-1 and
the punctuation the prose of this repository uses; anything else is written
as a question mark, because one wrong glyph is a smaller failure than a
stream whose byte count no longer matches its dictionary. Encryption,
tagged PDF, and forms.
