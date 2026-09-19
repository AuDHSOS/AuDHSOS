# audhsos-deflate and audhsos-symbols audit findings

Repository: AuDHSOS/AuDHSOS. Audit of audhsos-deflate, audhsos-symbols at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #412
Title: audhsos-symbols: a version 5 line table with a zero-pair entry format loops on its declared count
Labels: bug
Body:
`Format::read` accepts an entry format that declares zero content type and form pairs and returns `Format { count: 0 }` in `crates/symbols/src/line.rs:574-592`. `entry_v5` then reads the entry count as an unbounded `uleb` and loops `while number < count` in `crates/symbols/src/line.rs:551-559`, and `skip_table` loops the same way in `crates/symbols/src/line.rs:680-687`. Each iteration calls `read_entry`, whose `for (kind, form) in self.pairs.iter().take(self.count)` reads nothing when `self.count` is zero, so the cursor does not advance and the loop makes no progress toward its end in `crates/symbols/src/line.rs:603-610`.

A crafted `.debug_line` version 5 unit that sets the `directory_entry_format_count` byte to 0 and follows it with a `directories_count` ULEB of `0xFFFFFFFFFFFFFFFF` drives `skip_table` through 2^64 no-op iterations, which does not return in any usable time. `LineProgram::row_for` reaches this table for any address the unit's program covers, so a single lookup on an attacker-supplied ELF file hangs the symbolizer. `SymbolError::Truncated` never fires because no read is attempted.

Fix: in `Format::read`, return an error when `declared` is 0, since a table entry format with no pairs describes no entry; the alternative of bounding `count` by `cursor.remaining()` still walks a large loop and does not reject the malformed format.

---

## F02 — issue #413
Title: audhsos-symbols: nested v0 backreferences expand a short symbol name into exponential parse work
Labels: bug
Body:
The `v0` demangler re-parses the target of every backreference without memoizing, in `Parser::at_backref` at `crates/symbols/src/demangle.rs:310-320`, called from the `B` arms of `kind_inner` (`crates/symbols/src/demangle.rs:478-482`), `path_inner` (`crates/symbols/src/demangle.rs:370-373`), and `constant` (`crates/symbols/src/demangle.rs:529-531`). A tuple type reached through `kind_inner` parses each element in turn (`crates/symbols/src/demangle.rs:455-467`), so a type that is a two-element tuple of two backreferences to an earlier two-element tuple doubles the parse work at each chain level. `MAX_DEPTH` caps nesting at 64 in `crates/symbols/src/demangle.rs:20,195-206`, and each chain hop costs about two depth levels, so the parser performs on the order of 2^32 operations before the cap stops it.

A symbol name of a few hundred bytes that chains about thirty such tuple definitions through backreferences forces roughly 2^32 parse steps, run twice because `Demangled::fmt` parses once into `Discard` and once into the formatter (`crates/symbols/src/demangle.rs:44-52`), which occupies the symbolizer for seconds to minutes on one untrusted name. The module doc claims the bounded nesting stops the parser from looping or overflowing the stack, and that claim holds, but it does not bound the time.

Fix: carry a shared operation counter through the parser and fail once it exceeds a limit proportional to the input length, rather than relying on `MAX_DEPTH` alone, which bounds nesting depth but not fan-out reuse.

---

## F03 — issue #414
Title: audhsos-deflate: the decoder does not reject over-subscribed or incomplete Huffman code lengths
Labels: bug
Body:
`Tree::new` builds a decode tree from code lengths by counting them, with no check that the lengths form a complete code and do not over-subscribe the bit-length space, in `crates/deflate/src/huffman.rs:439-462`. `Tree::decode` walks the counts and returns whatever symbol the bits select, guarding only the array bound and the sign of the index in `crates/deflate/src/huffman.rs:464-483`. RFC 1951 section 3.2.2 defines a code only for lengths whose Kraft sum equals one, except the single distance code of section 3.2.5 which is a length-one code with one unused code.

