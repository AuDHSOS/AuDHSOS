// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::calls::device`: interrupts, port ranges, device memory,
//! and the information about the machine.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::layout::{MAX_MESSAGE_BYTES, TICKS_PER_SECOND};
use audhsos_abi::{Error, Handle, ObjectType, Rights, Syscall};
use kernel_objects::object::{
    AnyObjectId, Interrupt, IoPortRange, MemoryKind, Notification, SystemControl,
};

use super::double::{
    Call, Fixture, MESSAGE_ADDRESS, MESSAGE_BASE, MESSAGE_VECTORS, call, error_of, request,
    value_of,
};

/// The handle to the system control capability, which the root task receives
/// at boot and every test installs by hand.
fn control(fixture: &mut Fixture) -> u64 {
    fixture
        .install(AnyObjectId::of(SystemControl::ID), Rights::MANAGE)
        .raw()
}

#[test]
fn an_interrupt_object_takes_the_vector_the_plan_gives_its_line() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let raw = value_of(
        &mut fixture,
        request(Syscall::InterruptCreate, &[system, 3]),
    );
    let handle = Handle::from_raw(raw).unwrap();
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    assert_eq!(entry.object_type(), ObjectType::Interrupt);
    let id = entry.object.typed::<Interrupt>().unwrap();
    let held = *fixture.objects.interrupts.get(id).unwrap();
    assert_eq!(held.line, Some(3));
    assert_eq!(held.vector, 0x43, "the double routes line n to 0x40 plus n");
    assert!(!held.masked);
    assert_eq!(fixture.environment.count(&Call::Route(3, 0x43)), 1);
}

#[test]
fn a_line_the_plan_reserves_no_vector_for_is_refused() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    fixture.environment.unroutable.push(7);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptCreate, &[system, 7])
        ),
        Some(Error::InvalidArgument)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptCreate, &[system, 0x1_0000])
        ),
        Some(Error::InvalidArgument),
        "a line above what a byte holds"
    );
    assert_eq!(fixture.objects.interrupts.live(), 0);
}

#[test]
fn a_line_an_interrupt_object_already_names_is_refused() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    value_of(
        &mut fixture,
        request(Syscall::InterruptCreate, &[system, 1]),
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptCreate, &[system, 1])
        ),
        Some(Error::AlreadyExists)
    );
    assert_eq!(fixture.objects.interrupts.live(), 1);
}

#[test]
fn an_interrupt_that_has_nowhere_to_go_gives_its_quota_back() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    aperture(&mut fixture);
    fixture.fill_handles();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptCreate, &[system, 2])
        ),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(fixture.objects.interrupts.live(), 0);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .kernel_object_quota
            .used(),
        0
    );
}

#[test]
fn a_binding_arms_the_line_names_one_bit_and_an_acknowledgement_unmasks_it_again() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let interrupt = value_of(
        &mut fixture,
        request(Syscall::InterruptCreate, &[system, 0]),
    );
    let notification = value_of(&mut fixture, request(Syscall::NotificationCreate, &[]));
    assert!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptBind, &[interrupt, notification, 3])
        )
        .is_none()
    );
    let id = fixture
        .objects
        .entry(fixture.process, Handle::from_raw(interrupt).unwrap())
        .unwrap()
        .object
        .typed::<Interrupt>()
        .unwrap();
    let bound = fixture.objects.interrupts.get(id).unwrap().notification;
    assert!(bound.is_some());
    assert_eq!(bound.map(|(_, bit)| bit), Some(3));
    // The binding armed the line: a created interrupt is routed masked,
    // because until a notification names it the line would assert into
    // nothing.
    assert_eq!(
        fixture.environment.count(&Call::Unmask(0)),
        1,
        "the binding did not arm the line"
    );

    // The interrupt arrives, the line is masked, and the acknowledgement
    // lets the next one through.
    kernel_ipc::deliver(&mut fixture.objects, &mut fixture.scheduler, 0x40).unwrap();
    assert!(fixture.objects.interrupts.get(id).unwrap().masked);
    assert!(error_of(&mut fixture, request(Syscall::InterruptAck, &[interrupt])).is_none());
    assert!(!fixture.objects.interrupts.get(id).unwrap().masked);
    assert_eq!(fixture.environment.count(&Call::Unmask(0)), 2);
}

