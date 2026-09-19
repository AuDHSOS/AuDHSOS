# kernel-hal-x86_64, kernel-x86-tables, kernel-acpi audit findings

Repository: AuDHSOS/AuDHSOS. Audit of kernel-hal-x86_64, kernel-x86-tables, kernel-acpi at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #82
Title: kernel-hal-x86_64: the user entry trampoline hands kernel register contents to a new thread
Labels: bug, part::kernel
Body:
`switch` at `crates/kernel/hal-x86_64/src/context.rs:33-51` saves and restores only the six callee-saved registers and leaves `rsi` (the argument `to`), `rax`, `rcx`, `rdx`, and `r8` to `r11` as the caller left them. `enter_user_trampoline` at `crates/kernel/hal-x86_64/src/context.rs:88-90` pops `rdi` and runs `iretq` without clearing any other register. `prepare_user` at `crates/kernel/x86-tables/src/context.rs:81-101` zeroes only the six callee-saved words of the frame.

A thread that starts through the frame `prepare_user` built runs its first instruction with `rsi` equal to the `to` value `switch_to` received at `crates/kernel/bin/src/task.rs:246`, which is the virtual address of the thread's own kernel stack in the kernel half, and with `rax`, `rcx`, `rdx`, `r8` to `r11` holding whatever `task::run` held at the call. The user thread reads a kernel address and kernel data out of its registers at its entry point.

Fix: clear `rax`, `rcx`, `rdx`, `rsi`, and `r8` to `r11` in `enter_user_trampoline` before `iretq`, which is the only path a new thread enters user mode through; clearing them in `switch` instead would cost every kernel-to-kernel switch the same instructions.

---

## F02 — issue #85
Title: kernel-hal-x86_64: PhysicalWindow is Copy, so safe code creates two mutable references to one frame
Labels: bug, part::kernel
Body:
`PhysicalWindow` derives `Clone` and `Copy` at `crates/kernel/hal-x86_64/src/window.rs:23`. `frame_bytes_mut` at `crates/kernel/hal-x86_64/src/window.rs:106-113` and `FrameAccess::table_mut` at `crates/kernel/hal-x86_64/src/window.rs:141-147` are safe methods whose SAFETY comment relies on the exclusive borrow of `self` to keep two references to one frame apart. The borrow is of one value; a copy of the window is another value with no borrow relation to the first.

Safe code with one window `w` obtained through the one `unsafe` call at `crates/kernel/bin/src/task.rs:496-500` writes `let mut a = w; let mut b = w; let x = a.frame_bytes_mut(f); let y = b.frame_bytes_mut(f);` and holds `x` and `y` at the same time, two `&mut [u8; 4096]` over the same physical frame, which is undefined behavior without an `unsafe` block. `read_array` at `crates/kernel/hal-x86_64/src/acpi.rs:176-183` and `read_table` at `crates/kernel/hal-x86_64/src/acpi.rs:188` already pass copies of the window by value.

Fix: remove `Clone` and `Copy` from `PhysicalWindow` and pass `&PhysicalWindow` in `read_array` and `read_table`; making `frame_bytes_mut` and `table_mut` `unsafe fn` instead would move the aliasing proof to every caller in `kernel-mm` and the kernel binary.

---

## F03 — issue #87
Title: kernel-hal-x86_64: read_byte and write_byte are safe functions that dereference any address
Labels: bug, part::kernel
Body:
`read_byte` at `crates/kernel/hal-x86_64/src/testing.rs:300-305` and `write_byte` at `crates/kernel/hal-x86_64/src/testing.rs:310-318` are `pub fn`, not `pub unsafe fn`, and each runs a volatile access on a pointer built from the `u64` argument. The SAFETY comment inside each names a promise of the caller, which a safe function cannot ask for. The module is compiled when both `debug-uart` and `test-exit` are set (`crates/kernel/hal-x86_64/src/lib.rs:27-28`), both are default features (`crates/kernel/hal-x86_64/Cargo.toml:14`), and the kernel binary takes the default set (`crates/kernel/bin/Cargo.toml:96`), so both functions are part of the product kernel's safe API.

Safe code in the kernel binary calls `write_byte(address_of_some_static, 0)` and overwrites kernel data without an `unsafe` block; `read_byte` on an unmapped address takes a page fault from safe code. Rule R2 in `docs/04-safety-policy.md` makes an `unsafe` block sound by its precondition, and here the precondition is unenforced.

