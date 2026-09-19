# user-proto, server-display, server-input, app-canvas, gfx audit findings

Repository: AuDHSOS/AuDHSOS. Audit of user-proto, server-display, server-input, app-canvas, gfx at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #139
Title: server-display: surface numbers saturate and repeat after 2^32 creations
Labels: bug, part::userland
Body:
`Display::create` numbers every surface from `next_id` and advances it with `saturating_add(1)` at `crates/user/servers/display/src/state.rs:157-162`. `destroy` and `forget` free no number (`crates/user/servers/display/src/state.rs:195-206`). After `u32::MAX - 1` creations every further surface carries the id `u32::MAX`.

`holding` answers the first surface in creation order with the requested id and refuses it when its badge differs (`crates/user/servers/display/src/state.rs:178-188`). One client that runs `CreateSurface` and `DestroySurface` in a loop 2^32 - 1 times exhausts the counter with about 8.6 billion messages. From then on a second client that creates a surface receives the id `u32::MAX` while the first client's surface holds the same id, and every `Present` and `DestroySurface` of the second client is answered with `AccessDenied` for its own surface until the first client destroys its surface.

Fix: derive the id from the slot or reuse a freed id, so that ids of live surfaces are unique without a counter; refusing `create` once the counter is saturated is the option not taken, since it makes the display unusable after one client's loop.

---

## F02 — issue #141
Title: gfx: `cargo test -p gfx` does not compile without the feature `test-strategies`
Labels: enhancement, part::userland
Body:
`crates/gfx/src/tests/surface.rs:11` imports `crate::strategies`, and `crates/gfx/src/lib.rs:12-13` compiles that module only under `feature = "test-strategies"`. `test-support`, the dependency the module needs, is a dev-dependency without a feature gate at `crates/gfx/Cargo.toml:20-21`.

`cargo test -p gfx` fails with `error[E0432]: unresolved import crate::strategies` and runs no test of the crate. `cargo test -p gfx --features test-strategies` and the xtask's `cargo test --workspace --all-features` (`crates/tools/xtask/src/commands.rs:131`) pass all 60 tests. `server-input` gates its doubles with `cfg(any(test, feature = "test-doubles"))` at `crates/user/servers/input/src/lib.rs:8-9` and compiles under a bare `cargo test`.

Fix: gate the module with `#[cfg(any(test, feature = "test-strategies"))]` as `server-input` does; making `test-support` a plain dependency is the option not taken, since it pulls a host crate into a `no_std` build.

---

## F03 — issue #144
Title: user-proto: the plan names an error and a message shape the display protocol does not have
Labels: bug, part::userland
Body:
`docs/10-implementation-plan.md:2455` states that the display server answers `PermissionDenied` for a surface of another badge. `audhsos_abi::Error` has no such variant (`crates/abi/src/error.rs:60` defines `AccessDenied`), and `Display::holding` answers `Error::AccessDenied` at `crates/user/servers/display/src/state.rs:184-186`. `docs/10-implementation-plan.md:2447` gives `CreateSurface { width, height }`, while `Request::CreateSurface` carries a third field `process: Handle` at `crates/user/proto/src/display.rs:127`.

A reader of section 10.9.2 who writes a client against the document matches an error code no server sends and encodes a `CreateSurface` without the handle, which `Request::decode` refuses with `CodecError::Truncated` at `crates/user/proto/src/display.rs:288`.

Fix: change `PermissionDenied` to `AccessDenied` and add the `process` handle to the message in section 10.9.2.

---

## F04 — issue #145
Title: user-proto: a surface id above its field is refused under the wrong message number
Labels: bug, part::userland
Body:
`surface_id` at `crates/user/proto/src/display.rs:318-320` answers `ProtoError::Message(Protocol::Display, PRESENT)` for every caller. It is called from the `DESTROY_SURFACE` arm of `Request::decode` at `crates/user/proto/src/display.rs:299` and from the `CREATE_SURFACE` arm of `Reply::decode` at `crates/user/proto/src/display.rs:392`.

A `DestroySurface` whose id word is `u32::MAX + 1` is reported as a bad `present`, and `Display` prints "3 is no message of the display protocol" for a message labeled `4`. The test at `crates/user/proto/src/tests/display.rs:177-194` sends a `DESTROY_SURFACE` label and asserts the `PRESENT` error, so it pins the wrong number.

Fix: pass the message number into `surface_id` and assert `DESTROY_SURFACE` and `CREATE_SURFACE` in the tests.

---

## F05 — issue #148
Title: user-proto: an address of an unknown family is refused under message number zero
Labels: bug, part::userland
Body:
`read_address` at `crates/user/proto/src/socket.rs:807-818` answers `ProtoError::Message(Protocol::Socket, 0)` for a family word that is neither 4 nor 6 or an octet string of the wrong length. No message of the socket protocol has the number zero; `crates/user/proto/src/socket.rs:40-77` number them 1 to 13.

