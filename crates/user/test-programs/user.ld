/* SPDX-License-Identifier: AGPL-3.0-only */
/* Copyright (C) 2026 Manuel Baesler and contributors */

ENTRY(_start)

/* Must equal the base the test kernels map a program at; the xtask checks
   it against `USER_TEST_BASE`. */
USER_BASE = 0x400000;

SECTIONS {
    . = USER_BASE;

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