Fix: make both `unsafe fn` with a `# Safety` section that states the mapping requirement, and add `unsafe` blocks at the call sites in the test images; gating the module behind a non-default feature would still leave the two functions safe for every test image.

---

## F04 — issue #90
Title: kernel-hal-x86_64: cli and sti carry nomem, which lets the compiler move memory accesses across the interrupt guard
Labels: bug, part::kernel
Body:
`disable_interrupts` at `crates/kernel/hal-x86_64/src/instructions.rs:48` and `enable_interrupts` at `crates/kernel/hal-x86_64/src/instructions.rs:61` declare `options(nomem, nostack)`. The Rust reference defines `nomem` as "the asm! block does not read from or write to any memory accessible outside of the asm! block", which "allows the compiler to cache the values of modified global variables in registers across the asm! block". `InterruptGuard` at `crates/kernel/hal-x86_64/src/instructions.rs:72-109` is built on these two functions, and `with_controller` at `crates/kernel/hal-x86_64/src/interrupts.rs:154-158` relies on the guard to keep a handler from finding the cell busy.

The compiler is permitted to sink the release store of the borrow flag or the writes to `Apics` past the `sti` of the guard's `Drop`, or to hoist the acquire of the flag above the `cli`; a timer interrupt that arrives in that window finds `CONTROLLER` borrowed, `acknowledge` at `crates/kernel/hal-x86_64/src/interrupts.rs:195-202` returns without an end-of-interrupt, and the local APIC delivers no further interrupt at that priority.

Fix: drop `nomem` from both sites so the `asm!` block is a full compiler barrier, as `write_page_table_root` at `crates/kernel/hal-x86_64/src/instructions.rs:248` already is; `compiler_fence(SeqCst)` around each call would fix the same sites in twice the lines.

---

## F05 — issue #92
Title: kernel-hal-x86_64: entry.rs and ports.rs name the feature-gated console and exit modules unconditionally
Labels: bug, part::kernel
Body:
`crates/kernel/hal-x86_64/src/lib.rs:13-14` compiles `console` only with `debug-uart` and `crates/kernel/hal-x86_64/src/lib.rs:18-19` compiles `exit` only with `test-exit`. `crates/kernel/hal-x86_64/src/entry.rs:17` and `crates/kernel/hal-x86_64/src/entry.rs:19` import both without a `cfg`, `crates/kernel/hal-x86_64/src/entry.rs:90` calls `crate::console::reclaim`, and `crates/kernel/hal-x86_64/src/ports.rs:175` calls `crate::console::give_up`.

`cargo check -p kernel-hal-x86_64 --no-default-features --target x86_64-unknown-none` fails with `E0432: unresolved import crate::console` at `entry.rs:17`, `E0432` at `entry.rs:19`, `E0433` at `entry.rs:90` and `ports.rs:175`, plus two unused-extern-crate errors. The two features cannot be turned off, so the product kernel always carries the serial console and the exit device.

Fix: gate the imports, `fail`, `succeed`, and `note` on the two features and make `set_ecam`'s caller in `start` independent of them; removing the two features from the manifest instead would make `docs/04-safety-policy.md:80`, which names them, wrong.

---

## F06 — issue #95
Title: kernel-hal-x86_64: the inline assembly inventory in the safety policy omits cpuid and rdseed
Labels: bug, part::kernel
Body:
Section 4.5 of `docs/04-safety-policy.md:62-81` lists every `asm!` site of `kernel-hal-x86_64`, and `docs/04-safety-policy.md:64` states that every site is a one-line wrapper unless it is a naked function. `has_rdseed` at `crates/kernel/hal-x86_64/src/instructions.rs:170-181` runs `cpuid` in a four-instruction block, and `read_seed` at `crates/kernel/hal-x86_64/src/instructions.rs:206-212` runs `rdseed` and `setc` in one block. Neither instruction appears in any row of the table.

D-132 at `docs/09-decisions.md:142` raised the `asm!` budget for the two sites, so the budget table counts them and the inventory does not. A reviewer using 4.5 as the checklist R10 refers to finds two sites the checklist does not name.

Fix: add a row `feature query and entropy source | cpuid, rdseed, setc` to the table in `docs/04-safety-policy.md` after line 76 and restate line 64 as "one instruction group per site"; leaving line 64 as it is keeps a claim `reload_segments` at `crates/kernel/hal-x86_64/src/instructions.rs:325-337` already contradicts.

