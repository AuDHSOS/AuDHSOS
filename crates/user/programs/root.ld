/* SPDX-License-Identifier: AGPL-3.0-only */
/* Copyright (C) 2026 Manuel Baesler and contributors */

ENTRY(_start)

/* Must equal `audhsos_abi::layout::ROOT_TASK_BASE`; the xtask checks it. */
ROOT_TASK_BASE = 0x10000000;

SECTIONS {
    . = ROOT_TASK_BASE;

    /* The entry point is the first byte of the image: the kernel maps a
       flat binary and jumps at its base, so nothing may come before it. */
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

    /* `.bss` lies inside the file. A flat binary carries what it says and
       nothing more: the kernel copies the bytes of the image into frames
       and zeroes nothing behind them, so the zeros have to be in the
       image. */
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
