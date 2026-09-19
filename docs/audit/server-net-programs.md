# server-net and user-net-programs audit findings

Repository: AuDHSOS/AuDHSOS. Audit of server-net, user-net-programs at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #165
Title: user-net-programs: the SAFETY comment of the DMA region claims exclusivity the device breaks
Labels: bug, part::net
Body:
`crates/user/net-programs/src/bin/server_net.rs:156-162` builds `&mut [u8]` over the DMA region through `Mapping::bytes` and states that "the values built here are the only ones that reach those bytes". The contract of `Mapping::bytes` at `crates/user/programs/src/mapping.rs:110-123` requires that nothing else reaches the bytes. The device writes the used ring and the receive buffers of that region: `crates/user/net-programs/src/net_dma.rs:127-129` hands the used ring out as a plain `&[u8]`, `crates/user/net-programs/src/net_dma.rs:170-173` hands a device-written frame buffer out as a plain `&[u8]`, and `crates/virtio/queue/src/queue.rs:361-362` with `crates/virtio/queue/src/memory.rs:119-127` reads the used index with an ordinary load. The only ordering is the `fence` at `crates/user/net-programs/src/net_dma.rs:136-138`; no access of the device-written pieces is volatile or atomic.

A `&mut [u8]` asserts to the compiler that no other agent writes the bytes while the borrow stands. The borrow stands for the whole program (`crates/user/net-programs/src/bin/server_net.rs:148-150`). Every frame the device delivers is a write into memory the program holds exclusively, which the Rust memory model calls undefined behavior; the compiler may keep an earlier load of the used index and never observe a delivered frame. The socket pages of the same program are reached atomically for this reason (`crates/user/net-programs/src/bin/server_net.rs:169-171`).

Fix: hold the used ring of each queue and the receive area as device-shared memory read through `read_volatile` (a raw pointer in `Ring` and `Area` of `net_dma.rs`, with `used_ring` and `bytes` copying through volatile loads), keep `&mut [u8]` for the descriptor table, the available ring and the transmit area only, and state in the SAFETY comment which pieces the device writes; the option not taken, relying on the `fence`, orders accesses but does not make the exclusivity claim of `&mut` true.

---

## F02 — issue #167
Title: server-net: a client without a badge is served, so every name-server client shares one socket table
Labels: bug, part::net
Body:
`crates/user/net-programs/src/bin/server_net.rs:348-373` answers every message whose badge is not `DEVICE_BADGE` or `TICK_BADGE`, and `crates/user/servers/net/src/server.rs:152-180` and `crates/user/servers/net/src/sockets.rs:84-98` accept badge `0`. A capability found under a name carries no badge and the kernel delivers it as `0` (`crates/kernel/ipc/src/endpoint.rs:73-78`). `crates/abi/src/startup.rs:136` states that a server refuses such an endpoint, and the input server does so with `NOBODY` at `crates/user/servers/input/src/state.rs:26-34` and `crates/user/servers/input/src/state.rs:123-125`. Both network programs fall back to the name server: `crates/user/net-programs/src/bin/app_net.rs:139-145` and `crates/user/net-programs/src/bin/app_ssh.rs:644-650`.

Two programs that reach the server through `lookup` are one client to `Sockets::slot` (`crates/user/servers/net/src/sockets.rs:133-137`). Program B sends `TcpRecv` or `TcpAccept` with the socket number of program A; the reply carries A's ring object (`crates/user/servers/net/src/server.rs:383-386`), and B maps and reads A's inbound ring. Program B can also close A's socket with `TcpClose`.

Fix: `serve` answers `Error::AccessDenied` to every request whose badge is `0` before decoding it, as the input server does with `NOBODY`; the option not taken, refusing inside `Sockets::open`, leaves `Interface` and `Resolve` open to the badge-less client.

---

