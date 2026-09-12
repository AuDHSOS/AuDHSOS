// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::boot`.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::layout::PHYS_WINDOW_BASE;
use audhsos_abi::{Ecam, Framebuffer, FramebufferFormat};
use kernel_hal_api::doubles::{RecordingConsole, RecordingExit, ScriptedPlatform};
use kernel_hal_api::exit::ExitStatus;
use kernel_hal_api::platform::{MemoryRegionKind, Platform};
use kernel_types::{PhysAddr, VirtAddr};
use test_support::generators::{one_of, range, vec};
use test_support::property::check;

use crate::boot::{BootError, NAME, VERSION, abort, finish, kind_name, run, usable_kib};

const MIB: u64 = 1024 * 1024;

fn window() -> VirtAddr {
    VirtAddr::new(PHYS_WINDOW_BASE).unwrap()
}

fn machine() -> ScriptedPlatform {
    ScriptedPlatform::new(window())
        .region(
            PhysAddr::new(0).unwrap(),
            640 * 1024,
            MemoryRegionKind::Usable,
        )
        .region(
            PhysAddr::new(640 * 1024).unwrap(),
            384 * 1024,
            MemoryRegionKind::Reserved,
        )
        .region(
            PhysAddr::new(MIB).unwrap(),
            255 * MIB,
            MemoryRegionKind::Usable,
        )
}

fn boot(platform: &ScriptedPlatform) -> (Result<(), BootError>, String) {
    let mut console = RecordingConsole::new();
    let result = run(platform, &mut console);
    (result, console.text())
}

#[test]
fn the_banner_names_the_kernel_and_its_version() {
    let (result, text) = boot(&machine());
    assert_eq!(result, Ok(()));
    let first = text.lines().next().unwrap_or_default();
    assert_eq!(first, format!("{NAME} {VERSION}"));
    assert!(!VERSION.is_empty());
}

#[test]
fn every_region_is_reported_with_its_bounds_and_its_kind() {
    let (result, text) = boot(&machine());
    assert_eq!(result, Ok(()));
    assert!(text.contains("3 memory regions"), "{text}");
    assert!(text.contains("0x0-0xa0000 usable"), "{text}");
    assert!(text.contains("0xa0000-0x100000 reserved"), "{text}");
    assert!(text.contains("0x100000-0x10000000 usable"), "{text}");
    assert!(text.contains("usable 261760 KiB"), "{text}");
}

#[test]
fn the_physical_window_and_the_acpi_pointer_are_reported() {
    let (_, text) = boot(&machine());
    assert!(
        text.contains("physical window at 0xffff800000000000"),
        "{text}"
    );
    assert!(text.contains("no acpi root pointer"), "{text}");

    let with_acpi = machine().rsdp(PhysAddr::new(0xE_0000).unwrap());
    let (_, text) = boot(&with_acpi);
    assert!(text.contains("acpi root pointer at 0xe0000"), "{text}");
}

#[test]
fn a_window_the_kernel_does_not_expect_stops_the_boot() {
    let elsewhere = ScriptedPlatform::new(VirtAddr::new(PHYS_WINDOW_BASE + 0x1000).unwrap())
        .region(PhysAddr::new(MIB).unwrap(), MIB, MemoryRegionKind::Usable);
    let (result, _) = boot(&elsewhere);
    assert_eq!(
        result,
        Err(BootError::WindowBase(PHYS_WINDOW_BASE + 0x1000))
    );
}

#[test]
fn a_machine_without_usable_memory_stops_the_boot() {
    let empty = ScriptedPlatform::new(window());
    assert_eq!(boot(&empty).0, Err(BootError::NoUsableMemory));
    let reserved = ScriptedPlatform::new(window()).region(
        PhysAddr::new(MIB).unwrap(),
        MIB,
        MemoryRegionKind::Reserved,
    );
    assert_eq!(boot(&reserved).0, Err(BootError::NoUsableMemory));
}

#[test]
fn a_region_whose_end_is_not_representable_is_reported_with_its_length() {
    let odd = ScriptedPlatform::new(window())
        .region(PhysAddr::MAX, u64::MAX, MemoryRegionKind::Usable)
        .region(PhysAddr::new(MIB).unwrap(), MIB, MemoryRegionKind::Usable);
    let (result, text) = boot(&odd);
    assert_eq!(result, Ok(()));
    assert!(text.contains('+'), "the length is reported instead: {text}");
}

