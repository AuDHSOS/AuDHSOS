// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the process calls.

use audhsos_abi::{Error, Handle, ObjectType, Rights, Syscall, ThreadState};
use kernel_objects::config::HANDLES_PER_PROCESS;
use kernel_objects::object::{AnyObjectId, Process};

use crate::tests::double::{Call, Fixture, call, error_of, request, value_of};

/// The arguments of a `process_create` that works: eight handles, sixteen
/// frames, and four objects out of what the creator holds.
fn creation(fixture: &Fixture) -> [u64; 5] {
    [fixture.own_process.raw(), 8, 16, 4, 0]
}

/// The arguments of a `process_create` that takes as little from its
/// creator as a process can, so that a test reaches the pool before the
/// quota.
fn cheap_creation(fixture: &Fixture) -> [u64; 5] {
    [fixture.own_process.raw(), 1, 0, 0, 0]
}

/// The process a handle names.
fn process_behind(fixture: &Fixture, raw: u64) -> kernel_objects::object::ProcessId {
    let handle = Handle::from_raw(raw).expect("a handle");
    fixture
        .objects
        .entry(fixture.process, handle)
        .unwrap()
        .object
        .typed::<Process>()
        .unwrap()
}

#[test]
fn creating_a_process_gives_it_an_address_space_and_the_creator_a_handle() {
    let mut fixture = Fixture::new();
    let arguments = creation(&fixture);
    let raw = value_of(&mut fixture, request(Syscall::ProcessCreate, &arguments));
    let id = process_behind(&fixture, raw);
    let child = fixture.objects.processes.get(id).unwrap();
    assert_eq!(child.handles.capacity(), 8);
    assert_eq!(child.quota.limit(), 16);
    assert_eq!(child.kernel_object_quota.limit(), 4);
    assert_eq!(child.thread_count(), 0);
    assert!(child.regions.is_empty());
    assert_eq!(
        fixture
            .environment
            .count(&Call::CreateAddressSpace(child.root)),
        1
    );

    let entry = fixture
        .objects
        .entry(fixture.process, Handle::from_raw(raw).unwrap())
        .unwrap();
    assert_eq!(entry.object.object_type(), ObjectType::Process);
    assert!(
        entry
            .rights
            .contains(Rights::MANAGE | Rights::MAP | Rights::INSTALL)
    );
}

#[test]
fn what_the_creator_gives_away_it_no_longer_has() {
    let mut fixture = Fixture::new();
    let before = *fixture.objects.processes.get(fixture.process).unwrap();
    let arguments = creation(&fixture);
    let _ = value_of(&mut fixture, request(Syscall::ProcessCreate, &arguments));
    let after = fixture.objects.processes.get(fixture.process).unwrap();
    assert_eq!(after.quota.used(), before.quota.used() + 16);
    assert_eq!(
        after.kernel_object_quota.used(),
        before.kernel_object_quota.used() + 4 + 1,
        "the quotas of the child and the child itself"
    );
}

#[test]
fn a_process_beyond_the_frame_quota_of_its_creator_is_refused() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[2] = 1000;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(fixture.objects.processes.live(), 1);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .kernel_object_quota
            .used(),
        0,
        "the object it had already charged went back"
    );
}

#[test]
fn a_process_beyond_the_object_quota_of_its_creator_is_refused() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[3] = 1000;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .quota
            .used(),
        0,
        "the frames it had already charged went back"
    );
}

#[test]
fn a_handle_capacity_above_what_a_process_may_hold_is_refused() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[1] = u64::try_from(HANDLES_PER_PROCESS).unwrap() + 1;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)),
        Some(Error::InvalidArgument)
    );
    let mut arguments = creation(&fixture);
    arguments[1] = u64::from(u32::MAX) + 1;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn the_reserved_argument_of_process_create_must_be_zero() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[4] = 1;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn a_process_without_an_address_space_is_refused_and_costs_nothing() {
    let mut fixture = Fixture::new();
    let arguments = creation(&fixture);
    fixture.environment.no_address_space = true;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)),
        Some(Error::OutOfKernelMemory)
    );
    assert_eq!(fixture.objects.processes.live(), 1);
    let holder = fixture.objects.processes.get(fixture.process).unwrap();
    assert_eq!(holder.quota.used(), 0);
    assert_eq!(holder.kernel_object_quota.used(), 0);
}