---

## F07 — issue #97
Title: kernel-acpi: the MADT parser reads the local APIC address and the flags outside the announced length
Labels: bug, part::kernel
Body:
`parse` at `crates/kernel/acpi/src/madt.rs:245-250` checks the header through `SdtHeader::parse`, which validates the length and the checksum over `header.length` bytes, and then reads the local APIC address at offset 36 and the flags at offset 40 with `u32_at` against the whole slice, not against `header.length`. The README at `crates/kernel/acpi/README.md:17-19` states that every parser validates length and checksum first and reads fields afterwards.

A slice of 44 bytes whose header announces length 36 with a checksum that sums the first 36 bytes to zero passes `SdtHeader::parse`, and `parse` returns a `Madt` whose `lapic_address` and `flags` come from the eight bytes the checksum did not cover. The test at `crates/kernel/acpi/src/tests/madt.rs:153-165` truncates the slice to the announced length, so the case is untested. The kernel's caller at `crates/kernel/hal-x86_64/src/acpi.rs:187-191` hands over exactly `length` bytes, so the product path is not reached today.

Fix: reject a table whose `header.length` is below `MADT_HEADER_LEN` with `AcpiError::Length` before reading the two fields; slicing `bytes` to `length` at the top of `parse` would fix the same read but keep `TooShort` as the reported variant.

---

## F08 — issue #98
Title: kernel-hal-x86_64: the table walk stops at the first table whose header does not parse
Labels: enhancement, part::kernel
Body:
`find_madt` at `crates/kernel/hal-x86_64/src/acpi.rs:89-95` and `find_mcfg` at `crates/kernel/hal-x86_64/src/acpi.rs:125-131` call `read_table(...)?` and `SdtHeader::parse(bytes)?` for every entry of the root table in order and return the first error.

A root table that lists a vendor table with a wrong checksum, or one whose address lies above the window limit, before the `APIC` entry makes `find_madt` return `Table(Checksum)` or `Unreachable`, and `bring_up` at `crates/kernel/hal-x86_64/src/interrupts.rs:104` fails, so the kernel has no interrupt controller although the `APIC` table is intact. The reference machine's tables are all well formed, so the walk succeeds there.

Fix: skip an entry whose `read_table` or `SdtHeader::parse` fails and continue to the next one, returning `NoMadt` or `NoMcfg` only when the walk ends without the signature; recording the first error in the `NoMadt` variant would keep the diagnostic at the cost of a larger error type.

---

## F09 — issue #100
Title: kernel-acpi: a revision two root pointer with an XSDT address of zero names physical address zero as the root table
Labels: enhancement, part::kernel
Body:
`parse_rsdp` at `crates/kernel/acpi/src/rsdp.rs:128-133` accepts the XSDT address of a revision two pointer through `address`, which rejects only values beyond the physical width, and `Rsdp::root` at `crates/kernel/acpi/src/rsdp.rs:59-64` returns the XSDT address whenever the pointer is revision two.

A revision two pointer whose `XsdtAddress` field is zero, which ACPICA and Linux treat as "no XSDT" and fall back to the RSDT for, makes `find_madt` at `crates/kernel/hal-x86_64/src/acpi.rs:87` read a table header at physical address zero and return `Table(Signature(..))` or `Table(Length(..))`, and the RSDT the same pointer names is not read.

Fix: map an XSDT address of zero to `xsdt: None` in `parse_rsdp` so `root` answers with the RSDT; rejecting the pointer with `AcpiError::Address(0)` would refuse a machine the RSDT could serve.

---

## F10 — issue #103
Title: kernel-hal-x86_64: install moves a 16 KiB stack and a 4 KiB table through the boot stack
Labels: enhancement, part::kernel
Body:
`install` at `crates/kernel/hal-x86_64/src/descriptors.rs:66-67` calls `Global::init([0; DOUBLE_FAULT_STACK_LEN])` with a 16 KiB array built as a temporary, and `crates/kernel/hal-x86_64/src/descriptors.rs:113-115` builds the 4 KiB interrupt descriptor table as a local and moves it into `IDT`. D-66 at `docs/09-decisions.md:76` records that `Global::init` takes its value by move over the boot stack and introduced `Preset` (`crates/sync/src/lib.rs:202`) so that large zeroed values reach `.bss` without a stack copy.

