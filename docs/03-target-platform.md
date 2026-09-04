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
  -no-reboot \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04
```

Accelerator: TCG. `-no-reboot` turns a triple fault into a QEMU exit, which
the test runner reports as a crash.

### 3.1.2 Devices

| Device | Access path | Used by | Phase |
|--------|-------------|---------|-------|
| UEFI boot services and configuration table | `extern "efiapi"` calls through project-defined bindings | loader | 2 |
| Serial 16550, COM1 (I/O ports `0x3F8`-`0x3FF`, IRQ 4) | port I/O | kernel debug output (`debug-uart` feature); userland console driver via `IoPortRange` and `Interrupt` | 2, 7 |
| Local APIC (xAPIC, MMIO base from ACPI MADT) | MMIO through the physical memory window | timer tick, end-of-interrupt, spurious vector | 4 |
| I/O APIC (MMIO base from ACPI MADT) | MMIO | routing IRQ lines to vectors, masking and unmasking | 4 |
| Legacy 8259 PIC | port I/O | masked once at boot, never used again | 4 |
| PIT channel 2 | port I/O | one-time calibration of the local APIC timer frequency | 4 |
| ACPI RSDP (from the UEFI configuration table), RSDT/XSDT, MADT | bytes read through the physical window, parsed in safe Rust | APIC discovery | 4 |
| `isa-debug-exit` (I/O port `0xF4`) | port I/O | test exit codes from loader and kernel | 2 |
| PCI configuration space via ECAM (`MCFG`) | MMIO via `Device` memory objects | userland virtio drivers | later |
| virtio-blk, virtio-net over PCI | MMIO, interrupts | userland drivers | later |
| VGA/framebuffer | not used | - | - |

### 3.1.3 Loader

The loader is the crate `boot-uefi-x86_64`, built for the target
`x86_64-unknown-uefi` and installed on the disk image as
`EFI/BOOT/BOOTX64.EFI`.

- UEFI bindings are defined in the crate `audhsos-uefi` (logic crate,
  `repr(C)` structures and constants only, no calls): system table, boot
  services table, `EFI_LOADED_IMAGE_PROTOCOL`,
  `EFI_SIMPLE_FILE_SYSTEM_PROTOCOL`, `EFI_FILE_PROTOCOL`,
  `EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL`, memory descriptor, memory types,
  configuration table entries, the ACPI 2.0 table GUID. Structure layouts
  are tested for size and field offsets on the host.
- The loader calls exactly these services: `HandleProtocol`,
  `AllocatePages`, `FreePages`, `GetMemoryMap`, `ExitBootServices`, the
  file protocol's `Open`, `GetInfo`, `Read`, `Close`, and the text output
  protocol's `OutputString` for diagnostics.
- Files read from the boot volume: `AUDHSOS/KERNEL.ELF` and
  `AUDHSOS/BOOT.IMG`. File names are 8.3 names.
- The kernel ELF is parsed by `audhsos-elf`. The page tables are built by
  the `kernel-mm` mapper with the loader's `FrameAccess` adapter over the
  identity mapping.
- The loader contains one naked function: write `CR3`, load the boot stack
  pointer, jump to the kernel entry with the boot information address.
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
| `region_count` | `u32` | number of entries in the region array, at most `MAX_BOOT_REGIONS` |
| `regions` | `[BootRegion; region_count]` | `{ start: u64, len: u64, kind: u32, reserved: u32 }` |

Region kinds: `Usable`, `Reserved`, `AcpiReclaimable`, `AcpiNvs`,
`MmioReserved`. Loader code and data are reported as `Usable`.

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

The root task is a flat binary linked for the fixed user address
`ROOT_TASK_BASE` with its entry point at offset 0 and its `.bss` included in
the file. The kernel maps the whole root task range read/write/execute at
`ROOT_TASK_BASE`. Every other process is loaded from ELF by the root task
with segment permissions.

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
[summary] passed=<n> failed=<m>
```

Userland end-to-end tests use the same protocol through the console driver.

## 3.2 HAL trait surface

The HAL trait crate is architecture neutral. Only `x86_64` is implemented in
the first release.

| Trait | Responsibility | `x86_64` adapter |
|-------|----------------|------------------|
| `Platform` | boot information: memory regions, boot image location, physical window offset, ACPI root pointer | validated `BootInfo` |
| `Cpu` | halt, wait for interrupt, interrupt enable/disable with a guard type | `hlt`, `cli`, `sti` as project-defined `asm!` wrappers |
| `InterruptController` | map a line to a vector, mask, unmask, end-of-interrupt, spurious handling | local APIC and I/O APIC register blocks |
| `Timer` | start a periodic tick with a frequency, read the tick counter | local APIC timer calibrated with the PIT |
| `Paging` | constants (levels, page size, canonical range), `FrameAccess`, `TlbControl`, `activate(root)` | page tables through the physical window, `invlpg`, `CR3` |
| `Context` | build an initial user context, switch between kernel stacks, enter user mode | synthesized interrupt frame, naked context-switch function |
| `Traps` | install the kernel's exception, interrupt, and system call handlers | project-defined GDT, TSS, IDT types; `lgdt`, `lidt`, `ltr` |
| `DebugConsole` | write bytes (feature `debug-uart`) | `driver-uart16550` logic over direct port I/O |
| `TestExit` | exit the machine with success or failure (feature `test-exit`) | `isa-debug-exit` |
| `PortIo` | read and write 8/16/32-bit ports (feature `port-io`, `x86_64` only) | `in`, `out` wrappers |

The `x86_64` adapter must not leak into the trait crate: no port I/O, no
segment registers, no APIC concepts appear in the traits. The `PortIo`
trait is feature-gated.

## 3.3 Later targets

| Target | Machine | Boot | HAL adapter content |
|--------|---------|------|---------------------|
| `aarch64-unknown-none` | QEMU `virt`, GICv3, PL011 UART, generic timer, virtio-mmio, device tree | QEMU loads the kernel ELF directly (`-kernel`); no loader | boot stub that sets the stack pointer, exception vector table with sixteen entries as naked functions, MMU setup before the higher-half jump, semihosting exit; runs under HVF on Apple Silicon |
| `riscv64gc-unknown-none-elf` | QEMU `virt`, OpenSBI, PLIC, SBI timer and console | OpenSBI in QEMU loads the kernel ELF | trap vector as a naked function, SBI calls, Sv39 page tables, `sifive_test` MMIO exit device |
