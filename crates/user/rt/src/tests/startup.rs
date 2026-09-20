// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::startup`.

use audhsos_abi::FramebufferFormat;
use audhsos_abi::Handle;
use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::startup::{Location, Role, Screen, StartupError, Writer};

use crate::handle::Typed;
use crate::startup::{MAX_BLOCK_DEVICES, MAX_RAM_OBJECTS, ReadError, Startup};

/// A handle every table could hand out.
fn handle(index: u32) -> Handle {
    Handle::new(index, 1).unwrap()
}

/// A buffer carrying the startup message for `pairs`.
fn message(pairs: &[(Role, Handle)]) -> [u8; SIZE] {
    let mut bytes = [0u8; SIZE];
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    for (role, handle) in pairs {
        writer.give(&mut view, *role, *handle).unwrap();
    }
    writer.finish(&mut view).unwrap();
    bytes
}

/// What a program reads out of the message for `pairs`.
fn read(pairs: &[(Role, Handle)]) -> Result<Startup, ReadError> {
    let bytes = message(pairs);
    Startup::read(Buffer::new(&bytes))
}

#[test]
fn a_process_that_was_given_nothing_has_no_field_set() {
    let startup = read(&[]).unwrap();
    assert_eq!(startup.own_process, None);
    assert_eq!(startup.own_endpoint, None);
    assert_eq!(startup.system_control, None);
    assert_eq!(startup.boot_image, None);
    assert_eq!(startup.name_server, None);
    assert_eq!(startup.memory_server, None);
    assert_eq!(startup.log, None);
    assert_eq!(startup.io_ports, None);
    assert_eq!(startup.interrupt, None);
    assert!(startup.ram.is_empty());
}

#[test]
fn a_new_startup_is_the_same_as_one_read_from_an_empty_message() {
    let startup = Startup::new();
    assert_eq!(startup.own_process, None);
    assert!(startup.ram.is_empty());
}

#[test]
fn every_role_lands_in_the_field_it_names() {
    let startup = read(&[
        (Role::OwnProcess, handle(1)),
        (Role::OwnEndpoint, handle(2)),
        (Role::SystemControl, handle(3)),
        (Role::BootImage, handle(4)),
        (Role::NameServer, handle(5)),
        (Role::MemoryServer, handle(6)),
        (Role::Log, handle(7)),
        (Role::IoPorts, handle(8)),
        (Role::Interrupt, handle(9)),
        (Role::Parent, handle(10)),
        (Role::Framebuffer, handle(11)),
        (Role::DisplayServer, handle(12)),
        (Role::AuxInterrupt, handle(13)),
        (Role::InputServer, handle(14)),
        (Role::Ecam, handle(15)),
        (Role::BlockRegisters, handle(16)),
        (Role::BlockInterrupt, handle(17)),
        (Role::BlockNotification, handle(18)),
        (Role::NetRegisters, handle(19)),
        (Role::NetInterrupt, handle(20)),
        (Role::NetNotification, handle(21)),
        (Role::NetServer, handle(22)),
        (Role::FileServer, handle(23)),
    ])
    .unwrap();
    assert_eq!(startup.own_process.unwrap().handle(), handle(1));
    assert_eq!(startup.own_endpoint.unwrap().handle(), handle(2));
    assert_eq!(startup.system_control.unwrap().handle(), handle(3));
    assert_eq!(startup.boot_image.unwrap().handle(), handle(4));
    assert_eq!(startup.name_server.unwrap().handle(), handle(5));
    assert_eq!(startup.memory_server.unwrap().handle(), handle(6));
    assert_eq!(startup.log.unwrap().handle(), handle(7));
    assert_eq!(startup.io_ports.unwrap().handle(), handle(8));
    assert_eq!(startup.interrupt.unwrap().handle(), handle(9));
    assert_eq!(startup.parent.unwrap().handle(), handle(10));
    assert_eq!(startup.framebuffer.unwrap().handle(), handle(11));
    assert_eq!(startup.display_server.unwrap().handle(), handle(12));
    assert_eq!(startup.aux_interrupt.unwrap().handle(), handle(13));
    assert_eq!(startup.input_server.unwrap().handle(), handle(14));
    assert_eq!(startup.ecam.unwrap().handle(), handle(15));
    let device = startup.blocks.iter().next().unwrap();
    assert_eq!(device.registers.handle(), handle(16));
    assert_eq!(device.interrupt.unwrap().handle(), handle(17));
    assert_eq!(device.notification.unwrap().handle(), handle(18));
    let net = startup.net.unwrap();
    assert_eq!(net.registers.handle(), handle(19));
    assert_eq!(net.interrupt.unwrap().handle(), handle(20));
    assert_eq!(net.notification.unwrap().handle(), handle(21));
    assert_eq!(startup.net_server.unwrap().handle(), handle(22));
    assert_eq!(startup.file_server.unwrap().handle(), handle(23));
    // The name of this test is a promise, and a role added later would
    // break it silently otherwise: every role but `Ram`, which is a list
    // and has a test of its own, and the fifteen value roles, which carry
    // no handle and are read in
    // `the_mode_of_the_framebuffer_comes_as_two_words`,
    // `the_bus_range_of_the_configuration_window_comes_as_one_word`,
    // `the_four_structures_of_the_block_device_come_as_four_words` and
    // `the_four_structures_of_the_network_device_come_as_four_words`, is
    // one field above.
    assert_eq!(
        Role::ALL.len(),
        39,
        "a role was added; give it a field and a line here"
    );
}

