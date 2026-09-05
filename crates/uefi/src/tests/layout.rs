// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the structure layouts, covering the layout items of the
//! catalog 6.6.14. Every offset and size is the value the UEFI
//! specification names for a 64-bit implementation.

#![allow(clippy::arithmetic_side_effects)]

use core::mem::{align_of, offset_of, size_of};

use crate::memory_map::{DESCRIPTOR_LEN, MemoryDescriptor};
use crate::protocols::{
    FILE_INFO_HEADER_LEN, FileInfo, FileProtocol, LoadedImageProtocol, SimpleFileSystemProtocol,
    SimpleTextOutputProtocol, Time,
};
use crate::tables::{BOOT_SERVICES_SIGNATURE, BootServices, SYSTEM_TABLE_SIGNATURE, SystemTable};
use crate::types::{ConfigurationTable, Guid, TableHeader};

#[test]
fn the_table_header_has_the_specified_offsets_and_size() {
    assert_eq!(size_of::<TableHeader>(), 24);
    assert_eq!(offset_of!(TableHeader, signature), 0);
    assert_eq!(offset_of!(TableHeader, revision), 8);
    assert_eq!(offset_of!(TableHeader, header_size), 12);
    assert_eq!(offset_of!(TableHeader, crc32), 16);
    assert_eq!(offset_of!(TableHeader, reserved), 20);
}

#[test]
fn a_guid_is_sixteen_bytes_with_four_byte_alignment() {
    assert_eq!(size_of::<Guid>(), 16);
    assert_eq!(align_of::<Guid>(), 4);
    assert_eq!(offset_of!(Guid, data1), 0);
    assert_eq!(offset_of!(Guid, data2), 4);
    assert_eq!(offset_of!(Guid, data3), 6);
    assert_eq!(offset_of!(Guid, data4), 8);
    assert_eq!(size_of::<ConfigurationTable>(), 24);
    assert_eq!(offset_of!(ConfigurationTable, vendor_table), 16);
}

#[test]
fn the_system_table_has_the_specified_offsets_and_size() {
    let offsets = [
        (offset_of!(SystemTable, header), 0),
        (offset_of!(SystemTable, firmware_vendor), 24),
        (offset_of!(SystemTable, firmware_revision), 32),
        (offset_of!(SystemTable, console_in_handle), 40),
        (offset_of!(SystemTable, console_in), 48),
        (offset_of!(SystemTable, console_out_handle), 56),
        (offset_of!(SystemTable, console_out), 64),
        (offset_of!(SystemTable, standard_error_handle), 72),
        (offset_of!(SystemTable, standard_error), 80),
        (offset_of!(SystemTable, runtime_services), 88),
        (offset_of!(SystemTable, boot_services), 96),
        (offset_of!(SystemTable, table_entries), 104),
        (offset_of!(SystemTable, configuration_table), 112),
    ];
    for (actual, expected) in offsets {
        assert_eq!(actual, expected);
    }
    assert_eq!(size_of::<SystemTable>(), 120);
    assert_eq!(SYSTEM_TABLE_SIGNATURE.to_le_bytes(), *b"IBI SYST");
}

#[test]
fn the_boot_services_table_holds_forty_four_slots_in_order() {
    let offsets = [
        (offset_of!(BootServices, header), 0),
        (offset_of!(BootServices, raise_tpl), 24),
        (offset_of!(BootServices, allocate_pages), 40),
        (offset_of!(BootServices, free_pages), 48),
        (offset_of!(BootServices, get_memory_map), 56),
        (offset_of!(BootServices, allocate_pool), 64),
        (offset_of!(BootServices, handle_protocol), 152),
        (offset_of!(BootServices, reserved), 160),
        (offset_of!(BootServices, exit_boot_services), 232),
        (offset_of!(BootServices, calculate_crc32), 344),
        (offset_of!(BootServices, create_event_ex), 368),
    ];
    for (actual, expected) in offsets {
        assert_eq!(actual, expected);
    }
    assert_eq!(size_of::<BootServices>(), 376);
    assert_eq!(
        (size_of::<BootServices>() - size_of::<TableHeader>()) / 8,
        44,
        "forty-four service slots"
    );
    assert_eq!(BOOT_SERVICES_SIGNATURE.to_le_bytes(), *b"BOOTSERV");
}