#[test]
fn a_binding_needs_a_bit_of_the_word_and_a_notification_it_may_bind() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let interrupt = value_of(
        &mut fixture,
        request(Syscall::InterruptCreate, &[system, 0]),
    );
    let notification = value_of(&mut fixture, request(Syscall::NotificationCreate, &[]));
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptBind, &[interrupt, notification, 64])
        ),
        Some(Error::InvalidArgument)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptBind, &[interrupt, notification, 0x1_0000])
        ),
        Some(Error::InvalidArgument)
    );
    // A notification handle without `BIND` is refused.
    let id = fixture
        .objects
        .entry(fixture.process, Handle::from_raw(notification).unwrap())
        .unwrap()
        .object
        .typed::<Notification>()
        .unwrap();
    let weak = fixture.install(AnyObjectId::of(id), Rights::WAIT).raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptBind, &[interrupt, weak, 0])
        ),
        Some(Error::AccessDenied)
    );
    // A second interrupt may name the same notification, on a bit of its
    // own: a controller with two lines and one output buffer is drained by
    // one thread, and that thread waits on one notification (D-108).
    let second = value_of(
        &mut fixture,
        request(Syscall::InterruptCreate, &[system, 1]),
    );
    assert!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptBind, &[interrupt, notification, 0])
        )
        .is_none()
    );
    assert!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptBind, &[second, notification, 1])
        )
        .is_none()
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptBind, &[interrupt, 0, 0])
        ),
        Some(Error::InvalidHandle),
        "a word that names no handle"
    );
}

#[test]
fn a_port_range_covers_what_it_says_and_overlapping_one_is_refused() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let raw = value_of(
        &mut fixture,
        request(Syscall::IoPortCreate, &[system, 0x40, 4]),
    );
    let handle = Handle::from_raw(raw).unwrap();
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    assert_eq!(entry.object_type(), ObjectType::IoPortRange);
    let id = entry.object.typed::<IoPortRange>().unwrap();
    let held = *fixture.objects.ports.get(id).unwrap();
    assert_eq!((held.first, held.count), (0x40, 4));
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::IoPortCreate, &[system, 0x43, 1])
        ),
        Some(Error::AlreadyExists)
    );
    assert!(
        error_of(
            &mut fixture,
            request(Syscall::IoPortCreate, &[system, 0x44, 1])
        )
        .is_none(),
        "a range beside it is its own"
    );
}

#[test]
fn a_port_range_of_no_ports_or_past_the_last_one_is_refused() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    for arguments in [
        [system, 0x40, 0],
        [system, 0xFFFF, 2],
        [system, 0x1_0000, 1],
        [system, 0x40, 0x1_0000],
    ] {
        assert_eq!(
            error_of(&mut fixture, request(Syscall::IoPortCreate, &arguments)),
            Some(Error::InvalidArgument),
            "{arguments:?}"
        );
    }
    assert_eq!(fixture.objects.ports.live(), 0);
}

#[test]
fn a_port_is_read_and_written_only_inside_the_range_of_its_capability() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let range = value_of(
        &mut fixture,
        request(Syscall::IoPortCreate, &[system, 0x40, 4]),
    );
    fixture.environment.port_reads.insert(0x41, 0x5A);
    assert_eq!(
        value_of(
            &mut fixture,
            request(Syscall::IoPortRead, &[range, 0x41, 1])
        ),
        0x5A
    );
    assert!(
        error_of(
            &mut fixture,
            request(Syscall::IoPortWrite, &[range, 0x42, 2, 0x1234])
        )
        .is_none()
    );
    assert_eq!(
        fixture.environment.count(&Call::WritePort(0x42, 2, 0x1234)),
        1
    );
    for arguments in [
        [range, 0x44, 1, 0],
        [range, 0x3F, 1, 0],
        [range, 0x42, 4, 0],
        [range, 0x40, 3, 0],
        [range, 0x1_0000, 1, 0],
        [range, 0x40, 0x1_0000, 0],
    ] {
        assert_eq!(
            error_of(&mut fixture, request(Syscall::IoPortWrite, &arguments)),
            Some(Error::InvalidArgument),
            "{arguments:?}"
        );
        assert_eq!(
            error_of(&mut fixture, request(Syscall::IoPortRead, &arguments[..3])),
            Some(Error::InvalidArgument),
            "{arguments:?}"
        );
    }
}

