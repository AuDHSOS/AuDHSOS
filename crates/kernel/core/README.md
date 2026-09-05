# kernel-core

The kernel without a machine: the boot sequence, the memory bring-up, the
trap report, the counts that size the kernel tables, and the cells that
hold the global kernel state. Everything here reaches the hardware through
the traits of `kernel-hal-api`, so the whole sequence runs on the host
against recording doubles and the same code runs in QEMU against the
adapter.
