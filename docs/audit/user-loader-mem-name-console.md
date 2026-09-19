# user-loader, server-memory, server-name, server-console audit findings

Repository: AuDHSOS/AuDHSOS. Audit of user-loader, server-memory, server-name, server-console at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #115
Title: server-memory: one client exhausts the live table and the free memory of every other client
Labels: bug, part::userland
Body:
`Store::allocate` at `crates/user/servers/memory/src/store.rs:156-218` counts nothing per owner: the only limits are `self.live.is_full()` at `crates/user/servers/memory/src/store.rs:170-172` and the free list. The binary sizes the store as `Store<256, 256>` at `crates/user/programs/src/bin/server_memory.rs:47`, one table for every client of the system.

A client that sends 256 `Allocate` requests of one page each fills `live`; every later `allocate` of every other client answers `Error::PoolExhausted` at `crates/user/servers/memory/src/store.rs:170-172`. A client that asks for the sum of `free_bytes()` in one request takes every free page; every other client answers `Error::OutOfMemory` at `crates/user/servers/memory/src/store.rs:173`. The test `a_full_live_table_takes_no_further_object` at `crates/user/servers/memory/src/tests/store.rs:503-516` shows the table filled by one badge.

Fix: keep a per-owner count of objects and bytes in `Store` and refuse an `allocate` that exceeds a quota the constructor takes, rather than a per-request cap, which a client defeats by repetition.

---

## F02 — issue #117
Title: server-memory: a client that keeps its handle after a release fills the retired table and blocks every release and every reclaim of a dead client
Labels: bug, part::userland
Body:
`release` moves the object into `retired` at `crates/user/servers/memory/src/store.rs:259-261` and refuses when `retired` is full at `crates/user/servers/memory/src/store.rs:252-254`. `reclaim` at `crates/user/servers/memory/src/store.rs:273-285` frees an object only when `pages.references(object.handle) == Ok(1)`; the client's own handle keeps the count at two. `retired` has `LIVE` slots, shared by every client, at `crates/user/servers/memory/src/store.rs:69`.

A client allocates one page, sends `Release` for it, and never closes its handle; the object stays retired. After 256 such pairs `retired` is full. From then on every `release` of every client answers `Error::PoolExhausted` at `crates/user/servers/memory/src/store.rs:252-254`, and `forget_client` for a client that died answers `Error::PoolExhausted` at `crates/user/servers/memory/src/store.rs:301-303` with the dead client's objects still in `live`. The test `returned_shared_pages_are_neither_zeroed_nor_reallocated_until_exclusive` at `crates/user/servers/memory/src/tests/store.rs:89-112` shows one object held retired by a foreign reference.

Fix: keep a returned object in its `live` slot with a retired flag instead of moving it to a second table, so a release needs no slot and the quota of F01 bounds retired objects with live ones; a separate per-owner cap on `retired` would need a second count.

---

## F03 — issue #119
Title: server-memory: a refused release leaves the received capability in the server's handle table
Labels: bug, part::userland
Body:
`release` closes `returned.handle` only on the accepted path at `crates/user/servers/memory/src/store.rs:255-258`. The four refusals at `crates/user/servers/memory/src/store.rs:244-254` return before the close. The binary passes the handle the message carried into `release` at `crates/user/programs/src/bin/server_memory.rs:100-104` and closes nothing on an error. The test `a_second_release_of_the_same_object_is_refused_and_zeroes_nothing` at `crates/user/servers/memory/src/tests/store.rs:246-261` asserts that a refused release makes no call at all.

A client sends `Release` with a capability to an object the store never handed out, or with a capability to an object it already released. Every such message installs one handle in the memory server's table, and `release` answers `Error::NotFound` at `crates/user/servers/memory/src/store.rs:244` with the handle open. The table is finite: `Error::OutOfHandles` at `crates/abi/src/error.rs:64`. Once it is full the server can receive no message that carries a handle.

Fix: close `returned.handle` in `release` on every refusal path before returning the error, and change the test at `crates/user/servers/memory/src/tests/store.rs:257-260` to expect the close; closing in the binary instead would leave the store's contract at `crates/user/servers/memory/src/store.rs:220-229` silent about the handle.

---

