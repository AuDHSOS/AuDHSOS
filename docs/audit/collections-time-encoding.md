# audhsos-collections, audhsos-time, audhsos-encoding audit findings

Repository: AuDHSOS/AuDHSOS. Audit of audhsos-collections, audhsos-time, audhsos-encoding at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #387
Title: audhsos-encoding: PEM decoder writes into the output buffer before it reports BufferTooSmall
Labels: bug
Body:
`EncodingError::BufferTooSmall` is documented at `crates/encoding/src/error.rs:12-15` as "Nothing was written: a function of this crate either writes its whole answer or none of it." `pem::decode` and `pem::decode_wrapped` decode the body one quantum at a time in `Quanta::push` at `crates/encoding/src/pem.rs:363-393`; each quantum is written into `out[written..]` at `crates/encoding/src/pem.rs:379-385` before the next one is examined, and the buffer is checked only per quantum. `base64::decode` (`crates/encoding/src/base64.rs:109-114`), `hex::decode` (`crates/encoding/src/hex.rs:66-69`) and `pem::encode_wrapped` (`crates/encoding/src/pem.rs:131-135`) check the whole size before writing.

`pem::decode(b"-----BEGIN X-----\nAQIDBAUGBwgJ\n-----END X-----\n", &mut [0u8; 4])` returns `Err(BufferTooSmall)` and leaves `[1, 2, 3, 0]` in the buffer. A caller that reuses `out` after an error, as the doc comment of the error permits, reads a partial block as data. The test at `crates/encoding/src/tests/pem.rs:168-172` asserts the error and does not check the buffer.

Fix: check `out.len()` against `base64::decoded_len` of the body character count (the body span is known at `crates/encoding/src/pem.rs:278-287`, its newlines subtracted) before the first quantum is written; the option not taken is to amend the doc comment of `BufferTooSmall` to exclude the PEM decoders, which leaves the partial write in place.

---

## F02 — issue #388
Title: audhsos-encoding: decode_wrapped refuses a text encode_wrapped wrote at an odd width
Labels: bug
Body:
`Quanta::take` at `crates/encoding/src/pem.rs:353-355` returns `EncodingError::Padding` for a pad character on any body line but the last. `encode_wrapped` at `crates/encoding/src/pem.rs:141-146` writes the Base64 body in one piece and splits it at `wrap` characters, so a body whose length modulo `wrap` is one and whose pad is two characters places the first pad on the second-to-last line. `decode_wrapped` is documented at `crates/encoding/src/pem.rs:218-222` as reading back "A text written by `encode_wrapped` at that width". Every even width, including 64 and 70, keeps the pad on the last line because the body length is a multiple of four.

`encode_wrapped("X", &[1], NonZeroUsize::new(3), out)` writes `-----BEGIN X-----\nAQ=\n=\n-----END X-----\n`; `decode_wrapped` of that text at the same width returns `Err(Padding)`. RFC 7468 section 3, Figure 1, rule `base64finl`, permits `base64pad *WSP eol base64pad` ("...AB= <EOL> = <EOL> is not good, but is valid").

Fix: for `exact == false` drop the per-line pad check in `Quanta::take` and rely on `Quanta::push`, which already refuses a character after a padded quantum at `crates/encoding/src/pem.rs:364-366`; the option not taken is to reject odd widths in `encode_wrapped`, which keeps a valid RFC 7468 text unreadable.

---

## F03 — issue #389
Title: audhsos-encoding: PEM decoder refuses a text whose line terminator is a lone carriage return
Labels: bug
Body:
`Lines::next` at `crates/encoding/src/pem.rs:474-489` splits the input at `\n` only (line 478) and strips one `\r` before it. RFC 7468 section 3 defines `eol = CRLF / CR / LF` in Figure 1 and reuses `eol` in the strict form of Figure 3; section 2 states that parsers "MUST handle different newline conventions". `crates/encoding/README.md:16-18` says the crate follows the strict form of RFC 7468.

`pem::decode(b"-----BEGIN X-----\rQUJD\r-----END X-----\r", out)` returns `Err(Label)`: the whole text is one line, `framed` at `crates/encoding/src/pem.rs:418-421` strips `-----BEGIN ` and `-----`, and `check_label` at `crates/encoding/src/pem.rs:426-442` rejects the `\r` and the hyphens of the remainder. The error names the label, not the terminator.

Fix: in `Lines::next` split at the first of `\r` or `\n` and consume a `\n` that directly follows a `\r`; the option not taken is to document CR-only texts as unsupported, which contradicts the MUST in RFC 7468 section 2.

---

## F04 — issue #390
Title: audhsos-encoding: PEM label check refuses a hyphen between label characters
Labels: bug
Body:
`check_label` at `crates/encoding/src/pem.rs:436` accepts `0x21..=0x2C | 0x2E..=0x7E` and a single space between words, and returns `EncodingError::Label` for `0x2D`. RFC 7468 section 3, Figure 1, defines `label = [ labelchar *( ["-" / SP] labelchar ) ]`: a single hyphen-minus between two label characters is valid, and section 2 states that labels "do not contain consecutive spaces or hyphen-minuses, nor do they contain spaces or hyphen-minuses at either end". The doc comment at `crates/encoding/src/pem.rs:423-425` ("printable ASCII without the hyphen"), the test at `crates/encoding/src/tests/pem.rs:144` (`"WITH-HYPHEN"`), and `docs/06-testing-strategy.md:1249-1251` encode the narrower rule.

