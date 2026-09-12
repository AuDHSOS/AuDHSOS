# 15. The disk on the machine

## 15.0 How to read this document

Every step below has the same six parts, in the same order: Status,
Depends on, Size, Needs, Does, Produces, Done when. Nothing is implied.
Every term is defined in 15.1. Every term has exactly one name, used
everywhere.

## 15.1 Terms

| Term | Meaning |
|------|---------|
| root task | `server-init`, the first user process. It holds `Role::SystemControl` and starts every other process. |
| file system server | `server-fs`, the process this document is about. It does not exist yet. |
| driver process | Any user process that drives a hardware device. The file system server is one. |
| register window | A `Device` memory object over the base address register of the virtio block device. The driver process reads and writes device registers through it. |
| DMA region | A `Ram` memory object that the device reads and writes. It holds the virtqueue rings and the request slots. |
| scratch disk | The second `virtio-blk-pci` disk of the reference machine, added by D-136. It is not the boot disk. |
| boot disk | The disk the firmware reads. It holds the loader, the kernel and the boot image. |
| boot set | The programs that must start before a file is readable: root task, memory server, name server, console server, file system server. |
| enumerate | Read the PCI configuration space of every bus, device and function in turn. The crate `pci` does this. |
| sector | 512 bytes. Virtio counts in 512 bytes regardless of the device's block size (virtio 5.2.5). |

## 15.2 Goal

At the end of the track, two things are true that are not true now:

1. A program can create, read, write and delete a file on a disk.
2. The root task starts programs by reading them off that disk, instead
   of reading them out of the archive in the boot image.

## 15.3 What is already built

These four crates are finished, host-tested, and under the coverage gate.
None of them has ever run on a machine.

| Crate | What it does | Decided in |
|-------|--------------|------------|
| `virtio-queue` | The split virtqueue and the device initialization state machine. | F1, D-52 |
| `fs-fat` | FAT32 over a `BlockDevice` trait. | F2, D-53 |
| `fs-gpt` | The GUID partition table over the same trait. | F3, D-138 |
| `driver-virtio-blk` | The virtio block device: registers, features, initialization, request framing. | F4, D-139 |

Four more pieces are also finished:

| Piece | What it does | Built in |
|-------|--------------|----------|
| `Mmio` in `user-sys-x86_64` | Volatile read and write of a mapped region, bounds-checked. | Phase 13, D-113 |
| `pci` | Base address register decoding, the capability list, the virtio structures, the MSI-X capability. | Phase 13 |
| `interrupt_create_msi`, `notification_wait_until` | Allocate a message interrupt; wait with a deadline. | Phase 12 |
| The reference machine | It can carry a scratch disk when a run asks for one. | 3.1.1, D-136 |

## 15.4 What is missing

Five things are missing. Each one is in the root task or in the startup
message. None is in a logic crate.

| # | Missing | Why it is missing |
|---|---------|-------------------|
| 1 | A driver process cannot reach device registers. | The root task creates device memory only for what `system_info` reports. No code in the root task enumerates the PCI bus. |
| 2 | A driver process cannot allocate a message interrupt. | `interrupt_create_msi` requires the system control handle. Only the root task receives `Role::SystemControl`. |
| 3 | No implementation of `virtio_queue::QueueMemory` exists, except the test double `RamQueue`. | Nobody has written one. |
| 4 | No implementation of `driver_virtio_blk::Registers` exists. | Nobody has written one. |
| 5 | No file protocol exists in `user-proto`. | Nobody has written one. |

One further gap is about the machine, not the code: no automatic run
attaches the scratch disk. A person types `--scratch` by hand.

## 15.5 Decision D1: who writes the MSI-X table entry

This decision must be made before S1 starts. It changes how many startup
roles S1 adds.

**The decision: the root task writes the entry.**

Reason 1: the root task holds the system control handle, so it already
has the values.
Reason 2: the root task can map the register window while it writes, and
unmap it afterwards.
Reason 3: `pci::msix` already provides `write_entry`, `set_enabled` and
`set_function_mask`.
Result: the startup message carries one handle, for the notification. It
carries no MSI-X address, no MSI-X data and no table entry number.