## F03 — issue #169
Title: user-net-programs: a client can drain the server's random generator and end every connection, lease and resolution
Labels: bug, part::net
Body:
`crates/user/net-programs/src/bin/server_net.rs:627-633` implements `Entropy` with a source that always answers `Unavailable`. `ChaChaRng::fill` at `crates/crypto/rng/src/chacha.rs:130-137` reseeds when the request exceeds the budget of `RESEED_BYTES` (`crates/crypto/rng/src/chacha.rs:32`, one mebibyte) and returns the reseed error without restoring the budget, so every later `fill` fails. Each `TcpConnect` and each `TcpListen` draws four bytes for the initial sequence number (`crates/net/tcp/src/connection.rs:1242-1251`). The comment at `crates/user/net-programs/src/bin/server_net.rs:618-626` claims the server never reaches a reseed.

A client that sends `TcpConnect` and `TcpClose` 262,144 times exhausts the budget. From then on every `TcpConnect`, `TcpListen` and `UdpBind { port: 0 }` of every client answers `InvalidArgument` through `refusal` (`crates/user/servers/net/src/server.rs:675-683`), the address configuration client cannot draw a transaction identifier for a renewal, and the resolver cannot draw a port, so the server loses its lease and its names. Each refused `TcpConnect` and `TcpListen` also loses the window pair that went into the call (`crates/user/servers/net/src/server.rs:321-328` and `crates/user/servers/net/src/server.rs:346-353`), and each refused `UdpBind` loses its buffer (`crates/user/servers/net/src/server.rs:298-304`).

Fix: `serve` reseeds the generator from `gate.random_bytes()` before the budget can run out, by rebuilding it with `seed(gate)` every 65,536 client requests; the option not taken, giving `Seed` the gate, is what the generator's type cannot hold, as the comment says.

---

## F04 — issue #171
Title: server-net: a second connect to the same remote loses a window pair from the pool for the life of the server
Labels: bug, part::net
Body:
`crates/user/servers/net/src/server.rs:318-328` takes a window pair out of the pool and passes it into `Stack::connect_to`; on a refusal the pair is not returned. `Stack::connect_to` names the local end with port `0` (`crates/net/stack/src/stack.rs:490`), and `Connections::connect` refuses with `PortInUse` when a connection already joins the same local and remote ends (`crates/net/tcp/src/table.rs:158-173`, `crates/net/tcp/src/table.rs:224-229`, `crates/net/tcp/src/table.rs:289-294`). The invariant at `crates/user/servers/net/src/server.rs:15-18` says the stack is asked first for every call that can refuse; `connect` asks only `source_for`.

A client sends `TcpConnect { remote: X }` twice while the first connection is open. The second answers `InvalidArgument` and one of the four window pairs (`crates/user/servers/net/src/memory.rs:247`) is gone. After four such calls every `TcpConnect` and every `TcpListen` of every client answers `OutOfMemory` (`crates/user/servers/net/src/server.rs:321`, `crates/user/servers/net/src/server.rs:346`) until the server restarts.

Fix: `connect` asks the stack whether it already holds the four-tuple before taking the pair, through a `Stack::holds(source, remote)` beside `is_bound` and `listens_on`; the option not taken, returning the buffers from `connect_to` on refusal, changes the signature of `net-stack` for every caller.

---

## F05 — issue #173
Title: server-net: the slot of a client that exited is never offered to another, so four programs exhaust the table
Labels: bug, part::net
Body:
`Sockets::free_for` at `crates/user/servers/net/src/sockets.rs:109-119` hands a slot only to a badge that owns it or to nobody, and `Sockets::release` at `crates/user/servers/net/src/sockets.rs:123-129` is the only place an owner is cleared. `release` is called from `Server::forget` (`crates/user/servers/net/src/server.rs:195`), and `forget` is called once in the program, for a request that fails to decode (`crates/user/net-programs/src/bin/server_net.rs:365-371`). No thread of the server learns that a client exited; the display server has one (`crates/user/programs/src/bin/server_display.rs:391-419`, `crates/user/programs/src/bin/server_display.rs:218-224`). `docs/13-the-network-on-the-machine.md:520-523` states that a slot is offered to another client once the server has learned the first one is gone. Every child of init carries a fresh badge (`crates/user/programs/src/bin/server_init.rs:570-571`).

