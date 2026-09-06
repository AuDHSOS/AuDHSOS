// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The initialization state machine: the order of the status bits, and
//! the two ways a device refuses.

use crate::device::{
    Device, DeviceRegisters, F_EVENT_IDX, F_INDIRECT_DESC, F_RING_PACKED, F_VERSION_1, Phase,
    STATUS_ACKNOWLEDGE, STATUS_DRIVER, STATUS_DRIVER_OK, STATUS_FAILED, STATUS_FEATURES_OK,
    UNSUPPORTED,
};
use crate::doubles::ScriptedRegisters;
use crate::error::DeviceError;
use crate::tests::support::live_device;

#[test]
fn the_status_bits_are_written_in_the_order_the_specification_gives() {
    let mut registers = ScriptedRegisters::new(F_VERSION_1);
    let mut device = Device::new();
    assert_eq!(device.phase(), Phase::Reset);

    device.reset(&mut registers);
    device.acknowledge(&mut registers).expect("acknowledge");
    assert_eq!(device.phase(), Phase::Acknowledged);
    device.driver(&mut registers).expect("driver");
    assert_eq!(device.phase(), Phase::Driven);
    device.negotiate(&mut registers, 0).expect("negotiate");
    assert_eq!(device.phase(), Phase::FeaturesOk);
    device.driver_ok(&mut registers).expect("driver ok");
    assert_eq!(device.phase(), Phase::Live);

    assert_eq!(
        registers.status_writes(),
        [
            0,
            STATUS_ACKNOWLEDGE,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        ]
    );
    assert!(device.is_live());
}

#[test]
fn a_step_out_of_order_leaves_the_device_where_it_was() {
    let mut registers = ScriptedRegisters::new(F_VERSION_1);
    let mut device = Device::new();
    assert_eq!(device.driver(&mut registers), Err(DeviceError::OutOfOrder));
    assert_eq!(
        device.negotiate(&mut registers, 0),
        Err(DeviceError::OutOfOrder)
    );
    assert_eq!(
        device.driver_ok(&mut registers),
        Err(DeviceError::OutOfOrder)
    );
    assert_eq!(device.phase(), Phase::Reset);
    assert!(registers.status_writes().is_empty());

    device.acknowledge(&mut registers).expect("acknowledge");
    assert_eq!(
        device.acknowledge(&mut registers),
        Err(DeviceError::OutOfOrder)
    );
    assert_eq!(device.phase(), Phase::Acknowledged);
}

#[test]
fn a_device_that_clears_features_ok_leaves_the_machine_in_failure() {
    let mut registers = ScriptedRegisters::new(F_VERSION_1);
    registers.reject_features();
    let mut device = Device::new();
    device.acknowledge(&mut registers).expect("acknowledge");
    device.driver(&mut registers).expect("driver");

    assert_eq!(
        device.negotiate(&mut registers, 0),
        Err(DeviceError::FeaturesRejected)
    );
    assert_eq!(device.phase(), Phase::Failed(DeviceError::FeaturesRejected));
    assert_eq!(registers.status() & STATUS_FAILED, STATUS_FAILED);

    assert_eq!(
        device.driver_ok(&mut registers),
        Err(DeviceError::FeaturesRejected)
    );
    assert_eq!(
        device.negotiate(&mut registers, 0),
        Err(DeviceError::FeaturesRejected)
    );
    assert_eq!(device.features(), 0);
}

#[test]
fn a_device_that_asks_for_a_reset_refuses_every_further_step() {
    let mut registers = ScriptedRegisters::new(F_VERSION_1);
    let mut device = Device::new();
    device.acknowledge(&mut registers).expect("acknowledge");
    registers.request_reset();

    assert_eq!(device.driver(&mut registers), Err(DeviceError::NeedsReset));
    assert_eq!(device.phase(), Phase::Failed(DeviceError::NeedsReset));
    assert_eq!(
        device.negotiate(&mut registers, 0),
        Err(DeviceError::NeedsReset)
    );
    assert_eq!(
        device.driver_ok(&mut registers),
        Err(DeviceError::NeedsReset)
    );
    assert_eq!(device.poll(&registers), Err(DeviceError::NeedsReset));
}

#[test]
fn a_live_device_that_asks_for_a_reset_is_noticed_by_polling() {
    let (mut device, mut registers) = live_device();
    assert_eq!(device.poll(&registers), Ok(()));
    registers.request_reset();
    assert_eq!(device.poll(&registers), Err(DeviceError::NeedsReset));
    assert!(!device.is_live());
}

