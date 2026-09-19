# audhsos-der and audhsos-x509 audit findings

Repository: AuDHSOS/AuDHSOS. Audit of audhsos-der, audhsos-x509 at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #229
Title: audhsos-x509: RSA key size bound is applied to trust anchors only, not to intermediates or the leaf
Labels: bug, part::net
Body:
`MIN_RSA_BITS` at `crates/net/x509/src/algorithm.rs:16-23` is enforced by `check_usable` at `crates/net/x509/src/algorithm.rs:303-329`, whose only caller is `TrustAnchor::from_certificate` at `crates/net/x509/src/path.rs:103`. `verify_chain` verifies each link through `verify_signature` at `crates/net/x509/src/path.rs:181` and `crates/net/x509/src/certificate.rs:252-254`, which reaches `verify_rsa` at `crates/net/x509/src/algorithm.rs:379-396` and `RsaKey::new` at `crates/crypto/rsa/src/key.rs:41-50`; `RsaKey::new` accepts any odd modulus with its top bit set and no lower bound on its width. The leaf key at `crates/net/x509/src/path.rs:144-161` is never passed to `check_usable` either.

An intermediate certificate with a 512-bit or 1024-bit RSA key, signed by an anchor and signing the leaf, passes `verify_chain`; the comment at `crates/net/x509/src/algorithm.rs:20-22` ("no chain with such a key is ever walked") and `docs/11-cryptography-and-tls.md:28-31` ("for a key of two thousand and forty-eight to four thousand and ninety-six bits") both claim the opposite. A 512-bit modulus is factorable on commodity hardware, so a chain is only as strong as the weakest RSA key the anchor's policy lets an attacker obtain.

Fix: call `issuer.spki.check_usable()` in `check_authority` at `crates/net/x509/src/path.rs:220-243` and `end_entity.spki.check_usable()` at the start of `verify_chain`; moving the bound into `crypto-rsa` is what D-79 declined.

---

## F02 — issue #230
Title: audhsos-x509: README and error documentation list three signature algorithms while nine are verified
Labels: bug, part::net
Body:
`crates/net/x509/README.md:12-13` states "Signatures are ECDSA over P-256 with SHA-256 or SHA-384, and Ed25519." `crates/net/x509/src/error.rs:21-23` states an algorithm is "not one of the three this system verifies", and `crates/net/x509/src/algorithm.rs:289` states "the algorithm is one of the three". `SignatureAlgorithm` at `crates/net/x509/src/algorithm.rs:31-53` has nine variants, `SubjectPublicKey` at `crates/net/x509/src/algorithm.rs:198-214` has four key kinds, and `verify` at `crates/net/x509/src/algorithm.rs:361-368` accepts a P-384 key under `EcdsaSha256` and `EcdsaSha384`, whose doc comments at `crates/net/x509/src/algorithm.rs:32-35` name P-256 only.

A reader of the README or of the crate documentation judges the attack surface at three algorithms and one curve; six RSA schemes and P-384 are reachable from a network certificate.

Fix: rewrite the three sentences and the two variant comments to list ECDSA over P-256 and P-384 with SHA-256 or SHA-384, Ed25519, and RSA PKCS #1 v1.5 and PSS with SHA-256, SHA-384, or SHA-512.

---

## F03 — issue #231
Title: audhsos-x509: docs/11 section 11.9 describes an older API and denies the anchors the repository carries
Labels: bug, part::net
Body:
`docs/11-cryptography-and-tls.md:328` says the two time forms "become the `UnixTime`"; `read_time` at `crates/net/der/src/reader.rs:516` returns `CivilTime`. The code sketch at `docs/11-cryptography-and-tls.md:344-350` shows `not_before: UnixTime`, `signature: Signature<'a>`, and a two-variant `SubjectPublicKey`; the struct at `crates/net/x509/src/certificate.rs:68-99` has `validity: Validity`, `signature: &'a [u8]`, and `SubjectPublicKey` at `crates/net/x509/src/algorithm.rs:198-214` has four variants. `docs/11-cryptography-and-tls.md:363` says "`keyCertSign` on every CA"; `check_authority` at `crates/net/x509/src/path.rs:237-241` tests the bit only when the extension is present. `docs/11-cryptography-and-tls.md:370` says "The repository contains none" of trust anchors; `anchors/` holds five DER roots and `crates/net/x509/README.md:40-44` names them.

