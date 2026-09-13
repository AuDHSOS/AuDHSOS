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

## 15.4 What was missing

Five things were missing when this document was written. Each one was in
the root task or in the startup message; none was in a logic crate. Every
one of them is built, and this table says where.

| # | What was missing | Where it is now |
|---|------------------|-----------------|
| 1 | A driver process cannot reach device registers. | The root task enumerates the bus and makes the window: `find_block` in `crates/user/programs/src/bin/server_init.rs`. |
| 2 | A driver process cannot allocate a message interrupt. | The root task allocates the vector and binds it, and hands the driver the interrupt and the notification: `prepare`, same file. |
| 3 | No implementation of `virtio_queue::QueueMemory` exists, except the test double `RamQueue`. | `crates/user/programs/src/dma.rs`. |
| 4 | No implementation of `driver_virtio_blk::Registers` exists. | `crates/user/programs/src/registers.rs`. |
| 5 | No file protocol exists in `user-proto`. | `crates/user/proto/src/file.rs`. |

A sixth gap was about the machine and not the code: no automatic run
attached the scratch disk. `sh tools/xtask.sh test --e2e` now attaches one
of its own, blank on every run.

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
   first. See `crates/kernel/syscall/src/calls/memory.rs`, line 358.
3. An IPC transfer gives the receiver a handle with the same rights. See
   `crates/kernel/ipc/src/transfer.rs`, line 49.

Therefore the handle the memory server returns carries `INFO`, and
`memory_info` on it succeeds.

**Consequences:** no new startup role is needed for memory. The root task
needs no special case. D-115's requirement still holds, because a memory
object is one contiguous physical range, so an offset into the mapping is
the same offset into physical memory.

## 15.7 Decision D2: the `unsafe` budget of `user-programs`

This decision must be made before S1 starts. `user-programs` stands at
thirty-four `unsafe` sites against a budget of thirty-four
(`crates/tools/xtask/src/policy.rs`, line 825), so S1 fails
`sh tools/xtask.sh unsafe-budget` on its first mapping.

**The decision: raise the budget to thirty-nine, and name the five
sites.**

| Site | Step | What it maps |
|------|------|--------------|
| 1 | S1 | One bus of the configuration window, in the root task, for the enumeration. |
| 2 | S1 | The register window, in the root task, for `pci::msix::write_entry`. |
| 3 | S1 | The register window, in the program S1 ends with. |
| 4 | S2 | The register window, in the file system server, for the `Registers` adapter. |
| 5 | S3 | The DMA region, in the file system server, for the `QueueMemory` adapter. |

Reason 1: each site is one `unsafe { mapping.bytes() }`, which
`crates/user/programs/src/mapping.rs`, line 104, requires of every caller.
Reason 2: neither adapter holds `unsafe` of its own; each takes the byte
slice that call answers.
Reason 3: a named count is checkable. A sixth site fails the budget, and
is a change to this decision rather than to a number.

**The option not taken: make `Mapping::bytes` safe.** Three facts give the
call what it needs of the region: `Mapping::unmap` takes `self`
(`crates/user/programs/src/mapping.rs`, line 169), `unmap_all` is private
(line 207), and the kernel refuses a region that overlaps one the process
holds (`crates/kernel/mm/src/address_space.rs`, line 287). A live
`&mut Mapping` therefore names bytes that are mapped and that no second
`Mapping` covers, and the budget would fall by twelve instead of rising by
five. What the signature cannot give is the kind of object behind the
bytes. A `Mapping` carries `Device` objects as well as `Ram` ones — the
configuration window of `app-lspci`
(`crates/user/programs/src/bin/app_lspci.rs`, line 136) and the
framebuffer of `server-display`
(`crates/user/programs/src/bin/server_display.rs`, line 247) — and a
reference to device memory is read and written without `volatile`, which
is what `Mmio` answers (D-113). Site 5 is worse: the device writes the DMA
region while the program holds the slice. A safe `bytes` therefore costs a
second type for device mappings and a rewrite of eleven call sites in
seven files this track does not touch. It belongs in 8.18.

## 15.8 Decision D3: what the kernel calls device memory

This decision had to be made during S1. `memory_create_device` refused the
base address register of the block device, and no driver can reach a
register the root task cannot make an object over.

