# driver-virtio-net and pci audit findings

Repository: AuDHSOS/AuDHSOS. Audit of driver-virtio-net, pci at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #331
Title: driver-virtio-net: a used length above the buffer strands the receive buffer for good
Labels: bug, part::drivers
Body:
`Net::receive` returns at `crates/drivers/virtio-net/src/rx.rs:89` when `Queue::next_used` refuses the used element, before it clears `taken[head]` at `crates/drivers/virtio-net/src/rx.rs:92-100` and `posted[index]` at `crates/drivers/virtio-net/src/rx.rs:101-105`. `Queue::next_used` has by then consumed the element and freed the chain: `crates/virtio/queue/src/queue.rs:386-388` advance `last_used`, lower `in_flight` and free the descriptors before the length check at `crates/virtio/queue/src/queue.rs:389-390` refuses with `QueueError::UsedLength`. The descriptor is free in the queue while the driver still records the buffer as posted, so `Net::fill` skips it at `crates/drivers/virtio-net/src/rx.rs:45-47` on every later call.

A device that reports a used length of 2049 for a 2048-byte buffer removes one receive buffer from circulation per element. `server_net` answers every receive refusal with `fill` (`crates/user/net-programs/src/bin/server_net.rs:276-297`, `302-309`) and holds `QUEUE_SIZE = 8` buffers (`crates/user/net-programs/src/net_dma.rs:36-47`); eight such elements leave the receive ring empty and the server deaf, which is the outcome the comment at `crates/user/net-programs/src/bin/server_net.rs:289-293` says the refill prevents. The test at `crates/drivers/virtio-net/src/tests/rx.rs:164-181` checks the error and not the bookkeeping.

Fix: make `QueueError::UsedLength` carry the head it freed and have `receive` clear `taken[head]` and `posted[index]` before it reports the refusal; the option not taken, resetting the device on every queue refusal as `crates/drivers/virtio-net/src/rx.rs:76-81` prescribes, costs a reinitialization per malformed element and is not what `server_net` does.

---

## F02 — issue #335
Title: pci: a probe mask without contiguous size bits yields a length of 2^32, 2^64-1 or a non-power of two
Labels: bug, part::drivers
Body:
`length` at `crates/pci/src/bar.rs:309-311` and `wide_length` at `crates/pci/src/bar.rs:314-316` compute `!mask + 1` from the probed word with the flag bits cleared, and `decode` at `crates/pci/src/bar.rs:240-283` refuses a probe only when the whole word reads zero. The device supplies the probed word. A 32-bit memory register that answers `0x0000_0008` (prefetchable flag, no size bit) gives `length(0) = 0x1_0000_0000`; an I/O register that answers `0x0000_0001` gives the same 4 GiB; an I/O register that answers `0x0000_FFE1` gives `0xFFFF_0020`; a 64-bit register whose upper half answers zero gives `wide_length(join(0, low)) = 0xFFFF_FFFF_0000_4000` for `low = 0xFFFF_C00C`, and `u64::MAX` for `low = 0x0000_000C`; a register that answers `0xFFFF_E800` gives `0x1800`.

`server_init` maps the whole `Bar` it is given: `frames_of` at `crates/user/programs/src/bin/server_init.rs:1756-1767` turns `base + len` into a frame count and `device_memory` at `crates/user/programs/src/bin/server_init.rs:1741-1754` creates a device memory object over it, so a device answering `0x0000_0008` in a memory register asks the kernel for a 4 GiB window starting at that register's base.

Fix: derive the size from the lowest set address bit of the mask (`mask & mask.wrapping_neg()`), refuse a mask with no address bit as `None`, and keep the result a power of two by construction; the option not taken, refusing a non-power-of-two `!mask + 1`, also refuses an I/O register whose upper sixteen bits read zero.

---

## F03 — issue #337
Title: pci: structures beyond the sixteenth are dropped without an error
Labels: bug, part::drivers
Body:
`virtio::structures` at `crates/pci/src/virtio.rs:157-173` stores a structure only while `found.get_mut(count)` answers a slot (`crates/pci/src/virtio.rs:167-170`) and answers `Ok` when the sixteen slots of `MAX_STRUCTURES` (`crates/pci/src/virtio.rs:40`) are full. The capability walk answers up to `MAX_CAPABILITIES = 48` entries (`crates/pci/src/capability.rs:31`), each of which may be a vendor capability. `crates/pci/README.md:26-27` states that `virtio` reports every structure it found.

A device whose capability list holds seventeen vendor capabilities has its seventeenth structure absent from the answer, and a caller cannot tell an absent structure from one the device did not publish.

Fix: set `MAX_STRUCTURES` to `MAX_CAPABILITIES`, so the array holds every vendor capability the walk can answer; the option not taken, a `PciError` for the seventeenth structure, refuses a device this crate could describe whole.

