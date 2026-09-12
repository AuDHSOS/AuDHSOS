# Reference documents: OpenSSH

What OpenSSH specifies and no standards body did. Two things are here.
The cipher this system's Secure Shell client encrypts with,
`chacha20-poly1305@openssh.com`, is the one algorithm of
[document 14](../14-secure-shell-as-a-client.md), section 14.5, that was
written down in a file of the OpenSSH source rather than by a standards
body; OpenSSH has since replaced that file with a pointer to an IETF
draft, and both documents are kept. The other is `PROTOCOL.key`, the
container a private key is written into, which the client has to read
before it can authenticate with a key of its own.

Every file is verbatim and carries its checksum, so that a constant can
be checked against its source without a network and so that the source
cannot change under the crate that cites it. This is
[`docs/rfc/`](../rfc/README.md) for a body that is not a standards body,
under the same rule and for the same reason (D-59, D-100, D-124).

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies and is
untouched: `Cargo.lock` still lists only workspace members.

## What is here

| File | Document | Retrieved | Bytes | Lines | SHA-256 |
|------|----------|-----------|-------|-------|---------|
| `PROTOCOL.chacha20poly1305` | *chacha20-poly1305@openssh.com authenticated encryption cipher*, OpenSSH, `$OpenBSD: PROTOCOL.chacha20poly1305,v 1.5 2020/02/21 00:04:43 dtucker Exp $` | 2026-09-11, at the commit named below | 4631 | 107 | `360c459325d5b5b683b6ddaf26a5a45af2dbf32eef95cdefe83dd25a40e34e5e` |
| `PROTOCOL.key` | *This document describes the private key format for OpenSSH*, OpenSSH, `$OpenBSD: PROTOCOL.key,v 1.4 2024/03/30 05:56:22 djm Exp $` | 2026-09-12, at the commit named below | 1618 | 71 | `8479c767aeb256e4b2465d23f96dd98998d4210f12c0c9b41bd0a6f1f2e6f17c` |
| `draft-ietf-sshm-chacha20-poly1305-04.txt` | *Secure Shell (SSH) authenticated encryption cipher: chacha20-poly1305*, D. Miller, S. G. Tatham, S. Josefsson, Internet-Draft, IETF `sshm` working group, 26 May 2026 | 2026-09-11 | 33112 | 840 | `a8aa91696db885d22c0a64541c295f7c547fc8d025c0eca0045823266ada1de1` |

## Why the cipher file is a revision and not the current one

It is not in the current source. OpenSSH removed it on 2025-08-05, in
commit `6ebd472c391a73574abe02771712d407c48e130d`, whose message says
why:

> a bunch of the protocol extensions we support now have RFCs
>
> and I-Ds that are more complete and detailed than what we have in the
> PROTOCOL.\* files. Refer to these when possible instead of documenting
> them here.

What is kept is the file as it stood at the parent of that commit,
`ec3465f59c651405e395092f3ad606f8992328d8`, which is revision 1.5 and
the last one there was. Section 1.7 of the `PROTOCOL` file that remains
in the source carries a pointer to the draft and nothing else.

The file is kept even so, because it is where the algorithm name and the
construction come from, because implementations on the wire today were
written against it, and because a name with `@openssh.com` in it is the
local namespace of RFC 4251, section 6 — the owner of that domain is the
authority on what it means.

## Why the draft is here and not in `docs/rfc/`

It is not an RFC. An Internet-Draft says of itself that it is a working
document, valid for at most six months, and that it is inappropriate to
cite it other than as work in progress. That is what the client is
written against for this one algorithm, so it is stated rather than
hidden: revision 04 is in working group last call in the `sshm` working
group of the IETF, and the three authors are OpenSSH, PuTTY, and an
author of RFC 8731 and RFC 8032, both of which this client also reads.

A numbered revision is fixed. `draft-ietf-sshm-chacha20-poly1305-04.txt`
is archived at its address and cannot be replaced; a later revision gets
a later number. That is the property this directory needs, and the
version number is in the file name so that a citation says which text it
read.

When the draft becomes an RFC, that RFC belongs in `docs/rfc/` under
D-59, and this copy goes away with the citations that point at it. The
OpenSSH file stays where it is.

## What `PROTOCOL.key` is

`openssh-key-v1`, the file `ssh-keygen` writes and `ssh` reads. One
structure inside a PEM-style armor: the magic string, a cipher name, a
KDF name and its options, a count, that many public keys, and one string
holding every private key after the cipher has been applied. Inside that
string are two `checkint` words, then one private key and one comment per
key, padded with the bytes 1, 2, 3, … up to the cipher's block size.