#[test]
fn the_memory_objects_come_back_as_a_list_in_the_order_they_were_given() {
    let startup = read(&[
        (Role::Ram, handle(10)),
        (Role::OwnProcess, handle(1)),
        (Role::Ram, handle(11)),
        (Role::Ram, handle(12)),
    ])
    .unwrap();
    let given: Vec<Handle> = startup.ram.iter().map(|memory| memory.handle()).collect();
    assert_eq!(given, vec![handle(10), handle(11), handle(12)]);
    assert_eq!(startup.own_process.unwrap().handle(), handle(1));
}

#[test]
fn a_role_that_takes_one_handle_may_not_appear_twice() {
    let outcome = read(&[(Role::NameServer, handle(1)), (Role::NameServer, handle(2))]);
    assert_eq!(outcome.unwrap_err(), ReadError::Duplicate(Role::NameServer));
}

#[test]
fn more_memory_objects_than_the_list_holds_are_refused() {
    let mut pairs = Vec::new();
    for index in 0..=MAX_RAM_OBJECTS {
        let raw = u32::try_from(index).unwrap().wrapping_add(1);
        pairs.push((Role::Ram, handle(raw)));
    }
    assert_eq!(
        Startup::read(Buffer::new(&message(&pairs))).unwrap_err(),
        ReadError::TooManyRam
    );
}

#[test]
fn exactly_as_many_memory_objects_as_the_list_holds_are_taken() {
    let mut pairs = Vec::new();
    for index in 0..MAX_RAM_OBJECTS {
        let raw = u32::try_from(index).unwrap().wrapping_add(1);
        pairs.push((Role::Ram, handle(raw)));
    }
    let startup = Startup::read(Buffer::new(&message(&pairs))).unwrap();
    assert_eq!(startup.ram.len(), MAX_RAM_OBJECTS);
}

#[test]
fn a_message_that_is_no_startup_message_is_passed_on_as_it_stands() {
    let mut bytes = message(&[(Role::OwnProcess, handle(1))]);
    BufferMut::new(&mut bytes).set_label(0);
    assert_eq!(
        Startup::read(Buffer::new(&bytes)).unwrap_err(),
        ReadError::Message(StartupError::NotStartup)
    );
}

#[test]
fn every_error_renders_a_message() {
    let cases = [
        ReadError::Message(StartupError::NotStartup),
        ReadError::Duplicate(Role::Log),
        ReadError::TooManyRam,
    ];
    for case in cases {
        assert!(!format!("{case}").is_empty(), "{case:?} has no message");
    }
}

/// A buffer carrying a startup message of handles and values.
fn mixed_message(handles: &[(Role, Handle)], values: &[(Role, u64)]) -> [u8; SIZE] {
    let mut bytes = [0u8; SIZE];
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    for (role, handle) in handles {
        writer.give(&mut view, *role, *handle).unwrap();
    }
    for (role, value) in values {
        writer.tell(&mut view, *role, *value).unwrap();
    }
    writer.finish(&mut view).unwrap();
    bytes
}

