# user-programs library and root task audit findings

Repository: AuDHSOS/AuDHSOS. Audit of user-programs (library and server-init) at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #166
Title: user-programs: `Dma` reaches the bytes the device writes through a plain `&mut [u8]`
Labels: bug, part::userland
Body:
`Dma::new` at `crates/user/programs/src/dma.rs:365-369` takes the whole region as `&mut [u8]` and stores it in `Dma::bytes` (`crates/user/programs/src/dma.rs:352-355`). `Dma::status` at `crates/user/programs/src/dma.rs:432-435` reads the status byte with `self.bytes.get(at)`, and `QueueMemory::used_ring` at `crates/user/programs/src/dma.rs:460-462` answers the used ring as a plain slice. The device writes both while the slice stands. `crates/user/sys-x86_64/src/mmio.rs:7-11` states that a plain slice is what only the framebuffer tolerates and that a register is reached by `read_volatile` or `write_volatile`. The SAFETY comment of `Mapping::bytes` at `crates/user/programs/src/mapping.rs:119-121` promises a mapping that is "unshared", which a region shared with the device is not.

`crates/user/programs/src/bin/server_fs.rs:215-221` builds the `Dma` over the region the device reads and writes, and `crates/user/programs/src/bin/server_fs.rs:508-517` reads the used ring and the status byte after a wake-up while the device has written them. A `&mut [u8]` over memory another agent writes is a data race in the Rust memory model, and the `fence` in `barrier` at `crates/user/programs/src/dma.rs:469-471` orders atomic accesses but does not make a non-atomic read through an exclusive reference reload.

Fix: read the status byte and every used-ring word through `Mmio::read_u8` and `Mmio::read_u16`/`read_u32`, which means `Dma` holds an `Mmio` over the region and `QueueMemory::used_ring` answers bytes copied out volatile rather than a borrowed slice; the option not taken, relying on the fence as a compiler barrier, depends on LLVM's treatment of a fence and not on a rule of the language.

---

## F02 — issue #168
Title: user-programs: `app-lspci` receives the configuration window writable
Labels: bug, part::userland
Body:
`Grant::Ecam` at `crates/user/programs/src/bin/server_init.rs:1094-1101` installs the ECAM window into the child with `ObjectRights::DEVICE`, which is `READ | WRITE | MAP | INFO` (`crates/user/programs/src/bin/server_init.rs:1268-1271`). The only program with that grant is `app-lspci` (`crates/user/programs/src/bin/server_init.rs:315-328`), which reads the bus and writes nothing (`crates/user/programs/src/bin/app_lspci.rs:142`, `crates/user/programs/src/bin/app_lspci.rs:178`). `docs/02-architecture.md:626-627` states that the root task gives each program the minimum it needs.

`app-lspci` maps every bus of the window writable (`map_all` maps with `permissions::WRITE` unconditionally at `crates/user/programs/src/mapping.rs:224-231`) and `MappedSpace::write_u32` at `crates/user/programs/src/config_space.rs:338-346` reaches every function's configuration space. An application with this handle can clear `COMMAND_BUS_MASTER` of the block device that `server-fs` drives, rewrite its base address registers, or disable its MSI-X capability, which stops a driver of a different process.

Fix: install the window with `Rights::READ | Rights::MAP | Rights::INFO` and give `Mapping` a read-only map mode (`map_all` passes `permissions::READ` for it); the option not taken, keeping `DEVICE` for the window, leaves an application with write access to the configuration space of every device.

---

## F03 — issue #170
Title: user-programs: the count of outstanding reporters is taken from the table and not from what started
Labels: bug, part::userland
Body:
`reporters()` at `crates/user/programs/src/bin/server_init.rs:433-435` counts every line of `PROGRAMS` with `reports: true`, and `main` hands that number to `serve_faults` at `crates/user/programs/src/bin/server_init.rs:517`. `serve_faults` at `crates/user/programs/src/bin/server_init.rs:2095-2108` decrements it once per `Finished` message from any badge, and ends the machine at zero. A reporter that never started (`crates/user/programs/src/bin/server_init.rs:610-617`, `crates/user/programs/src/bin/server_init.rs:628-635`, `crates/user/programs/src/bin/server_init.rs:636-649`) and a reporter that faults (`crates/user/programs/src/bin/server_init.rs:2109-2124`) leave the count unchanged; a reporter that sends `Finished` twice decrements it twice.

