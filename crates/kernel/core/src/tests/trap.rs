// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::trap`.

use kernel_hal_api::doubles::{RecordingConsole, RecordingExit};
use kernel_hal_api::exit::ExitStatus;

use crate::state::KernelState;
use crate::trap::{EXCEPTION_VECTORS, Exception, Response, on_exception, on_user_fault};

fn report(exception: Exception) -> String {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    let mut state = KernelState::new();
    on_exception(exception, &mut state, &mut console, &mut exit);
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
    assert_eq!(state.traps, 1);
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
        user: false,
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
        user: false,
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
        user: false,
    });
    assert!(text.contains("general protection"), "{text}");
    assert!(text.contains("error code 0x20"), "{text}");
    assert!(!text.contains("faulting address"), "{text}");
}

#[test]
fn every_reported_trap_is_counted() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    let mut state = KernelState::new();
    for vector in [3u8, 6, 13] {
        on_exception(
            Exception {
                vector,
                ..Exception::default()
            },
            &mut state,
            &mut console,
            &mut exit,
        );
    }
    assert_eq!(state.traps, 3);
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
    assert_eq!(KernelState::new().traps, 0);
    assert_eq!(KernelState::default(), KernelState::new());
}

#[test]
fn an_exception_of_the_kernel_stops_the_machine_and_one_of_a_thread_stops_the_thread() {
    let kernel = Exception {
        vector: 14,
        user: false,
        ..Exception::default()
    };
    assert_eq!(kernel.response(), Response::StopMachine);
    let thread = Exception {
        vector: 14,
        user: true,
        ..Exception::default()
    };
    assert_eq!(thread.response(), Response::StopThread);
    assert_eq!(
        Exception::default().response(),
        Response::StopMachine,
        "an exception that says nothing came from the kernel"
    );
}

#[test]
fn a_user_fault_is_reported_in_full_and_the_machine_runs_on() {
    let mut console = RecordingConsole::new();
    let mut state = KernelState::new();
    on_user_fault(
        Exception {
            vector: 14,
            error_code: 0x5,
            ip: 0x40_0000,
            sp: 0x7F_FFF0,
            cr2: 0xFFFF_FFFF_8000_0000,
            user: true,
        },
        &mut state,
        &mut console,
    );
    let text = console.text();
    assert!(text.contains("a user thread faulted"), "{text}");
    assert!(text.contains("page fault"), "{text}");
    assert!(text.contains("error code 0x5"), "{text}");
    assert!(
        text.contains("faulting address 0xffffffff80000000"),
        "{text}"
    );
    assert_eq!(state.traps, 1, "a user fault is a trap like any other");
}

#[test]
fn a_user_fault_writes_the_same_report_a_kernel_exception_does() {
    let exception = Exception {
        vector: 13,
        error_code: 0x20,
        ip: 0x40_1000,
        sp: 0x7F_FF00,
        cr2: 0,
        user: true,
    };
    let mut console = RecordingConsole::new();
    let mut state = KernelState::new();
    on_user_fault(exception, &mut state, &mut console);
    let user = console.text();

    let fatal = report(Exception {
        user: false,
        ..exception
    });

    assert!(user.ends_with(&fatal), "{user} against {fatal}");
}

#[test]
fn the_six_kinds_the_interface_names_come_out_of_their_vectors() {
    use audhsos_abi::FaultKind;

    let wanted = [
        (0, FaultKind::DivideError),
        (3, FaultKind::Breakpoint),
        (6, FaultKind::InvalidOpcode),
        (13, FaultKind::GeneralProtection),
        (14, FaultKind::PageFault),
        (17, FaultKind::AlignmentCheck),
    ];
    for (vector, kind) in wanted {
        let exception = Exception {
            vector,
            error_code: 0b101,
            ip: 0x40_1000,
            sp: 0x7F_F000,
            cr2: 0xDEAD_0000,
            user: true,
        };
        let fault = exception.fault().expect("a kind of its own");
        assert_eq!(fault.kind, kind, "vector {vector}");
        assert_eq!(fault.instruction_pointer, 0x40_1000);
        assert_eq!(fault.error_code, 0b101);
        let address = if vector == 14 { 0xDEAD_0000 } else { 0x40_1000 };
        assert_eq!(fault.address, address, "vector {vector}");
        assert_eq!(exception.kind(), Some(kind));
    }
}

#[test]
fn every_other_vector_stops_a_thread_without_a_message() {
    for vector in 0..64_u8 {
        let exception = Exception {
            vector,
            ..Exception::default()
        };
        let named = matches!(vector, 0 | 3 | 6 | 13 | 14 | 17);
        assert_eq!(exception.fault().is_some(), named, "vector {vector}");
        assert_eq!(exception.kind().is_some(), named, "vector {vector}");
    }
}
