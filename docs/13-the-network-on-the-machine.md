# 13. The Network on the Machine

Document 12 built a network stack that has never seen a device, and
document 11 built a TLS client that has never seen a socket. Both stop at
the same wall: the running system has no way to reach a network card, and
no way to tell the time. This document specifies what is between them and
a sent packet, and orders it into four phases.

It stands to phases 12 to 15 as document 11 stands to the cryptography
track: the roadmap names the deliverables and the acceptance, this
document says what the parts are and why they have the shape they have.

## 13.1 Purpose

`driver-virtio-net` is one crate of maybe eight hundred lines. Everything
in this document exists because that crate cannot be written until seven
other things are true, and not one of the seven is about networking:

1. A userland thread can read a monotonic clock.
2. A userland thread can wait until a point in time.
3. A userland process can draw unpredictable bytes.
4. A device can raise an interrupt that is not an ISA line.
5. A userland process can find a PCI device and its registers.
6. A userland process can touch memory-mapped registers from safe code.
7. A driver can hand a physical address to a device.

The first three are the kernel's business and are wanted by more than the
network. The rest are the bus and the device. The network itself — the
driver, the server, and the protocol its clients speak — is the last of
the four phases and the smallest surprise in it, because the logic below
it was finished in document 12.

## 13.2 What is there and what is missing

| Needed | State today |
|--------|-------------|
| Split virtqueue, chain arithmetic, initialization state machine | `virtio-queue`, step F1 of 12.7.1 |
| Ethernet, ARP, IPv4, IPv6, UDP, TCP, DNS, DHCP, HTTP, the facade | `net-wire` to `net-stack`, steps D1 to D9 |
| `Instant`, `Duration`, the calendar | `audhsos-time`, step E1 |
| A generator and the `Entropy` trait it is seeded through | `crypto-rng`, step T4 |
| A TLS 1.3 client | `audhsos-tls`, step T7 |
| Device memory objects, interrupt objects, notifications | Phases 6 and 9 |
| ACPI root pointer, root table, MADT | `kernel-acpi`, Phase 4 |
| A monotonic clock in userland | **missing** |
| A wait with a deadline | **missing** |
| A source of entropy | **missing**, decided in D-43 and never built |
| MSI-X | **missing** |
| The MCFG table and the ECAM window | **missing** |
| PCI enumeration, BARs, capabilities | **missing** |
| Volatile MMIO from a `forbid(unsafe_code)` crate | **missing** |
| `driver-virtio-net`, `server-net`, the socket protocol | **missing** |

The kernel counts ticks — `KernelState::ticks` — and `system_info`
reports the tick frequency, so the clock is arithmetic over a number the
kernel already has. The scheduler has four blocked states and none of them
has a deadline, so the wait is new work in `kernel-sched`.

## 13.3 Time: a clock, and a thread that waits until

Every timer in the stack of document 12 is a deadline compared against a
`now` the caller passes in: the retransmission timer of RFC 6298, the
lease timers of RFC 2131, the resolver's deadline, the neighbor cache
schedule of RFC 4861. `Stack::poll_at` answers with the `Instant` at
which the stack next has work. A server that cannot sleep until that
instant has to spin, and a server that spins is a system that never
idles.

Two system calls, and one change in the scheduler.

`clock_now` takes no handle and returns one word: microseconds since the
kernel started, which is `Instant::from_micros` without further
arithmetic. The kernel computes it from `KernelState::ticks` and the
calibrated frequency it already reports through `system_info`; the
resolution is therefore the tick and not the microsecond, and the
documentation says so rather than implying a precision the timer does not
have.

`notification_wait_until` takes the notification handle and a deadline in
the same scale. It returns the signalled bits, or zero when the deadline
passed first — a caller that must tell the two apart compares the clock,
and the wrapper in `user-rt` does that once. The existing
`notification_wait` is untouched: a call that works keeps working, and a
new call carries the new argument (D-108).