The frame of `install` holds at least 20 KiB in a debug build against the 256 KiB boot stack of `crates/abi/src/layout.rs:63`, which is safe today; a `DOUBLE_FAULT_STACK_LEN` raised to the size of a thread's kernel stack (32 KiB, `crates/abi/src/layout.rs:81`) or a call of `install` from a thread's kernel stack would overflow it.

Fix: hold `DOUBLE_FAULT_STACK` in a `Preset<[u8; DOUBLE_FAULT_STACK_LEN]>` and fill `IDT` through a `Preset` borrow with `traps::fill`, so neither value is built on the stack; keeping `Global` and marking `install` `#[inline(never)]` would bound the frame but not remove the copy.

---

## F11 — issue #105
Title: kernel-hal-x86_64: the local APIC identifier is read before the unit's enable bit is set
Labels: enhancement, part::kernel
Body:
`Apics::new` at `crates/kernel/hal-x86_64/src/apic.rs:336-350` reads `local.id()` through the register window at line 341 and keeps it as the destination of every redirection entry and every message address. `bring_up` calls `Apics::new` at `crates/kernel/hal-x86_64/src/interrupts.rs:124` and sets the enable bit of `IA32_APIC_BASE` afterwards through `enable` at `crates/kernel/hal-x86_64/src/interrupts.rs:128` and `crates/kernel/hal-x86_64/src/apic.rs:120-130`.

On a firmware that leaves bit 11 of `IA32_APIC_BASE` clear, the register window is not decoded at the time of the read, `destination` holds whatever the bus returned, and every routed line and every allocated message interrupt targets a processor that does not exist. The firmware of the reference machine leaves the bit set, so the read returns the identifier there.

Fix: call `enable` before `Apics::new`, or read the identifier inside `enable` after the model-specific register write and store it then; reading `id()` on every `entry_for` would add a register read to each routing call.

---

## F12 — issue #108
Title: kernel-hal-x86_64: the SAFETY comment of Ports names a capability check the type does not carry
Labels: enhancement, part::kernel
Body:
`Ports::read_u8` at `crates/kernel/hal-x86_64/src/ports.rs:32-36` justifies its `unsafe` block with "the system call layer has checked the port against an `IoPortRange` capability of the caller", and the five other methods refer to that comment. `Ports` derives `Default` at `crates/kernel/hal-x86_64/src/ports.rs:19` and has a safe `pub const fn new` at `crates/kernel/hal-x86_64/src/ports.rs:25-27`, so any crate that depends on `kernel-hal-x86_64` builds one and reaches every port through the safe `PortAccess` methods without passing through the system call layer.

The precondition the comment states is a property of one caller, not of the value; a second caller of `Ports::new` in the kernel binary reaches the exit device or the serial controller from safe code, and the comment does not hold for it. Rule R2 in `docs/04-safety-policy.md` asks the comment to name what makes the block sound.

Fix: make `Ports::new` an `unsafe fn` whose `# Safety` section states that the caller checks every port against a capability, and drop `Default`; keeping it safe and rewriting the six comments to "port I/O from ring zero breaks no memory-safety guarantee of Rust" would state a fact the type can carry but drop the capability claim the module doc at `crates/kernel/hal-x86_64/src/ports.rs:6-9` makes.

---

## F13 — issue #110
Title: kernel-hal-x86_64: the doc comment of address_of describes the end of a stack, and the function returns the start of a value
Labels: enhancement, part::kernel
Body:
`address_of` at `crates/kernel/hal-x86_64/src/descriptors.rs:49-51` returns the address of the value passed in. Its doc comment at `crates/kernel/hal-x86_64/src/descriptors.rs:47-48` says "The address a static array of `len` bytes at `base` ends at, which is where a stack starts growing down from", and names two parameters, `len` and `base`, that the function does not have.

The caller at `crates/kernel/hal-x86_64/src/descriptors.rs:73` adds `DOUBLE_FAULT_STACK_LEN` itself to reach the end, and the callers at lines 86, 95, and 121 use the start for the task state segment base and the table pointers, so a reader who takes the doc comment at its word computes the stack top twice.

Fix: replace the comment with "The address of `value`."; leaving the comment and adding a second function for the stack top would document one line of arithmetic twice.
