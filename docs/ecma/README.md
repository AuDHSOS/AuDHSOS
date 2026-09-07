# Reference documents: ECMAScript

The language specification, kept here so that a clause can be read
against its source without a network, and so that the source cannot
change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. Decision D-59 records the
arrangement for `docs/rfc/`, and this directory is the same arrangement
for a standard that is not an RFC.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `ecma262.html` | ECMA-262, *ECMAScript® 2027 Language Specification*, eighteenth edition, Ecma International (TC39), draft of 2 September 2026 — the specification alone, cut from the page as described below | 2026-09-07 from `https://tc39.es/ecma262/` | 7130196 | `e1b2b58edb3ab3252329f056b8fbedfcf2fa0f119ab6d4a3bb2ec1e50a0ae5be` |
| `img/ecma-logo.svg` | The Ecma International logo on the title page | 2026-09-07 from `https://tc39.es/ecma262/img/ecma-logo.svg` | 5662 | `fb5ffb5014cfafe6603201d92dc20af5c37987326e3ea4c451904de6d80c5fd1` |
| `img/figure-1.svg` | Figure 1, *Object/Prototype Relationships* | 2026-09-07 from `https://tc39.es/ecma262/img/figure-1.svg` | 2989 | `4c9d26b1fc8a7254a68c141480bd72b330b6bea7590254ed070548a99f363161` |
| `img/figure-2.svg` | Figure 6 (informative), *Generator Objects Relationships* — the file is named `figure-2` and the caption reads Figure 6 | 2026-09-07 from `https://tc39.es/ecma262/img/figure-2.svg` | 10592 | `e71a080964bd34ab3c8cf95289167519d914a2f7ecd45c0b19dd26ed57c5b739` |
| `img/module-graph-simple.svg` | Figure 2, *A simple module graph* | 2026-09-07 from `https://tc39.es/ecma262/img/module-graph-simple.svg` | 1220 | `5969865e9ee4225a30ee08800e430020671f4f322439c4a56490380d73cd2173` |
| `img/module-graph-missing.svg` | Figure 3, *A module graph with an unresolvable module* | 2026-09-07 from `https://tc39.es/ecma262/img/module-graph-missing.svg` | 821 | `e3956df768d2daa68048bfc85885f8fb0664b47c451267cf9fffa0a871f6ec9d` |
| `img/module-graph-cycle.svg` | Figure 4, *A cyclic module graph* | 2026-09-07 from `https://tc39.es/ecma262/img/module-graph-cycle.svg` | 1458 | `f63a9ef34fba3eb0a0243e4fc253a97bf78bb6c4a827b8c33ba9d60cad9d0f29` |
| `img/module-graph-cycle-async.svg` | Figure 5, *An asynchronous cyclic module graph* | 2026-09-07 from `https://tc39.es/ecma262/img/module-graph-cycle-async.svg` | 2438 | `36070394a8ceeb9c945897940dcbd4f960eba277fda85f372ad1c6254c3d2cc5` |

The checksums are here so that a reader can tell a file has not been
edited since. Every file was fetched twice and the two fetches agreed.
`ecma262.html` is 51713 lines; the seven images are byte for byte what
the server delivered, and each is self-contained SVG that names no
resource outside itself.

## What was cut, and what was not

The page at `https://tc39.es/ecma262/` is 7627866 bytes, SHA-256
`2118687f28a406439135418d8ede1ad9bd660b58c8ec8bf08c22886f02f95ba9`. Only
the contents of its `<div id="spec-container">` are kept. What that
leaves out is the reading apparatus and nothing else: the menu, the
search box, the pin panel, the shortcut help, the stylesheets, and the
scripts — all of which are a website around the document rather than the
document. The specification begins at its own title and ends at the last
line of the copyright annex, and all of that is here.

Three things were added to what was cut out, and they are the whole of
the difference:

- a minimal `html`, `head`, and `body` around the fragment, so that the
  file is a document and its UTF-8 encoding is declared;
- a `title` element, with the specification's own title in it;
- a comment at the top of the body stating the source, the date, and
  these changes, which is the notice Ecma's licence asks for.

No text, grammar production, algorithm step, table, or clause number is
altered. The stylesheets are gone, so a browser lays the file out with
default styling and the clause structure reads as plain nested blocks;
the text, the anchors, and the clause numbers are all still there, which
is what reading it or searching it needs.

The images are the reason the `img/` directory exists. The document names
them by the relative paths `img/…`, the same paths they have on the
server, so those paths are kept as they stand and the files now sit where
they point. Nothing in `ecma262.html` reaches the network. The multi-page
rendering under `multipage/` is a second arrangement of the same content
and is not kept.

## What this is a snapshot of

`https://tc39.es/ecma262/` is not a numbered edition. It is the editor's
draft: the most recent yearly snapshot plus every finished proposal, and
it moves whenever one lands. What is recorded above is one moment of it —
the draft of 2 September 2026, which is the eighteenth edition in
progress, ECMAScript 2027. The server named the page it was cut from
`ETag: "6a9ddacf-74645a"`, last modified 2026-09-06 at 21:27:43 UTC.

That is the reason the file is here rather than the link. A citation of
the URL says nothing a month later; a citation of this file says exactly
what was read. When a newer draft is wanted, it is a second file with its
own row, not an overwrite of this one.

## Terms

These documents are not covered by this repository's licence. The
specification carries Ecma International's alternative copyright notice,
© 2026 Ecma International, printed in full in its own *Copyright &
Software License* clause, which is kept. That licence permits copying and
redistribution of the work, with or without modification and without fee,
provided each copy carries the notice, any pre-existing disclaimers, and
a statement of any change made. The changes made here are the three
listed above; the statement of them is in the file itself as well as in
this README.

Software contained in the document — the code it prints, as distinct from
its prose — is placed under the BSD Licence by the same clause. This
project transcribes nothing from the document yet; when it does, the
clause is named at the point of transcription, as decision D-40 requires.

## Why it is here

Nothing in the workspace implements ECMA-262 today. The file is kept
ahead of that, on the same reasoning as the RFCs: the moment a test or a
comment cites a clause number, the clause it cited has to be readable
from the repository, at the wording that was read, without asking a
server that has moved on.
