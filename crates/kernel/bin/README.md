# audhsos-kernel

The kernel image. It holds the entry the loader jumps to, the panic
handler, and nothing else: the machine work is `kernel-hal-x86_64`, the
kernel work is `kernel-core`. The linker script places the image at
`KERNEL_BASE` and gives every section its own frame, so that the loader can
map code read and execute and data read and write.