## F04 — issue #121
Title: server-name: register drops a replaced endpoint and a refused endpoint without closing either
Labels: bug, part::userland
Body:
`Registry::replace` overwrites `entry.endpoint` at `crates/user/servers/name/src/registry.rs:165` and returns `()`; the old handle is gone from the registry and from the caller. `register` returns `Err(Error::AlreadyExists)` at `crates/user/servers/name/src/registry.rs:163`, `Err(Error::InvalidArgument)` at `crates/user/servers/name/src/registry.rs:89-91` and `Err(Error::PoolExhausted)` at `crates/user/servers/name/src/registry.rs:101` with the endpoint handle the message carried untouched. The binary passes the received handle in and closes nothing at `crates/user/programs/src/bin/server_name.rs:66-73`.

A client sends `Register` for its own name once per message; each message installs one endpoint handle in the name server's table, and `replace` at `crates/user/servers/name/src/registry.rs:160-167` forgets the previous one. A client sends `Register` for a name another client holds; each message leaves one handle open. The table is finite: `Error::OutOfHandles` at `crates/abi/src/error.rs:64`. Once it is full the name server can receive no `Register`.

Fix: make `register` return `Result<Option<Handle>, (Handle, Error)>`, the handle the caller has to close, and close it in the binary; closing inside the registry is not possible because the crate makes no system call by design at `crates/user/servers/name/README.md`.

---

## F05 — issue #123
Title: server-name: one client registers all sixty-four names
Labels: bug, part::userland
Body:
`register` at `crates/user/servers/name/src/registry.rs:88-103` pushes until `entries` is full; `CAPACITY` is 64 at `crates/user/servers/name/src/registry.rs:26` and no count per owner exists. The test `a_full_registry_takes_no_further_name` at `crates/user/servers/name/src/tests/registry.rs:107-126` fills the registry from one badge.

A client sends 64 `Register` requests with 64 distinct names. Every later `register` of every other client, a server of the boot set included, answers `Error::PoolExhausted` at `crates/user/servers/name/src/registry.rs:101`. `lookup` of those 64 names hands every client the endpoint of the one client.

Fix: count names per owner in `register` and refuse above a per-owner limit the constructor takes; reserving names for the boot set alone would leave every later program exposed.

---

## F06 — issue #125
Title: server-memory: a split or a wipe that fails inside allocate loses the free object
Labels: bug, part::userland
Body:
`allocate` removes the chosen object from `free` at `crates/user/servers/memory/src/store.rs:183` before the first fallible call. `pages.split` at `crates/user/servers/memory/src/store.rs:186` and `crates/user/servers/memory/src/store.rs:200` and `wipe` at `crates/user/servers/memory/src/store.rs:210` return with `?`. On each of those returns `piece` is in neither `free` nor `live`, while the kernel object it names still exists under the server's handle.

The double reproduces it: `kernel.fail_after = Some((0, Error::OutOfKernelMemory))` and `allocate` of one page on an adopted region of eight pages, as in `an_error_from_the_kernel_comes_back_out` at `crates/user/servers/memory/src/tests/store.rs:566-575`, leaves `store.free_bytes()` at zero and `store.live_bytes()` at zero; the test checks the error and not the store. After the second split fails the lower piece is back in `free` at `crates/user/servers/memory/src/store.rs:197` and the upper piece is lost. Every failure of this kind shrinks the memory the server can hand out, for the rest of its life.

Fix: put `piece` back with `insert_free` before returning on the three error paths, with the free slot check at `crates/user/servers/memory/src/store.rs:179-181` accounting for it; leaving the object in `free` and removing it after the last fallible call would need the two splits to work on a copy.

---

## F07 — issue #126
Title: server-memory: a close that fails inside release loses the returned object
Labels: bug, part::userland
Body:
`release` removes the object from `live` at `crates/user/servers/memory/src/store.rs:255` and then calls `pages.close(returned.handle)?` at `crates/user/servers/memory/src/store.rs:256-258`. When the close returns an error the object is in neither `live` nor `retired`.

`kernel.fail_after = Some((0, Error::InvalidHandle))` before a `release` of an object that came back under another handle, as in `an_object_that_comes_back_under_another_name_is_recognized_and_the_name_given_up` at `crates/user/servers/memory/src/tests/store.rs:177-199`, returns the error with `store.live_objects()` at zero, `store.free_bytes()` unchanged and the object gone from the store.

Fix: push the object into `retired` before the close, and report the close error after it; closing before the `live.remove` would leave the object live under its owner on a failed close, which is the state the doc at `crates/user/servers/memory/src/store.rs:236-237` promises for refusals.