**The decision: a range is device memory when it lies inside an aperture
the firmware marked, or when no region of the firmware's memory map
reaches any frame of it.**

Reason 1: the reference machine puts the four structures in a sixty-four
bit register at `0xc000004000`, and the firmware's map describes nothing
between `0xf0000000` and `0xfd00000000`. The register lies in that gap.
Reason 2: a firmware describes the memory of the machine and what it uses
itself. A window it leaves to an operating system it does not describe, so
an undescribed range is a window and nothing else.
Reason 3: a range that meets memory is refused before this is asked, by
`meets_ram`, so the rule gives away no memory.
Reason 4: a range that meets a described region is still refused, whatever
that region is for, so the ACPI tables and the firmware's own reserved
ranges stay out of reach.

Where: `crates/kernel/core/src/memory.rs`, `KernelMemory::is_device_memory`.
The bring-up keeps every region of the map, rounded outward to frames,
beside the apertures it already kept.

**The option not taken: read the apertures of the host bridge from ACPI.**
The `_CRS` of the bridge names them, which is the answer the firmware
itself would give. It costs an AML interpreter in the kernel, which this
system does not have and should not gain for one range.

**The option not taken: add the apertures in the loader.** UEFI's
`EFI_PCI_ROOT_BRIDGE_IO_PROTOCOL` answers them, and the loader already
adds an MMIO region for a framebuffer the firmware did not mark
(`crates/boot/uefi-x86_64/src/bootinfo.rs`, line 118). It costs a protocol
this loader does not use and works only where the firmware is UEFI.

## 15.9 The order of the steps

| Step | Name | Status | Depends on | Size |
|------|------|--------|------------|------|
| S1 | The root task finds the device and hands it over | built | D1 (15.5), D2 (15.7), D3 (15.8) | L |
| S2 | The register adapter | built | S1 | M |
| S3 | The queue memory adapter | built | S1 | M |
| S4 | The file system server reaches the disk | built | S2, S3 | L |
| S5 | The file protocol | built | nothing | M |
| S6 | The file system server answers clients | built | S4, S5 | M |
| S7 | The machine carries the disk automatically | built | S4 | S |
| S8 | The end-to-end tests | built | S6, S7 | M |
| S9 | Programs move onto the volume | blocked, see D4 (15.19) | S6, S8 | L |

S5 depends on nothing. It can be built at any time before S6. Every other
step depends on the step before it.

## 15.10 S1. The root task finds the device and hands it over

Status: built.
Depends on: decisions D1 (15.5) and D2 (15.7).
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

### Produces: nine new startup roles

Add each role to the `roles!` table in `audhsos-abi::startup`, which ends
at `EcamBuses = 19` today (`crates/abi/src/startup.rs`, line 140).

| Number | Role | Kind | Carries |
|--------|------|------|---------|
| 20 | `BlockRegisters` | handle | The register window. |
| 21 | `BlockCommon` | value | Offset of the common configuration structure in the high half of the word, its length in the low half. |
| 22 | `BlockNotify` | value | The same two numbers for the notification structure. |
| 23 | `BlockIsr` | value | The same two numbers for the interrupt status structure. |
| 24 | `BlockConfig` | value | The same two numbers for the device configuration structure. |
| 25 | `BlockNotifyMultiplier` | value | The multiplier of virtio 4.1.4.4. |
| 26 | `BlockInterrupt` | handle | The interrupt object. The driver process acknowledges the interrupt through it. |
| 27 | `BlockNotification` | handle | The notification the vector is bound to. |
| 28 | `BlockVectorBit` | value | The bit of that notification. |

Each role costs three things: one line in `roles!`, one match arm in
`user_rt::Startup`, and one test. The test checks that the arm accepts
the role once and refuses a second copy of it.

### Done when

A program in the archive does all of the following and then ends:

1. It reports every role it received.
2. It reads the device status byte through the register window.
3. It reports that the status byte is zero.

### What was built, where it differs

1. The enumeration runs when the program that receives the device is
   started, and not at the start of the machine. Reason: the console
   driver is not up at the start, so a refusal there is a boot that says
   nothing. `crates/user/programs/src/bin/server_init.rs`,
   `block_device`.