---

## F04 — issue #340
Title: pci: the MSI-X table and pending BIR values 6 and 7 are reported rather than refused
Labels: bug, part::drivers
Body:
`location` at `crates/pci/src/msix.rs:209-214` takes the low three bits of the table and pending words as the BAR index and answers `Location { bar: 6 }` or `bar: 7`, which a type-0 header has no register for (`MAX_BARS = 6`, `crates/pci/src/bar.rs:31`). `virtio::read` refuses the same range with `PciError::CapabilityBar` at `crates/pci/src/virtio.rs:200-203`.

A device whose MSI-X table word reads `0x0000_0006` makes `msix::read` answer a `MsiX` whose `table.bar` indexes past every `[Option<Bar>; MAX_BARS]` a caller holds; `server_init` catches it at `crates/user/programs/src/bin/server_init.rs:1727-1738` with `Error::Unsupported` and no name for the cause.

Fix: refuse a BIR above 5 in `msix::read` with `PciError::CapabilityBar`, as `virtio::read` does.

---

## F05 — issue #343
Title: pci: a capability that leaves the list is refused with the error of an offset outside the configuration space
Labels: bug, part::drivers
Body:
`fits` at `crates/pci/src/capability.rs:105-110` answers `PciError::Offset(offset)` for a capability that runs past the 256-byte list. `PciError::Offset` is documented at `crates/pci/src/error.rs:38-39` as an offset at or beyond the end of the configuration space and prints as `outside a configuration space` at `crates/pci/src/error.rs:98-100`.

`msix::read(space, address, 0xF8)` reports that `0xf8` lies outside a configuration space of 4096 bytes (`crates/pci/src/tests/msix.rs:97-102`), and a caller that matches `PciError::Offset` to mean an unreachable word acts on the wrong cause.

Fix: add a variant `PciError::CapabilityTruncated { offset, len }` and answer it from `fits`.

---

## F06 — issue #344
Title: driver-virtio-net: document 13 says the crate depends on pci
Labels: bug, part::drivers
Body:
`docs/13-the-network-on-the-machine.md:351-353` states that `driver-virtio-net` depends on `pci` and `virtio-queue`. `crates/drivers/virtio-net/Cargo.toml:16-17` lists `virtio-queue` as the only dependency, and `docs/05-code-organization.md:169` and `docs/05-code-organization.md:308-312` state that the crate parses no capability and needs nothing of `pci` (D-139).

A reader of document 13 who checks the layering of rule 13 in document 5 finds the two documents in disagreement about the dependencies of one crate.

Fix: change `docs/13-the-network-on-the-machine.md:351-353` to name `virtio-queue` alone and cite D-139.

---

## F07 — issue #345
Title: pci: document 13 places the six base address registers in header.rs
Labels: bug, part::drivers
Body:
`docs/13-the-network-on-the-machine.md:240-244` lists the six base address registers among the fields `header.rs` reads. `Header` at `crates/pci/src/header.rs:98-124` carries no base address register, and every read of one is in `crates/pci/src/bar.rs:130-180` and `crates/pci/src/bar.rs:197-213`.

A reader looking for the base address registers of a header in `header.rs` finds none.

Fix: remove the six base address registers from the `header.rs` entry at `docs/13-the-network-on-the-machine.md:240-244`; the `bar.rs` entry at `docs/13-the-network-on-the-machine.md:251-262` already names them.

---

## F08 — issue #347
Title: driver-virtio-net: send writes the frame before the queue refuses it
Labels: bug, part::drivers
Body:
The doc comment at `crates/drivers/virtio-net/src/tx.rs:65-66` states that nothing is written in any refusal of `Net::send`. `send` writes the header at `crates/drivers/virtio-net/src/tx.rs:91` and the frame at `crates/drivers/virtio-net/src/tx.rs:93-96` before `Queue::add` at `crates/drivers/virtio-net/src/tx.rs:98` can refuse with `NetError::Queue`.

A `send` over a queue whose descriptors are all in flight while a transmit buffer is free leaves the header and the frame in that buffer and answers `NetError::Queue(QueueFull)`; the buffer stays free, so a retry overwrites it.

Fix: state at `crates/drivers/virtio-net/src/tx.rs:65-66` that a queue refusal leaves the frame in a buffer the device does not hold; the option not taken, checking `queue.free_count()` before writing, adds a check the queue repeats.

---

## F09 — issue #349
Title: driver-virtio-net: a chain is published before the driver records it
Labels: enhancement, part::drivers
Body:
`post` calls `Queue::add` at `crates/drivers/virtio-net/src/rx.rs:125` and records the head in `taken` at `crates/drivers/virtio-net/src/rx.rs:126-129` afterwards, and `send` does the same at `crates/drivers/virtio-net/src/tx.rs:98-102`. `fill` and `send` bound `frames.count()` by `SLOTS` (`crates/drivers/virtio-net/src/rx.rs:39-42`, `crates/drivers/virtio-net/src/tx.rs:74-77`) and bound `queue.size()` nowhere.

