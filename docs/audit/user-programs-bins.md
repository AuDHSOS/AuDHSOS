# user-programs binaries and user-test-programs audit findings

Repository: AuDHSOS/AuDHSOS. Audit of user-programs (binaries except server_init.rs) and user-test-programs at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #132
Title: server-name: a lookup hands every client the registrant's endpoint with RECV and BADGE rights
Labels: bug, part::userland
Body:
The name server stores the handle a registrant sent and answers a lookup with that same handle: `crates/user/programs/src/bin/server_name.rs:57-73` and `crates/user/servers/name/src/registry.rs:88-115`. The kernel copies rights and badge on every transfer: `crates/kernel/ipc/src/transfer.rs:45-49` and `crates/kernel/ipc/src/transfer.rs:132-142`. A server registers its own endpoint, which the root task installed with `RECV | BADGE | SEND | TRANSFER | DUPLICATE`: `crates/user/programs/src/bin/server_init.rs:967` and `crates/user/programs/src/bin/server_init.rs:1247-1254`. Every application holds a name server capability: `crates/user/programs/src/bin/server_init.rs:246-305` (`names: true`).

An application that calls `lookup(names, b"console")` (`crates/user/programs/src/bin/app_hello.rs:78`) receives a handle that passes the `RECV` check of `ipc_recv` (`crates/kernel/syscall/src/calls/ipc.rs:322`) and the `BADGE` check of `endpoint_badge` (`crates/kernel/syscall/src/calls/ipc.rs:92-109`). With `ipc_recv` the application takes requests other clients sent to the console, the display, the input server or the file server. With `endpoint_badge(console, 0xC0_1DE)` it mints the badge the console driver trusts at `crates/user/programs/src/bin/server_console.rs:139-143`, and every word it sends is put into the input ring as typed bytes (`crates/user/programs/src/bin/server_console.rs:278-298`). With `endpoint_badge(display, 0x60_4E)` it mints the badge checked at `crates/user/programs/src/bin/server_display.rs:221-225`, and a word of `0xF` releases the surfaces of every other client, unmaps their pixels and closes their process handles (`crates/user/programs/src/bin/server_display.rs:460-500`). With `endpoint_badge(input, 0x1_4042)` it mints the badge checked at `crates/user/programs/src/bin/server_input.rs:146-158`, injects scancodes into every subscriber's ring through `arrived` (`crates/user/programs/src/bin/server_input.rs:433-462`) or releases the rings of other clients through `release_gone` with `GONE_LABEL` (`crates/user/programs/src/bin/server_input.rs:390-429`).

Fix: in `server_name.rs`, on `Register`, replace the received handle by `gate.handle_duplicate(endpoint, Rights::SEND | Rights::TRANSFER)` (wrapper at `crates/user/sys-x86_64/src/gate.rs:710-716`; the registered handle carries `DUPLICATE`) and close the received one, so a lookup can hand out nothing above `SEND`; the root task needs no lookup for badging, because it keeps the endpoints it created (`crates/user/programs/src/bin/server_init.rs:2027-2040`). Stripping rights on every `Lookup` instead would duplicate a handle per request and leak one per failed reply.

---

## F02 — issue #137
Title: server-display: the process handle of CreateSurface and every extra handle stay installed on failure
Labels: bug, part::userland
Body:
`create` takes the transferred process handle as a bare `Handle` (`crates/user/programs/src/bin/server_display.rs:339`) and returns without closing it when `display.create` refuses (`server_display.rs:344`), when no slot is free (`server_display.rs:345-346`), when `allocate` or `Mapping::new` fails (`server_display.rs:350-357`). The main loop decodes without reading the handle area (`server_display.rs:226-244`), so a handle attached to `Info`, `Present`, `DestroySurface` or `SetCursor`, or a second handle attached to any request, is never closed. The kernel installs up to four handles per received message before the server sees it (`crates/kernel/ipc/src/transfer.rs:93-101`), and the display server's table holds 64 handles (`crates/user/programs/src/bin/server_init.rs:211`).

A client with a display capability sends `Info` with four `TRANSFER`-able handles attached, sixteen times. The table is full. Every `allocate` reply from the memory server arrives with `handle_count` zero and `PARTIAL` set (`crates/kernel/ipc/src/transfer.rs:96-99`), `reader.handle()` answers `Truncated` (`crates/user/rt/src/message.rs:292-295`), and no client can create a surface until the display server restarts. A client that sends `CreateSurface` a second time (`AlreadyExists` at `crates/user/servers/display/src/state.rs:152-154`) leaks one handle per call and keeps a reference to its own process object with each.