In `kernel-sched` a thread waiting with a deadline is the existing
`BlockedNotification` with a deadline field — a plain word of microseconds
and not an `Instant`, because no kernel crate depends on `audhsos-time`
and comparing two integers is no reason for the first one to — and the
deadlines live in one list ordered by that number. The tick handler wakes
every thread whose deadline has passed, which is a walk from the front of
that list and stops at the first entry that has not. A woken thread is `Ready` with no bits
set, and a thread signalled before its deadline leaves the list when it
leaves the state. The list is bounded by the thread count, so it needs no
allocation and no pool of its own.

`IndexList` of `audhsos-collections` is the list, and it does not yet have
the one operation this needs: it can push at either end and unlink
anywhere, but it cannot insert in the middle, and the `Link` fields are
not reachable from outside the crate, so no caller can splice for it. It
gains `insert_after(links, node, after)`, with `push_front` as the case
where `after` is `None`. That is a small addition to a finished crate and
it belongs to Phase 12, with its own line in the catalog.

This is not the tickless timer of 8.18. The wake is at tick resolution and
a deadline between two ticks waits for the later one, which for a stack
whose shortest timer is a hundred milliseconds is not a cost worth a
design.

## 13.4 Randomness

D-43 decided the shape and named `RDSEED` in `kernel-hal-x86_64` behind a
system call, and left the specification to the phase that needs it. This
is that phase.

`random_bytes` takes no handle and returns four words, which is the
thirty-two bytes a `ChaChaRng` seed is. The adapter runs `RDSEED` once per
word with a bounded number of retries — the instruction reports failure in
the carry flag when the hardware entropy pool is momentarily empty — and
answers `Unavailable`, which is a code Phase 12 adds, when a word cannot
be filled inside that bound, rather than looping or returning a word the
hardware did not give.

The reference machine does not have the instruction. The CPU model
`qemu64` carries neither `rdrand` nor `rdseed`, which was checked against
QEMU 11.1 through `query-cpu-model-expansion` and not assumed; TCG
provides both once they are asked for, and `-cpu qemu64,+rdrand,+rdseed`
starts without complaint under `enforce`. Section 3.1.1 gains the two
flags (D-110).

A process seeds one `ChaChaRng` from one call at startup and draws
everything else from the generator. Nothing in the system calls
`random_bytes` per packet.

## 13.5 Message interrupts

A PCI device on the `q35` machine can raise an interrupt two ways. The
legacy way is an INTx pin routed through the host bridge to an I/O APIC
input, and which input it lands on is written in the `_PRT` object of the
ACPI namespace — which is AML, which needs an interpreter this system does
not have and will not get. The other way is MSI-X: the device writes a
value the driver chose to an address the driver chose, and on `x86_64`
that address is the local APIC's message region. No routing table, no
namespace, no interpreter.

The system therefore uses MSI-X for PCI devices and keeps the I/O APIC
path for the ISA lines it already drives — the UART and the i8042 (D-111).

`interrupt_create_msi` on `SystemControl` allocates one vector and returns
three things: the `Interrupt` handle, the message address, and the message
data. The driver writes the address and the data into the device's MSI-X
table, which is in the device's own BAR and therefore in the driver's
address space, not the kernel's. `interrupt_bind` and `interrupt_ack` are
the calls they already are.

One consequence is stated rather than hidden. For a line, `interrupt_ack`
unmasks at the I/O APIC, so the kernel can hold off a device that
misbehaves. For MSI-X the mask bit is in the device's table, which is
mapped in the driver and not in the kernel, so `interrupt_ack` on such an
object only clears the outstanding flag and touches no hardware. A device
that raises interrupts faster than its driver services them is quieted by
its driver — for virtio, by suppression through the used ring flag that
`virtio-queue` already implements — and not by the kernel. Masking a
vector the kernel does not own would need the kernel to map device
registers on a driver's behalf, which is a bigger hole than the one it
closes.

The vector space is the same one the I/O APIC lines are allocated from,
and an exhausted space is an error rather than a reused vector. Neither
that error nor the one `random_bytes` needs exists today: the table in
`audhsos-abi` ends at `OutOfMemory`, so Phase 12 adds two codes,
`NoVector` for a vector space with nothing left and `Unavailable` for a
source that would not deliver.