Four programs over the uptime of the machine each open one socket, close it and exit. Each slot keeps its owner. The fifth program answers `OutOfHandles` to `UdpBind`, `TcpConnect` and `TcpListen` (`crates/user/servers/net/src/server.rs:540-544`) for as long as the server runs.

Fix: a watcher thread as the display server's `start_watcher`, sending the badges of exited clients to the endpoint under a badge of its own, with `serve` calling `Server::forget` for each; the option not taken, clearing the owner on close, hands rings a client still maps to another client.

---

## F06 — issue #176
Title: server-net: one client holds the resolver and a datagram buffer for its lifetime by asking once
Labels: bug, part::net
Body:
`crates/user/servers/net/src/server.rs:226-235` answers `Busy` to every other client while `resolving` is set, and `resolving` is cleared only in `forget_resolution` (`crates/user/servers/net/src/server.rs:282-286`), which `resolution` calls for the client that asked (`crates/user/servers/net/src/server.rs:258-278`) and `forget` calls when that client is gone (`crates/user/servers/net/src/server.rs:196-200`). `forget` is reached only through an undecodable request (`crates/user/net-programs/src/bin/server_net.rs:365-371`). `pump` and `poll` do not look at the resolver.

A client sends `Resolve { name }` once and never asks again. The resolution completes in the stack and stays. Every other client's `Resolve` answers `Busy` until the first client sends an undecodable message, and one of the four datagram buffers (`crates/user/servers/net/src/memory.rs:244`, taken at `crates/user/servers/net/src/server.rs:238`) stays out of the pool.

Fix: `resolve` for another client ends a resolution whose stack status is `Done` or `Failed` with `forget_resolution` and starts its own, the first client then answering `NotFound`; the option not taken, a deadline in `pump`, needs the clock in a call that has none.

---

## F07 — issue #178
Title: server-net: a shutdown sends the FIN after at most one chunk and the rest of the ring is discarded
Labels: bug, part::net
Body:
`shutdown` at `crates/user/servers/net/src/server.rs:441-454` calls `push` once and then `connection.close()`. `push` moves at most `CHUNK` (1024) bytes bounded by the send buffer (`crates/user/servers/net/src/server.rs:604-609`). `close` sets `closing` (`crates/net/tcp/src/connection.rs:451-462`). `push` checks `can_send()` only (`crates/user/servers/net/src/server.rs:601`), reads the ring (`crates/user/servers/net/src/server.rs:609`) and then calls `Connection::write`, which refuses while `closing` is set (`crates/net/tcp/src/connection.rs:421-427`); the refusal becomes `0` (`crates/user/servers/net/src/server.rs:613-615`) and the bytes read out of the ring are gone. The comment at `crates/user/servers/net/src/server.rs:446-448` states that the FIN follows everything the ring holds. The test at `crates/user/servers/net/src/tests/server.rs:1395-1420` writes 38 bytes.

A client writes 2000 bytes into the outbound ring and sends `TcpShutdown { direction: Write }` before a `TcpSend`. 1024 bytes go into the connection, the FIN is queued behind them, and the next `pump` reads the remaining 976 bytes out of the ring and drops them. The peer receives 1024 bytes and a FIN.

Fix: `shutdown` pushes and answers `WouldBlock` while the outbound ring still holds bytes, calling `close` only on an empty ring, and `push` returns before reading the ring when the connection is closing; the option not taken, looping `push` inside `shutdown`, cannot pass a full send buffer.

---

## F08 — issue #180
Title: server-net: TcpClose retires an open connection without a FIN or a reset
Labels: bug, part::net
Body:
`close_connection` at `crates/user/servers/net/src/server.rs:457-465` calls `give_up`, and `release_connection` at `crates/user/servers/net/src/server.rs:575-583` calls `Stack::close_connection` (`crates/net/stack/src/stack.rs:516-523`), which takes the connection out of the table at once (`crates/net/tcp/src/table.rs:183-193`) with no segment sent. RFC 9293 sends a FIN for CLOSE in ESTABLISHED (`docs/rfc/rfc9293.txt:3124-3150`) and a reset for ABORT (`docs/rfc/rfc9293.txt:3177-3215`). The doc comment of `Connections::close` at `crates/net/tcp/src/table.rs:175-182` describes a reset the caller fetches with one last poll; the server fetches none. The comment at `crates/user/servers/net/src/server.rs:577-578` justifies the missing reset with the listener case alone.