Fix: snapshot the handle area before decoding and close every handle the request did not keep, as `crates/user/servers/input/src/request.rs:21-50` does; the alternative of closing in each error arm of `create` misses the handles of the other four requests.

---

## F03 — issue #140
Title: server-display: a destroyed surface leaves its process watch armed on a slot bit the next client reuses
Labels: bug, part::userland
Body:
`create` arms a watch on the client process with the slot index as the bit: `crates/user/programs/src/bin/server_display.rs:361-364`. `give_back` unmaps, releases and closes the process handle but calls no `process_unwatch`: `server_display.rs:492-500`. The kernel keeps the watch in the process object (`crates/kernel/syscall/src/calls/process.rs:85-89`); `handle_close` does not remove it (no reference to watchers in `crates/kernel/syscall/src/calls/handle.rs` or `crates/kernel/syscall/src/lifetime.rs`), and `release_gone` acts on the bit alone: `server_display.rs:467-488`. The input server handles the same situation with an unwatch in `give_back` and a check of the unwatch result: `crates/user/programs/src/bin/server_input.rs:376` and `server_input.rs:404-411`.

Client A creates a surface (slot 0, watch bit 0 on A), sends `DestroySurface`, and stays alive. Client B creates a surface and gets slot 0. A exits. The kernel signals bit 0, the watcher forwards it, and `release_gone` unmaps B's pixels, releases B's memory and closes B's process handle while B is running; B's next `Present` answers `NotFound`. A client that cycles through all four slots also fills its own four watcher entries (`crates/abi/src/layout.rs:157`), after which the input server's `process_watch` for that client fails with `QuotaExceeded`.

Fix: call `gate.process_unwatch(slot.process, notification, bit)` in `give_back` before closing the handle, and in `release_gone` treat only an unwatch that answers `true` as an ended client, as `server_input.rs:404-411` does.

---

## F04 — issue #143
Title: server-display: a failed process_watch is ignored and the surface is never released
Labels: bug, part::userland
Body:
`create` converts the received handle with `ProcessHandle::from_handle` without a type or rights check and discards the result of `process_watch`: `crates/user/programs/src/bin/server_display.rs:360-364`. `process_watch` refuses a handle that is not a process or lacks `INFO` (`crates/kernel/syscall/src/calls/process.rs:52-58`). The surface is created either way.

A client sends `CreateSurface` with a memory handle, or a process handle duplicated without `INFO`, in the handle slot. The watch fails, the surface exists, the client exits. Nothing signals the watcher, the slot is held until the display server restarts, and four such clients leave no slot for anyone (`server_display.rs:70`). A client may also hand over a handle to another long-lived process it holds, with the same effect until that process ends.

Fix: make `process_watch` failure an error of `create`, unmapping the window and releasing the memory before answering, as the input server does at `crates/user/programs/src/bin/server_input.rs:298-306`.

---

## F05 — issue #146
Title: server-console and server-fs: handles attached to any request are never closed
Labels: bug, part::userland
Body:
Neither loop reads the handle area of a received message: `crates/user/programs/src/bin/server_console.rs:144-161` and `crates/user/programs/src/bin/server_fs.rs:598-613`. No request of either protocol takes a handle, and the kernel installs up to four per message regardless (`crates/kernel/ipc/src/transfer.rs:93-101`). Each server's table holds 64 handles (`crates/user/programs/src/bin/server_init.rs:174` and `server_init.rs:194`).

A client sends `Write` to the console, or `Flush` to the file server, with four transferable handles attached, sixteen times. The table is full. Each installed handle holds a reference to whatever the client sent, which for a memory object prevents the memory server from reclaiming it (`crates/user/servers/memory/src/store.rs:276`), and neither server can receive a handle again until it restarts.

Fix: after `receive`, close every handle the message carries in both servers, since neither protocol takes one; a generic close in `serve::receive` would be the same fix for all servers but would break the input server, which keeps two handles of a successful `Subscribe`.

---

## F06 — issue #151
Title: server-fs: every client that looked the server up shares one open-file table under badge zero
Labels: bug, part::userland
Body:
The file server passes `serving.badge` to `answer` without refusing zero: `crates/user/programs/src/bin/server_fs.rs:608-613`. `answer` keys every open file by that badge (`crates/user/servers/fs/src/serve.rs:135`, `serve.rs:150`, `serve.rs:193`) and has no `NOBODY` check, unlike the display (`crates/user/servers/display/src/state.rs:145-147`) and the input server (`crates/user/servers/input/src/state.rs:124-126`). The root task hands out no badged file server capability (the program table has no such role: `crates/user/programs/src/bin/server_init.rs:116-134`), so `app-files` reaches the server through a lookup (`crates/user/programs/src/bin/app_files.rs:365-368`), whose handle carries badge zero (`crates/user/servers/name/src/registry.rs:110-115`).

