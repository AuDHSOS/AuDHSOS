/* SPDX-License-Identifier: AGPL-3.0-only */
/* Copyright (C) 2026 Manuel Baesler and contributors */

ENTRY(kernel_entry)

/* Must equal audhsos_abi::layout::KERNEL_BASE; the xtask checks it. */
KERNEL_BASE = 0xFFFFFFFF80000000;

SECTIONS {
    . = KERNEL_BASE;

    .text : ALIGN(4096) {
        *(.text .text.*)
    }

    .apstart : ALIGN(4096) {
        __apstart_start = .;
        KEEP(*(.apstart))
        __apstart_end = .;
    }
    ASSERT(__apstart_end - __apstart_start < 0xF00, "AP startup code overlaps parameters")

    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4096) {
        *(.data .data.*)
        *(.got .got.*)
    }

    .bss : ALIGN(4096) {
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