/// What a program reads out of such a message.
fn read_mixed(handles: &[(Role, Handle)], values: &[(Role, u64)]) -> Result<Startup, ReadError> {
    let bytes = mixed_message(handles, values);
    Startup::read(Buffer::new(&bytes))
}

#[test]
fn the_mode_of_the_framebuffer_comes_as_two_words() {
    let screen = Screen {
        width: 1280,
        height: 800,
        stride: 1280,
        format: FramebufferFormat::Bgrx8888,
    };
    let startup = read_mixed(
        &[(Role::Framebuffer, handle(3))],
        &[
            (Role::FramebufferGeometry, screen.geometry()),
            (Role::FramebufferLine, screen.line()),
        ],
    )
    .unwrap();
    assert_eq!(startup.framebuffer.unwrap().handle(), handle(3));
    assert_eq!(startup.screen(), Some(screen));
}

#[test]
fn the_bus_range_of_the_configuration_window_comes_as_one_word() {
    let buses = audhsos_abi::startup::BusRange {
        segment: 0,
        first_bus: 0,
        last_bus: 255,
    };
    let startup = read_mixed(
        &[(Role::Ecam, handle(4))],
        &[(Role::EcamBuses, buses.word())],
    )
    .unwrap();
    assert_eq!(startup.ecam.unwrap().handle(), handle(4));
    assert_eq!(startup.bus_range(), Some(buses));
}

#[test]
fn a_process_given_no_configuration_window_has_no_bus_range() {
    let startup = read(&[(Role::OwnProcess, handle(1))]).unwrap();
    assert_eq!(startup.bus_range(), None);
}

#[test]
fn half_a_description_of_the_framebuffer_is_no_mode() {
    let startup = read_mixed(&[], &[(Role::FramebufferGeometry, 1)]).unwrap();
    assert_eq!(startup.screen(), None);
    let startup = read_mixed(&[], &[(Role::FramebufferLine, 1)]).unwrap();
    assert_eq!(startup.screen(), None);
    assert_eq!(read_mixed(&[], &[]).unwrap().screen(), None);
}

#[test]
fn a_value_role_may_not_appear_twice_either() {
    let outcome = read_mixed(
        &[],
        &[(Role::FramebufferLine, 1), (Role::FramebufferLine, 2)],
    );
    assert_eq!(
        outcome.unwrap_err(),
        ReadError::Duplicate(Role::FramebufferLine)
    );
}

#[test]
fn the_four_structures_of_the_block_device_come_as_four_words() {
    let places = [
        Location { offset: 0, len: 56 },
        Location {
            offset: 0x3000,
            len: 0x1000,
        },
        Location {
            offset: 0x1000,
            len: 1,
        },
        Location {
            offset: 0x2000,
            len: 8,
        },
    ];
    let startup = read_mixed(
        &[
            (Role::BlockRegisters, handle(3)),
            (Role::BlockInterrupt, handle(4)),
            (Role::BlockNotification, handle(5)),
        ],
        &[
            (Role::BlockCommon, places[0].word()),
            (Role::BlockNotify, places[1].word()),
            (Role::BlockIsr, places[2].word()),
            (Role::BlockConfig, places[3].word()),
            (Role::BlockNotifyMultiplier, 4),
            (Role::BlockVectorBit, 7),
        ],
    )
    .unwrap();
    let device = startup.blocks.iter().next().unwrap();
    assert_eq!(device.structures(), Some(places));
    assert_eq!(device.notify_multiplier, Some(4));
    assert_eq!(device.vector_bit, Some(7));
}

#[test]
fn the_four_structures_of_the_network_device_come_as_four_words() {
    let places = [
        Location { offset: 0, len: 56 },
        Location {
            offset: 0x3000,
            len: 0x1000,
        },
        Location {
            offset: 0x1000,
            len: 1,
        },
        Location {
            offset: 0x2000,
            len: 8,
        },
    ];
    let startup = read_mixed(
        &[
            (Role::NetRegisters, handle(3)),
            (Role::NetInterrupt, handle(4)),
            (Role::NetNotification, handle(5)),
        ],
        &[
            (Role::NetCommon, places[0].word()),
            (Role::NetNotify, places[1].word()),
            (Role::NetIsr, places[2].word()),
            (Role::NetConfig, places[3].word()),
            (Role::NetNotifyMultiplier, 4),
            (Role::NetVectorBit, 0),
        ],
    )
    .unwrap();
    let device = startup.net.unwrap();
    assert_eq!(device.structures(), Some(places));
    assert_eq!(device.notify_multiplier, Some(4));
    assert_eq!(device.vector_bit, Some(0));
}

