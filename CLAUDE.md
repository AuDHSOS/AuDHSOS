# wiriting style

Keep everything in US English. Write clear and distinct sentences. Get to the point.
Do not write rule of thumb sentences. Especially not in headlines.

While you work report only bugs / critical findings / problems etc. Keep it short.

In your final summary message report your done work.
Keep it also short. Overall keep your messages short.
Your final summary message is not only short but shorter.
Overall keep your messages shorter.

Keep comments in source code also short or shorter.

If you encounter a long source code comment then make it shorter.

Really do a "reduce to the max" without losing information.

# reference documents (RFCs etc.)

Cite standards always by lookup in the concrete document.
Locations:
- `docs/cipa` EXIF specs
- `docs/ecma` everything related to ECMAScript
- `docs/itu` for JPEG
- `docs/oasis` virtio specifications
- `docs/pcisig` some information related to PCI
- `docs/rfc` RFC documents
- `docs/ti` the 16550 serial controller
- `docs/w3c` documents from the web consortium
- `docs/whatwg` documents from WHATWG

If the subdirectory is missing read `rfc/README.md` to see how RFC documents are
handle and reflect the style.

IF the document is missing read `README.md` of the subdirectory and create
the file in the subdirectory (mandatory).

# build an check commands

before a `git commit` the check must exit with 0.
Do not take any shortcut so that exit 0 in the check is reached.

Every cargo call on macOS is run through wrappers in `tools/`.
`cargo` directly is not the nightly build.

- `sh tools/xtask.sh <subcommand>` e.g. `lint`, `test`, `doc`, `fuzz`, `--help` for help
- `sh tools/xtask-check.sh` — full check before every commit. Must exit with 0. Takes around 3 minutes when compiled

If `cargo fmt --all` fails use `cargo fmt -p <name>` to format one package (parallel sessions).

(Optional read for reason: `ER-1` in `extended-read.md`.

# Python

* `uv` is the only tool for managing packages or venvs
* do not create permanent python scripts