/// A request that carries `bytes` in the message area, as the wrapper of
/// `ioport_write_string` puts them there: the payload words little-endian,
/// which is the run in order.
fn string_request(range: u64, port: u64, bytes: &[u8], count: u64) -> [u8; SIZE] {
    let mut buffer = request(Syscall::IoPortWriteString, &[range, port, count]);
    let mut writer = BufferMut::new(&mut buffer);
    for (index, chunk) in bytes.chunks(8).enumerate() {
        let mut word = [0u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        assert!(writer.set_word(index, u64::from_le_bytes(word)));
    }
    buffer
}

#[test]
fn a_string_write_moves_the_message_area_to_one_port() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let range = value_of(
        &mut fixture,
        request(Syscall::IoPortCreate, &[system, 0x3F8, 8]),
    );
    let line = b"[canvas] cursor 993 600\n";
    let count = u64::try_from(line.len()).unwrap();
    assert_eq!(
        value_of(&mut fixture, string_request(range, 0x3F8, line, count)),
        count,
        "the call answers how many bytes went out"
    );
    assert_eq!(fixture.environment.port_string, line.to_vec());
    assert_eq!(
        fixture
            .environment
            .count(&Call::WritePortString(0x3F8, line.len())),
        1,
        "one call, whatever the run holds"
    );
}

#[test]
fn a_string_write_is_refused_outside_the_range_and_above_the_message_area() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let range = value_of(
        &mut fixture,
        request(Syscall::IoPortCreate, &[system, 0x40, 4]),
    );
    for (port, count) in [
        (0x44, 1),
        (0x3F, 1),
        (0x1_0000, 1),
        (
            0x40,
            u64::try_from(MAX_MESSAGE_BYTES).unwrap().saturating_add(1),
        ),
    ] {
        assert_eq!(
            error_of(&mut fixture, string_request(range, port, b"x", count)),
            Some(Error::InvalidArgument),
            "port {port:#x} count {count}"
        );
    }
    assert!(fixture.environment.port_string.is_empty());
}

#[test]
fn a_string_write_of_nothing_touches_no_port() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let range = value_of(
        &mut fixture,
        request(Syscall::IoPortCreate, &[system, 0x40, 4]),
    );
    assert_eq!(
        value_of(&mut fixture, string_request(range, 0x40, b"", 0)),
        0
    );
    assert!(fixture.environment.port_string.is_empty());
}

#[test]
fn a_port_access_needs_the_right_for_its_direction() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let raw = value_of(
        &mut fixture,
        request(Syscall::IoPortCreate, &[system, 0x60, 2]),
    );
    let id = fixture
        .objects
        .entry(fixture.process, Handle::from_raw(raw).unwrap())
        .unwrap()
        .object
        .typed::<IoPortRange>()
        .unwrap();
    let readable = fixture.install(AnyObjectId::of(id), Rights::READ).raw();
    let writable = fixture.install(AnyObjectId::of(id), Rights::WRITE).raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::IoPortWrite, &[readable, 0x60, 1, 0])
        ),
        Some(Error::AccessDenied)
    );
    assert_eq!(
        error_of(&mut fixture, string_request(readable, 0x60, b"x", 1)),
        Some(Error::AccessDenied),
        "a string write is a write"
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::IoPortRead, &[writable, 0x60, 1])
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn a_device_memory_object_is_an_aperture_and_meets_no_memory() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    aperture(&mut fixture);
    fixture.environment.ram = Some(
        kernel_types::PhysFrameRange::new(
            kernel_types::PhysFrame::from_number(0x100).unwrap(),
            0x100,
        )
        .unwrap(),
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryCreateDevice, &[system, 0x180, 4])
        ),
        Some(Error::InvalidArgument),
        "it would cover memory of the machine"
    );
    let raw = value_of(
        &mut fixture,
        request(Syscall::MemoryCreateDevice, &[system, 0x1000, 2]),
    );
    let entry = fixture
        .objects
        .entry(fixture.process, Handle::from_raw(raw).unwrap())
        .unwrap();
    assert_eq!(entry.object_type(), ObjectType::MemoryObject);
    let id = entry
        .object
        .typed::<kernel_objects::object::MemoryObject>()
        .unwrap();
    let held = *fixture.objects.memory.get(id).unwrap();
    assert_eq!(held.kind, MemoryKind::Device);
    assert_eq!(held.cache, kernel_types::CachePolicy::Uncached);
    assert_eq!(held.frames.count(), 2);
}