A dynamic block whose literal or distance code lengths over-subscribe the space (Kraft sum above one) is accepted, and `decode` returns symbols that a conformant decoder rejects, so `decompress` reports success on a stream another decoder refuses and yields the wrong bytes. A block whose lengths are incomplete (Kraft sum below one) is likewise accepted, and fails only later with `Error::Input` if the compressed data uses one of the missing codes.

Fix: in `Tree::new`, track the running Kraft sum and return an error on over-subscription and on an incomplete code, while allowing the one-symbol length-one distance code that section 3.2.5 permits; the alternative of leaving validation to `decode` accepts malformed tables that decode without error.

---

## F04 — issue #415
Title: audhsos-deflate: the zlib decoder does not reject a CINFO above seven
Labels: bug
Body:
`unwrap` checks the compression method, the FCHECK modulus, and the FDICT flag, and never reads the CINFO field in bits 4 to 7 of the first header byte, in `crates/deflate/src/zlib.rs:222-236`. RFC 1950 states that for CM equal to 8 the values of CINFO above 7 are not allowed.

A zlib stream whose first byte is `0xF8` (CM 8, CINFO 15) with a matching FCHECK passes every check and is decoded, although it declares a window larger than the format allows. The decoder keeps the whole output rather than a sliding window, so the wrong CINFO changes no output, but a stream the specification forbids is accepted.

Fix: reject a first header byte whose high nibble exceeds 7, next to the existing method check in `unwrap`.

---

## F05 — issue #416
Title: audhsos-deflate: the decoder rebuilds the fixed Huffman trees for every fixed block
Labels: enhancement
Body:
`inflate` constructs `Tree::<LITERALS>::new` over 288 symbols and `Tree::<DISTANCES>::new` over 32 symbols on each iteration that reads a fixed block, in `crates/deflate/src/decode.rs:44-48`. `Tree::new` costs O(MAX_BITS * N) because it scans the lengths once per bit length, in `crates/deflate/src/huffman.rs:441-461`, so each fixed block pays about 4600 operations to rebuild a table that never changes.

An input of empty final-less fixed blocks, each ten bits long, forces one tree rebuild per 1.25 bytes of input, which multiplies decode work by about three thousand over the input size. The trees are identical for every fixed block.

Fix: build the two fixed trees once before the block loop and pass references into `block`, as the two fixed-length tables are constants.

---

## F06 — issue #418
Title: audhsos-deflate: Codes::new computes each symbol's rank in quadratic time
Labels: enhancement
Body:
`Codes::new` assigns each symbol its code by counting, for that symbol, how many earlier symbols share its length, with `lengths.iter().take(symbol).filter(...).count()` inside the per-symbol loop, in `crates/deflate/src/huffman.rs:396-411`. The count is O(N) per symbol, so building the code is O(N^2).

The compressor calls `Codes::<LITERALS>::new` over 288 symbols on every `compress` call at `crates/deflate/src/encode.rs:71`, which is about 41000 operations for a table that the canonical algorithm of RFC 1951 section 3.2.2 builds in O(N) with a per-length next-code counter.

Fix: keep a `next_code` array indexed by length and increment the entry as each symbol of that length is assigned, replacing the inner `take(symbol).filter().count()`.

---

## F07 — issue #420
Title: audhsos-symbols: the symbol table reader assumes a 24-byte entry and ignores the section's declared entry size
Labels: enhancement
Body:
`Functions::new` reads the symbol table content and its linked string table without consulting the section's `entry_size` field, in `crates/symbols/src/functions.rs:487-500`, and `len` and `get` stride the entries with the hardcoded `SYM_LEN` of 24 in `crates/symbols/src/functions.rs:441,512-543`. The ELF64 symbol entry is 24 bytes, so the constant is correct for a well-formed file.

A symbol table section whose header declares an `entry_size` other than 24 is read with the wrong stride, so the entries parse as garbage while staying inside the section bounds. The `Section` struct already carries `entry_size` at `crates/elf/src/sections.rs:57`.

Fix: read `table.entry_size` and return an empty set when it is not 24, so a malformed entry size yields no functions rather than misaligned ones.