## 13.6 PCI: from the MCFG table to a device

### 13.6.1 Who reads what

The ECAM window's base address is in the MCFG table, which the firmware
places among the other ACPI tables in memory the boot information calls
`AcpiReclaim`. That memory is not MMIO, so `memory_create_device` refuses
it, and there is no other way for a userland process to read it. Either
the kernel reads the table, or the kernel grows a way to hand out
ordinary memory it did not allocate.

The kernel reads the table. `kernel-acpi` gains `mcfg.rs` beside
`madt.rs`, of the same shape: signature, length and checksum first, then
the allocation structures, each naming a base address, a segment group,
and the first and last bus it covers. Nothing else about PCI is in the
kernel (D-112).

`system_info` reports the first allocation as four further result words —
base address, segment group, first bus, last bus — and four zero words on
a machine whose firmware published no MCFG, which is the same shape the
framebuffer got in Phase 9. `Platform` gains `fn ecam(&self) -> Option<Ecam>`,
`ScriptedPlatform` gains it, and `boot::run` reports the window it found —
`[info] ecam=<base> segment=<n> buses=<first>..=<last>` — or
`[info] ecam=absent`. No address is written down here: the base is
whatever the `MCFG` table says, and nothing in the design depends on the
value the reference machine's firmware happens to program.

The window itself is MMIO and must pass the check `memory_create_device`
makes, which admits only ranges the boot information marks
`MmioReserved` — the ranges `kernel-core::memory` collects at bring-up and
`Environment::is_device_memory` answers from. The firmware's memory map
does not always mark the ECAM window, so the kernel records the range the
`MCFG` table named beside those regions and `is_device_memory` answers
from both. The root task then creates one `Device` memory object of
`(last_bus - first_bus + 1) << 20` bytes and grants it to the process that
enumerates.

### 13.6.2 The crate `pci`

A layer-1 logic crate at `crates/pci`, `no_std`,
`#![forbid(unsafe_code)]`, no allocation, no workspace dependency. It
knows nothing about how the bytes are reached: everything goes through

```rust
pub trait ConfigSpace {
    fn read_u32(&self, address: Address, offset: u16) -> Option<u32>;
    fn write_u32(&mut self, address: Address, offset: u16, value: u32);
}
```

where `Address` is the segment, bus, device and function. The crate owns
the ECAM arithmetic — the offset `(bus << 20) | (device << 15) |
(function << 12)` into the window — as a pure function, and the adapter
that holds the mapping calls it rather than deriving it again; a host test
implements the trait over a recorded configuration space of a real `q35`
machine with a virtio-net device on it.

Modules:

- `address.rs`: `Address`, the ECAM offset arithmetic, and the bounds of
  each field, so that the adapter has nothing left to get wrong.
- `header.rs`: the type-0 header — vendor and device id, command and
  status, revision, class, subclass and programming interface, header
  type, the six base address registers, the subsystem ids, and the
  capabilities pointer. A header whose vendor id is `0xFFFF` is an absent
  function and not an error.
- `enumerate.rs`: the walk over buses, devices and functions, bounded by
  the bus range the MCFG named; a multi-function device is recognized by
  bit 7 of the header type, and a function that is not present ends the
  walk of that device rather than the bus. Bridges are read and reported
  and not descended into: the machine of 3.1.1 puts its devices on bus 0,
  and a walk that follows secondary bus numbers is work with no consumer
  (D-112).
- `bar.rs`: base address register decoding. Bit 0 separates memory from
  I/O; bits 2 and 1 say whether a memory register is 32 or 64 bits wide,
  and a 64-bit register takes the next register as its upper half, which
  is why a bar index is checked against what the previous index consumed.
  Size probing writes all ones, reads back, masks the type bits, inverts
  and adds one — and because that write makes the device decode nothing
  meaningful in the meantime, the memory decode bit of the command
  register is cleared first and restored after, which the crate does in
  one function so that a caller cannot forget the second half.
