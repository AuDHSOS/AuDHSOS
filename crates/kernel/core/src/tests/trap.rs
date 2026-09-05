// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::trap`.

use kernel_hal_api::doubles::{RecordingConsole, RecordingExit};
use kernel_hal_api::exit::ExitStatus;

use crate::trap::{EXCEPTION_VECTORS, Exception, on_exception};

fn report(exception: Exception) -> String {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    on_exception(exception, &mut console, &mut exit);
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
    console.text()
}

#[test]
fn every_processor_vector_has_a_name() {
    for vector in 0..EXCEPTION_VECTORS {
        let exception = Exception {
            vector,
            ..Exception::default()
        };
        assert!(!exception.name().is_empty(), "vector {vector}");
    }
    assert_eq!(
        Exception {
            vector: 14,
            ..Exception::default()
        }
        .name(),
        "page fault"
    );
    assert_eq!(
        Exception {
            vector: 200,
            ..Exception::default()
        }
        .name(),
        "exception",
        "a vector the processor does not define"
    );
}

#[test]
fn the_vectors_that_push_an_error_code_are_the_documented_ones() {
    let with_code = [8u8, 10, 11, 12, 13, 14, 17, 21, 29, 30];
    for vector in 0..=255u8 {
        let exception = Exception {
            vector,
            ..Exception::default()
        };
        assert_eq!(
            exception.has_error_code(),
            with_code.contains(&vector),
            "vector {vector}"
        );
    }
}

#[test]
fn a_reported_exception_names_its_vector_and_its_registers() {
    let text = report(Exception {
        vector: 6,
        error_code: 0,
        ip: 0xFFFF_FFFF_8000_1234,
        sp: 0xFFFF_FFFF_8010_0000,
        cr2: 0,
    });
    assert!(text.contains("invalid opcode"), "{text}");
    assert!(text.contains("vector 6"), "{text}");
    assert!(text.contains("0xffffffff80001234"), "{text}");
    assert!(text.contains("0xffffffff80100000"), "{text}");
    assert!(!text.contains("error code"), "vector 6 pushes none");
    assert!(!text.contains("faulting address"));
}

#[test]
fn a_page_fault_reports_the_error_code_and_the_faulting_address() {
    let text = report(Exception {
        vector: 14,
        error_code: 0x7,
        ip: 0x1000,
        sp: 0x2000,
        cr2: 0xDEAD_BEEF,
    });
    assert!(text.contains("page fault"), "{text}");
    assert!(text.contains("error code 0x7"), "{text}");
    assert!(text.contains("faulting address 0xdeadbeef"), "{text}");
}

#[test]
fn a_general_protection_reports_the_error_code_but_no_address() {
    let text = report(Exception {
        vector: 13,
        error_code: 0x20,
        ip: 0x1000,
        sp: 0x2000,
        cr2: 0x9999,
    });
    assert!(text.contains("general protection"), "{text}");
    assert!(text.contains("error code 0x20"), "{text}");
    assert!(!text.contains("faulting address"), "{text}");
}

#[test]
fn reporting_a_trap_counts_it_in_the_kernel_state() {
    let before = crate::state::KERNEL
        .borrow(&audhsos_sync::UncontendedToken)
        .map_or(0, |state| state.traps);
    report(Exception {
        vector: 3,
        ..Exception::default()
    });
    let after = crate::state::KERNEL
        .borrow(&audhsos_sync::UncontendedToken)
        .map_or(0, |state| state.traps);
    assert!(after > before, "the trap was counted");
}