2. The message table lies in another base address register than the four
   structures — on this machine `bar1` and `bar4` — so the root task makes
   a second device memory object over that register, writes the entry
   through it, and closes it again. Step 8 assumed one register for both.
3. The offsets the four value roles carry count from the start of the
   window and not from the base address register. Reason: the window is
   aligned outward to frames, so a register that does not begin at a frame
   would otherwise cost every driver the same addition.
4. `COMMAND_MEMORY` and `COMMAND_BUS_MASTER` go on **before** the entry of
   the message table is written. Reason: `pci::bar::probe` leaves the
   command register as the firmware left it, and a firmware enables the
   decode of what it uses itself. The scratch disk is not the boot disk,
   so its decode was off and the first write of the entry reached nothing.
5. The entry is stored through `Mmio`, one word at a time, out of a buffer
   `pci::msix::write_entry` filled. Reason: the table is a register of the
   device, and a store through a plain reference to it is not volatile and
   was dropped — the entry then read back as the zeros the device reset it
   to.
6. The program of `Done when` was `app-vblk` while the work was done. It
   was replaced by the file system server itself, because two programs
   that reset one device are two drivers of it. What it proved is now the
   end-to-end run's `block_lines`, which reads the capacity and the
   geometry off the machine.

## 15.11 S2. The register adapter

Status: built.
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

`Registers::read` answers `u64` and `Registers::write` answers nothing, so
neither call carries a refusal. `Mmio` answers `None` for a read outside
the region and `false` for a write
(`crates/user/sys-x86_64/src/mmio.rs`, line 74). The adapter answers zero
for the first and drops the second. Reason: step 3 refuses the access
already, so the substitute stands only where step 3 is wrong.

### Produces

Site 4 of D2: the file system server maps the register window and hands
the byte slice to `Mmio`. The adapter itself contains no `unsafe`.

### Done when

The file system server can read the device status through the adapter.
S1's program already proved the same read through a direct call.

### What was built, where it differs

The adapter is `user_programs::registers::Window` and not a module of the
server. Reason: the server is a binary of `user-programs`, and the mapping
it works through is that crate's, so the adapter lives beside `Mapping`
where both reach it.

## 15.12 S3. The queue memory adapter

Status: built.
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

### What was built, where it differs

The adapter is `user_programs::dma::Dma`, beside the register adapter and
for the same reason. The whole region is 1312 bytes, which is one page,
and one of the two request slots is used: one thread has one request in
flight, and the second slot is what a second thread would take.

## 15.13 S4. The file system server reaches the disk

Status: built.
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
| `sectors` | Answers the sector count the server kept at mount. |
| `read` | Submits one chain, waits, copies the sector out of the slot. |
| `write` | Copies the sector into the slot, submits one chain, waits. |

The capacity is read once, at mount, with
`driver_virtio_blk::config::capacity`, which answers
`Result<u64, BlkError>`. `BlockDevice::sectors` answers `u32` and carries
no refusal, so the server keeps the number and mounts nothing when the
read fails or the value exceeds `u32::MAX`.

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

### What was built, where it differs

1. The transport is held in one `RefCell`. Reason:
   `fs_fat::BlockDevice::read` takes `&self`, and a read is a queue, a
   register window and a wait — all of which need exclusive access. No
   borrow is held across a call into `fs-fat`, and the read takes the
   borrow with `try_borrow_mut` rather than panicking.
2. The two windows are mapped in `main` and the gate is moved into the
   transport. Reason: the byte slices live as long as the program and a
   borrow cannot outlive the mapping it came from; and `Gate::adopt`
   allows one gate per buffer, so the driver and the loop share the one
   the program was started with.
3. The moment a new entry carries is rounded down to an even second and
   up to 1980-01-01. Reason: `fs_fat::time::to_entry` refuses an odd
   second and a year before 1980 rather than rounding, so the server is
   where the rounding belongs — `server_fs::moment`.

## 15.14 S5. The file protocol

Status: built.
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

## 15.15 S6. The file system server answers clients

Status: built.
Depends on: S4, S5.
Size: M.

### Does

The loop of 15.13 gains four steps:

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

### What was built

`app-files` does that sequence, and two more: a file whose length crosses
a cluster, written and read back byte for byte, and a listing of the root
directory. The end-to-end run reads all three off the console.

## 15.16 S7. The machine carries the disk automatically