- `capability.rs`: the walk of the capability list from the pointer at
  offset `0x34`, bounded by the number of capabilities that fit in
  configuration space so that a list pointing at itself is an error and
  not a hang, with each entry's id and offset.
- `msix.rs`: the MSI-X capability — the table size, the enable and
  function-mask bits of the message control word, and the BAR index and
  offset of both the table and the pending-bit array. The sixteen-byte
  table entry (address low, address high, data, vector control) is
  written by this crate into a slice the caller provides, so the crate
  computes the bytes and the driver's adapter is what stores them.
- `virtio.rs`: the vendor-specific capabilities of virtio 1.x, from
  section 4.1.4 of the specification in
  [`docs/oasis/`](oasis/README.md) — `cfg_type`, the BAR index, the
  offset and the length of each structure, and the notify multiplier that
  only the notify capability carries. All seven types the specification
  defines are named, the four this driver uses and the three it does not,
  and a value the specification reserves is skipped rather than refused,
  because a device may publish more than a driver knows. Two rules of
  4.1.4 that a first reading loses: a device may offer more than one
  structure of the same type — the example in the specification is a
  notification structure in an I/O BAR and a second in a memory BAR — and
  the order of the capability list is the device's order of preference, so
  the crate reports every structure it found in that order and the driver
  takes the first it can use, rather than the crate deciding for it; and a
  BAR index outside `0` to `5` is a capability that names no register
  block and is refused.

PCI-SIG does not publish its specifications freely, so the layout numbers
of this crate cannot be checked against a document kept beside the code
the way D-59 asks. Every constant therefore names the document and the
revision it comes from in its own doc comment, and
[`docs/pcisig/README.md`](pcisig/README.md) records which documents those
are and how to obtain them (D-124).

## 13.7 MMIO out of safe code

`server-display` writes into a framebuffer and carries
`#![forbid(unsafe_code)]`, because the mapping reaches it as a byte slice
that `user-sys-x86_64` made. Registers cannot be reached that way: a
compiler may fold, reorder, or drop accesses through a plain slice, and a
device register is not memory that behaves.

`user-sys-x86_64` therefore grows a small volatile accessor over a mapped
region — read and write of `u8`, `u16`, `u32` and `u64` at a checked
offset, each one `read_volatile` or `write_volatile` — and its `unsafe`
budget grows by that many sites. Everything above it is safe: `pci`
implements its `ConfigSpace` over the accessor, and `driver-virtio-net`
reaches its registers through a trait the accessor implements, exactly as
`driver-uart16550` reaches its own registers through its `Registers` trait
and `virtio-queue` reaches queue memory through `QueueMemory`. The
virtio-net trait carries the same name as the UART's for the same reason,
in a crate of its own.

The accessor checks the offset against the length of the region it was
made from, so an out-of-range register access is a `None` and not a wild
write. That check is what makes the `unsafe` inside it local: the
precondition is the bound, and the bound is tested on the host (D-113).

## 13.8 DMA without an IOMMU

Section 2.7 already says how: a driver receives a `Ram` memory object with
the `INFO` right and programs the physical address into the device. What
Phase 14 adds is the practice.

The root task creates one memory object for the network driver, maps it
into the driver's process, and grants it with `READ | WRITE | MAP | INFO`.
`memory_info` answers with the physical start, and because a memory object
is one contiguous physical range by construction, an offset into the
mapping is the same offset into physical memory. The driver's
`QueueMemory` implementation is that arithmetic and nothing more.

The region holds, in this order: the descriptor table, the available ring
and the used ring of the receive queue; the same three for the transmit
queue; and the frame buffers. One receive buffer is 2048 bytes, which
holds the twelve-byte header and the largest Ethernet frame the stack
writes; the count of buffers and the queue size are one constant each and
the region's length follows from them.

No IOMMU, and therefore a driver that can write a physical address into a
device can make that device write anywhere. Section 2.11 states that as a
limit the first release accepts, and the mitigation is that the region is
the only thing the driver ever names: the addresses it programs are all
derived from `memory_info` of the one object it holds, and the crate that
derives them has no other source of an address (D-115).

