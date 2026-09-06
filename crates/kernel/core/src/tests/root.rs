// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::root`: the one process the kernel builds itself.

use audhsos_abi::ipc_buffer::{Buffer, SIZE};
use audhsos_abi::layout::{PAGE_SIZE, ROOT_TASK_BASE, ipc_buffer_address};
use audhsos_abi::startup::{Given, Role};
use audhsos_abi::{Error, ObjectType};
use kernel_hal_api::doubles::{RecordingConsole, RecordingDevices, RecordingTlb};
use kernel_hal_api::paging::FrameBytes;
use kernel_mm::page_table::X86Entry;
use kernel_objects::store::Objects;
use kernel_syscall::environment::Environment;
use kernel_types::{PhysFrame, PhysFrameRange};

use crate::root::{Grants, PRIORITY, STACK_PAGES, STACK_TOP, build};
use crate::syscall::KernelEnvironment;

/// A machine of a handful of slots.
type Small = Objects<4, 8, 32, 256>;

/// The frame with the given number.
fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

/// A range of `count` frames from `first`.
fn range(first: u64, count: u64) -> PhysFrameRange {
    PhysFrameRange::new(frame(first), count).unwrap()
}

/// Everything a build needs, held together so that the borrows live long
/// enough.
struct Fixture {
    machine: super::memory::Machine,
    memory: crate::memory::KernelMemory,
}

impl Fixture {
    fn new() -> Self {
        let mut machine = super::memory::Machine::full();
        let mut tlb = RecordingTlb::new();
        let memory = crate::memory::bring_up::<X86Entry, _, _, _>(
            &super::memory::platform(),
            machine.root,
            &mut machine.access,
            &mut tlb,
            0,
        )
        .unwrap();
        Fixture { machine, memory }
    }
}

/// Builds a root task out of `program` over a fresh machine.
fn root_task(
    fixture: &mut Fixture,
    objects: &mut Small,
    program: &[u8],
    ram: &[PhysFrameRange],
) -> Result<crate::root::RootTask, Error> {
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut fixture.memory,
        &mut fixture.machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
    );
    let grants = Grants {
        boot_image: range(0x900, 2),
        ram,
    };
    build(&mut environment, objects, program, &grants)
}

/// The pairs of the startup message the build wrote.
fn startup(fixture: &mut Fixture, frame: PhysFrame) -> Vec<Given> {
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut fixture.memory,
        &mut fixture.machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
    );
    environment
        .with_buffer(frame, |bytes: &mut [u8; SIZE]| {
            audhsos_abi::startup::read(Buffer::new(bytes))
                .map(Iterator::collect::<Vec<Given>>)
                .unwrap_or_default()
        })
        .unwrap()
}

#[test]
fn the_root_task_begins_where_it_is_linked_and_stands_on_its_own_stack() {
    let mut fixture = Fixture::new();
    let mut objects = Small::new();
    let task = root_task(&mut fixture, &mut objects, &[0xF4; 100], &[]).unwrap();
    assert_eq!(task.entry.as_u64(), ROOT_TASK_BASE);
    assert_eq!(task.stack.as_u64(), STACK_TOP);
    assert_eq!(STACK_TOP, ROOT_TASK_BASE - PAGE_SIZE);
    assert_eq!(task.buffer.as_u64(), ipc_buffer_address(0).unwrap());
}

#[test]
fn the_thread_runs_at_the_highest_priority_and_has_not_started() {
    let mut fixture = Fixture::new();
    let mut objects = Small::new();
    let task = root_task(&mut fixture, &mut objects, &[0xF4; 100], &[]).unwrap();
    let thread = objects.threads.get(task.thread).unwrap();
    assert_eq!(thread.priority, PRIORITY);
    assert_eq!(thread.max_priority, PRIORITY);
    assert_eq!(thread.process, task.process);
    assert_eq!(thread.state, audhsos_abi::ThreadState::Inactive);
}

#[test]
fn a_program_of_no_bytes_is_no_root_task() {
    let mut fixture = Fixture::new();
    let mut objects = Small::new();
    assert_eq!(
        root_task(&mut fixture, &mut objects, &[], &[]).unwrap_err(),
        Error::InvalidArgument
    );
}