#[test]
fn finishing_exits_with_a_success_and_aborting_with_a_failure() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    finish(&mut console, &mut exit);
    assert_eq!(exit.status(), Some(ExitStatus::Success));
    assert!(console.text().contains("boot complete"));

    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    abort(BootError::NoUsableMemory, &mut console, &mut exit);
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
    assert!(
        console.text().contains("no usable memory"),
        "{}",
        console.text()
    );
    assert!(!format!("{}", BootError::WindowBase(1)).is_empty());
}

#[test]
fn every_region_kind_has_a_name() {
    let kinds = [
        MemoryRegionKind::Usable,
        MemoryRegionKind::Reserved,
        MemoryRegionKind::AcpiReclaimable,
        MemoryRegionKind::AcpiNvs,
        MemoryRegionKind::MmioReserved,
        MemoryRegionKind::Kernel,
        MemoryRegionKind::BootImage,
        MemoryRegionKind::PageTables,
        MemoryRegionKind::BootStack,
        MemoryRegionKind::BootInfo,
    ];
    let mut names: Vec<&str> = kinds.iter().map(|kind| kind_name(*kind)).collect();
    names.sort_unstable();
    let unique = names.len();
    names.dedup();
    assert_eq!(names.len(), unique, "every kind has its own name");
}

#[test]
fn property_the_report_never_panics_and_counts_the_usable_memory() {
    let kinds = one_of(vec![
        MemoryRegionKind::Usable,
        MemoryRegionKind::Reserved,
        MemoryRegionKind::MmioReserved,
        MemoryRegionKind::Kernel,
    ]);
    let regions = vec(
        test_support::generators::pair(range(0u64..=1 << 40), kinds),
        0..=16,
    );
    check("boot_report", &regions, |cases| {
        let mut platform = ScriptedPlatform::new(window());
        let mut expected = 0u64;
        for (start, kind) in cases {
            platform = platform.region(PhysAddr::new(*start).unwrap_or(PhysAddr::ZERO), MIB, *kind);
            if *kind == MemoryRegionKind::Usable {
                expected = expected.saturating_add(MIB >> 10);
            }
        }
        let (result, text) = boot(&platform);
        if usable_kib(platform.memory_regions()) != expected {
            return Err("the usable memory does not add up".to_owned());
        }
        match result {
            Ok(()) if expected > 0 => Ok(()),
            Err(BootError::NoUsableMemory) if expected == 0 => Ok(()),
            other => Err(format!("{other:?} for {expected} KiB: {text}")),
        }
    });
}

#[test]
fn a_machine_without_a_framebuffer_says_so() {
    let (_, text) = boot(&machine());
    assert!(text.contains("[info] framebuffer=absent"), "{text}");
}

#[test]
fn the_framebuffer_is_reported_with_its_mode_and_its_address() {
    for (format, name) in [
        (FramebufferFormat::Bgrx8888, "bgrx8888"),
        (FramebufferFormat::Rgbx8888, "rgbx8888"),
    ] {
        let platform = machine().framebuffer(Framebuffer {
            phys_start: 0x8000_0000,
            len: 0x0040_0000,
            width: 1280,
            height: 800,
            stride: 1360,
            format,
        });
        let (_, text) = boot(&platform);
        assert!(
            text.contains("[info] framebuffer=1280x800 stride=1360"),
            "{text}"
        );
        assert!(
            text.contains(&format!("format={name} at 0x80000000")),
            "{text}"
        );
    }
}

#[test]
fn a_machine_whose_firmware_published_no_window_says_so() {
    let (_, text) = boot(&machine());
    assert!(text.contains("[info] ecam=absent"), "{text}");
}

#[test]
fn the_configuration_window_is_reported_with_its_segment_and_its_buses() {
    let platform = machine().ecam(Ecam {
        base: 0xE000_0000,
        segment: 0,
        first_bus: 0,
        last_bus: 255,
    });
    let (_, text) = boot(&platform);
    assert!(
        text.contains("[info] ecam=0xe0000000 segment=0 buses=0..=255"),
        "{text}"
    );
}
