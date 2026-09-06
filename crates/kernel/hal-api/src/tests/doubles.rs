// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::doubles`.

use crate::console::DebugConsole;
use crate::doubles::{
    CountingFrameSource, FakeInterruptController, FakeTimer, Flush, IrqEvent, MemoryFrameAccess,
    RecordingConsole, RecordingExit, RecordingTlb, ScriptedPlatform,
};
use crate::exit::{ExitStatus, TestExit};
use crate::interrupt::{InterruptController, InterruptError, InterruptLine, Vector};
use crate::paging::{FrameAccess, FrameSource, TlbControl};
use crate::platform::{MemoryRegionKind, Platform};
use crate::timer::{Timer, TimerError};
use kernel_types::{PhysAddr, PhysFrame, VirtAddr};

fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

#[test]
fn frame_access_reaches_only_inserted_frames() {
    let mut access: MemoryFrameAccess<[u64; 4]> = MemoryFrameAccess::new();
    assert!(access.is_empty());
    assert!(access.table(frame(1)).is_none());
    access.insert(frame(1), [1, 2, 3, 4]);
    assert!(access.contains(frame(1)) && access.len() == 1);
    assert_eq!(access.table(frame(1)), Some(&[1, 2, 3, 4]));
    access.table_mut(frame(1)).unwrap()[0] = 9;
    assert_eq!(access.remove(frame(1)), Some([9, 2, 3, 4]));
    assert!(access.table_mut(frame(1)).is_none());
    assert_eq!(access.remove(frame(1)), None);
}

#[test]
fn tlb_records_flushes_in_order_and_clears() {
    let mut tlb = RecordingTlb::new();
    let page = VirtAddr::new(0x1000).unwrap().page();
    tlb.flush_page(page);
    tlb.flush_all();
    assert_eq!(tlb.flushes(), &[Flush::Page(page), Flush::All]);
    tlb.clear();
    assert!(tlb.flushes().is_empty());
}

#[test]
fn frame_source_counts_up_to_its_limit_and_records() {
    let mut source = CountingFrameSource::new(frame(10), Some(2));
    assert_eq!(source.allocate_frame(), Some(frame(10)));
    assert_eq!(source.allocate_frame(), Some(frame(11)));
    assert_eq!(source.allocate_frame(), None);
    assert_eq!(source.outstanding(), 2);
    source.release_frame(frame(10));
    assert_eq!(source.released(), &[frame(10)]);
    assert_eq!(source.allocated(), &[frame(10), frame(11)]);
    assert_eq!(source.outstanding(), 1);
    let mut unlimited = CountingFrameSource::new(frame(0), None);
    assert_eq!(unlimited.allocate_frame(), Some(frame(0)));
    let mut at_top = CountingFrameSource::new(frame(kernel_types::phys::MAX_FRAME_NUMBER), None);
    assert!(at_top.allocate_frame().is_some());
    assert_eq!(at_top.allocate_frame(), None);
}

#[test]
fn console_collects_bytes_and_text() {
    let mut console = RecordingConsole::new();
    console.write_bytes(b"hel");
    console.write_bytes(b"lo");
    assert_eq!(console.output(), b"hello");
    assert_eq!(console.text(), "hello");
}

#[test]
fn exit_records_the_first_status_only() {
    let mut exit = RecordingExit::new();
    assert_eq!(exit.status(), None);
    exit.exit(ExitStatus::Failure);
    exit.exit(ExitStatus::Success);
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
}

#[test]
fn timer_starts_advances_and_rejects_scripted_frequencies() {
    let mut timer = FakeTimer::new(vec![7]);
    assert_eq!(timer.frequency(), None);
    assert_eq!(
        timer.start_periodic(7),
        Err(TimerError::UnsupportedFrequency(7))
    );
    assert_eq!(timer.start_periodic(1000), Ok(()));
    assert_eq!(timer.frequency(), Some(1000));
    assert_eq!(timer.ticks(), 0);
    timer.advance(5);
    assert_eq!(timer.ticks(), 5);
    timer.advance(u64::MAX);
    assert_eq!(timer.ticks(), 4);
}

#[test]
fn interrupt_controller_models_routing_and_masking() {
    let mut controller = FakeInterruptController::new(4);
    let line = InterruptLine::new(1);
    let vector = Vector::new(40).unwrap();
    assert_eq!(
        controller.route(InterruptLine::new(4), vector),
        Err(InterruptError::UnknownLine(4))
    );
    assert_eq!(controller.route(line, vector), Ok(()));
    assert_eq!(
        controller.route(line, vector),
        Err(InterruptError::AlreadyRouted(1))
    );
    assert!(controller.is_masked(line));
    controller.unmask(line);
    assert!(!controller.is_masked(line));
    controller.mask(line);
    assert!(controller.is_masked(line));
    controller.end_of_interrupt(vector);
    assert_eq!(controller.route_of(line), Some(vector));
    assert!(!controller.is_masked(InterruptLine::new(2)));
    controller.mask(InterruptLine::new(2));
    controller.unmask(InterruptLine::new(2));
    assert!(!controller.is_masked(InterruptLine::new(2)));
    assert_eq!(controller.route_of(InterruptLine::new(2)), None);
    assert_eq!(
        controller.events(),
        &[
            IrqEvent::Route(line, vector),
            IrqEvent::Unmask(line),
            IrqEvent::Mask(line),
            IrqEvent::EndOfInterrupt(vector),
            IrqEvent::Mask(InterruptLine::new(2)),
            IrqEvent::Unmask(InterruptLine::new(2)),
        ]
    );
}