A reader of document 11 expects a `UnixTime` window and two key kinds and reads `verify_chain` as refusing a CA without a key usage extension; none of the three holds.

Fix: rewrite the sketch and the four sentences of section 11.9 against `certificate.rs`, `algorithm.rs`, and `path.rs`.

---

## F04 — issue #232
Title: audhsos-x509: every reader error on the version field is reported as NotVersionThree
Labels: bug, part::net
Body:
`parse_body` at `crates/net/x509/src/certificate.rs:130-132` and `TrustAnchor::from_certificate` at `crates/net/x509/src/path.rs:89-91` map every `DerError` of `read_context(0)` to `X509Error::NotVersionThree`. `read_context` fails with `Truncated`, `LengthOutOfRange`, `NonMinimalLength`, `IndefiniteLength`, or `TooDeep` as well as with `UnexpectedTag`.

A body starting with `A0 80` (indefinite length) or `A0 81 05` (non-minimal length) is reported as a version-one certificate rather than as an encoding error, so the `Encoding(DerError)` variant that names the rule is bypassed on this field alone.

Fix: map only `DerError::UnexpectedTag` and `DerError::EndOfInput` to `NotVersionThree` and pass every other error through `X509Error::from`.

---

## F05 — issue #233
Title: audhsos-x509: an anchor found by name ends the walk before an intermediate of the same name is tried
Labels: enhancement, part::net
Body:
`verify_chain` at `crates/net/x509/src/path.rs:170-175` returns the result of verifying under the first anchor whose subject equals the issuer; `find` at `crates/net/x509/src/path.rs:127-129` returns the first match only. An intermediate with that subject, and a second anchor with that subject and another key, is never tried.

An operator's table holds root `R` with key `K1`. The server sends the leaf signed by `K2` and a self-issued certificate `R'` (subject `R`, issuer `R`, key `K2`, signed by `K1`), which RFC 5280 section 4.2.1.9 and section 6.1.4 step (l) name as a self-issued intermediate. `verify_chain` returns `SignatureFailed` at `crates/net/x509/src/path.rs:174`; the path through `R'` is valid. The same happens for a table with two anchors of one subject and two keys.

Fix: try every anchor whose subject matches and continue to `next_issuer` when none verifies; the test at `crates/net/x509/src/tests/path.rs:566-597` keeps its result because it passes no intermediates.

---

## F06 — issue #234
Title: audhsos-x509: self-issued intermediates count against pathLenConstraint
Labels: enhancement, part::net
Body:
`verify_chain` at `crates/net/x509/src/path.rs:184` increments `below` for every issuer, and `check_authority` at `crates/net/x509/src/path.rs:231-236` compares it against the constraint. RFC 5280 section 4.2.1.9 defines `pathLenConstraint` as "the maximum number of non-self-issued intermediate certificates", and section 6.1.4 step (l) decrements only when "the certificate was not self-issued".

Chain: anchor `A` signs `I1` (subject `I`, `pathLen` 0), `I1` signs `I2` (subject `I`, issuer `I`, a key rollover), `I2` signs the leaf. At the second link `below` is 1 and `I1` carries limit 0, so `verify_chain` returns `PathLengthExceeded`; RFC 5280 accepts the path.

Fix: leave `below` unchanged at `crates/net/x509/src/path.rs:184` when `issuer.subject == issuer.issuer`.

---

## F07 — issue #235
Title: audhsos-x509: the leaf's digitalSignature bit is never checked and its accessor has no caller
Labels: enhancement, part::net
Body:
`KeyUsage::digital_signature` at `crates/net/x509/src/certificate.rs:59-63` has no caller in the workspace. `verify_chain` at `crates/net/x509/src/path.rs:155-161` checks the leaf's window, extended key usage, and name, and `crates/net/tls/src/client.rs` reads no key usage either. RFC 8446 section 4.4.2.2 at `docs/rfc/rfc8446.txt:3715-3717` requires the digitalSignature bit when the key usage extension is present.