Trigger one: `APP-HELL.ELF` missing from the scratch volume. The root task writes `[init] app-hello at volume: ...`, the other eight reporters finish, `outstanding` stays at one, and the machine runs until the harness times out. Trigger two: `app-hello` sends `parent::Request::Finished` twice. The machine ends through `end_machine` while `app-paint` still runs.

Fix: keep the badges of the reporters that started in a `u64` bitmask (badges run from one and eighteen programs exist), clear a badge on its first `Finished` or on its fault, and end the machine when the mask is zero; the option not taken, decrementing on a fault as well, still leaves a reporter that never started counted.

---

## F04 — issue #172
Title: user-programs: a failed step of `install_all`, `copy_region` or `zero` leaves the `SCRATCH` window mapped
Labels: bug, part::userland
Body:
`install_all` maps the child's buffer page at `SCRATCH` at `crates/user/programs/src/bin/server_init.rs:956` and unmaps it at `crates/user/programs/src/bin/server_init.rs:1051`; every `?` between the two (`crates/user/programs/src/bin/server_init.rs:965-1033`, `crates/user/programs/src/bin/server_init.rs:1045-1049`) returns with the window mapped. `copy_region` at `crates/user/programs/src/bin/server_init.rs:1965` and `zero` at `crates/user/programs/src/bin/server_init.rs:1984` call `Mapping::new` at `SCRATCH`, whose documentation at `crates/user/programs/src/mapping.rs:45-48` states that a call that fails halfway leaves what it mapped and that the caller takes it back; neither caller does. `read_program` at `crates/user/programs/src/bin/server_init.rs:747-755` takes the `PROGRAM` window back on the same kind of failure.

The kernel refuses a page that is mapped with `Error::AlreadyMapped` (`crates/kernel/syscall/src/environment.rs:49`). One refusal in `install_all`, for example `endpoint_badge` at `crates/user/programs/src/bin/server_init.rs:976` refusing for a program whose objects quota is spent, leaves one page mapped at `SCRATCH`; every later program then fails at `copy` in `copy_region`, and the root task reports every program after that one as not started.

Fix: on every error path of `install_all`, `copy_region` and `zero`, take the window back with `Mapping::adopt(SCRATCH, len).unmap(gate, world.own)` before returning, as `read_program` does with `unmap_window`.

---

## F05 — issue #174
Title: user-programs: `serve_faults` says a faulted child is killed and kills nothing
Labels: bug, part::userland
Body:
The documentation of `serve_faults` at `crates/user/programs/src/bin/server_init.rs:2083-2086` states that a child that faulted is reported and killed because what it holds is what somebody else needs. The body at `crates/user/programs/src/bin/server_init.rs:2109-2124` writes one line and continues; the comment at `crates/user/programs/src/bin/server_init.rs:2121-2123` states that there is no handle to kill with. `start` drops the child's `ProcessHandle` at the end of `crates/user/programs/src/bin/server_init.rs:872-925` and `World` at `crates/user/programs/src/bin/server_init.rs:521-565` keeps none.

`app-faulter` faults, its thread stays stopped, and its regions, its stack, its buffer page and its handles stay allocated for the life of the machine, which contradicts what the documentation says happens.

Fix: keep the child's `ProcessHandle` per badge in `World` and call `process_kill` on it when its fault arrives; the option not taken is to correct the documentation to state that the thread stays stopped and its memory is kept.

---