#[test]
fn platform_returns_what_was_scripted() {
    let base = VirtAddr::new(0xFFFF_8000_0000_0000).unwrap();
    let start = PhysAddr::new(0x1000).unwrap();
    let platform = ScriptedPlatform::new(base).region(start, 0x2000, MemoryRegionKind::Usable);
    assert_eq!(platform.memory_regions().len(), 1);
    assert_eq!(platform.memory_regions()[0].kind, MemoryRegionKind::Usable);
    assert_eq!(platform.physical_window_base(), base);
    assert_eq!(platform.acpi_rsdp(), None);
    let with_rsdp = platform.rsdp(PhysAddr::new(0xE_0000).unwrap());
    assert_eq!(with_rsdp.acpi_rsdp(), PhysAddr::new(0xE_0000).ok());
}

#[cfg(feature = "port-io")]
#[test]
fn ports_replay_scripted_reads_and_record_writes() {
    use crate::doubles::{PortEvent, RecordingPorts};
    use crate::port::PortAccess;
    let mut ports = RecordingPorts::new();
    ports.script_read(0x3F8, 0x41);
    ports.script_read(0x3F8, 0x1_0042);
    assert_eq!(ports.read_u8(0x3F8), 0x41);
    assert_eq!(ports.read_u16(0x3F8), 0x0042);
    assert_eq!(ports.read_u32(0x3F8), 0);
    ports.write_u8(0x3F9, 1);
    ports.write_u16(0x3F9, 2);
    ports.write_u32(0x3FA, 3);
    assert_eq!(ports.writes_to(0x3F9), vec![1, 2]);
    assert_eq!(
        ports.events()[0],
        PortEvent::Read {
            port: 0x3F8,
            width: 1,
            value: 0x41
        }
    );
    assert_eq!(
        ports.events()[5],
        PortEvent::Write {
            port: 0x3FA,
            width: 4,
            value: 3
        }
    );
}

/// Takes a console by value, so that only the blanket implementation lets
/// a reference through.
fn write_through(mut console: impl DebugConsole, bytes: &[u8]) {
    console.write_bytes(bytes);
}

/// The same for an exit device.
fn exit_through(mut exit: impl TestExit, status: ExitStatus) {
    exit.exit(status);
}

#[test]
fn a_mutable_reference_to_a_console_or_an_exit_is_one() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    write_through(&mut console, b"through the reference");
    exit_through(&mut exit, ExitStatus::Failure);
    assert_eq!(console.text(), "through the reference");
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
}

#[test]
fn the_bytes_of_a_frame_read_back_what_was_written_into_them() {
    use crate::doubles::MemoryFrameBytes;
    use crate::paging::{FRAME_BYTES, FrameBytes};

    let mut frames = MemoryFrameBytes::new();
    assert!(frames.is_empty());
    let frame = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
    assert!(frames.frame_bytes(frame).is_none());
    assert!(frames.frame_bytes_mut(frame).is_none());
    frames.add(frame);
    assert!(frames.contains(frame));
    assert_eq!(frames.len(), 1);
    assert!(!frames.is_empty());
    assert_eq!(frames.frame_bytes(frame), Some(&[0; FRAME_BYTES]));
    frames.frame_bytes_mut(frame).unwrap()[7] = 0xAB;
    assert_eq!(frames.frame_bytes(frame).unwrap().get(7), Some(&0xAB));
    frames.add(frame);
    assert_eq!(
        frames.frame_bytes(frame).unwrap().get(7),
        Some(&0xAB),
        "a frame that is already there keeps what it holds"
    );
    let other = PhysFrame::containing(PhysAddr::new(0x30_0000).unwrap());
    assert!(frames.frame_bytes(other).is_none());
    assert_eq!(MemoryFrameBytes::default().len(), 0);
}

#[test]
fn the_devices_of_one_machine_forward_to_the_controller_and_the_ports() {
    use crate::device::Devices;
    use crate::doubles::RecordingDevices;

    fn drive(devices: &mut impl Devices) {
        let line = InterruptLine::new(3);
        let vector = Vector::new(0x43).unwrap();
        devices.route(line, vector).unwrap();
        devices.mask(line);
        devices.unmask(line);
        devices.end_of_interrupt(vector);
        devices.write_u8(0x40, 0x36);
        devices.write_u16(0x42, 0x1234);
        devices.write_u32(0x44, 0xDEAD_BEEF);
        assert_eq!(devices.read_u8(0x40), 0x11);
        assert_eq!(devices.read_u16(0x42), 0x2222);
        assert_eq!(devices.read_u32(0x44), 0x3333_3333);
    }

    let mut devices = RecordingDevices::new(8);
    devices.ports.script_read(0x40, 0x11);
    devices.ports.script_read(0x42, 0x2222);
    devices.ports.script_read(0x44, 0x3333_3333);
    drive(&mut devices);
    assert_eq!(
        devices.interrupts.route_of(InterruptLine::new(3)),
        Some(Vector::new(0x43).unwrap())
    );
    assert!(!devices.interrupts.is_masked(InterruptLine::new(3)));
    assert_eq!(devices.interrupts.events().len(), 4);
    assert_eq!(devices.ports.writes_to(0x40), vec![0x36]);
    assert_eq!(devices.ports.writes_to(0x42), vec![0x1234]);
    assert_eq!(devices.ports.writes_to(0x44), vec![0xDEAD_BEEF]);
    assert_eq!(RecordingDevices::default().interrupts.events().len(), 0);
}