## 13.9 `driver-virtio-net`

A layer-2 logic crate at `crates/drivers/virtio-net`, `no_std`,
`#![forbid(unsafe_code)]`, no allocation, depending on `pci` and
`virtio-queue` and on nothing of the network track: it hands out frames as
byte slices and takes them as byte slices, and what a frame means is
`server-net`'s business. Its MAC address is a `[u8; 6]` for the same
reason.

Register access is a trait with a scripted double behind the feature
`test-doubles`, following `driver-uart16550`:

```rust
pub trait Registers {
    fn read(&self, structure: Structure, offset: u16, width: Width) -> u64;
    fn write(&mut self, structure: Structure, offset: u16, width: Width, value: u64);
}
```

`Structure` is one of the four the virtio capabilities located — common
configuration, notification, ISR status, device configuration — so the
crate never computes an address and the adapter never interprets a field.

Modules:

- `common.rs`: the common configuration structure of section 4.1.4.3 —
  the feature selectors and windows, the queue selector and the queue's
  size, notify offset, and the three physical addresses of its rings, the
  device status byte, and the MSI-X vector fields.
- `features.rs`: negotiation. The driver accepts `VIRTIO_F_VERSION_1`
  (bit 32) and `VIRTIO_NET_F_MAC` (bit 5) and refuses every other offered
  bit by name, as D-52 has `virtio-queue` refuse packed rings, indirect
  descriptors and `EVENT_IDX`. In particular `VIRTIO_NET_F_MRG_RXBUF` is
  refused, which is what makes one receive buffer hold one whole frame,
  and no control queue is negotiated, which is what makes the device two
  queues and not three (D-114).
- `init.rs`: the initialization sequence of sections 3.1 and 4.1.5.1 over
  the state machine `virtio-queue` already has — reset, acknowledge,
  driver, feature negotiation, features-ok, the queues, driver-ok — with
  the MSI-X vector configured per queue as section 4.1.5.1.2 describes,
  and the failure path into `Failed` for each step that can refuse.
- `rx.rs`: receive. Every buffer of the receive area is in the available
  ring from the start; a used element yields the frame behind its
  twelve-byte header, the caller copies what it wants out of it, and the
  buffer goes back into the available ring in the same call, so the device
  is never left with fewer buffers than the driver believes.
- `tx.rs`: transmit. A frame is written into a free transmit buffer behind
  a zeroed header, added as one output chain, and the device notified
  through the notify structure at the queue's own offset. Completions are
  drained before the next send, and a send with no free buffer is a
  refusal the caller retries, not a wait.
- `net.rs`: the device configuration of section 5.1.4, which for the
  negotiated feature set is the six bytes of the MAC address.