**The option not taken: the driver process writes the entry.** D-111 says
this. It costs three extra startup roles, which carry an address word and
a data word that a driver process cannot interpret.

**Consequence for D-111:** D-111 then applies to the network device only.
D-111 was written for the network device.

## 15.6 Question closed: where the DMA region comes from

This was an open question. It is now answered, so it is not a decision.

**The answer: the driver process asks the memory server, like any other
program, and then calls `memory_info`.**

The chain of evidence has three links:

1. The root task grants `Ram` objects to the memory server with
   `ObjectRights::MEMORY`. That rights set contains `INFO`.
2. `memory_split` installs the second object with the rights of the
   first. See `crates/kernel/syscall/src/calls/memory.rs`, line 356.
3. An IPC transfer gives the receiver a handle with the same rights. See
   `crates/kernel/ipc/src/transfer.rs`, line 49.

Therefore the handle the memory server returns carries `INFO`, and
`memory_info` on it succeeds.

**Consequences:** no new startup role is needed for memory. The root task
needs no special case. D-115's requirement still holds, because a memory
object is one contiguous physical range, so an offset into the mapping is
the same offset into physical memory.

## 15.7 The order of the steps

| Step | Name | Status | Depends on | Size |
|------|------|--------|------------|------|
| S1 | The root task finds the device and hands it over | not started | D1 (15.5) | L |
| S2 | The register adapter | not started | S1 | M |
| S3 | The queue memory adapter | not started | S1 | M |
| S4 | The file system server reaches the disk | not started | S2, S3 | L |
| S5 | The file protocol | not started | nothing | M |
| S6 | The file system server answers clients | not started | S4, S5 | M |
| S7 | The machine carries the disk automatically | part built | S4 | S |
| S8 | The end-to-end tests | not started | S6, S7 | M |
| S9 | Programs move onto the volume | not started | S6, S8 | L |

S5 depends on nothing. It can be built at any time before S6. Every other
step depends on the step before it.

## 15.8 S1. The root task finds the device and hands it over

Status: not started.
Depends on: decision D1 (15.5).
Size: L.

### Needs (already built)

- `pci`, for enumeration, base address registers, capabilities and MSI-X.
- `memory_create_device`, to create the register window.
- `interrupt_create_msi`, to allocate a message interrupt.
- `notification_create` and `interrupt_bind`, to attach the interrupt to
  a notification.
- The mapping technique of `app-lspci`: map one bus of the configuration
  window at a time, at one address, and unmap it before the next bus.
  One bus costs one mebibyte of page tables (8.15).

### Does, in order

1. Map one bus of the configuration window. Enumerate it. Unmap it.
   Repeat for every bus.
2. Look for a function whose vendor is `0x1AF4` and whose device is
   `pci::virtio::BLOCK_DEVICE`, which is `0x1042`.
3. If no such function exists, stop. Start no file system server. This is
   the run the machine performs today, and it must keep working.
4. Read the four virtio structures with `pci::virtio::structures`. Each
   structure has a base address register, an offset and a length.
5. Read the notification multiplier from the notification structure.
6. Read the base address registers with `pci::bar::probe`. This call
   clears the memory decode bit and restores it inside the same call, so
   it must run before the device is used.
7. Read the MSI-X capability with `pci::msix::read`.
8. Create one `Device` memory object over the base address register that
   holds the four structures. In QEMU's `virtio-blk-pci` all four lie in
   one 64-bit register, so one object covers all four. Align the window
   outward to whole frames. Keep the four offsets relative to the
   register base, not to the frame.
9. Grant that object with `ObjectRights::DEVICE`.
10. Call `interrupt_create_msi` to allocate one vector.
11. Call `notification_create`.
12. Call `interrupt_bind` to attach the vector to one bit of that
    notification.
13. Map the register window into the root task, write the MSI-X table
    entry with `pci::msix::write_entry`, enable MSI-X, and unmap the
    window. This is decision D1.
14. Send the startup message with the roles below.

### Produces: six new startup roles

Add each role to the `roles!` table in `audhsos-abi::startup`.