A leaf whose `keyUsage` is `keyEncipherment` only passes `verify_chain` and its key then signs `CertificateVerify`.

Fix: in `verify_chain` return `NotForServerAuthentication` when `end_entity.key_usage` is present without bit zero; the alternative, deleting the accessor, drops a check RFC 8446 states.

---

## F08 — issue #236
Title: audhsos-x509: a wildcard over a single label matches every name under that label
Labels: enhancement, part::net
Body:
`matches_dns` at `crates/net/x509/src/name.rs:316-324` accepts any presented name of the form `*.<rest>` where `rest` is non-empty (`strip_wildcard` at `crates/net/x509/src/name.rs:336-339`) and requires the reference to carry exactly one more label. RFC 6125 section 7.2 lists a wildcard over "a so-called public suffix (e.g., *.co.uk or *.com)" as the case that "automatically vouch[es] for any and all host names".

A certificate with `dNSName` `*.com` matches `ServerName::Dns("example.com")`, and `*.test` matches every name of the test zone.

Fix: require at least two labels after the wildcard, so `rest` must contain a dot; a public-suffix list is the option not taken and needs a table this crate does not carry.

---

## F09 — issue #237
Title: audhsos-x509: the reference name is compared as bytes without the A-label conversion RFC 6125 requires
Labels: enhancement, part::net
Body:
`ServerName::Dns` at `crates/net/x509/src/name.rs:272-276` takes any `&str`, and `matches_dns` at `crates/net/x509/src/name.rs:309-333` compares the bytes with `eq_ignore_ascii_case`. RFC 6125 section 6.4.2 requires the client to "convert any U-labels in the domain name to A-labels before checking the domain name". No conversion exists in this crate or in the caller at `crates/net/tls/src/client.rs:849`, and the type carries no statement that the caller must pass A-labels.

`ServerName::Dns("bücher.example")` against a certificate carrying `xn--bcher-kva.example` returns `NameMismatch`, so an internationalized name can never be connected to.

Fix: state on `ServerName::Dns` that the name is ASCII A-labels and return `false` from `matches_dns` for a reference byte above `0x7F`; an IDNA encoder is the option not taken and needs the Unicode tables.

---

## F10 — issue #238
Title: audhsos-x509: a second authorityKeyIdentifier or unknown extension is accepted
Labels: enhancement, part::net
Body:
`read_extensions` at `crates/net/x509/src/certificate.rs:216-224` reads `authorityKeyIdentifier` for its shape and ignores an unknown non-critical extension without recording either. RFC 5280 section 4.2 states "A certificate MUST NOT include more than one instance of a particular extension", naming the authority key identifier as the example. Only the five recorded extensions are refused on repetition at `crates/net/x509/src/certificate.rs:256-302`.

A certificate carrying two `authorityKeyIdentifier` extensions parses.

Fix: record `authorityKeyIdentifier` as a `bool` on `Certificate` and refuse the second like the others; refusing repeated unknown identifiers would need a comparison of every pair, O(n²) over the extension count.

---

## F11 — issue #239
Title: audhsos-x509: an empty Extensions or GeneralNames sequence is accepted
Labels: enhancement, part::net
Body:
`parse_body` at `crates/net/x509/src/certificate.rs:170-176` and the loop at `crates/net/x509/src/certificate.rs:183` accept an extensions sequence with no element; `set_subject_alt_name` at `crates/net/x509/src/certificate.rs:288-292` accepts an empty `GeneralNames`. RFC 5280 section 4.1.2.9 defines `Extensions ::= SEQUENCE SIZE (1..MAX) OF Extension` and section 4.2.1.6 defines `GeneralNames ::= SEQUENCE SIZE (1..MAX) OF GeneralName`.

A body ending in `A3 02 30 00`, or a `subjectAltName` value of `30 00`, parses; a strict parser refuses both, so two systems read the certificate differently, which `crates/net/der/README.md:10-13` names as the reason for strictness.

