/* SPDX-License-Identifier: AGPL-3.0-only */
/* Copyright (C) 2026 Manuel Baesler and contributors */

ENTRY(_start)

/* Where the programs of the archive are linked. It leaves room below for
   the stack `user-loader` puts there, and lies far below the IPC buffers
   at the top of the address space. The xtask checks it against
   `PROGRAM_BASE`. */
PROGRAM_BASE = 0x1000000;

SECTIONS {
    . = PROGRAM_BASE;

    .text : {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    .rodata : {
        *(.rodata .rodata.*)
    }

    .data : {
        *(.data .data.*)
        *(.got .got.*)
    }

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note .note.*)
        *(.comment)
    }
}