| Role | Kind | Carries |
|------|------|---------|
| `BlockRegisters` | handle | The register window. |
| `BlockCommon` | value | Offset of the common configuration structure in the high half of the word, its length in the low half. |
| `BlockNotify` | value | The same two numbers for the notification structure. |
| `BlockIsr` | value | The same two numbers for the interrupt status structure. |
| `BlockConfig` | value | The same two numbers for the device configuration structure. |
| `BlockNotifyMultiplier` | value | The multiplier of virtio 4.1.4.4. |
| `BlockInterrupt` | handle | The interrupt object. The driver process acknowledges the interrupt through it. |
| `BlockNotification` | handle | The notification the vector is bound to. |
| `BlockVectorBit` | value | The bit of that notification. |

Each role costs three things: one line in `roles!`, one match arm in
`user_rt::Startup`, and one test. The test checks that the arm accepts
the role once and refuses a second copy of it.

### Done when

A program in the archive does all of the following and then ends:

1. It reports every role it received.
2. It reads the device status byte through the register window.
3. It reports that the status byte is zero.

## 15.9 S2. The register adapter

Status: not started.
Depends on: S1.
Size: M.

### Needs (already built)

- The trait `driver_virtio_blk::Registers` (D-139).
- `Mmio` in `user-sys-x86_64`, which checks every access against the
  length of the region (D-113).

### Does

The adapter stores the mapping and the four structure locations from the
startup message. For each call:

1. Take the `Structure`, the offset and the `Width`.
2. Add the structure's own offset to the given offset.
3. Check the sum against that structure's length.
4. Read or write through `Mmio` at the width, using a match on `Width`.

`Mmio` already answers zero and writes nothing for an access outside the
region.

### Produces

No change to the `unsafe` budget. The adapter calls `Mmio` and contains
no `unsafe` of its own.

### Done when

The file system server can read the device status through the adapter.
S1's program already proved the same read through a direct call.

## 15.10 S3. The queue memory adapter

Status: not started.
Depends on: S1.
Size: M.

### Needs (already built)

- The trait `virtio_queue::QueueMemory`.
- `memory_info`, which answers the physical start of a memory object.
- D-115: a memory object is one contiguous physical range.

### Does

1. Ask the memory server for the DMA region. Map it.
2. Call `memory_info`. Keep the physical start.
3. For any offset into the mapping, the physical address is the physical
   start plus that offset. This is the whole of the address arithmetic.

Lay the region out at fixed offsets, in this order:

| Offset | Contents |
|--------|----------|
| 0 | descriptor table |
| after it | available ring |
| after it | used ring |
| after it | request slots |

One request slot holds three things: a sixteen-byte request header, a
one-byte status, and one sector of 512 bytes.

Two constants fix the size: 8 descriptors and 2 request slots. Two slots
carry one sector at a time, which is what `fs-fat` asks for. The length
of the region follows from these two constants.

### The barrier

`QueueMemory::barrier` does nothing by default. That default is wrong
here. Virtio 2.7.13.3.1 and 2.7.13.4.1 require the ordering.

Use `core::sync::atomic::fence(Ordering::SeqCst)`. A processor fence is
not required, because on this machine the device is software running on
the same processor. The call is safe code.

### Done when

`Queue::new` succeeds over the adapter, and a chain added to the queue
appears in the available ring at the right physical address.

## 15.11 S4. The file system server reaches the disk

Status: not started.
Depends on: S2, S3.
Size: L.

### Needs (already built)

- `driver-virtio-blk`, for the device.
- `fs-gpt`, for the partition table.
- `fs-fat`, for the volume.
- The wall clock of D-137, for the timestamp a directory entry carries.
- `notification_wait_until`, for the deadline below.

### Where the code goes

- Logic: a layer-u2 crate at `crates/user/servers/fs`, host-tested.
- Process: one binary of `user-programs`, as every other server has.

### The thread count: one

The server uses one thread. `server-net` uses three.

A disk read is synchronous. It has five steps and none of them can be
skipped:

1. Submit the three-part chain to the queue.
2. Notify the device.
3. Wait for the interrupt.
4. Drain the used ring.
5. Copy the sector out of the slot.