A `TcpConnect` with family word 5 is reported as "0 is no message of the socket protocol", and the message number of the request, which `Request::decode` holds at `crates/user/proto/src/socket.rs:473`, is lost. A client that switches on the message number of the error finds none.

Fix: add a `ProtoError` variant for a bad address family, or thread the message number of the caller into `read_address` as the `TCP_SHUTDOWN` arm does for `Direction::from_code` at `crates/user/proto/src/socket.rs:514-515`.

---

## F06 — issue #150
Title: gfx: the `PixelSink` contract says a run is trimmed and the `Surface` sink drops it whole
Labels: bug, part::userland
Body:
The trait doc at `crates/gfx/src/present.rs:57-59` says "Bytes that do not fit the row are dropped". The implementation for `Surface` at `crates/gfx/src/present.rs:112-119` asks `row_bytes_mut` for the whole run and writes nothing when `x + count > width`, since `range` at `crates/gfx/src/surface.rs:170-173` answers `None` for a run that reaches past the width.

`write_row(0, 0, &[white; 16])` on a surface of width 2 writes no pixel, as the test at `crates/gfx/src/tests/present.rs:170-179` asserts, while the contract says the first two pixels are written. A second `PixelSink` written to the trait doc behaves differently from `Surface` for the same call.

Fix: state in the trait doc that a run that does not fit is dropped whole; trimming the run in the `Surface` impl is the option not taken, since `present` at `crates/gfx/src/present.rs:87-93` never sends a run past the sink.

---

## F07 — issue #153
Title: user-proto: the README names three of the eight protocols
Labels: bug, part::userland
Body:
`crates/user/proto/README.md:3-5` says the crate holds "the name protocol, the console protocol, and the memory protocol". `Protocol` at `crates/user/proto/src/label.rs:46-63` lists eight, and `crates/user/proto/src/lib.rs:8-19` compiles the modules `display`, `file`, `input`, `keyboard`, `parent`, `ring` and `socket` beside the three. The manifest at `crates/user/proto/Cargo.toml:6` names five.

`crates/user/proto/src/lib.rs:6` includes the README as the crate documentation, so `cargo doc` publishes the three-protocol claim on the crate's front page.

Fix: list every protocol of `Protocol::ALL` in the README and the manifest description.

---

## F08 — issue #155
Title: user-proto: the keyboard doc denies the third level the German layout has
Labels: bug, part::userland
Body:
`crates/user/proto/src/keyboard.rs:13-16` says the two layouts differ "in the punctuation, in the two letters the German layout swaps, and in nothing else". `Keyboard::character` at `crates/user/proto/src/keyboard.rs:280-296` answers twelve characters for `Layout::De` under `alt_graph` that `Layout::Us` has under no modifier.

`Keyboard::new(Layout::De)` with the right alt key held answers `'@'` for `KeyCode::Q`; `Keyboard::new(Layout::Us)` answers `'q'`, which the doc says cannot differ.

Fix: name the AltGr level of the German layout in the module doc.

---

## F09 — issue #157
Title: gfx: a merged damage rectangle is not merged again with the rest of the set
Labels: enhancement, part::userland
Body:
`Damage::push` at `crates/gfx/src/rect.rs:187-206` unions the new rectangle with the first held rectangle it overlaps and returns. The union can overlap a rectangle later in the set, which stays as it is.

Push `(0,0,4,4)`, `(6,0,4,4)` and `(3,0,4,4)`: the third merges into the first, which becomes `(0,0,7,4)`, and the set holds `(0,0,7,4)` and `(6,0,4,4)`. `present` at `crates/gfx/src/present.rs:87-93` copies column 6 of rows 0 to 3 twice. The set holds at most 16 rectangles, so the extra work is bounded by 16 overlapping copies per presentation, O(pixels of the overlap).

Fix: after a union, remove every later rectangle the union overlaps and union it in, then place the result.

---

## F10 — issue #159
Title: user-proto: the record count of the ring page is a literal beside the constant that names it
Labels: enhancement, part::userland
Body:
`RING_CAPACITY` at `crates/user/proto/src/input.rs:133` is 254. The array of records at `crates/user/proto/src/input.rs:167` and its initializer at `crates/user/proto/src/input.rs:190` write the literal `254`, and `RingPage::record` indexes with `seq % RING_CAPACITY` at `crates/user/proto/src/input.rs:214-217`.

A change of `RING_CAPACITY` alone leaves the array at 254 records; the `const` asserts at `crates/user/proto/src/input.rs:172-173` check the page size and the header offset, not the record count, so a `RING_CAPACITY` of 255 compiles while `record` answers `None` for sequence number 254 and every push at that number is dropped. `crates/user/proto/src/ring.rs:40-44` defines `RING_DATA` for the same purpose in the byte ring.

Fix: add a `const RING_RECORDS: usize` derived from `RING_CAPACITY` and size the array and the initializer with it.
