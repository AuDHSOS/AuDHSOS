# doc-svg

Reads the SVG figures of this repository's documents into the marks a PDF
page is made of: paths, in the coordinate system PDF draws in, and lines
of text in the standard fourteen fonts. It draws nothing itself and knows
nothing about pages — `doc-pdf` draws, `docpdf` decides where.

A figure of a specification is a diagram: boxes, arrows, curves, and
labels. That is what this crate reads. It is not a renderer, and the line
between what it does and what it does not is drawn where those diagrams
end.

## What it reads

The shapes — `path`, `rect` with corners as round as it asks for,
`circle`, `ellipse`, `line`, `polygon`, `polyline` — and every command of
a path except the elliptical arc, which is drawn as the straight line to
where it ends. `g` and `use`, with the definitions they point at. `text`,
with its family, size, weight, and anchor. The transforms `translate`,
`scale`, `matrix`, and `rotate`. Fills and strokes by name, by
hexadecimal, or by their three components, with their widths, their
dashes, and their winding rule. And the marker at the end of a stroke,
which is what makes an arrow an arrow.

The stylesheet an SVG carries is read too, because a diagram written with
`class="box"` says nothing about its own colour otherwise. What is read
of it are the selectors those drawings use — a name, a class, or a name
and a class — with the declarations of the strongest match standing last.

## Two numbers it does without

There is no floating-point number here. A length is a whole number of
thousandths of a user unit, the way a length in `doc-pdf` is a whole
number of thousandths of a point, so that two runs on two machines draw
the same figure to the same thousandth.

And there is no trigonometry. A `rotate` by name is read as the nearest
quarter turn, which is what a diagram is ever turned by; and the rotation
that points a marker along the line it sits at the end of needs no angle
at all, because the direction of the line *is* the rotation: for a unit
vector `(dx, dy)`, the matrix `[dx dy -dy dx]` turns the marker exactly
that far. The unit vector needs a square root, and that one is an integer
square root of an integer.

## What it does not read

Gradients, patterns, filters, clipping and masking, opacity, images
inside an SVG, and anything that needs a script. A `tspan` that moves
itself, and text on a path. Those are the parts of SVG a drawing program
writes and a diagram does not.

## Turning it over

SVG counts down the page and PDF counts up it. The first thing every
point goes through is the matrix that flips it, so everything after that
is in the coordinate system the writer draws in, and a caller scales and
places a finished drawing without thinking about which way is up. Text is
placed by that same matrix and then set upright: a mirrored letter is not
what anybody meant by a mirrored coordinate system.

## Where the markup comes from

An SVG is XML, and tags, attributes, text, and a tree are all this crate
needs of a parser. `doc-html` has one, and it is public for this reason;
carrying a second copy of it here would be two tokenizers to keep right
instead of one.
