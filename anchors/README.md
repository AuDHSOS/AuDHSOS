# anchors

The trust anchors a build puts into the image, and the ones `tls-probe`
pins on the development machine.

Every file here is one certificate authority's root certificate, in DER or
in PEM. `cargo xtask image` reads the directory, sorts the files by name,
and writes them as one table onto the boot volume as
`AUDHSOS/ANCHORS.BIN`; `audhsos-x509::anchors` is both the writer and the
reader of that table, so the tool and the program cannot disagree about
its shape.

An operator adds a root by dropping the file in and building the image
again (D-42, D-147). A build with no directory and a build with an empty
one both write a table of no anchors, and a program that finds one refuses
every chain rather than trusting the next best thing.

## What is here

| File | Root | Reached by |
|------|------|------------|
| `gts-root-r1.der` | GTS Root R1, RSA 4096 | `google.de` and the rest of Google Trust Services |
| `gts-root-r4.der` | GTS Root R4, P-384 | the chain `google.de` presented before it moved to R1 |
| `isrg-root-x1.der` | ISRG Root X1, RSA 4096 | Let's Encrypt, among them `www.ietf.org` and `www.rust-lang.org` |
| `isrg-root-x2.der` | ISRG Root X2, P-384 | Let's Encrypt's elliptic-curve chain |
| `globalsign-root-r46.der` | GlobalSign Root R46, RSA 4096 | `www.bbc.co.uk` |

These five are public documents: a root certificate is the authority's own
published bytes, carries no secret, and is what every browser ships. They
are tracked so that a checkout can reach a real server without an operator
first fetching anything, which is the one point on which D-147 amends
D-42.

## Checking a file

The bytes of a root are published by the authority. To see what one holds
without trusting this directory:

```sh
openssl x509 -in anchors/isrg-root-x1.der -inform der -noout -subject -fingerprint -sha256
```

The fingerprint is what an authority prints on its own site; nothing in
this repository verifies it for you, and nothing can — a root is trusted
because a person decided to trust it.