---

## F08 — issue #128
Title: user-loader: a segment of no bytes at an unaligned address becomes a whole page in the plan
Labels: bug, part::userland
Body:
`region_of` at `crates/user/loader/src/program.rs:235-252` rounds `vaddr` down and `vaddr + mem_size` up without a case for `mem_size == 0`. `audhsos-elf` keeps a `PT_LOAD` of `mem_size` zero and exempts it from its overlap check at `crates/elf/src/image.rs:88-96`.

A segment with `mem_size` 0 at `vaddr` `0x401010` yields `Region { vaddr: 0x401000, len: 4096, file_size: 0, .. }`; the root task allocates and maps one page the program does not ask for. The same segment placed inside the page of a preceding segment fails `plan` with `ProgramError::Overlap` at `crates/user/loader/src/program.rs:188-192`, so a program that a linker emits with an empty `PT_LOAD` beside a full one is refused.

Fix: skip a segment with `mem_size == 0` in the loop at `crates/user/loader/src/program.rs:186-196`; refusing it in `audhsos-elf` would change every user of that crate.

---

## F09 — issue #130
Title: user-loader: the doc of TarError::Truncated claims a truncated header is reported
Labels: bug, part::userland
Body:
The doc at `crates/user/loader/src/tar.rs:69-73` says `Truncated` is answered when the bytes end inside a header. `Entries::step` at `crates/user/loader/src/tar.rs:270-276` answers `Ok(None)` for a final block shorter than 512 bytes, and the comment there says why.

An archive of one full header followed by 100 bytes of a second header ends the walk with no error; `Archive::find` at `crates/user/loader/src/tar.rs:222-230` answers `Ok(None)` for a name that stands in the cut header. A caller that reads the variant doc expects `Err(Truncated)`.

Fix: change the doc at `crates/user/loader/src/tar.rs:69-70` to say the bytes end inside the file a header describes; answering `Truncated` for a short final block would turn every archive without an end marker into an error.

---

## F10 — issue #133
Title: user-loader: the doc of split_name names the last slash and the code takes the first
Labels: bug, part::userland
Body:
The doc at `crates/user/loader/src/tar.rs:653-655` says a long name is cut at the last slash that leaves at most a hundred bytes behind it. The filter at `crates/user/loader/src/tar.rs:660-670` takes `.next()`, the first slash whose index is at most 155 and whose rest is at most 100 bytes.

A name `a/b/` followed by 99 `c` bytes, 103 bytes in all, is cut after `a` by the code and after `b` by the doc. Both cuts read back to the same path through `path_of` at `crates/user/loader/src/tar.rs:420-437`, so the archive is not wrong; the doc is.

Fix: change the doc to say the first such slash; taking the last slash instead would put more of the name into the 155-byte prefix and change the archives the build writes.

---

## F11 — issue #135
Title: server-memory: the errors section of release omits two variants it returns
Labels: enhancement, part::userland
Body:
The doc at `crates/user/servers/memory/src/store.rs:231-237` lists `NotFound`, `AccessDenied`, the zeroing errors and the join errors. `release` also returns `Error::InvalidArgument` for a length that differs from the one handed out at `crates/user/servers/memory/src/store.rs:249-251` and `Error::PoolExhausted` for a full retired table at `crates/user/servers/memory/src/store.rs:252-254`.

A caller that maps the documented variants to replies, as `crates/user/programs/src/bin/server_memory.rs:100-104` does, forwards two variants the doc does not name.

Fix: add both variants to the errors section.

---

## F12 — issue #136
Title: server-memory: every allocate makes one references system call per retired object
Labels: enhancement, part::userland
Body:
`allocate` calls `reclaim` at `crates/user/servers/memory/src/store.rs:166`, and `reclaim` at `crates/user/servers/memory/src/store.rs:273-285` calls `pages.references` for every retired object on every call. With R retired objects each allocation costs O(R) system calls before its own work, and with the retired table of F02 at 256 entries that is 256 calls per allocation.

A client that keeps 255 objects retired as in F02 makes every allocation of every other client 256 system calls slower, without any object being reclaimed.

Fix: run `reclaim` only when `first_fit` at `crates/user/servers/memory/src/store.rs:173` finds nothing, or when `live` is full; the current placement makes the common case pay for the rare one.