Fix: return `X509Error::Encoding(DerError::EndOfInput)` when `extensions` or `names` is empty.

---

## F12 — issue #240
Title: audhsos-x509: every accepted link is verified twice
Labels: enhancement, part::net
Body:
`next_issuer` at `crates/net/x509/src/path.rs:200` verifies the signature of `current` under a candidate, and `verify_chain` at `crates/net/x509/src/path.rs:181` verifies the same signature under the returned issuer again.

A chain of `k` links costs `2k` signature verifications instead of `k`; for RSA-4096 each is one modular exponentiation with a 4096-bit modulus.

Fix: delete the call at `crates/net/x509/src/path.rs:181`, whose result `next_issuer` already established.

---

## F13 — issue #241
Title: audhsos-x509: eight intermediates are admitted but can never reach an anchor
Labels: enhancement, part::net
Body:
`verify_chain` at `crates/net/x509/src/path.rs:151-154` refuses more than `MAX_CHAIN` intermediates. The loop at `crates/net/x509/src/path.rs:169-186` runs `MAX_CHAIN` times and each iteration either returns at an anchor or consumes one intermediate, so with eight intermediates the eighth is consumed in the last iteration and its issuer is never looked up.

A server that sends eight intermediates costs eight signature verifications (sixteen with F12) before `ChainTooLong` at `crates/net/x509/src/path.rs:186`, whatever the chain contains.

Fix: refuse `intermediates.len() >= MAX_CHAIN` at `crates/net/x509/src/path.rs:151`.

---

## F14 — issue #242
Title: audhsos-x509: KeyUsage::has wraps the position above fifteen
Labels: enhancement, part::net
Body:
`KeyUsage::has` at `crates/net/x509/src/certificate.rs:48-51` computes `0x8000u16.wrapping_shr(position)`; `wrapping_shr` reduces the shift modulo 16.

`has(16)` reports bit zero and `has(21)` reports `keyCertSign`. The method is public and takes any `u8`.

Fix: use `checked_shr` and return `false` when it yields `None`.

---

## F15 — issue #243
Title: audhsos-x509: X509Error::BadName has no producer
Labels: enhancement, part::net
Body:
`X509Error::BadName` at `crates/net/x509/src/error.rs:39-40` is constructed nowhere in `crates/net/x509/src` or `crates/net/tls/src`; `matches` at `crates/net/x509/src/name.rs:285-296` returns `Encoding` for a malformed extension and `false` for a name it cannot match.

The variant and its message at `crates/net/x509/src/error.rs:92` are dead code, and a caller matching on it writes an arm that never runs.

Fix: remove the variant.

---

## F16 — issue #244
Title: audhsos-der: Tag::context silently masks a number above thirty
Labels: enhancement, part::net
Body:
`Tag::context` at `crates/net/der/src/tag.rs:155-158` computes `number & HIGH_FORM`, so `context(32, c)` equals `context(0, c)` and `context(31, c)` is the high tag number form that `read_any` at `crates/net/der/src/reader.rs:307-309` refuses.

A caller passing 32 reads the `[0]` value without an error.

Fix: return `Option<Tag>` and `None` for a number above thirty.

---

## F17 — issue #245
Title: audhsos-x509: docs/rfc lacks RFC 5280 and RFC 6125, which the crate implements
Labels: enhancement, part::net
Body:
`docs/rfc/README.md:3-5` says the directory holds "the standards this system implements". `crates/net/x509/src/path.rs:6`, `crates/net/x509/src/name.rs:4`, `crates/net/der/src/time.rs:574`, and `docs/11-cryptography-and-tls.md:45-46` implement and cite RFC 5280 and RFC 6125; neither file is in `docs/rfc/` nor in the list of `docs/rfc/fetch.sh:10-13`.

A section cited in this crate cannot be checked against its source without a network, which is what the directory exists to allow.

Fix: add 5280 and 6125 to `docs/rfc/fetch.sh` and to the table of `docs/rfc/README.md`.