#[test]
fn the_memory_descriptor_has_the_specified_offsets_and_size() {
    assert_eq!(size_of::<MemoryDescriptor>(), DESCRIPTOR_LEN);
    assert_eq!(offset_of!(MemoryDescriptor, type_), 0);
    assert_eq!(offset_of!(MemoryDescriptor, physical_start), 8);
    assert_eq!(offset_of!(MemoryDescriptor, virtual_start), 16);
    assert_eq!(offset_of!(MemoryDescriptor, pages), 24);
    assert_eq!(offset_of!(MemoryDescriptor, attribute), 32);
}

#[test]
fn the_loaded_image_protocol_has_the_specified_offsets_and_size() {
    let offsets = [
        (offset_of!(LoadedImageProtocol, revision), 0),
        (offset_of!(LoadedImageProtocol, parent_handle), 8),
        (offset_of!(LoadedImageProtocol, system_table), 16),
        (offset_of!(LoadedImageProtocol, device_handle), 24),
        (offset_of!(LoadedImageProtocol, file_path), 32),
        (offset_of!(LoadedImageProtocol, reserved), 40),
        (offset_of!(LoadedImageProtocol, load_options_size), 48),
        (offset_of!(LoadedImageProtocol, load_options), 56),
        (offset_of!(LoadedImageProtocol, image_base), 64),
        (offset_of!(LoadedImageProtocol, image_size), 72),
        (offset_of!(LoadedImageProtocol, image_code_type), 80),
        (offset_of!(LoadedImageProtocol, image_data_type), 84),
        (offset_of!(LoadedImageProtocol, unload), 88),
    ];
    for (actual, expected) in offsets {
        assert_eq!(actual, expected);
    }
    assert_eq!(size_of::<LoadedImageProtocol>(), 96);
}

#[test]
fn the_file_protocols_have_the_specified_offsets_and_sizes() {
    assert_eq!(size_of::<SimpleFileSystemProtocol>(), 16);
    assert_eq!(offset_of!(SimpleFileSystemProtocol, open_volume), 8);

    let offsets = [
        (offset_of!(FileProtocol, revision), 0),
        (offset_of!(FileProtocol, open), 8),
        (offset_of!(FileProtocol, close), 16),
        (offset_of!(FileProtocol, delete), 24),
        (offset_of!(FileProtocol, read), 32),
        (offset_of!(FileProtocol, write), 40),
        (offset_of!(FileProtocol, get_position), 48),
        (offset_of!(FileProtocol, set_position), 56),
        (offset_of!(FileProtocol, get_info), 64),
        (offset_of!(FileProtocol, set_info), 72),
        (offset_of!(FileProtocol, flush), 80),
        (offset_of!(FileProtocol, open_ex), 88),
        (offset_of!(FileProtocol, flush_ex), 112),
    ];
    for (actual, expected) in offsets {
        assert_eq!(actual, expected);
    }
    assert_eq!(size_of::<FileProtocol>(), 120);
}

#[test]
fn the_time_and_file_information_structures_match_the_specification() {
    assert_eq!(size_of::<Time>(), 16);
    assert_eq!(offset_of!(Time, year), 0);
    assert_eq!(offset_of!(Time, nanosecond), 8);
    assert_eq!(offset_of!(Time, time_zone), 12);
    assert_eq!(offset_of!(Time, daylight), 14);

    assert_eq!(size_of::<FileInfo>(), FILE_INFO_HEADER_LEN);
    assert_eq!(offset_of!(FileInfo, size), 0);
    assert_eq!(offset_of!(FileInfo, file_size), 8);
    assert_eq!(offset_of!(FileInfo, physical_size), 16);
    assert_eq!(offset_of!(FileInfo, create_time), 24);
    assert_eq!(offset_of!(FileInfo, last_access_time), 40);
    assert_eq!(offset_of!(FileInfo, modification_time), 56);
    assert_eq!(offset_of!(FileInfo, attribute), 72);
}