#[test]
fn a_program_of_several_pages_is_mapped_whole_and_the_bytes_are_there() {
    let mut fixture = Fixture::new();
    let mut objects = Small::new();
    let mut program = vec![0xF4u8; usize::try_from(PAGE_SIZE * 2 + 1).unwrap()];
    // A byte in each page that says which page it is.
    *program.first_mut().unwrap() = 0xA0;
    *program
        .get_mut(usize::try_from(PAGE_SIZE).unwrap())
        .unwrap() = 0xA1;
    *program
        .get_mut(usize::try_from(PAGE_SIZE * 2).unwrap())
        .unwrap() = 0xA2;
    let task = root_task(&mut fixture, &mut objects, &program, &[]).unwrap();

    for (index, marker) in [0xA0u8, 0xA1, 0xA2].into_iter().enumerate() {
        let offset = u64::try_from(index).unwrap().wrapping_mul(PAGE_SIZE);
        let page = page_at(ROOT_TASK_BASE.wrapping_add(offset));
        let (frame, permissions) = translate(&mut fixture, task.address_space, page).unwrap();
        assert!(permissions.execute, "the program is not executable");
        assert!(!permissions.write, "the program is writable");
        let bytes = fixture.machine.access.frame_bytes(frame).unwrap();
        assert_eq!(*bytes.first().unwrap(), marker, "page {index}");
    }

    // The stack is below the program, writable, and not executable.
    let page = page_at(STACK_TOP.wrapping_sub(PAGE_SIZE));
    let (_, permissions) = translate(&mut fixture, task.address_space, page).unwrap();
    assert!(permissions.write);
    assert!(!permissions.execute);

    // The page between them is unmapped, so a stack that runs over faults.
    assert!(translate(&mut fixture, task.address_space, page_at(STACK_TOP)).is_none());

    // And the lowest page of the stack is the last one that is mapped.
    let below = STACK_TOP.wrapping_sub(STACK_PAGES.wrapping_add(1).wrapping_mul(PAGE_SIZE));
    assert!(translate(&mut fixture, task.address_space, page_at(below)).is_none());
}

/// The page that begins at `address`.
fn page_at(address: u64) -> kernel_types::Page {
    kernel_types::Page::from_start(kernel_types::VirtAddr::new(address).unwrap()).unwrap()
}

/// What `page` maps to in the address space rooted at `root`.
fn translate(
    fixture: &mut Fixture,
    root: PhysFrame,
    page: kernel_types::Page,
) -> Option<(PhysFrame, kernel_mm::page_table::Permissions)> {
    let mut tlb = RecordingTlb::new();
    let mut frames = *fixture.memory.frames();
    let mapper = kernel_mm::mapper::Mapper::<'_, X86Entry, _, _, _>::new(
        root,
        &mut fixture.machine.access,
        &mut tlb,
        &mut frames,
    );
    mapper.translate(page)
}

#[test]
fn the_root_task_is_given_itself_the_system_and_the_boot_image() {
    let mut fixture = Fixture::new();
    let mut objects = Small::new();
    let task = root_task(&mut fixture, &mut objects, &[0xF4; 100], &[]).unwrap();
    let given = startup(&mut fixture, task.buffer_frame);
    let roles: Vec<Role> = given.iter().map(|pair| pair.role).collect();
    assert_eq!(
        roles,
        vec![Role::OwnProcess, Role::SystemControl, Role::BootImage]
    );
    for pair in &given {
        let entry = objects.entry(task.process, pair.handle).unwrap();
        let kind = entry.object.object_type();
        let expected = match pair.role {
            Role::OwnProcess => ObjectType::Process,
            Role::SystemControl => ObjectType::SystemControl,
            _ => ObjectType::MemoryObject,
        };
        assert_eq!(kind, expected, "{:?}", pair.role);
        assert_eq!(entry.rights, expected.rights_mask());
    }
}

#[test]
fn every_free_region_becomes_a_memory_object_of_the_root_task() {
    let mut fixture = Fixture::new();
    let mut objects = Small::new();
    let ram = [range(0x1000, 4), range(0x2000, 8), range(0x4000, 16)];
    let task = root_task(&mut fixture, &mut objects, &[0xF4; 100], &ram).unwrap();
    let given = startup(&mut fixture, task.buffer_frame);
    let regions: Vec<u64> = given
        .iter()
        .filter(|pair| pair.role == Role::Ram)
        .map(|pair| {
            let entry = objects.entry(task.process, pair.handle).unwrap();
            let id = entry
                .object
                .typed::<kernel_objects::object::MemoryObject>()
                .unwrap();
            objects.memory.get(id).unwrap().frames.start().number()
        })
        .collect();
    assert_eq!(regions, vec![0x1000, 0x2000, 0x4000]);
}

#[test]
fn the_boot_image_object_covers_the_frames_it_was_given() {
    let mut fixture = Fixture::new();
    let mut objects = Small::new();
    let task = root_task(&mut fixture, &mut objects, &[0xF4; 100], &[]).unwrap();
    let given = startup(&mut fixture, task.buffer_frame);
    let pair = given
        .iter()
        .find(|pair| pair.role == Role::BootImage)
        .unwrap();
    let entry = objects.entry(task.process, pair.handle).unwrap();
    let id = entry
        .object
        .typed::<kernel_objects::object::MemoryObject>()
        .unwrap();
    let object = objects.memory.get(id).unwrap();
    assert_eq!(object.frames.start().number(), 0x900);
    assert_eq!(object.frames.count(), 2);
}

#[test]
fn a_machine_whose_pools_are_full_builds_no_root_task() {
    let mut fixture = Fixture::new();
    let mut objects: Objects<4, 8, 2, 256> = Objects::new();
    // Two memory slots, and the boot image alone takes one: three regions
    // do not fit.
    let ram = [range(0x1000, 4), range(0x2000, 8), range(0x4000, 16)];
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut fixture.memory,
        &mut fixture.machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
    );
    let grants = Grants {
        boot_image: range(0x900, 2),
        ram: &ram,
    };
    assert_eq!(
        build(&mut environment, &mut objects, &[0xF4; 100], &grants).unwrap_err(),
        Error::PoolExhausted
    );
}
