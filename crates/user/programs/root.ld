/* SPDX-License-Identifier: AGPL-3.0-only */
/* Copyright (C) 2026 Manuel Baesler and contributors */

ENTRY(_start)

/* Must equal `audhsos_abi::layout::ROOT_TASK_BASE`; the xtask checks it. */
ROOT_TASK_BASE = 0x10000000;

SECTIONS {
    . = ROOT_TASK_BASE;

    .text : {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    /* Every section on a page of its own: a loader maps pages, and two
       segments that share one cannot both have the permissions they ask
       for. */
    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .rodata : {
        *(.rodata .rodata.*)
    }

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .data : {
        *(.data .data.*)
        *(.got .got.*)
    }

    . = ALIGN(CONSTANT(MAXPAGESIZE));
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
