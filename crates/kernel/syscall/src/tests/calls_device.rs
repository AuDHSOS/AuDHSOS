// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::calls::device`: interrupts, port ranges, device memory,
//! and the information about the machine.

use audhsos_abi::ipc_buffer::Buffer;
use audhsos_abi::layout::TICKS_PER_SECOND;
use audhsos_abi::{Error, Handle, ObjectType, Rights, Syscall};
use kernel_objects::object::{
    AnyObjectId, Interrupt, IoPortRange, MemoryKind, Notification, SystemControl,
};

use super::double::{Call, Fixture, call, error_of, request, value_of};

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
    assert_eq!(held.line, 3);
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
fn a_binding_names_one_bit_and_an_acknowledgement_unmasks_the_line() {
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

    // The interrupt arrives, the line is masked, and the acknowledgement
    // lets the next one through.
    kernel_ipc::deliver(&mut fixture.objects, &mut fixture.scheduler, 0x40).unwrap();
    assert!(fixture.objects.interrupts.get(id).unwrap().masked);
    assert!(error_of(&mut fixture, request(Syscall::InterruptAck, &[interrupt])).is_none());
    assert!(!fixture.objects.interrupts.get(id).unwrap().masked);
    assert_eq!(fixture.environment.count(&Call::Unmask(0)), 1);
}

#[test]
fn a_binding_needs_a_bit_of_the_word_and_a_notification_of_its_own() {
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
    // And a second interrupt cannot take a notification that is bound.
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
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::InterruptBind, &[second, notification, 1])
        ),
        Some(Error::AlreadyExists)
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
    let system = control(&mut fixture);
    let mut buffer = request(Syscall::SystemInfo, &[system]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], 20, "twenty words, as the convention says");
    let view = Buffer::new(&buffer);
    let header = view.message().unwrap();
    assert_eq!(header.label, 0);
    assert_eq!(header.word_count, 20);
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