## F06 — issue #175
Title: user-programs: the program counts in the README, the crate comments and the manifest are stale
Labels: bug, part::userland
Body:
`crates/user/programs/README.md:3-7` states seven programs: the root task, three servers and three applications. `crates/user/programs/src/lib.rs:9-11` and `crates/user/programs/src/bin/server_init.rs:26-27` state thirteen programs. `crates/user/programs/Cargo.toml:6` states five servers. The manifest declares fifteen binaries (`crates/user/programs/Cargo.toml:19-101`): the root task, six servers (`server-memory`, `server-name`, `server-console`, `server-display`, `server-input`, `server-fs`) and eight applications.

A reader who counts the servers from the manifest description looks for a fifth server and a sixth program that does not exist.

Fix: state fifteen programs, six servers and eight applications in the README, in the two comments and in the manifest description; the option not taken is to drop the numbers, which costs the reader the check the README exists for.

---

## F07 — issue #177
Title: user-programs: a program that fails to start keeps its process, its endpoint and its memory
Labels: enhancement, part::userland
Body:
`start` creates the child process at `crates/user/programs/src/bin/server_init.rs:872-874` and its endpoint at `crates/user/programs/src/bin/server_init.rs:875`, then takes one object per region at `crates/user/programs/src/bin/server_init.rs:878`, the stack at `crates/user/programs/src/bin/server_init.rs:891` and the buffer at `crates/user/programs/src/bin/server_init.rs:903`. Every `?` after `crates/user/programs/src/bin/server_init.rs:872` returns without `process_kill`, `handle_close` or `release`.

A program that fails at `map` (`crates/user/programs/src/bin/server_init.rs:887`) leaves a process with the frames of its quota, its regions and its stack in the memory server's accounting under `INIT_BADGE`, and its endpoint, for the life of the machine. Eighteen programs bound the loss, so the machine still runs.

Fix: wrap the steps after `process_create` so that a failure kills the child, closes the endpoint and the badged handles, and releases every object taken from the memory server.

---

## F08 — issue #179
Title: user-programs: `serve_faults` never closes the reply handle a fault message carries
Labels: enhancement, part::userland
Body:
A fault is delivered as a call whose reply resumes the thread (`crates/kernel/syscall/src/fault.rs:11-14`), so `ipc_recv` at `crates/user/programs/src/bin/server_init.rs:2089-2091` answers a `Received` whose `reply` is `Some` (`crates/user/sys-x86_64/src/gate.rs:106-111`). `serve_faults` drops it at `crates/user/programs/src/bin/server_init.rs:2124` without `handle_close`.

Every fault leaves one reply handle in the root task's table, which holds 4096 entries (`crates/kernel/objects/src/config.rs:62`). A child with `MANAGE` on its own process (`crates/user/programs/src/bin/server_init.rs:1242-1246`) creates as many threads as its objects quota allows and faults each once, so the loss is bounded by the sum of the objects quotas of the table.

Fix: close the reply handle with `gate.handle_close(reply.handle())` after the line is written, or keep it so that the process can be killed through F05.

---

## F09 — issue #182
Title: user-programs: the root task keeps a badged capability to every server for every child
Labels: enhancement, part::userland
Body:
`install_all` derives a badged handle per server and child with `endpoint_badge` at `crates/user/programs/src/bin/server_init.rs:976`, `crates/user/programs/src/bin/server_init.rs:983`, `crates/user/programs/src/bin/server_init.rs:993`, `crates/user/programs/src/bin/server_init.rs:1002`, `crates/user/programs/src/bin/server_init.rs:1011`, `crates/user/programs/src/bin/server_init.rs:1022` and `crates/user/programs/src/bin/server_init.rs:1029`, installs a copy into the child, and never closes its own. `process_install_handle` copies the entry and retains the object (`crates/kernel/syscall/src/calls/process.rs:273-281`), so both tables hold it.

The root task holds up to seven send capabilities per child that carry the child's badge, about one hundred and twenty of its 4096 handle entries, and can send to any server as any child. The root task is the most trusted program, so the consequence today is the table space.