Status: built.
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

### What was built, where it differs

1. The root task starts the file system server whether or not the
   enumeration found a device, and the server answers `Unavailable` to
   every request when it was given none. Reason: it is what the display
   server does for a machine without a screen, and a client that asks for
   a file has to hear that there is none rather than find no server at
   all. The run without a graphics adapter carries no scratch disk and is
   where `[files] no disk` is read.
2. The disk of the end-to-end run starts blank on every run, and the two
   boots of one run are the pair that proves persistence. A run a person
   starts with `--scratch` keeps its disk across runs, as D-136 has it.

## 15.17 S8. The tests

Status: built.
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

### What was built

| # | Where |
|---|-------|
| 1 | `block_lines` in `crates/tools/xtask/src/commands.rs`: the capacity the device answered and the geometry the server mounted. |
| 2 | `file_lines`: `app-files` writes 700 bytes, which crosses a cluster of one sector, and reads them back. |
| 3 | `test_the_same_disk_again`: the second boot of one disk finds what the first wrote, byte for byte. |
| 4 | `Blk::submit` refuses a request past the capacity, which `driver-virtio-blk` covers on the host; the adapter of S4 refuses the sector before it frames anything. |
| 5 | The run without a graphics adapter carries no disk and reads `[files] no disk`. |

`sh tools/xtask-check.sh` exits with 0.

## 15.18 S9. Programs move onto the volume

Status: blocked, see D4 (15.19).
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

## 15.19 Decision D4: what S9 needs that this machine has not

This decision is open. S9 cannot be built until it is made.

**The problem.** S9 reads programs off the boot disk, because the firmware
reads that disk and the system writes nothing to it (15.18, Two limits).
The boot disk of the reference machine is attached with
`-drive format=raw,file=…` and no interface
(`crates/tools/xtask/src/qemu.rs`, line 461), which the `q35` machine puts
on its AHCI controller — `00:1f.2`, class `01:06:01`, as `app-lspci`
reports it. This system drives virtio block devices and no other kind, so
nothing it has can read that disk.

**The options.**

| # | Option | What it costs |
|---|--------|---------------|
| 1 | A driver for the AHCI controller. | A driver crate the size of `driver-virtio-blk`, its own host tests, and a second transport under `fs-fat`. |
| 2 | Attach the boot disk as a second `virtio-blk-pci` device. | The reference machine changes (3.1.1, D-136); S1 hands over two devices instead of one, which is a second set of the nine roles or a role that appears twice; the server mounts two volumes and tells them apart. |
| 3 | Put the programs on the scratch disk. | It contradicts D-136: the scratch disk is what the system writes and arrives blank, and the image writer would have to prepare it. |

**Not decided here.** Which one is taken changes what S1 and S4 are, so it
belongs to whoever owns the reference machine.

## 15.20 Risks

| # | Risk | Effect | What reduces it |
|---|------|--------|-----------------|
| 1 | The root task gains bus enumeration. | The process with the full authority grows larger. | The enumeration code is `pci`'s, not the root task's. S1 ends with a program that proves the handover before anything is built on top of it. |
| 2 | The MSI-X handover is wrong, and no test catches it. | The server waits for an interrupt that never arrives. | S1's program reports every value it received. The first wait in S4 carries a deadline. |
| 3 | One thread makes the server slow. | A client waits while another client's sector is in flight. | 15.13 states this cost. The second thread is a later version, not a repair. |
| 4 | A device fails part way through a request. | The server answers with wrong bytes. | The status byte is judged. The completion's length bounds the read. `virtio-queue` refuses a used element that names a descriptor it did not hand out. |
| 5 | The server formats a blank disk. | A test passes on an empty disk and fails on a real volume. | The server reads where a volume exists and formats only where none does. The persistence test boots the same disk twice. |
| 6 | An 8.3 name is too small for a program name. | File names are hard to read. | This is D-09's known limit. Long file names are 8.18's later work. |
| 7 | The server keeps a table per client and learns of no client that ends. | Sixteen programs that open a file and exit take every table, and the seventeenth is answered `OutOfHandles` for as long as the machine runs. | Nothing yet. The display server watches a client through the process capability that client hands it (D-106), and the file protocol carries no handle at all, so this costs a message that gives one. It is the first thing to add to 15.14. |