#[test]
fn a_process_that_does_not_fit_the_pool_takes_its_address_space_apart() {
    let mut fixture = Fixture::new();
    // The pool of the fixture holds four processes and one is taken.
    for index in 0..3 {
        let arguments = cheap_creation(&fixture);
        assert!(
            error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)).is_none(),
            "process {index}"
        );
    }
    let arguments = cheap_creation(&fixture);
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)),
        Some(Error::PoolExhausted)
    );
    let made = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::CreateAddressSpace(_)))
        .count();
    let unmade = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::DestroyAddressSpace(_)))
        .count();
    assert_eq!(made, 4);
    assert_eq!(unmade, 1, "the one that did not fit");
}

#[test]
fn installing_a_handle_makes_it_a_handle_of_the_other_process() {
    let mut fixture = Fixture::new();
    let arguments = creation(&fixture);
    let child_handle = value_of(&mut fixture, request(Syscall::ProcessCreate, &arguments));
    let child = process_behind(&fixture, child_handle);

    let rights = u64::from(Rights::MANAGE.bits());
    let own = fixture.own_thread.raw();
    let installed = value_of(
        &mut fixture,
        request(Syscall::ProcessInstallHandle, &[child_handle, own, rights]),
    );
    let handle = Handle::from_raw(installed).expect("a handle");
    let entry = fixture.objects.entry(child, handle).unwrap();
    assert_eq!(entry.rights, Rights::MANAGE);
    assert_eq!(entry.object, AnyObjectId::of(fixture.thread));
    assert_eq!(
        fixture.objects.entry(fixture.process, handle),
        Err(Error::InvalidHandle),
        "the number belongs to the child, not to the caller"
    );
}

#[test]
fn installing_a_handle_that_may_not_be_transferred_is_refused() {
    let mut fixture = Fixture::new();
    let arguments = creation(&fixture);
    let child_handle = value_of(&mut fixture, request(Syscall::ProcessCreate, &arguments));
    let bound = fixture
        .install(AnyObjectId::of(fixture.thread), Rights::MANAGE)
        .raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::ProcessInstallHandle,
                &[child_handle, bound, u64::from(Rights::MANAGE.bits())]
            )
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn installing_more_rights_than_the_handle_carries_is_refused() {
    let mut fixture = Fixture::new();
    let arguments = creation(&fixture);
    let child_handle = value_of(&mut fixture, request(Syscall::ProcessCreate, &arguments));
    let all = u64::from(Rights::ALL.bits());
    let own = fixture.own_thread.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessInstallHandle, &[child_handle, own, all])
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn installing_a_handle_the_caller_does_not_hold_is_refused() {
    let mut fixture = Fixture::new();
    let arguments = creation(&fixture);
    let child_handle = value_of(&mut fixture, request(Syscall::ProcessCreate, &arguments));
    let ghost = Handle::new(29, 4).unwrap().raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessInstallHandle, &[child_handle, ghost, 0])
        ),
        Some(Error::InvalidHandle)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::ProcessInstallHandle,
                &[child_handle, 1, u64::from(Rights::MANAGE.bits())]
            )
        ),
        Some(Error::InvalidHandle),
        "a word that is no handle"
    );
}

#[test]
fn installing_beyond_the_capacity_of_the_target_is_refused() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[1] = 1;
    let child_handle = value_of(&mut fixture, request(Syscall::ProcessCreate, &arguments));
    let rights = u64::from(Rights::MANAGE.bits());
    let own = fixture.own_thread.raw();
    assert!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessInstallHandle, &[child_handle, own, rights])
        )
        .is_none()
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessInstallHandle, &[child_handle, own, rights])
        ),
        Some(Error::QuotaExceeded)
    );
}