The kernel allows a thread to wait on one object at a time. So the one
thread waits on the endpoint at the top of its loop, and waits on the
notification inside step 3.

What this server therefore does not need: a badge for the device, a word
shared between threads, and a second thread.

The cost: while one client's sector is in flight, another client waits.
A queue of two slots produces that same wait anyway.

A second thread and a deeper queue are a later version. They are not a
repair of this one.

### The deadline

The wait in step 3 carries a deadline. Without a deadline, a device that
stops answering leaves the server asleep forever.

When the deadline passes: answer the request with a failure, and reset
the queue.

### The block device adapter

Implement `fs_fat::BlockDevice` over the driver.

| Method | What it does |
|--------|--------------|
| `sectors` | Answers `Blk::capacity`. |
| `read` | Submits one chain, waits, copies the sector out of the slot. |
| `write` | Copies the sector into the slot, submits one chain, waits. |

The copy is necessary because the caller owns its buffer and the device
reads only the DMA region.

`Blk::submit` already refuses everything virtio 5.2.6.1 forbids. Two
checks remain for this adapter:

1. Judge the status byte with `request::status`.
2. Read no further into the slot than the completion's length.

### Mounting the volume

Two cases.

Case A, the disk has a partition table:
1. `fs_gpt::read` reads the table.
2. `fs_gpt::find` finds the partition whose type is `ESP_TYPE_GUID`.
3. Make a `BlockDevice` over that partition's sectors.
4. `fs_fat::FileSystem::mount` mounts it.

Case B, the disk has no partition table:
1. `fs_fat::FileSystem::format` formats the whole disk.

Case B runs first, because the scratch disk arrives blank.

### Done when

The server mounts the scratch disk and prints its geometry on the
console: the cluster count, where the tables are, how many clusters are
free. It has no clients and no protocol at this point.

## 15.12 S5. The file protocol

Status: not started.
Depends on: nothing.
Size: M.

### Where the code goes

`user-proto`, beside the display and input protocols. A request is a type
with `encode` and `decode`. It contains no system call.

### The messages

| Message | Answer |
|---------|--------|
| `Open { parent, name }` | A handle, the size, and whether it is a directory. |
| `Create { parent, name, directory }` | A handle. |
| `Read { file, offset, len }` | The bytes, up to what one message carries. |
| `Write { file, offset }` | How many bytes the server took. |
| `ReadDir { dir, cursor }` | One entry and the next cursor, or the end. |
| `Stat { file }` | Size, attributes, and the moment the file was made. |
| `Remove { parent, name }` | Nothing. |
| `Close { file }` | Nothing. |
| `Flush` | Nothing. |

### Three rules

1. A handle is a number the server chose. It is not a kernel object,
   because a file is not a kernel object.
2. The server keeps one table of open files per client. It finds the
   table by the badge on the client's endpoint. The display server finds
   a surface the same way.
3. Bulk data travels in the message area. The protocol states the maximum
   size, so a client loops over a large file instead of discovering the
   limit by failing.

A ring per open file, as D-116 describes, is the next version of this
protocol. It is not needed for this one.

### Done when

The protocol encodes and decodes every message above, and the host tests
cover every refusal.

## 15.13 S6. The file system server answers clients

Status: not started.
Depends on: S4, S5.
Size: M.

### Does

The loop of 15.11 gains four steps:

1. Decode the request.
2. Find the client's table of open files by the badge.
3. Call into `fs-fat`.
4. Encode the answer.

Every refusal of `fs-fat` maps to a refusal of the protocol. Write one
`From` implementation and one table test, as every other error mapping in
this system does.

### Done when

A program in the archive performs this sequence and prints the result:

1. Create a file.
2. Write one line into it.
3. Close it.
4. Open it again.
5. Read the line back.
6. Compare the line with what it wrote.

## 15.14 S7. The machine carries the disk automatically

Status: part built.
Depends on: S4.
Size: S.

### Already built (D-136)