#[test]
fn a_role_of_the_network_device_before_its_window_is_refused() {
    let outcome = read(&[(Role::NetInterrupt, handle(2))]);
    assert_eq!(
        outcome.unwrap_err(),
        ReadError::DeviceWithoutRegisters(Role::NetInterrupt)
    );
}

#[test]
fn a_second_network_device_is_refused() {
    let outcome = read(&[
        (Role::NetRegisters, handle(1)),
        (Role::NetRegisters, handle(2)),
    ]);
    assert_eq!(
        outcome.unwrap_err(),
        ReadError::Duplicate(Role::NetRegisters)
    );
}

#[test]
fn a_block_device_described_in_part_names_no_structures() {
    let startup = read_mixed(
        &[(Role::BlockRegisters, handle(3))],
        &[(Role::BlockCommon, Location { offset: 0, len: 56 }.word())],
    )
    .unwrap();
    let device = startup.blocks.iter().next().unwrap();
    assert_eq!(device.structures(), None);
}

#[test]
fn every_role_of_one_block_device_is_refused_a_second_time() {
    for role in [Role::BlockInterrupt, Role::BlockNotification] {
        let outcome = read(&[
            (Role::BlockRegisters, handle(1)),
            (role, handle(2)),
            (role, handle(3)),
        ]);
        assert_eq!(outcome.unwrap_err(), ReadError::Duplicate(role));
    }
    let values = [
        Role::BlockCommon,
        Role::BlockNotify,
        Role::BlockIsr,
        Role::BlockConfig,
        Role::BlockNotifyMultiplier,
        Role::BlockVectorBit,
    ];
    for role in values {
        let outcome = read_mixed(
            &[(Role::BlockRegisters, handle(1))],
            &[(role, 1), (role, 2)],
        );
        assert_eq!(outcome.unwrap_err(), ReadError::Duplicate(role));
    }
}

#[test]
fn the_nine_roles_of_a_second_device_describe_a_second_device() {
    let first = Location { offset: 0, len: 56 };
    let second = Location {
        offset: 0x1000,
        len: 64,
    };
    let mut bytes = [0u8; SIZE];
    let mut view = BufferMut::new(&mut bytes);
    let mut writer = Writer::new();
    for (index, place) in [(16u32, first), (26, second)] {
        writer
            .give(&mut view, Role::BlockRegisters, handle(index))
            .unwrap();
        writer
            .tell(&mut view, Role::BlockCommon, place.word())
            .unwrap();
        writer
            .give(&mut view, Role::BlockInterrupt, handle(index + 1))
            .unwrap();
    }
    writer.finish(&mut view).unwrap();
    let startup = Startup::read(Buffer::new(&bytes)).unwrap();
    assert_eq!(startup.blocks.len(), 2);
    let devices: Vec<Handle> = startup
        .blocks
        .iter()
        .map(|device| device.registers.handle())
        .collect();
    assert_eq!(devices, vec![handle(16), handle(26)]);
    let commons: Vec<Option<u64>> = startup.blocks.iter().map(|device| device.common).collect();
    assert_eq!(commons, vec![Some(first.word()), Some(second.word())]);
}

#[test]
fn a_block_role_before_the_one_that_opens_a_device_is_refused() {
    let outcome = read(&[(Role::BlockInterrupt, handle(1))]);
    assert_eq!(
        outcome.unwrap_err(),
        ReadError::BlockWithoutRegisters(Role::BlockInterrupt)
    );
    let outcome = read_mixed(&[], &[(Role::BlockCommon, 1)]);
    assert_eq!(
        outcome.unwrap_err(),
        ReadError::BlockWithoutRegisters(Role::BlockCommon)
    );
}

#[test]
fn more_block_devices_than_the_list_holds_are_refused() {
    let mut pairs = Vec::new();
    for index in 0..=MAX_BLOCK_DEVICES {
        let raw = u32::try_from(index).unwrap().wrapping_add(1);
        pairs.push((Role::BlockRegisters, handle(raw)));
    }
    assert_eq!(read(&pairs).unwrap_err(), ReadError::TooManyBlocks);
}