#[test]
fn installing_without_the_install_right_is_refused() {
    let mut fixture = Fixture::new();
    let weak = fixture
        .install(AnyObjectId::of(fixture.process), Rights::MANAGE)
        .raw();
    let own = fixture.own_thread.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::ProcessInstallHandle, &[weak, own, 0])
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn killing_a_process_ends_its_threads_and_takes_its_address_space_apart() {
    let mut fixture = Fixture::new();
    let arguments = creation(&fixture);
    let child_handle = value_of(&mut fixture, request(Syscall::ProcessCreate, &arguments));
    let child = process_behind(&fixture, child_handle);
    let root = fixture.objects.processes.get(child).unwrap().root;

    // A thread of the child, made by the caller through its handle.
    let thread_handle = value_of(
        &mut fixture,
        request(
            Syscall::ThreadCreate,
            &[child_handle, 0x40_0000, 0x50_0000, 2, 4, 0],
        ),
    );
    assert_eq!(fixture.environment.stacks_out(), 1);

    let mut buffer = request(Syscall::ProcessKill, &[child_handle]);
    let (status, _, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(
        fixture.environment.count(&Call::DestroyAddressSpace(root)),
        1
    );
    assert_eq!(crate::reaper::reap(&mut fixture.machine(), None), 1);
    assert_eq!(fixture.environment.stacks_out(), 0, "its thread's stack");
    assert_eq!(fixture.environment.frames_out(), 0, "and its buffer");
    assert!(
        fixture.objects.processes.get(child).is_err(),
        "the process is gone"
    );
    // The handle the caller held to the thread of the dead process names
    // nothing any more, because the thread went with it.
    let handle = Handle::from_raw(thread_handle).unwrap();
    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    let id = entry
        .object
        .typed::<kernel_objects::object::Thread>()
        .unwrap();
    assert!(fixture.objects.threads.get(id).is_err());
}

#[test]
fn killing_a_process_asks_for_a_switch_when_it_held_the_processor() {
    let mut fixture = Fixture::new();
    // The caller kills its own process, which is running.
    let own = fixture.own_process.raw();
    let mut buffer = request(Syscall::ProcessKill, &[own]);
    let (status, _, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert!(outcome.reschedule);
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::Exited
    );
}

#[test]
fn killing_a_process_without_manage_is_refused() {
    let mut fixture = Fixture::new();
    let weak = fixture
        .install(AnyObjectId::of(fixture.process), Rights::MAP)
        .raw();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessKill, &[weak])),
        Some(Error::AccessDenied)
    );
    assert!(fixture.objects.processes.get(fixture.process).is_ok());
}

#[test]
fn a_process_whose_handle_has_nowhere_to_go_is_taken_apart_again() {
    let mut fixture = Fixture::new();
    let arguments = creation(&fixture);
    fixture.fill_handles();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ProcessCreate, &arguments)),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(fixture.objects.processes.live(), 1);
    let unmade = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::DestroyAddressSpace(_)))
        .count();
    assert_eq!(unmade, 1);
    let holder = fixture.objects.processes.get(fixture.process).unwrap();
    assert_eq!(holder.quota.used(), 0, "every quota went back");
    assert_eq!(holder.kernel_object_quota.used(), 0);
}

#[test]
fn installing_a_badged_handle_carries_the_badge_with_it() {
    // What a server knows a client by is the badge of the capability the
    // message came through, and a capability a parent installs into a child
    // has to arrive with the badge the parent put on it.
    let mut fixture = Fixture::new();
    let own = fixture.own_process.raw();
    let endpoint = value_of(&mut fixture, request(Syscall::EndpointCreate, &[]));
    let badged = value_of(
        &mut fixture,
        request(Syscall::EndpointBadge, &[endpoint, 0x5EED]),
    );
    let child = value_of(
        &mut fixture,
        request(Syscall::ProcessCreate, &[own, 4, 1, 1, 0]),
    );
    let installed = value_of(
        &mut fixture,
        request(
            Syscall::ProcessInstallHandle,
            &[child, badged, Rights::SEND.bits().into()],
        ),
    );
    let target = Handle::from_raw(child)
        .and_then(|handle| fixture.objects.entry(fixture.process, handle).ok())
        .and_then(|entry| entry.object.typed::<Process>().ok())
        .expect("a process");
    let handle = Handle::from_raw(installed).expect("a handle");
    let entry = fixture.objects.entry(target, handle).expect("the handle");
    assert_eq!(entry.badge, 0x5EED, "the badge did not travel");
    assert_eq!(entry.rights, Rights::SEND);
}