#[test]
fn a_reset_is_the_way_out_of_failure() {
    let mut registers = ScriptedRegisters::new(F_VERSION_1);
    let mut device = Device::new();
    device.acknowledge(&mut registers).expect("acknowledge");
    registers.request_reset();
    assert_eq!(device.driver(&mut registers), Err(DeviceError::NeedsReset));

    device.reset(&mut registers);
    assert_eq!(device.phase(), Phase::Reset);
    assert_eq!(device.features(), 0);
    device.acknowledge(&mut registers).expect("acknowledge");
    assert_eq!(device.phase(), Phase::Acknowledged);
}

#[test]
fn negotiation_keeps_what_both_sides_offer_and_always_version_one() {
    const F_MAC: u64 = 1 << 5;
    const F_MRG_RXBUF: u64 = 1 << 15;
    let mut registers = ScriptedRegisters::new(F_VERSION_1 | F_MAC);
    let mut device = Device::new();
    device.acknowledge(&mut registers).expect("acknowledge");
    device.driver(&mut registers).expect("driver");

    let accepted = device
        .negotiate(&mut registers, F_MAC | F_MRG_RXBUF)
        .expect("negotiate");
    assert_eq!(accepted, F_VERSION_1 | F_MAC);
    assert_eq!(registers.driver_features(), F_VERSION_1 | F_MAC);
    assert_eq!(device.features(), F_VERSION_1 | F_MAC);
}

#[test]
fn a_legacy_device_is_refused_and_told_so() {
    let mut registers = ScriptedRegisters::new(0);
    let mut device = Device::new();
    device.acknowledge(&mut registers).expect("acknowledge");
    device.driver(&mut registers).expect("driver");

    assert_eq!(
        device.negotiate(&mut registers, 0),
        Err(DeviceError::Legacy)
    );
    assert_eq!(device.phase(), Phase::Failed(DeviceError::Legacy));
    assert_eq!(registers.status() & STATUS_FAILED, STATUS_FAILED);
    assert_eq!(registers.driver_features(), 0);
}

#[test]
fn the_three_features_this_crate_does_not_implement_are_refused_before_anything_is_written() {
    assert_eq!(UNSUPPORTED, F_INDIRECT_DESC | F_EVENT_IDX | F_RING_PACKED);
    for feature in [F_INDIRECT_DESC, F_EVENT_IDX, F_RING_PACKED] {
        let mut registers = ScriptedRegisters::new(F_VERSION_1 | feature);
        let mut device = Device::new();
        device.acknowledge(&mut registers).expect("acknowledge");
        device.driver(&mut registers).expect("driver");

        assert_eq!(
            device.negotiate(&mut registers, feature),
            Err(DeviceError::UnsupportedFeature(feature))
        );
        assert_eq!(device.phase(), Phase::Driven);
        assert_eq!(registers.driver_features(), 0);
    }
}

#[test]
fn a_device_offering_an_unimplemented_feature_is_driven_without_it() {
    let mut registers = ScriptedRegisters::new(F_VERSION_1 | F_EVENT_IDX | F_INDIRECT_DESC);
    let mut device = Device::new();
    device.acknowledge(&mut registers).expect("acknowledge");
    device.driver(&mut registers).expect("driver");

    let accepted = device.negotiate(&mut registers, 0).expect("negotiate");
    assert_eq!(accepted, F_VERSION_1);
    assert_eq!(accepted & UNSUPPORTED, 0);
}

#[test]
fn a_device_is_the_same_whether_it_was_made_or_defaulted() {
    assert_eq!(Device::default(), Device::new());
}

#[test]
fn every_refusal_says_what_it_is() {
    assert_eq!(
        DeviceError::OutOfOrder.to_string(),
        "the step does not follow the device state"
    );
    assert_eq!(
        DeviceError::UnsupportedFeature(F_EVENT_IDX).to_string(),
        "the features 0x20000000 are not implemented"
    );
    assert_eq!(
        DeviceError::Legacy.to_string(),
        "the device does not offer VIRTIO_F_VERSION_1"
    );
    assert_eq!(
        DeviceError::FeaturesRejected.to_string(),
        "the device rejected the feature set"
    );
    assert_eq!(
        DeviceError::NeedsReset.to_string(),
        "the device asks for a reset"
    );
}