A client sends `TcpClose` on an established connection. The peer receives nothing and learns of the close only when its next segment meets `reset_for` (`crates/net/tcp/src/table.rs:208-213`); a peer that is waiting to read waits until its own timeout. A client that sends `TcpShutdown` and then `TcpClose` retires the connection before the FIN is acknowledged, so the peer's ACK and FIN are answered with a reset.

Fix: `release_connection` sends the reset of RFC 9293 3.10.5 for a connection in an open state other than `Listen` before the buffers go back, through a `Connection::abort` of `net-tcp` polled once as `crates/net/tcp/src/table.rs:175-182` describes; the option not taken, a graceful close on `TcpClose`, needs the client to wait for `TimeWait`.

---

## F09 — issue #181
Title: server-net: TcpRecv answers the bytes waiting in the ring, not the bytes moved
Labels: bug, part::net
Body:
`receive` at `crates/user/servers/net/src/server.rs:402-408` discards the count `take` moved and answers `page.inbound.held()`. `crates/user/proto/src/socket.rs:389` documents the reply as how many bytes of the connection went into the ring, and `docs/13-the-network-on-the-machine.md:498` as how many bytes moved through the ring. The client ignores the value (`crates/user/programs/src/socket.rs:474-487`).

A client that has 500 bytes waiting and asks `TcpRecv` while the connection holds nothing is answered `500` where the document promises `0`.

Fix: `receive` answers the count `take` returned; the option not taken, changing the two documents to "held", leaves `TcpSend` and `TcpRecv` answering different quantities under one table row.

---

## F10 — issue #183
Title: server-net: TcpSend ignores len and moves up to a chunk whatever the client asked
Labels: bug, part::net
Body:
`send` at `crates/user/servers/net/src/server.rs:393-398` binds `len` as `_len` and calls `push`, which moves `min(writable, CHUNK)` bytes (`crates/user/servers/net/src/server.rs:604-609`). `crates/user/proto/src/socket.rs:258-263` documents `len` as how many bytes of the ring to send.

A client writes 300 bytes into the ring and sends `TcpSend { len: 100 }`. 300 bytes go into the connection and the reply says `300`.

Fix: `push` takes a limit and `send` passes `min(len, CHUNK)`; the option not taken, dropping `len` from the request, changes the wire protocol.

---

## F11 — issue #185
Title: server-net: the README and document 13 name user-programs as the binary's package
Labels: bug, part::net
Body:
`crates/user/servers/net/README.md:5-6` and `docs/13-the-network-on-the-machine.md:437-439` state that the process around the crate is a binary of `user-programs`. The binary is `server-net` of `user-net-programs` (`crates/user/net-programs/Cargo.toml:21-23`, `docs/05-code-organization.md:96`).

A reader following the README looks for the binary in `crates/user/programs/src/bin` and does not find it.

Fix: name `user-net-programs` in both places and the reason for the split (D-97).

---

## F12 — issue #187
Title: user-net-programs: the package describes itself as two, three and four programs
Labels: enhancement, part::net
Body:
`crates/user/net-programs/Cargo.toml:6` says two programs. `crates/user/net-programs/src/bin/server_net.rs:32`, `crates/user/net-programs/src/bin/app_net.rs:23` and `crates/user/net-programs/src/bin/app_ssh.rs:26` say three. `crates/user/net-programs/README.md:3`, `crates/user/net-programs/src/bin/app_tls.rs:28` and the manifest at `crates/user/net-programs/Cargo.toml:21-47` have four. `docs/05-code-organization.md:96` lists three and omits `app-tls`.

The gallery card of the crate and the catalog of document 5 both undercount what the package builds.

Fix: say four in the manifest description, the three comments and `docs/05-code-organization.md:96`.