`pem::decode(b"-----BEGIN A-B-----\nQUJD\n-----END A-B-----\n", out)` returns `Err(Label)`; `pem::encode("A-B", ...)` returns `Err(Label)`. A file whose label carries a hyphen is refused although it is valid under the strict form the README claims.

Fix: treat `-` in `check_label` exactly as a space is treated (single, between two label characters, not at either edge), and update the doc comment, the test at `crates/encoding/src/tests/pem.rs:144`, and `docs/06-testing-strategy.md:1250`; the option not taken is to keep the narrower rule and state the deviation from RFC 7468 section 3 in the README.

---

## F05 — issue #391
Title: audhsos-encoding: docs/12 names pem::decode as the reader of the SSH key while the xtask calls decode_wrapped
Labels: bug
Body:
`docs/12-parallel-work.md:154-157` states that "the Secure Shell interop run reads its key material through `pem::decode`". `crates/tools/xtask/src/ssh.rs:228-232` calls `pem::decode_wrapped` at `OPENSSH_WRAP`, and `crates/tools/xtask/src/anchors.rs:102-107` calls `pem::decode_wrapped` as well. No caller outside `crates/encoding` calls `pem::decode`.

A reader of docs/12 who looks for the consumer of the strict reader finds none; `pem::decode` at `crates/encoding/src/pem.rs:211-216` has callers only in its own tests.

Fix: change `docs/12-parallel-work.md:155` to `pem::decode_wrapped` and state that both xtask consumers use the wrapped reader.

---

## F06 — issue #392
Title: audhsos-collections: IndexList derives Copy and Clone, so a copied header tears the list it names
Labels: enhancement
Body:
`IndexList` at `crates/collections/src/index_list.rs:95-105` derives `Clone` and `Copy`. The header holds `head`, `tail`, and `len` of a list whose membership lives in the caller's `[Link]`; two headers with one `id` over one slice pass the ownership checks at `crates/collections/src/index_list.rs:226-229` and `crates/collections/src/index_list.rs:265-268` for the same nodes.

Sequence with `links = [Link::new(); 4]` and `list = IndexList::new(7)`: `list.push_back(&mut links, 0)`; `let mut copy = list; copy.pop_front(&mut links)` returns `Some(0)`; `list.pop_front(&mut links)` returns `None` while `list.head()` is `Some(0)` and `list.len()` is 1; `list.push_back(&mut links, 1)` succeeds, sets `links[0].next = 1` through `crates/collections/src/index_list.rs:197` on a link whose `owner` is `NONE`, and `list.iter(&links)` yields `[0, 1]` while `list.contains(&links, 0)` is false and `list.unlink(&mut links, 0)` is `NotLinked`. Every step compiles without a warning because the copy is implicit.

Fix: remove `Clone` and `Copy` from the derive at `crates/collections/src/index_list.rs:95`, keeping `Debug`, `PartialEq`, `Eq`, and `Hash`; the option not taken is a documented rule against copying, which the compiler cannot check.

---

## F07 — issue #393
Title: audhsos-collections: IndexList::unlink wraps len below zero when two lists share an identifier
Labels: enhancement
Body:
`IndexList::unlink` at `crates/collections/src/index_list.rs:264-282` accepts any node whose `owner` equals `self.id` and decrements `len` with `wrapping_sub` at `crates/collections/src/index_list.rs:280`. `IndexList::new` at `crates/collections/src/index_list.rs:114-124` refuses only `NONE` as an identifier; two lists over one slice may carry the same identifier, and neither the README (`crates/collections/README.md:12-15`) nor the module doc (`crates/collections/src/index_list.rs:13-16`) states that identifiers are distinct per slice.

With `a = IndexList::new(1)`, `b = IndexList::new(1)`, and `a.push_back(&mut links, 0)`, the call `b.unlink(&mut links, 0)` returns `Ok(())`, sets `b.len()` to 4294967295, and leaves `a.head()` naming node 0 whose `owner` is now `NONE`. `b.iter(&links)` at `crates/collections/src/index_list.rs:369-382` reads `left = 4294967295` and stops only because `at` is `NONE`.

Fix: replace `wrapping_sub` at `crates/collections/src/index_list.rs:280` with `checked_sub` and return `CollectionError::NotLinked(node)` when `len` is zero, and state in the module doc that identifiers are distinct per slice; the option not taken is a registry of identifiers, which the crate has no storage for.

---

## F08 — issue #394
Title: audhsos-collections: push drops the value on Full while the doc says the caller keeps it
Labels: enhancement
Body:
`ArrayVec::push` at `crates/collections/src/array_vec.rs:68-76` takes `value: T` by move and returns `Err(CollectionError::Full)` without the value; the doc comment at `crates/collections/src/array_vec.rs:66-67` says "The value is returned to nobody: a caller that needs it back keeps it." `RingBuffer::push` at `crates/collections/src/ring.rs:74-83` and `IndexMap::insert` at `crates/collections/src/index_map.rs:79-82` behave the same. A caller can keep a `T` that is not `Clone` only by not pushing it.