What is not in the crate: link status changes, statistics, offloads,
multiqueue, and the control queue. Each is a named refusal in
`features.rs` and a line in the crate documentation, so that an omission
reads as a decision (D-50's rule, applied here).

## 13.10 `server-net`

A layer-u2 logic crate at `crates/user/servers/net`, host-tested, no
system call in it — the process around it is a binary of `user-programs`,
as every other server is.

It runs three threads, because a thread of this kernel waits on exactly
one thing: `Wait` names an endpoint or a notification, never both. That is
the shape the console driver already has, and this server needs one thread
more than it does.

- The **serving thread** owns the device, the driver and the stack, and
  waits on the endpoint. Everything below it is single-threaded, which is
  what keeps `net-stack` free of any question about who may call `poll`.
- The **interrupt thread** waits on the notification the MSI-X vector is
  bound to and sends to the same endpoint under a badge of its own, so
  that the serving thread tells a device interrupt from a client's
  request by the badge — the console driver's arrangement exactly.
- The **timer thread** waits with `notification_wait_until` and sends a
  tick under a third badge when the deadline passes.

One round of the serving thread is then: take the message; if it is the
device badge, drain the receive queue and hand each frame to
`Stack::poll`; if it is a client's, answer it; either way call
`Stack::poll` with no frame until it hands out nothing, sending each frame
it hands out through the device; then ask `Stack::poll_at` for the next
deadline and give it to the timer thread.

Giving it to the timer thread is the one place where two threads of this
server touch the same memory, and it is worth naming rather than
discovering: the deadline is one aligned word in a memory object the
serving thread owns and the timer thread maps, written by one thread and
read by the other, with a notification signalled after the write so that a
timer already asleep on a later deadline wakes and re-reads. No lock,
because there is one writer and one reader and the value is a single word;
no shared stack state, because that word is all there is. The console
driver needed none of this because it has no deadline to keep, and the
alternative — a periodic tick — would make a server that idles into one
that polls, which is what 13.3 built the deadline to avoid.

The stack is configured from the driver's MAC address, DHCP is started at
boot, and the addresses, routes and name servers the lease brings reach
the stack through the calls `net-stack` already has. Without a network
device the server reports no interface and exits, which is the shape
Phase 9 gave the display server on a machine with no framebuffer.

## 13.11 The socket protocol

In `user-proto`, beside the display and input protocols, and built the
same way: a request is a type with `encode` and `decode` and no system
call, and bulk data moves through a ring in a shared memory object rather
than through one IPC call per byte (D-31's shape, D-116).

| Message | Answer |
|---------|--------|
| `Interface` | the MAC address, the addresses, the routes, whether DHCP has a lease |
| `UdpBind { local }` | a socket handle, its ring memory object, and a notification |
| `UdpSendTo { socket, remote }` | the payload is already in the ring |
| `UdpClose { socket }` | - |
| `TcpConnect { socket, remote }` | pending; the notification says when it is established or refused |
| `TcpListen { local, backlog }` | a listener handle |
| `TcpAccept { listener }` | a socket handle and its ring, or pending |
| `TcpSend`, `TcpRecv` | how many bytes moved through the ring |
| `TcpShutdown { socket, direction }`, `TcpClose { socket }` | - |
| `Resolve { name }` | the addresses of both families, or a failure |

One ring per socket, each in its own memory object, each with a header of
write and read sequence numbers and a capacity, exactly as the input
protocol's ring has. A client that is not reading fills its ring and the
stack stops advancing its window, which is the back pressure TCP already
has; nothing in the server grows without bound.

## 13.12 The reference machine grows a network

Section 3.1.1 gains three things, and gains them the way it gained the QMP
socket in Phase 9 — named with the phase they arrive in, not written into
the machine every test has run on since Phase 2. From Phase 12 the CPU
model is `qemu64,+rdrand,+rdseed`. From Phase 13 the runner adds two more
lines:

```
-netdev user,id=n0,hostfwd=tcp:127.0.0.1:<free port>-:7 \
-device virtio-net-pci,netdev=n0,disable-legacy=on,mq=off
```

The device arrives in Phase 13 and not in Phase 14, one phase before
anything drives it, because what Phase 13 has to show is that the bus walk
finds a real device with real base address registers and a real MSI-X
table — and this is that device. Before Phase 13 the machine has none: a
device that neither a driver nor a bus walk looks at is one more thing for
an unrelated test to trip over.

`disable-legacy=on` makes it a non-transitional virtio 1.0 device, which
is the only kind `virtio-queue` and this driver implement; its PCI device
id is then `0x1041` and not the transitional `0x1000`. `mq=off` is the
default and is written down because the driver depends on it.

`-netdev user` is what makes the end-to-end tests need no host network and
no privileges: QEMU's user-mode network is a DHCP server, a gateway at
`10.0.2.2`, and a DNS forwarder at `10.0.2.3`, and `hostfwd` gives the
test a port on the loopback of the development machine that reaches a
listener inside the guest. The run without a network keeps the machine as
it is today and drops the two lines, and both Phase 13 and Phase 14 are
accepted on that run as well, as `-vga none` is the second run Phase 9 is
accepted on (D-118).

## 13.13 Testing

Everything above the two adapters is host-tested against a double:

| Crate | Double | What it stands for |
|-------|--------|--------------------|
| `pci` | `RecordedConfigSpace` | the configuration space of a `q35` machine with a virtio-net device, captured once and kept as a test fixture |
| `driver-virtio-net` | `ScriptedRegisters` | a device that answers a scripted sequence and records every write |
| `driver-virtio-net` | `virtio-queue`'s existing memory double | the rings, in a byte array a test owns |
| `server-net` | `net-stack`'s existing network double | a link that delays, duplicates, reorders and drops |
| `kernel-sched` | the existing scheduler tests | the deadline list, against a model that keeps the same deadlines in a sorted vector |

Two fuzz targets: `pci_config`, over a configuration space of arbitrary
bytes, which must enumerate or refuse and never loop; and `virtio_net_rx`,
over a used element and a buffer of arbitrary bytes, which must yield a
frame or refuse and never read outside the buffer.

The end-to-end tests are QEMU runs, and each is a line the runner checks:

- the driver reports the MAC address the command line gave the device;
- DHCP reaches a lease and the address is the `10.0.2.15` the built-in
  server hands out first;
- ARP resolves the gateway;
- a DNS query for a name the forwarder answers comes back;
- a TCP connection to the port `hostfwd` opened carries a payload both
  ways and closes cleanly;
- an HTTP `GET` over that connection returns a response the client parses;
- with the two network lines dropped, the server reports no interface and
  the run ends by itself.

Phase 15 adds one more: the same `GET` over TLS, against a server the test
starts on the development machine with a certificate the test builder of
`audhsos-x509` wrote.

## 13.14 Order of work

| Step | Content | Phase | Size |
|------|---------|-------|------|
| N1 | `clock_now`, `notification_wait_until`, `IndexList::insert_after`, and the deadline list in `kernel-sched` | 12 | M |
| N2 | `random_bytes` over `RDSEED`, and `+rdrand,+rdseed` on the reference machine | 12 | S |
| N3 | `interrupt_create_msi` and the MSI vector allocator | 12 | M |
| N4 | `kernel-acpi::mcfg`, the ECAM words of `system_info`, the device memory region | 13 | S |
| N5 | the crate `pci` | 13 | M |
| N6 | the volatile accessor in `user-sys-x86_64`, the network device on the reference machine, and a program that enumerates | 13 | S |
| N7 | `driver-virtio-net` | 14 | L |
| N8 | the DMA region, and `server-net` around the stack | 14 | L |
| N9 | the socket protocol in `user-proto`, and a client that uses it | 14 | M |
| N10 | the end-to-end tests of 13.13 | 14 | M |
| N11 | the TLS transport: step T8 of document 11 | 15 | M |

N1 to N3 are worth having whether or not the network follows: a clock, a
deadline, and entropy are wanted by every server that has a timeout, and
MSI-X is wanted by every PCI device. That is why they are a phase and not
the first third of one.

## 13.15 Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| MSI-X cannot be masked by the kernel | a device that floods keeps a core busy | the driver suppresses through the used ring flag `virtio-queue` implements; the consequence is written down in 13.5 rather than discovered |
| The firmware's memory map does not mark the ECAM window | `memory_create_device` refuses the window and nothing can enumerate | the kernel admits the range the MCFG table named, beside the `MmioReserved` regions |
| PCI-SIG layouts cannot be checked against a document in the repository | a constant is wrong and nothing catches it | every constant names its document and revision; the recorded configuration space of a real machine is the second check |
| The deadline list makes the tick handler slow | every interrupt costs more | the list is ordered, the walk stops at the first deadline that has not passed, and the length is bounded by the thread count |
| The stack meets a real device badly | rework in Phase 14 | the seam is `Stack::poll(now, rx, tx, rng)`, which is bytes in and bytes out, and both sides of it were tested against doubles before they met |
| Phase 14 is XL and stalls | the release slips | N7, N8 and N9 are separate crates with separate catalog items, and N10 needs only N7 and N8 |
| TCG has no `RDSEED` | the entropy call fails on the reference machine | measured against QEMU 11.1 before the decision: the flags are accepted and TCG provides both instructions |