- The two `virtio-blk-pci` lines on the reference machine.
- One blank scratch disk per run name, kept under `target/qemu/`.
- `sh tools/xtask.sh run --scratch`, which attaches it.
- Measured: QEMU 11.1 accepts the lines and reports the device as
  `1af4:1042`.

### Still missing

1. `sh tools/xtask.sh test --e2e` must attach a scratch disk of its own.
2. The root task must start the file system server only when the
   enumeration of S1 found a device. A run without the disk must still
   come up.
3. The end-to-end test must boot the same disk a second time. That second
   boot is the persistence test, and it is the reason the scratch disk
   exists.

### Done when

1. `sh tools/xtask.sh test --e2e` runs with a scratch disk attached and
   no flag typed by hand.
2. The same run without the disk reports no file system server and
   passes.

## 15.15 S8. The tests

Status: not started.
Depends on: S6, S7.
Size: M.

### Host tests

Three doubles already exist and cover most of the logic:

| Double | Crate | Stands in for |
|--------|-------|---------------|
| `RamDevice` | `driver-virtio-blk` | the device registers |
| `RamQueue` | `virtio-queue` | the rings |
| `RamDisk` | `fs-fat` | the volume |

The doubles cannot cover the adapters of S2 and S3, because those
adapters exist to touch real hardware. The machine tests cover them.

### Machine tests

1. The geometry the server reports matches what the FAT32 format
   requires.
2. A file written and read back returns byte for byte. Its length crosses
   a cluster boundary.
3. A file created in the first boot is found in the second boot, with the
   same bytes and the same timestamp.
4. The driver refuses a read past the capacity and sends nothing to the
   device.
5. A machine started without the scratch disk reports no file system
   server and comes up regardless.

### Done when

All five machine tests above pass, and `sh tools/xtask-check.sh` exits
with 0.

## 15.16 S9. Programs move onto the volume

Status: not started.
Depends on: S6, S8.
Size: L.

### Does

1. The image writer gains a directory `AUDHSOS/BIN/` and writes the
   program files into it.
2. The archive in the boot image shrinks to the boot set.
3. The root task reads every program outside the boot set off the volume.

### Why the boot set stays in the archive

The root task cannot read a file before the file system server runs. The
file system server is itself a file. So the boot set must come from
somewhere that needs no file system server, which is the archive. This is
also why S9 is the last step.

### What is already built

- The image writer already places files on a FAT32 volume through
  `fs-fat` (D-53, D-138).
- The ustar reader of `user-loader` already reads the archive. It stays,
  for the boot set.

### Two limits

1. An 8.3 name has eight characters for the name (D-09). The program
   `app-canvas` becomes the file `APPCANVA.ELF`. This stays until
   `fs-fat` learns long file names, which 8.18 lists as later work.
2. The firmware reads the boot disk. Therefore S9 reads programs from the
   boot disk and writes nothing to it. All writes go to the scratch disk
   (D-136).

### Done when

1. The archive in the boot image holds the boot set and nothing else.
2. A program outside the boot set starts after the root task reads it off
   the volume.
3. The end-to-end test passes with the smaller archive.

## 15.17 Risks

| # | Risk | Effect | What reduces it |
|---|------|--------|-----------------|
| 1 | The root task gains bus enumeration. | The process with the full authority grows larger. | The enumeration code is `pci`'s, not the root task's. S1 ends with a program that proves the handover before anything is built on top of it. |
| 2 | The MSI-X handover is wrong, and no test catches it. | The server waits for an interrupt that never arrives. | S1's program reports every value it received. The first wait in S4 carries a deadline. |
| 3 | One thread makes the server slow. | A client waits while another client's sector is in flight. | 15.11 states this cost. The second thread is a later version, not a repair. |
| 4 | A device fails part way through a request. | The server answers with wrong bytes. | The status byte is judged. The completion's length bounds the read. `virtio-queue` refuses a used element that names a descriptor it did not hand out. |
| 5 | The server formats a blank disk. | A test passes on an empty disk and fails on a real volume. | The server reads where a volume exists and formats only where none does. The persistence test boots the same disk twice. |
| 6 | An 8.3 name is too small for a program name. | File names are hard to read. | This is D-09's known limit. Long file names are 8.18's later work. |