Three of its details are the kind that are wrong when guessed. The
padding counts upward from one; it is not a length byte repeated. The
`checkint` pair is one random value written twice, and decryption is
judged right when the two halves match, so it is not a checksum over
anything. And a key with no passphrase is the same structure and not a
simpler one: the cipher is `none`, the KDF is `none`, and that KDF's
options are the empty string.

Section 3 stops short of the per-key encoding and defers to the rules of
the SSH agent. That document is RFC 9987, in
[`docs/rfc/`](../rfc/README.md); its section 5.2.3 is the Ed25519 case,
`string "ssh-ed25519"`, `string ENC(A)`, `string k || ENC(A)`, with the
public key repeated inside the private blob to stay compatible with what
is deployed.

Unlike the cipher file this one is current. Revision 1.4 of 2024-03-30 is
the last there is, the copy here is the file at
`2d2c068de8d696fe3246f390b146197f51ea1e83`, the commit that made that
revision, and the file at `master` on the day it was fetched is the same
bytes.

## What the client takes from the cipher documents

The two documents agree, and the draft is the longer of the two because
it restates ChaCha20 and Poly1305 rather than referring to them.

- The key material: 512 bits from the key derivation of RFC 4253,
  section 7.2, split into `K_2` first and `K_1` second.
- Two cipher instances: `K_1` encrypts the four-byte packet length,
  `K_2` encrypts the packet. The separation is what keeps the length
  confidential without making the length a decryption oracle for the
  payload.
- The nonce is the packet sequence number as a `uint64` under the SSH
  wire encoding; the Poly1305 key is the first 256 bits of the `K_2`
  stream at block counter zero, and the payload is encrypted from block
  counter one.
- The tag covers the encrypted length and the encrypted payload, and it
  is checked before anything is decrypted.
- Negotiation: selecting this cipher selects no MAC, and the MAC
  name-lists are ignored.
- Rekeying: the bound of the cipher is far above the one RFC 4253
  recommends, so the recommendation is what the client follows.
- Appendix A of the draft is a worked example: one `SSH_MSG_CHANNEL_DATA`
  packet with its padding, the sequence number it was sent under, the
  64 bytes of key material, and the bytes that went on the wire. It is
  the only published vector this track has for a whole packet, and under
  D-40 it is transcribed rather than recomputed.

## How to check these copies

    https://raw.githubusercontent.com/openssh/openssh-portable/ec3465f59c651405e395092f3ad606f8992328d8/PROTOCOL.chacha20poly1305
    https://www.ietf.org/archive/id/draft-ietf-sshm-chacha20-poly1305-04.txt
    https://raw.githubusercontent.com/openssh/openssh-portable/2d2c068de8d696fe3246f390b146197f51ea1e83/PROTOCOL.key

Fetch each and compare against the checksum above. Both were fetched
twice on 2026-09-11 and both fetches were identical; the draft was also
fetched from `https://datatracker.ietf.org/doc/id/` and that copy is the
same bytes again. The OpenSSH file is also in the OpenBSD source tree as
`src/usr.bin/ssh/PROTOCOL.chacha20poly1305`, revision 1.5, which is where
the portable repository takes it from.

## Terms

The two OpenSSH files are part of a distribution whose `LICENCE`
summarizes itself as "all components are under a BSD licence, or a
licence more free than that". Neither carries a notice of its own, and
nothing in those terms restricts a verbatim copy.

The draft carries

> Copyright (c) 2026 IETF Trust and the persons identified as the
> document authors. All rights reserved.

and is subject to BCP 78 and the IETF Trust's Legal Provisions, which
permit a complete and unmodified copy. It is kept whole and unmodified,
so the notice travels inside the copy. Code components extracted from it
would carry the Revised BSD License; nothing here extracts any.

All three are therefore the first case of D-124: documents whose licence
permits the copy.

## Who cites it

`audhsos-ssh`, step S3 of track S ([the roadmap](../08-roadmap.md),
section 8.26): the cipher over the binary packet, and the tests of the
catalog entry that owns it. Document 14 names the cipher in section 14.5
and its vector in section 14.12.

Nothing cites `PROTOCOL.key` yet. Step S5 signs with a key of this
client's own, and 14.13 holds the open question of where that key comes
from; whatever answers it reads a file in this format, and the document
is here before the code that will.