A `Queue` of more descriptors than `SLOTS` hands out a head at or above `SLOTS`; the chain is then in the available ring, `taken.get_mut(head)` answers `None`, `post` refuses with `NetError::UnknownBuffer`, and `posted[index]` stays `false`, so the next `fill` posts the same buffer a second time and the device holds one buffer through two descriptors. `server_net` makes both sizes eight (`crates/user/net-programs/src/net_dma.rs:36-47`), so no caller reaches this today.

Fix: refuse `queue.size() > SLOTS` with `NetError::Slots` at the top of `fill`, `receive`, `send` and `drain`.

---

## F10 — issue #350
Title: driver-virtio-net: a used element naming a buffer outside the area leaves the buffer posted
Labels: enhancement, part::drivers
Body:
`receive` replaces `taken[head]` with `NO_BUFFER` at `crates/drivers/virtio-net/src/rx.rs:97` and refuses at `crates/drivers/virtio-net/src/rx.rs:98-100` when the index is at or above `frames.count()`, without clearing `posted[index]`. The queue has freed the descriptor by then (`crates/virtio/queue/src/queue.rs:388`).

A caller that passes a `Frames` of fewer buffers than the one it filled from, as `crates/drivers/virtio-net/src/tests/rx.rs:213-228` does, leaves `posted[index]` set for a buffer no descriptor names, and `fill` over the original area skips that buffer from then on.

Fix: clear `posted[index]` where `index < SLOTS` before the refusal at `crates/drivers/virtio-net/src/rx.rs:98-100`.

---

## F11 — issue #352
Title: driver-virtio-net: fill accepts receive buffers shorter than the 1526 bytes of virtio 5.1.9.3.1
Labels: enhancement, part::drivers
Body:
`fill` at `crates/drivers/virtio-net/src/rx.rs:33-52` posts every buffer of `frames.len()` bytes (`crates/drivers/virtio-net/src/rx.rs:124`) without a lower bound. Virtio 1.4 section 5.1.9.3.1 (`docs/oasis/virtio-v1.4-cs01.html`) has a driver without `VIRTIO_NET_F_MRG_RXBUF` and without the guest offloads populate the receive queue with buffers of at least 1526 bytes, and section 5.1.9.3.2 has the device use a single descriptor, so a frame longer than the buffer cannot arrive.

A `Frames` of 64-byte buffers fills the ring, and every frame above 52 bytes of payload is then dropped by the device. `server_net` uses 2048-byte buffers (`crates/user/net-programs/src/net_dma.rs:40`), so no caller reaches this today.

Fix: refuse `frames.len() < 1526` in `fill` with a variant `NetError::BufferTooShort(u32)`.

---

## F12 — issue #354
Title: pci: a structure or an MSI-X location carries no check against the register it names
Labels: enhancement, part::drivers
Body:
`virtio::read` answers `offset` and `len` at `crates/pci/src/virtio.rs:217-218` and `msix::read` answers `table` and `pending` at `crates/pci/src/msix.rs:114-115` as the device wrote them; nothing in the crate compares `offset + len` or a table of `vectors * 16` bytes with the `len` of the `Bar` at that index. Each caller checks on its own: `crates/user/net-programs/src/net_registers.rs:62-69` bounds a register access by the structure's length and `crates/user/programs/src/bin/server_init.rs:1830-1838` computes the entry offset with `wrapping_add`.

A structure with `offset = 0xFFFF_F000` and `len = 0x1000` in a 4 KiB register passes both `read` functions, and whether a caller notices depends on the caller.

Fix: add `Bar::holds(offset: u64, len: u64) -> bool` in `crates/pci/src/bar.rs` and call it from `server_init` for the four structures and the two MSI-X locations.

---

## F13 — issue #355
Title: driver-virtio-net: send scans every transmit buffer per call
Labels: enhancement, part::drivers
Body:
`free` at `crates/drivers/virtio-net/src/tx.rs:111-113` scans `busy` from index zero on every `send`, which is O(SLOTS) per frame, and `drain` at `crates/drivers/virtio-net/src/tx.rs:87` walks the used ring before it.

With `SLOTS = 8` the scan is eight loads per frame; a driver with 256 transmit buffers and the device holding the first 255 pays 255 loads per frame on the send path.

Fix: keep the index of the last buffer freed by `drain` and start the scan there, which makes the common case O(1); the option not taken, a free list threaded through `sending`, adds a second structure to keep consistent with `busy`.