/// Tells the environment that the frames `0x1000` to `0x1010` are an
/// aperture of the machine, which is what a device object may cover.
fn aperture(fixture: &mut Fixture) {
    fixture.environment.device = Some(
        kernel_types::PhysFrameRange::new(
            kernel_types::PhysFrame::from_number(0x1000).unwrap(),
            0x10,
        )
        .unwrap(),
    );
}

#[test]
fn a_range_in_no_aperture_of_the_machine_is_no_device() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    aperture(&mut fixture);
    for (first, count, why) in [
        (0x2000, 1, "it lies past the aperture"),
        (0x0FFF, 2, "it begins before the aperture"),
        (0x100F, 2, "it ends past the aperture"),
    ] {
        assert_eq!(
            error_of(
                &mut fixture,
                request(Syscall::MemoryCreateDevice, &[system, first, count])
            ),
            Some(Error::InvalidArgument),
            "{why}"
        );
    }
    assert_eq!(fixture.objects.memory.live(), 0);
}

#[test]
fn a_machine_that_reported_no_aperture_makes_no_device_object() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryCreateDevice, &[system, 0x1000, 1])
        ),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn a_device_range_of_no_frames_or_beyond_the_address_space_is_refused() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    for arguments in [[system, 0x1000, 0], [system, u64::MAX, 1]] {
        assert_eq!(
            error_of(
                &mut fixture,
                request(Syscall::MemoryCreateDevice, &arguments)
            ),
            Some(Error::InvalidArgument),
            "{arguments:?}"
        );
    }
}

#[test]
fn a_creating_call_that_finds_no_handle_slot_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    aperture(&mut fixture);
    fixture.fill_handles();
    for arguments in [
        (Syscall::IoPortCreate, vec![system, 0x40, 4]),
        (Syscall::MemoryCreateDevice, vec![system, 0x1000, 1]),
    ] {
        let (syscall, args) = arguments;
        assert_eq!(
            error_of(&mut fixture, request(syscall, &args)),
            Some(Error::QuotaExceeded),
            "{}",
            syscall.name()
        );
    }
    assert_eq!(fixture.objects.ports.live(), 0);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .kernel_object_quota
            .used(),
        0
    );
}

#[test]
fn the_system_information_reports_every_pool_the_ticks_and_the_firmware() {
    let mut fixture = Fixture::new();
    fixture.environment.rsdp = 0x000F_2340;
    fixture.environment.framebuffer = Some(audhsos_abi::Framebuffer {
        phys_start: 0x8000_0000,
        len: 0x0040_0000,
        width: 1280,
        height: 800,
        stride: 1280,
        format: audhsos_abi::FramebufferFormat::Bgrx8888,
    });
    let system = control(&mut fixture);
    let mut buffer = request(Syscall::SystemInfo, &[system]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], 26, "twenty-six words, as the convention says");
    let view = Buffer::new(&buffer);
    let header = view.message().unwrap();
    assert_eq!(header.label, 0);
    assert_eq!(header.word_count, 26);
    assert_eq!(header.handle_count, 0);
    let capacities = fixture.objects.capacities();
    let counts = fixture.objects.counts();
    for (index, (capacity, live)) in capacities.iter().zip(counts.iter()).enumerate() {
        let at = index * 2;
        assert_eq!(
            view.word(at),
            Some(u64::from(*capacity)),
            "capacity {index}"
        );
        assert_eq!(
            view.word(at + 1),
            Some(u64::from(*live)),
            "live count {index}"
        );
    }
    assert_eq!(view.word(18), Some(u64::from(TICKS_PER_SECOND)));
    assert_eq!(view.word(19), Some(0x000F_2340));
    assert_eq!(view.word(20), Some(0x8000_0000));
    assert_eq!(view.word(21), Some(0x0040_0000));
    assert_eq!(view.word(22), Some(1280));
    assert_eq!(view.word(23), Some(800));
    assert_eq!(view.word(24), Some(1280));
    assert_eq!(view.word(25), Some(2));
}

