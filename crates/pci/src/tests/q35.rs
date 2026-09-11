// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The recorded configuration space of a `q35` machine with a virtio-net
//! device, read back the way the program of the archive reads it.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::address::{Address, Window};
use crate::bar::{Space, Width, probe};
use crate::capability::{ID_MSIX, find as find_capability, walk};
use crate::doubles::RecordedConfigSpace;
use crate::enumerate::{find, walk as enumerate};
use crate::header::{Kind, read};
use crate::msix;
use crate::virtio::{
    Kind as VirtioKind, NETWORK_DEVICE, VIRTIO_VENDOR, first, is_modern, structures,
};

/// The window of the machine: one segment group, bus zero.
fn window() -> Window {
    Window::new(0, 0, 0).expect("the range runs forwards")
}

#[test]
fn the_machine_reports_the_five_functions_it_has() {
    let space = RecordedConfigSpace::q35();
    let mut seen = Vec::new();
    let count = enumerate(&space, window(), |function| {
        seen.push((
            function.address.device(),
            function.address.function(),
            function.header.vendor,
            function.header.device,
        ));
    })
    .expect("the recorded space answers");
    assert_eq!(count, 5);
    assert_eq!(
        seen,
        vec![
            (0, 0, 0x8086, 0x29C0),
            (1, 0, VIRTIO_VENDOR, NETWORK_DEVICE),
            (31, 0, 0x8086, 0x2918),
            (31, 2, 0x8086, 0x2922),
            (31, 3, 0x8086, 0x2930),
        ]
    );
}

#[test]
fn the_virtio_device_is_a_modern_one_and_not_the_transitional_device() {
    let space = RecordedConfigSpace::q35();
    let found = find(&space, window(), VIRTIO_VENDOR, NETWORK_DEVICE)
        .unwrap()
        .expect("the machine carries the network device");
    assert_eq!(found.address, Address::new(0, 0, 1, 0).unwrap());
    assert_eq!(found.header.class, 0x02, "a network controller");
    assert_eq!(found.header.kind, Kind::Endpoint);
    assert!(is_modern(found.header.vendor, found.header.device, 1));
    assert_ne!(found.header.device, 0x1000);
}

#[test]
fn every_base_address_register_decodes_the_range_the_machine_reports() {
    let mut space = RecordedConfigSpace::q35();
    let address = space.address(1, 0).unwrap();
    let bars = probe(&mut space, address).expect("the probe answers");
    let memory = bars[1].expect("the device decodes memory in register one");
    assert_eq!(memory.base, 0x8004_1000);
    assert_eq!(memory.len, 0x1000);
    assert_eq!(
        memory.space,
        Space::Memory {
            width: Width::Bits32,
            prefetchable: false,
        }
    );
    let wide = bars[4].expect("the device decodes memory in registers four and five");
    assert_eq!(wide.base, 0x0000_00C0_0000_0000);
    assert_eq!(wide.len, 0x4000);
    assert_eq!(
        wide.space,
        Space::Memory {
            width: Width::Bits64,
            prefetchable: true,
        }
    );
    assert_eq!(bars[5], None, "the register the one below it consumed");
    assert_eq!(bars[0], None);
    assert_eq!(bars[2], None);
    assert_eq!(bars[3], None);
}

#[test]
fn the_sata_controller_decodes_ports_and_memory() {
    let mut space = RecordedConfigSpace::q35();
    let address = space.address(31, 2).unwrap();
    let bars = probe(&mut space, address).unwrap();
    let ports = bars[4].expect("register four is the port range");
    assert_eq!(ports.space, Space::Io);
    assert_eq!(ports.base, 0x6040);
    assert_eq!(ports.len, 0x20);
    let registers = bars[5].expect("register five is the register window");
    assert!(registers.is_memory());
    assert_eq!(registers.base, 0x8004_0000);
    assert_eq!(registers.len, 0x1000);
}

#[test]
fn the_four_structures_the_driver_needs_and_the_table_size_are_read_back() {
    let space = RecordedConfigSpace::q35();
    let address = space.address(1, 0).unwrap();
    let header = read(&space, address).unwrap().unwrap();
    let capabilities = walk(&space, address, &header).expect("the list is well formed");
    let structures = structures(&space, address, &capabilities).expect("every capability is one");

    let common = first(&structures, VirtioKind::Common).expect("the common configuration");
    assert_eq!((common.bar, common.offset, common.len), (4, 0x0000, 0x1000));
    let isr = first(&structures, VirtioKind::Isr).expect("the interrupt status");
    assert_eq!((isr.bar, isr.offset, isr.len), (4, 0x1000, 0x1000));
    let device = first(&structures, VirtioKind::Device).expect("the device configuration");
    assert_eq!((device.bar, device.offset, device.len), (4, 0x2000, 0x1000));
    let notify = first(&structures, VirtioKind::Notify).expect("the notification structure");
    assert_eq!((notify.bar, notify.offset, notify.len), (4, 0x3000, 0x1000));
    assert_eq!(notify.multiplier, Some(4));
    assert_eq!(
        first(&structures, VirtioKind::PciConfig).map(|found| found.kind),
        Some(VirtioKind::PciConfig),
        "the access structure the specification requires of every device"
    );

    let capability = find_capability(&capabilities, ID_MSIX).expect("the device raises msi-x");
    let msix = msix::read(&space, address, capability.offset).expect("the capability is one");
    assert_eq!(msix.vectors, 4);
    assert_eq!(msix.table.bar, 1);
    assert_eq!(msix.table.offset, 0x0000);
    assert_eq!(msix.pending.bar, 1);
    assert_eq!(msix.pending.offset, 0x0800);
    assert!(!msix.enabled, "the firmware left it off");
}

#[test]
fn the_host_bridge_and_the_isa_bridge_decode_nothing() {
    let mut space = RecordedConfigSpace::q35();
    for (device, function) in [(0u8, 0u8), (31, 0)] {
        let address = space.address(device, function).unwrap();
        let bars = probe(&mut space, address).unwrap();
        assert!(
            bars.iter().all(Option::is_none),
            "{device}.{function} decodes something"
        );
    }
}

#[test]
fn the_extended_configuration_space_of_this_machine_is_empty() {
    let space = RecordedConfigSpace::q35();
    let address = space.address(1, 0).unwrap();
    assert_eq!(
        crate::space::read_word(&space, address, 0x100),
        Ok(0),
        "the machine publishes no extended capability"
    );
}
