# 3. Target Platform

## 3.1 First target: QEMU `x86_64` with UEFI

The first release runs on `qemu-system-x86_64` with the `q35` machine, the
default `qemu64` CPU model, and the UEFI firmware bundled with QEMU.

### 3.1.1 Reference machine configuration

The build automation owns this command line; nobody types it by hand.

```
qemu-system-x86_64 \
  -machine q35 \
  -cpu qemu64 \
  -smp 1 \
  -m 256M \
  -drive if=pflash,format=raw,readonly=on,file=<qemu share dir>/edk2-x86_64-code.fd \
  -drive format=raw,file=<disk image> \
  -serial stdio \
  -display none \
  -vga none \
  -device VGA,edid=on,xres=1920,yres=1200 \
  -fw_cfg name=opt/ovmf/PcdVideoHorizontalResolution,string=1920 \
  -fw_cfg name=opt/ovmf/PcdVideoVerticalResolution,string=1200 \
  -no-reboot \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04
```

Accelerator: TCG. `-no-reboot` turns a triple fault into a QEMU exit, which
the test runner reports as a crash.

The screen is 1920x1200, and it takes three of the lines above to get it.
The default VGA device of the `q35` machine would be the same device, but
its EDID cannot be given a size on the command line, so `-vga none` leaves
the slot empty and `-device VGA` fills it again with the size named. The
firmware offers a mode through the Graphics Output Protocol only when the
adapter's EDID carries it, and it picks among the modes on offer by the two
settings the firmware configuration device carries. Either half alone
leaves the firmware on its own default of 1280x800. The mode costs
1920 x 1200 x 4 = 9.2 MiB of the sixteen the adapter has.

A height matters beyond the picture: the firmware draws its own text
console into the mode it sets, and it needs the twenty-five rows of
nineteen pixels a UEFI console has. Below four hundred seventy-five pixels
the firmware never reaches its boot manager and nothing is loaded at all.

Nothing in this system assumes the size. The loader reads the mode the
firmware set (D-30), the kernel reports it, and the tests read it from that
report.

From Phase 9 on the test runner adds `-qmp unix:<socket path>,server,nowait`
and injects input events and reads the screen through that socket.
`cargo xtask run --display` replaces `-display none` with `-display cocoa`
on macOS and `-display gtk` on Linux. The run without a graphics adapter
keeps `-vga none` and drops the three lines that follow it, which leaves
the firmware without a Graphics Output Protocol.

### 3.1.2 Devices

| Device | Access path | Used by | Phase |
|--------|-------------|---------|-------|
| UEFI boot services and configuration table | `extern "efiapi"` calls through project-defined bindings | loader | 2 |
| Serial 16550, COM1 (I/O ports `0x3F8`-`0x3FF`, IRQ 4) | port I/O | kernel debug output (`debug-uart` feature); userland console driver via `IoPortRange` and `Interrupt` | 2, 7 |
| Local APIC (xAPIC, MMIO base from ACPI MADT) | MMIO through the physical memory window, into which the kernel maps the aperture itself (D-60) | timer tick, end-of-interrupt, spurious vector | 4 |
| I/O APIC (MMIO base from ACPI MADT) | MMIO, mapped the same way | routing IRQ lines to vectors, masking and unmasking | 4 |
| Legacy 8259 PIC | port I/O | masked once at boot, never used again | 4 |
| PIT channel 2 | port I/O | one-time calibration of the local APIC timer frequency | 4 |
| ACPI RSDP (from the UEFI configuration table), RSDT/XSDT, MADT | bytes read through the physical window, parsed in safe Rust by `kernel-acpi` | APIC discovery | 4 |
| `isa-debug-exit` (I/O port `0xF4`) | port I/O | test exit codes from loader and kernel | 2 |
| PCI configuration space via ECAM (`MCFG`) | MMIO via `Device` memory objects | userland virtio drivers | later |
| virtio-blk, virtio-net over PCI | MMIO, interrupts | userland drivers | later |
| Standard VGA device (`q35` default) with a linear framebuffer exposed by the UEFI Graphics Output Protocol | loader: mode query through `EFI_GRAPHICS_OUTPUT_PROTOCOL`; userland: MMIO via a `Device` memory object | boot information; userland display server | 2, 9 |
| i8042 PS/2 controller (I/O ports `0x60` and `0x64`, IRQ 1 keyboard, IRQ 12 mouse) | port I/O via `IoPortRange`, `Interrupt` | userland input driver | 10 |