#[test]
fn a_machine_without_a_framebuffer_reports_six_zeros() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let mut buffer = request(Syscall::SystemInfo, &[system]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], 26);
    let view = Buffer::new(&buffer);
    for word in 20..26 {
        assert_eq!(view.word(word), Some(0), "word {word}");
    }
}

#[test]
fn the_root_authority_is_what_the_four_calls_check_and_nothing_else() {
    let mut fixture = Fixture::new();
    // A handle of another type is refused by the type check of the
    // dispatcher, before the call sees anything.
    let wrong = fixture.own_thread.raw();
    for (syscall, args) in [
        (Syscall::InterruptCreate, vec![wrong, 0]),
        (Syscall::IoPortCreate, vec![wrong, 0x40, 1]),
        (Syscall::MemoryCreateDevice, vec![wrong, 0x1000, 1]),
        (Syscall::SystemInfo, vec![wrong]),
    ] {
        assert_eq!(
            error_of(&mut fixture, request(syscall, &args)),
            Some(Error::WrongObjectType),
            "{}",
            syscall.name()
        );
    }
    // And one without `MANAGE` by the rights check.
    let weak = fixture
        .install(AnyObjectId::of(SystemControl::ID), Rights::DUPLICATE)
        .raw();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::SystemInfo, &[weak])),
        Some(Error::AccessDenied)
    );
}

/// Creates a message interrupt and answers the handle, the address, and the
/// data of it.
fn message_interrupt(fixture: &mut Fixture, system: u64) -> (Handle, u64, u64) {
    let mut buffer = request(Syscall::InterruptCreateMsi, &[system]);
    let (status, values, _) = call(fixture, &mut buffer);
    assert_eq!(status.error(), None);
    let view = Buffer::new(&buffer);
    (
        Handle::from_raw(values[0]).unwrap(),
        view.word(0).unwrap(),
        view.word(1).unwrap(),
    )
}

#[test]
fn a_message_interrupt_answers_a_handle_an_address_and_a_data_word() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let (handle, address, data) = message_interrupt(&mut fixture, system);
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    assert_eq!(entry.object_type(), ObjectType::Interrupt);
    assert_eq!(
        entry.rights,
        Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER
    );
    let id = entry.object.typed::<Interrupt>().unwrap();
    let held = *fixture.objects.interrupts.get(id).unwrap();
    assert_eq!(held.line, None, "a message interrupt has no line");
    assert_eq!(held.vector, MESSAGE_BASE);
    assert!(!held.masked);
    assert_eq!(address, MESSAGE_ADDRESS);
    assert_eq!(data, u64::from(MESSAGE_BASE));
    assert_eq!(
        fixture.environment.count(&Call::Route(0, 0)),
        0,
        "nothing routed"
    );
}

#[test]
fn a_vector_is_handed_out_once_and_the_exhausted_space_is_no_vector() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let mut vectors = Vec::new();
    for _ in 0..MESSAGE_VECTORS {
        let (handle, _, data) = message_interrupt(&mut fixture, system);
        vectors.push(data);
        let _ = handle;
    }
    let mut unique = vectors.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        vectors.len(),
        "no vector was handed out twice"
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptCreateMsi, &[system])
        ),
        Some(Error::NoVector)
    );
}

#[test]
fn a_message_interrupt_that_has_nowhere_to_go_gives_its_vector_back() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    fixture.fill_handles();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptCreateMsi, &[system])
        ),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(fixture.objects.interrupts.live(), 0);
    assert!(
        fixture.environment.messages.is_empty(),
        "the vector went back"
    );
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .kernel_object_quota
            .used(),
        0
    );
}

