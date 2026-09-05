# boot-uefi-x86_64

The loader. It runs as `EFI/BOOT/BOOTX64.EFI`, reads `AUDHSOS/KERNEL.ELF`
and `AUDHSOS/BOOT.IMG` from the boot volume, places the kernel image in
physical memory, builds the page tables with the kernel's own mapper,
writes the boot information page, leaves the boot services, and jumps to
the kernel. Every firmware call lives in one `unsafe` block of its own;
everything the loader decides is computed in safe Rust.