### 3.1.3 Loader

The loader is the crate `boot-uefi-x86_64`, built for the target
`x86_64-unknown-uefi` and installed on the disk image as
`EFI/BOOT/BOOTX64.EFI`.

- UEFI bindings are defined in the crate `audhsos-uefi` (logic crate,
  `repr(C)` structures and constants only, no calls): system table, boot
  services table, `EFI_LOADED_IMAGE_PROTOCOL`,
  `EFI_SIMPLE_FILE_SYSTEM_PROTOCOL`, `EFI_FILE_PROTOCOL`,
  `EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL`, `EFI_GRAPHICS_OUTPUT_PROTOCOL` with
  its mode and mode information structures, memory descriptor, memory
  types, configuration table entries, the ACPI 2.0 table GUID. Structure
  layouts are tested for size and field offsets on the host.
- The loader calls exactly these services: `HandleProtocol`,
  `LocateProtocol`, `AllocatePages`, `FreePages`, `GetMemoryMap`,
  `ExitBootServices`, the file protocol's `Open`, `GetInfo`, `Read`,
  `Close`, and the text output protocol's `OutputString` for diagnostics.
- The loader reads the framebuffer description from the mode the firmware
  has set through the Graphics Output Protocol. It does not change the
  mode. If the protocol is absent or the pixel format is not one of the two
  32-bit formats, the loader reports no framebuffer and continues.
- Files read from the boot volume: `AUDHSOS/KERNEL.ELF` and
  `AUDHSOS/BOOT.IMG`. File names are 8.3 names.
- The kernel ELF is parsed by `audhsos-elf`. The page tables are built by
  the `kernel-mm` mapper with the loader's `FrameAccess` adapter over the
  identity mapping.
- The loader contains one naked function: turn interrupts off, write
  `CR3`, load the boot stack pointer, jump to the kernel entry with the
  boot information address. It uses the `sysv64` calling convention,
  because `extern "C"` on the UEFI target is the Microsoft one while the
  kernel entry point takes its argument in `RDI`.
- Every failure in the loader prints a diagnostic through the text output
  protocol and exits QEMU with the failure code through `isa-debug-exit`.
- The loader is a build with `panic = "abort"` and no `alloc`.

### 3.1.4 Disk image

`cargo xtask image` writes the disk image without external tools:

- A GUID partition table: a protective MBR in sector 0 with one entry of
  type `0xEE` covering the whole disk, the primary GPT header in sector 1,
  the partition entry array in sectors 2 to 33, and the backup entry array
  and backup header at the end of the disk. Header and array carry CRC32
  checksums computed by the xtask's own CRC32 implementation.
- One partition with the EFI system partition type GUID
  `C12A7328-F81F-11D2-BA4B-00A0C93EC93B` and a fixed, project-defined unique
  partition GUID so that images are reproducible.
- A FAT32 file system in that partition, written by the project's own FAT32
  writer in the xtask: boot sector, FSInfo sector, backup boot sector in
  sector 6, two identical FAT copies, the root directory as a cluster chain.
  Directory entries use 8.3 names; no long file name entries are written.
- Files: `EFI/BOOT/BOOTX64.EFI`, `AUDHSOS/KERNEL.ELF`, `AUDHSOS/BOOT.IMG`.
- The partition holds at least 65525 clusters of one 512-byte sector each.
  The image size is 64 MiB unless the files need more, then rounded up to
  a multiple of 1 MiB. The file is written sparsely.

### 3.1.5 Boot information structure

Defined in `audhsos-abi` as `#[repr(C)]` with a fixed layout. Written by the
loader into a dedicated page; read by the kernel through the pointer passed
in the entry call.