Fix: close each `marked` handle with `handle_close` right after `process_install_handle` answers.

---

## F10 — issue #184
Title: user-programs: `BootImageHeader::parse` is given the image length as the usable memory size
Labels: enhancement, part::userland
Body:
`start_everything` calls `BootImageHeader::parse(bytes, len, len)` at `crates/user/programs/src/bin/server_init.rs:589`. The third argument is `ram_bytes` (`crates/abi/src/boot_image.rs:131`), and the reserve check at `crates/abi/src/boot_image.rs:200-204` refuses a `kernel_reserve_size` that is not below it.

The image the xtask writes carries a reserve of zero (`crates/tools/xtask/src/commands.rs:3138`), so the check passes today. An image with a reserve of at least the image length, which the kernel accepts against the machine's memory, makes the root task write `[init] the boot image header is not one` and start nothing.

Fix: pass `u64::MAX` as `ram_bytes`, since the kernel has already checked the reserve against the machine and the root task needs only the archive fields.

---

## F11 — issue #186
Title: user-programs: `program_directory` never closes the intermediate directory it opens
Labels: enhancement, part::userland
Body:
`program_directory` at `crates/user/programs/src/bin/server_init.rs:720-725` opens `AUDHSOS` and then `BIN` under it, overwriting `parent` with each handle, and keeps only the last in `world.bin`. The handle of `AUDHSOS` is never closed with `close_file` (`crates/user/programs/src/bin/server_init.rs:836-841`).

The file system server holds one open directory for the root task's badge for the life of the machine. One handle per boot bounds it.

Fix: close every handle of the walk but the last after the next step opened.

---

## F12 — issue #188
Title: user-programs: `Dma::slot_at` wraps a slot index instead of refusing it
Labels: enhancement, part::userland
Body:
`Dma::slot_at` at `crates/user/programs/src/dma.rs:396-398` computes `slot % SLOTS`, so `chain`, `sector`, `header`, `status` and `clear_status` (`crates/user/programs/src/dma.rs:404-444`) answer slot zero for slot two. The documentation at `crates/user/programs/src/dma.rs:273-274` states that every accessor answers a slice inside the region or an empty one; a wrapped index answers a slice inside the region that belongs to another request.

A caller that submits a request in slot two while slot zero is in flight overwrites the header and the sector the device is reading. Today `server-fs` uses one constant slot (`crates/user/programs/src/bin/server_fs.rs:457`).

Fix: take the slot as a `Slot` enum with `SLOTS` variants, so that a wrong index cannot be written; the option not taken, answering an empty slice for `slot >= SLOTS`, makes `chain` compute an address for slot zero anyway.

---

## F13 — issue #189
Title: user-programs: `end_machine` passes the port count as the write width
Labels: enhancement, part::userland
Body:
`end_machine` at `crates/user/programs/src/bin/server_init.rs:2143` calls `gate.ioport_write(ports, EXIT_PORT, EXIT_PORTS, EXIT_SUCCESS)`. The third parameter of `ioport_write` is `width` (`crates/user/sys-x86_64/src/gate.rs:985-993`); `EXIT_PORTS` is documented as how many ports the device has (`crates/user/programs/src/bin/server_init.rs:437-439`).

The two are both four, so the write goes out as four bytes and the machine ends. A change of the range, for example to one port, silently changes the access width.

Fix: name a constant `EXIT_WIDTH` for the access width and pass it.

---

## F14 — issue #190
Title: user-programs: `_Plan` is a dead type alias that keeps an unused import
Labels: enhancement, part::userland
Body:
`crates/user/programs/src/bin/server_init.rs:2173-2174` declares `type _Plan = Plan;` so that the `Plan` import at `crates/user/programs/src/bin/server_init.rs:58` is used. `start` names the type nowhere; `user_loader::plan` at `crates/user/programs/src/bin/server_init.rs:868` infers it.

The alias is dead code that survives only to keep an import that is itself unneeded.

Fix: drop the alias and `Plan` from the `use` line.