#[test]
fn a_machine_without_a_controller_refuses_a_message_interrupt() {
    let mut fixture = Fixture::new();
    fixture.environment.no_controller = true;
    let system = control(&mut fixture);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptCreateMsi, &[system])
        ),
        Some(Error::Unsupported)
    );
    assert_eq!(fixture.objects.interrupts.live(), 0);
}

#[test]
fn a_message_interrupt_needs_the_right_to_manage_the_machine() {
    let mut fixture = Fixture::new();
    let weak = fixture
        .install(AnyObjectId::of(SystemControl::ID), Rights::INFO)
        .raw();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::InterruptCreateMsi, &[weak])),
        Some(Error::AccessDenied)
    );
}

#[test]
fn a_message_interrupt_binds_like_any_other_and_its_ack_touches_no_hardware() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let (interrupt, _address, _data) = message_interrupt(&mut fixture, system);
    let notification = Handle::from_raw(value_of(
        &mut fixture,
        request(Syscall::NotificationCreate, &[]),
    ))
    .unwrap();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::InterruptBind,
                &[interrupt.raw(), notification.raw(), 5]
            )
        ),
        None
    );
    let unmasks = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::Unmask(_)))
        .count();
    assert_eq!(unmasks, 0, "there is no line to unmask");

    let id = fixture
        .objects
        .entry(fixture.process, interrupt)
        .unwrap()
        .object
        .typed::<Interrupt>()
        .unwrap();
    fixture.objects.interrupts.get_mut(id).unwrap().masked = true;
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptAck, &[interrupt.raw()])
        ),
        None
    );
    assert!(!fixture.objects.interrupts.get(id).unwrap().masked);
    let unmasks = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::Unmask(_)))
        .count();
    assert_eq!(unmasks, 0, "and none to unmask on the way out either");
}

#[test]
fn an_ack_without_an_outstanding_signal_is_no_error() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let (interrupt, _address, _data) = message_interrupt(&mut fixture, system);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptAck, &[interrupt.raw()])
        ),
        None
    );
}

#[test]
fn a_vector_freed_with_its_interrupt_object_is_handed_out_again() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let mut handles = Vec::new();
    for _ in 0..MESSAGE_VECTORS {
        let (handle, _, _) = message_interrupt(&mut fixture, system);
        handles.push(handle);
    }
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptCreateMsi, &[system])
        ),
        Some(Error::NoVector)
    );
    let first = handles.first().copied().unwrap();
    let freed = fixture
        .objects
        .entry(fixture.process, first)
        .unwrap()
        .object
        .typed::<Interrupt>()
        .unwrap();
    let vector = fixture.objects.interrupts.get(freed).unwrap().vector;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::HandleClose, &[first.raw()])),
        None
    );
    assert!(fixture.objects.interrupts.get(freed).is_err());
    let (_handle, _address, data) = message_interrupt(&mut fixture, system);
    assert_eq!(
        data,
        u64::from(vector),
        "the space handed the vector out again"
    );
}

#[test]
fn a_line_interrupt_that_is_closed_frees_no_message_vector() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let raw = value_of(
        &mut fixture,
        request(Syscall::InterruptCreate, &[system, 3]),
    );
    let handle = Handle::from_raw(raw).unwrap();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::HandleClose, &[handle.raw()])),
        None
    );
    let released = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::ReleaseMessage(_)))
        .count();
    assert_eq!(released, 0);
}

#[test]
fn a_message_interrupt_with_a_second_handle_keeps_its_vector() {
    let mut fixture = Fixture::new();
    let system = control(&mut fixture);
    let (handle, _, data) = message_interrupt(&mut fixture, system);
    let second = value_of(
        &mut fixture,
        request(
            Syscall::HandleDuplicate,
            &[handle.raw(), u64::from(Rights::MANAGE.bits())],
        ),
    );
    assert_eq!(
        error_of(&mut fixture, request(Syscall::HandleClose, &[handle.raw()])),
        None
    );
    let released = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::ReleaseMessage(_)))
        .count();
    assert_eq!(released, 0, "one handle of two is not the last reference");
    assert_eq!(
        error_of(&mut fixture, request(Syscall::HandleClose, &[second])),
        None
    );
    assert_eq!(
        fixture
            .environment
            .count(&Call::ReleaseMessage(u8::try_from(data).unwrap())),
        1
    );
}