| Field | Type | Content |
|-------|------|---------|
| `magic` | `[u8; 8]` | `AUDHBOOT` |
| `version` | `u32` | `1` |
| `size` | `u32` | total size in bytes of the structure including the region array |
| `phys_window_base` | `u64` | virtual base of the physical memory window (`PHYS_WINDOW_BASE`) |
| `kernel_phys_start`, `kernel_phys_len` | `u64` | physical range of the kernel image |
| `boot_image_phys_start`, `boot_image_phys_len` | `u64` | physical range of the boot image |
| `page_tables_phys_start`, `page_tables_phys_len` | `u64` | frames holding the initial page tables |
| `boot_stack_phys_start`, `boot_stack_phys_len` | `u64` | boot stack frames, guard page excluded |
| `acpi_rsdp` | `u64` | physical address of the RSDP, `0` if absent |
| `framebuffer_phys_start` | `u64` | physical base of the linear framebuffer, `0` if absent |
| `framebuffer_len` | `u64` | length of the framebuffer in bytes, a multiple of the frame size, `0` if absent |
| `framebuffer_width`, `framebuffer_height` | `u32` | visible pixels per row, and rows |
| `framebuffer_stride` | `u32` | pixels per scan line, at least `framebuffer_width` |
| `framebuffer_format` | `u32` | `0` absent, `1` `Rgbx8888` (red in the lowest byte), `2` `Bgrx8888` (blue in the lowest byte); four bytes per pixel in both |
| `region_count` | `u32` | number of entries in the region array, at most `MAX_BOOT_REGIONS` |
| `regions` | `[BootRegion; region_count]` | `{ start: u64, len: u64, kind: u32, reserved: u32 }` |

Region kinds: `Usable`, `Reserved`, `AcpiReclaimable`, `AcpiNvs`,
`MmioReserved`. Loader code and data are reported as `Usable`.

Framebuffer rules: with `framebuffer_format == 0` every framebuffer field
is `0`. Otherwise `framebuffer_phys_start` is frame-aligned and non-zero,
`framebuffer_len` covers `framebuffer_height * framebuffer_stride * 4`
bytes, `framebuffer_width` and `framebuffer_height` are non-zero, the range
overlaps no `Usable` region, and the loader reports it as one
`MmioReserved` region. The kernel exposes the description through
`system_info`; the root task creates the `Device` memory object for it.

The loader places the boot stack so that its top is `BOOT_STACK_TOP`
(`KERNEL_BASE - 0x100_0000`), with `BOOT_STACK_PAGES` pages below it and one
unmapped guard page below those, and maps the boot information page
read-only at `BOOT_INFO_VADDR` (`KERNEL_BASE - 0x200_0000`). All three
constants live in `audhsos-abi::layout`.

### 3.1.6 Boot image format

The boot image is a single file built by `cargo xtask image`. Little-endian
throughout.

| Offset | Size | Field | Validation |
|--------|------|-------|------------|
| 0 | 8 | magic `AUDHSOS\0` | exact match |
| 8 | 4 | format version | must be `1` |
| 12 | 4 | header length | `>= 64`, `<=` image length |
| 16 | 8 | root task offset | 4 KiB aligned, inside the image, after the header |
| 24 | 8 | root task length | `> 0`, `offset + length` inside the image without overflow |
| 32 | 8 | archive offset | 4 KiB aligned, inside the image, no overlap with the root task |
| 40 | 8 | archive length | `offset + length` inside the image without overflow; may be `0` |
| 48 | 8 | kernel reserve size | `0` selects the default; otherwise frame-aligned and below total RAM |
| 56 | 8 | flags | must be `0` in version 1 |

The root task is an ELF executable linked for the fixed user address
`ROOT_TASK_BASE`, with every section on a page of its own. The kernel reads
it with the same parser the loader uses and maps each segment with the
permissions its header names, refusing an image in which two segments share
a page (D-92). Every other process is loaded from ELF by the root task the
same way.

The archive is a ustar tar archive. The kernel never reads it.

### 3.1.7 Test exit protocol