`ArrayVec<Buffer<BYTES>, SLOTS>::push` at `crates/net/ip/src/fragment.rs:461` moves an owned value; every caller today constructs the value fresh, so nothing is lost. A future caller that pushes a handle or an owned frame into a full container loses it on `Err(Full)`.

Fix: return the refused value in the error, for example `Result<(), (T, CollectionError)>` for the three owning containers, and reword the doc comment; the option not taken is a `T: Clone` bound, which the `Option<T>` storage was chosen to avoid.

---

## F09 — issue #395
Title: audhsos-collections: IndexList has no caller and the README names the kernel queues as its use
Labels: enhancement
Body:
`crates/collections/README.md:9-12` describes `IndexList` as "what a run queue over a fixed array of threads and an endpoint wait queue over a fixed array of blocked threads both are (D-48)", and `docs/12-parallel-work.md:178-181` says the same. `docs/09-decisions.md:84` (D-74) records that the kernel queues are intrusive lists over `Thread` fields and not `IndexList`, and `docs/09-decisions.md:141` (D-131) records the same for the deadline list. No crate outside `crates/collections` names `IndexList` (`grep -rn IndexList crates --include=*.rs` matches only `crates/collections`), and no crate calls `IndexList::new`.

A reader of the README looks for the kernel run queue in this crate and finds it in `crates/kernel/sched/src/scheduler.rs` and `crates/kernel/objects/src/wait_queue.rs` instead. `IndexList::insert_after` at `crates/collections/src/index_list.rs:216-239` was added for the deadline list of D-120, which D-131 then placed elsewhere.

Fix: reword `crates/collections/README.md:9-12` and `docs/12-parallel-work.md:178-181` to cite D-74 and the next intended caller (the bus enumeration list D-131 names), and keep the type; the option not taken is to remove the type, which D-131 names as needed later.

---

## F10 — issue #396
Title: audhsos-time: seconds_of_day doc claims an error for any field out of range but the date fields are not checked
Labels: enhancement
Body:
`CivilTime::seconds_of_day` at `crates/time/src/civil.rs:137-152` checks `hour`, `minute`, and `second`; its doc comment at `crates/time/src/civil.rs:134-136` says "`TimeError` for the first field that is out of range", which follows the wording of `validate` at `crates/time/src/civil.rs:103-108` where "field" covers all six.

`CivilTime { year: 2000, month: 13, day: 40, hour: 1, minute: 0, second: 0 }.seconds_of_day()` returns `Ok(3600)`. `UnixTime::from_civil` at `crates/time/src/unix.rs:45-53` validates the date through `days_from_civil` first, so no caller in the crate reaches the gap.

Fix: reword the doc comment to "for the first of `hour`, `minute`, and `second` that is out of range"; the option not taken is to call `validate` first, which makes the function depend on the year range for a value that has no year in it.

---

## F11 — issue #397
Title: audhsos-encoding: pem::decode is documented as reading the first block while a second block is TrailingData
Labels: enhancement
Body:
The doc comments at `crates/encoding/src/pem.rs:198` and `crates/encoding/src/pem.rs:218` say "Reads the first PEM block of `input`" and "Reads the first block of `input`". The loop at `crates/encoding/src/pem.rs:272-276` returns `EncodingError::TrailingData` for any non-empty line after the end line, which a second block starts with. RFC 7468 section 2 states "Files MAY contain multiple textual encoding instances. This is used, for example, when a file contains several certificates."

`pem::decode` of two concatenated blocks returns `Err(TrailingData)`; a certificate chain file in the form RFC 7468 section 2 describes cannot be read by either decoder. The README at `crates/encoding/README.md:16-21` states the refusal as a decision; the doc comments state the opposite.

Fix: change both doc comments to "Reads the one PEM block of `input`" and name `TrailingData` for a second block; the option not taken is a `decode_next` that returns the rest of the input after the end line, which would make chain files readable.

---

## F12 — issue #398
Title: audhsos-encoding: docs/rfc holds neither RFC 4648 nor RFC 7468
Labels: enhancement
Body:
`docs/rfc/README.md:3-5` states that the directory holds "The standards this system implements, kept verbatim so that a vector can be checked against its source without a network". `crates/encoding/src/base64.rs:4` implements RFC 4648 and `crates/encoding/src/pem.rs:4` implements RFC 7468; `find docs -iname '*4648*' -o -iname '*7468*'` returns nothing.

The test module `crates/encoding/src/tests/pem.rs:4-5` is titled "PEM against RFC 7468" and the vectors at `crates/encoding/src/tests/base64.rs:13` cite RFC 4648 section 10; neither can be checked against its source in the repository, and F02, F03, and F04 cite sections a reviewer must fetch.

Fix: add `rfc4648.txt` and `rfc7468.txt` through `docs/rfc/fetch.sh` and record both in the table of `docs/rfc/README.md`.
