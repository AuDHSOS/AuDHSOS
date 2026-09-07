# doc-pdf

A writer for PDF 1.7. It knows pages, text in the six faces of the
standard fourteen fonts it uses, paths — filled, stroked, dashed, and
curved — links, and an outline. It knows nothing about Markdown, about
documents, or about files.

The format is written by hand, as everything in this repository is. A
content stream goes into the file as the plain ASCII operators it was
built from, which costs about three times the bytes of a deflated stream
and buys a document that can be read in a text editor when a page comes
out wrong. That is the default, and a document asked to `compress` gets
the other trade instead: every stream deflated by `audhsos-deflate`, in
the wrapper `FlateDecode` names, and a file about a third of the size.

That is a reason to write few operators, not to write them carelessly. A
page keeps what the stream is currently set to — the two colours, the face
and size, where the open text object last began a line, where the pen now
stands, and the distance between two lines — and writes nothing that is
already true.

A page of prose is therefore one text object, opened at its first word and
closed by whatever needs the pen back. The face is named only when it
changes. A run that begins exactly where the last one ended says nothing
at all about its place, because showing text is itself a move by the width
of what was shown, and that width is the number this crate measured the
line with. A run that begins the next line at the distance the stream is
already set to is shown by the operator that means *next line, then this*,
so a paragraph is one number and then one string per line. Only what is
neither of those moves by naming a distance, and only the first run of a
text object names a matrix. And a grey is written with the operator that
takes one number instead of the one that takes three.

What PDF has no operator for is a paragraph. The format places lines; the
line breaking is the layout's decision and the file records where each
line came to rest. So one string per line is the floor, and the
thousand-page specification under `docs/ecma/` reaches it: from fifty
bytes of operators for every run of text down to twenty-four, and from a
file of ten and a half megabytes to one of six and a third. There is no embedded font either. The standard
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

## Six faces, and what falls between them

Five of the six are set in `WinAnsiEncoding`: Helvetica in three weights
and Courier in two. The sixth is Symbol, and it is there for what the
other five have no glyph for. A character outside `WinAnsiEncoding` is
written three ways, in this order: in Symbol, if Symbol has it — the
infinity sign, the mathematical relations, the arrows, the Greek
alphabet; as the letters it is read as, if it has such a reading, which is
how the double-struck letters of a specification's numeric operations come
out as `F`, `R`, and `Z`; and as a question mark if it is neither, because
one wrong glyph is a smaller failure than a stream whose byte count no
longer matches its dictionary. A combining mark is dropped, since nothing
here can place one.

A line therefore does not go onto a page as one string. It goes as the
runs its characters need, each in its own font, and the width of the run
before it is what puts the next one in the right place — which is why the
splitting sits beside the metrics rather than in the page.

## Paths

A path is a list of moves, lines, and cubic curves, filled by either
winding rule, stroked with a width and a dash pattern, or both. That is
what a figure needs and no more: the drawing of one lives in `doc-svg`,
which turns an SVG into these segments, and the page never learns what a
picture is.

## What it does not do

Raster images, transparency, patterns, gradients, and anything that needs
a graphics state stack — every path is drawn in the coordinate system of
the page, because whoever hands one over has already put it there.
Encryption, tagged PDF, and forms.