Writing a byte `v` to port `0xF4` makes QEMU exit with status `(v << 1) | 1`.

| Writer | Byte | QEMU exit status | Runner reports |
|--------|------|------------------|----------------|
| kernel or loader | `0x10` | 33 | success |
| kernel or loader | `0x11` | 35 | test failure |
| loader | `0x12` | 37 | loader failure |
| - | any other status, or QEMU exits by itself | - | crash (triple fault, hang killed by timeout, QEMU error) |

The serial output carries a line protocol that the runner parses:

```
[test] <crate>::<name> ... ok
[test] <crate>::<name> ... FAILED: <message>
[bench] <crate>::<name> ... <ticks> ticks (n=<count>)
[summary] passed=<n> failed=<m>
[info] <subject>=<what>
```

An `[info]` line reports what the kernel found on the machine rather than
what a test made of it, and the runner reads it: `[info] framebuffer=absent`
is what a machine without a graphics adapter says, and a machine with one
names its mode. A `[bench]` line reports a measurement and not a test: `<ticks>` is the
median of `<count>` round trips, in ticks of the time-stamp counter, and
the line counts towards neither the passed nor the failed total. An image
that writes one writes its test lines and its summary like every other.

Userland end-to-end tests use the same protocol through the console driver.

## 3.2 HAL trait surface

The HAL trait crate is architecture neutral. Only `x86_64` is implemented in
the first release.

| Trait | Responsibility | `x86_64` adapter |
|-------|----------------|------------------|
| `Platform` | boot information: memory regions, boot image location, physical window offset, ACPI root pointer | validated `BootInfo` |
| `InterruptController` | map a line to a vector, mask, unmask, end-of-interrupt, spurious handling | local APIC and I/O APIC register blocks |
| `Timer` | start a periodic tick with a frequency, read the tick counter | local APIC timer calibrated with the PIT |
| `FrameAccess<T>` | a physical frame as a `&mut PageTable` of entry type `T` | the physical window |
| `FrameBytes` | a physical frame as bytes, and a range of frames as a slice | the same window |
| `TlbControl` | flush one page, flush all | `invlpg`, `CR3` reload |
| `AddressSpaceControl` | make an address space the one the processor translates through, and name the active one | `CR3` |
| `FrameSource` | supply and take back frames for page tables | the kernel frame allocator |
| `DebugConsole` | write bytes | `driver-uart16550` logic over direct port I/O (feature `debug-uart` of the adapter) |
| `TestExit` | exit the machine with success or failure | `isa-debug-exit` (feature `test-exit` of the adapter) |
| `PortAccess` | read and write 8/16/32-bit ports (feature `port-io`, `x86_64` only) | `in`, `out` wrappers |
| `Devices` | what a system call that touches hardware is given: an `InterruptController` and a `PortAccess` at once | the two of them |

What has no trait is what only the adapter can do at all and no logic crate
ever calls through an interface: the privileged instructions, the
descriptor tables, the context switch, and the trap entry are modules of
`kernel-hal-x86_64` — `instructions`, `descriptors`, `context`, `traps` —
and a second architecture writes its own. A trait exists where a logic
crate or a test double stands on the other side of it.

The `x86_64` adapter must not leak into the trait crate: no port I/O, no
segment registers, no APIC concepts appear in the traits. The `PortAccess`
trait is feature-gated.

## 3.3 Later targets

| Target | Machine | Boot | HAL adapter content |
|--------|---------|------|---------------------|
| `aarch64-unknown-none` | QEMU `virt`, GICv3, PL011 UART, generic timer, virtio-mmio, device tree | QEMU loads the kernel ELF directly (`-kernel`); no loader | boot stub that sets the stack pointer, exception vector table with sixteen entries as naked functions, MMU setup before the higher-half jump, semihosting exit; runs under HVF on Apple Silicon |
| `riscv64gc-unknown-none-elf` | QEMU `virt`, OpenSBI, PLIC, SBI timer and console | OpenSBI in QEMU loads the kernel ELF | trap vector as a naked function, SBI calls, Sv39 page tables, `sifive_test` MMIO exit device |