#[test]
fn the_text_output_protocol_has_the_specified_offsets_and_size() {
    assert_eq!(offset_of!(SimpleTextOutputProtocol, reset), 0);
    assert_eq!(offset_of!(SimpleTextOutputProtocol, output_string), 8);
    assert_eq!(offset_of!(SimpleTextOutputProtocol, enable_cursor), 64);
    assert_eq!(offset_of!(SimpleTextOutputProtocol, mode), 72);
    assert_eq!(size_of::<SimpleTextOutputProtocol>(), 80);
}

#[test]
fn the_identifiers_carry_the_values_the_specification_names() {
    use crate::protocols::{
        ACPI_20_TABLE, FILE_INFO, FILE_MODE_READ, FILE_MODE_WRITE, LOADED_IMAGE_PROTOCOL,
        SIMPLE_FILE_SYSTEM_PROTOCOL,
    };
    assert_eq!(
        ACPI_20_TABLE,
        Guid::new(
            0x8868_E871,
            0xE4F1,
            0x11D3,
            [0xBC, 0x22, 0x00, 0x80, 0xC7, 0x3C, 0x88, 0x81]
        )
    );
    assert_eq!(LOADED_IMAGE_PROTOCOL.data1, 0x5B1B_31A1);
    assert_eq!(SIMPLE_FILE_SYSTEM_PROTOCOL.data1, 0x964E_5B22);
    assert_eq!(FILE_INFO.data1, 0x0957_6E92);
    assert_ne!(LOADED_IMAGE_PROTOCOL, SIMPLE_FILE_SYSTEM_PROTOCOL);
    assert_eq!(FILE_MODE_READ, 1);
    assert_eq!(FILE_MODE_WRITE, 2);
}

#[test]
fn the_graphics_output_structures_have_the_specified_offsets_and_sizes() {
    use crate::protocols::{
        GraphicsOutputModeInformation, GraphicsOutputProtocol, GraphicsOutputProtocolMode,
    };

    assert_eq!(size_of::<GraphicsOutputModeInformation>(), 36);
    assert_eq!(offset_of!(GraphicsOutputModeInformation, version), 0);
    assert_eq!(
        offset_of!(GraphicsOutputModeInformation, horizontal_resolution),
        4
    );
    assert_eq!(
        offset_of!(GraphicsOutputModeInformation, vertical_resolution),
        8
    );
    assert_eq!(offset_of!(GraphicsOutputModeInformation, pixel_format), 12);
    assert_eq!(
        offset_of!(GraphicsOutputModeInformation, pixel_information),
        16
    );
    assert_eq!(
        offset_of!(GraphicsOutputModeInformation, pixels_per_scan_line),
        32
    );

    assert_eq!(size_of::<GraphicsOutputProtocolMode>(), 40);
    assert_eq!(offset_of!(GraphicsOutputProtocolMode, max_mode), 0);
    assert_eq!(offset_of!(GraphicsOutputProtocolMode, mode), 4);
    assert_eq!(offset_of!(GraphicsOutputProtocolMode, info), 8);
    assert_eq!(offset_of!(GraphicsOutputProtocolMode, size_of_info), 16);
    assert_eq!(
        offset_of!(GraphicsOutputProtocolMode, frame_buffer_base),
        24
    );
    assert_eq!(
        offset_of!(GraphicsOutputProtocolMode, frame_buffer_size),
        32
    );

    assert_eq!(size_of::<GraphicsOutputProtocol>(), 32);
    assert_eq!(offset_of!(GraphicsOutputProtocol, query_mode), 0);
    assert_eq!(offset_of!(GraphicsOutputProtocol, set_mode), 8);
    assert_eq!(offset_of!(GraphicsOutputProtocol, blt), 16);
    assert_eq!(offset_of!(GraphicsOutputProtocol, mode), 24);
}

#[test]
fn the_graphics_identifier_and_the_locate_service_are_in_place() {
    use crate::protocols::GRAPHICS_OUTPUT_PROTOCOL;

    assert_eq!(
        GRAPHICS_OUTPUT_PROTOCOL,
        Guid::new(
            0x9042_A9DE,
            0x23DC,
            0x4A38,
            [0x96, 0xFB, 0x7A, 0xDE, 0xD0, 0x80, 0x51, 0x6A]
        )
    );
    assert_eq!(
        offset_of!(BootServices, locate_protocol),
        320,
        "the loader calls it, so it is typed and its offset must be right"
    );
}