Every program with a name server capability that looks up `files` operates under badge zero. It opens, reads, writes and closes the files `app-files` has open, by guessing the small `u32` handles the server hands out in order, and a malformed message from it drops every open file of `app-files` through `clients.forget(0)` at `server_fs.rs:611`. Two such programs also share one per-client quota (`serve.rs:193`, `OutOfHandles`).

Fix: refuse badge zero in `serve` before `answer`, and add a `files` role to the root task's program table that installs a badged capability the way `draws` and `listens` do (`crates/user/programs/src/bin/server_init.rs:990-1003`), so `app-files` uses `startup` instead of a lookup; refusing zero alone leaves `app-files` without a server.

---

## F07 — issue #154
Title: server-display: a mapping that fails halfway leaves the slot window mapped and the slot dead
Labels: enhancement, part::userland
Body:
`Mapping::new` documents that a call which fails part way leaves what it mapped behind and that the caller unmaps the window it asked for: `crates/user/programs/src/mapping.rs:113-116`. `create` releases the memory on failure and unmaps nothing: `crates/user/programs/src/bin/server_display.rs:351-356`.

When `memory_map` fails after the first chunk of a surface larger than `MAX_PAGES_PER_CALL` pages (a page-table frame shortage in the kernel), the slot stays free in `held`, the window keeps the pages that were mapped, the released object cannot be reclaimed while the mapping stands (`crates/user/servers/memory/src/store.rs:276`), and every later `create` that picks that slot fails at the same address.

Fix: on a `Mapping::new` error, unmap the window with `Mapping::adopt(window_of(index), bytes).unmap(gate, process)` before releasing the memory, as `mapping.rs:147-156` provides for.

---

## F08 — issue #158
Title: server-display: the surface window size is not checked against SURFACE_SLOT
Labels: enhancement, part::userland
Body:
Each client surface is mapped at `SURFACES + index * SURFACE_SLOT` with `SURFACE_SLOT = 16 MiB`: `crates/user/programs/src/bin/server_display.rs:76-80` and `server_display.rs:563-568`. The mapped length is `whole_pages(made.bytes())` (`server_display.rs:349-351`), and `Display::create` bounds a surface by the screen only (`crates/user/servers/display/src/state.rs:148-151`), so a surface of a 3840x2160 screen is 33,177,600 bytes.

On such a screen the first client's window covers slots 0 to 2, and the second client's `Mapping::new` at slot 1 fails on an address that is already mapped, so the server that says it holds four clients holds one. The comment at `server_display.rs:78-79` claims the slot is larger than the largest screen, which the code does not check.

Fix: refuse a surface whose `whole_pages(bytes)` exceeds `SURFACE_SLOT` in `create` with `InvalidArgument`, or size `SURFACE_SLOT` from the mode at start.

---

## F09 — issue #160
Title: server-fs: a missing process or memory server capability is reported as AccessDenied
Labels: enhancement, part::userland
Body:
`main` stops with `Error::AccessDenied` when the startup message carries no own process or no memory server: `crates/user/programs/src/bin/server_fs.rs:115-117`. The same condition is `Error::NotFound` in the other servers (`crates/user/programs/src/bin/server_display.rs:341-343`, `crates/user/programs/src/bin/server_input.rs:293-295`).

The line `[files] access denied` on the console points a reader at a rights check that does not exist; the root task did not hand the capability over.

Fix: answer `Error::NotFound`, as the other servers do.

---

## F10 — issue #162
Title: user-test-programs: ipc_server keeps a dead `read` under an expect
Labels: enhancement, part::userland
Body:
`read` in `crates/user/test-programs/src/bin/ipc_server.rs:198-208` has no caller and is kept alive by `#[expect(dead_code)]`. The client half that reads has its own copy at `crates/user/test-programs/src/bin/ipc_client.rs:175-181`.

The function adds an `unsafe` block to the crate's budget for code that never runs.

Fix: delete `read` and its `expect` from `ipc_server.rs`.

---

## F11 — issue #163
Title: user-test-programs: every_wrapper documents a Safety section on a safe function
Labels: enhancement, part::userland
Body:
`fn main` in `crates/user/test-programs/src/bin/every_wrapper.rs:42-48` is a safe function whose doc comment carries a `# Safety` section; the precondition it states belongs to the `unsafe` block at `every_wrapper.rs:49-50`, which already carries it as a `// SAFETY:` comment.

A `# Safety` section tells a caller the function is `unsafe` to call, which it is not.

Fix: drop the `# Safety` section from the doc comment of `main`.
