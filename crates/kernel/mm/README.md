# kernel-mm

The memory management logic of the kernel, free of hardware access: the
normalization of the firmware memory map, the selection of the kernel
reserve, the bitmap frame allocator over that reserve, the `x86_64`
page-table entry format behind an architecture-neutral trait, the mapper
that walks and edits page tables through the `kernel-hal-api` traits, the
region table of an address space, and the pool of kernel stacks with their
guard pages. Everything here runs on the host
against the test doubles of `kernel-hal-api`.
