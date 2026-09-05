// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The runner itself.
//!
//! Invariants: every test that starts gets exactly one outcome; a run ends
//! with exactly one summary line and exactly one exit request; a run that
//! expects a panic fails when none arrives.

use core::fmt;
use core::panic::{Location, PanicInfo};

use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::exit::{ExitStatus, TestExit};

use crate::protocol;

/// Something the runner can run and name.
pub trait Testable {
    /// Runs the test; a failure panics.
    fn run(&self);

    /// The name the protocol line carries.
    fn name(&self) -> &'static str;
}

impl<F: Fn()> Testable for F {
    fn run(&self) {
        self();
    }

    fn name(&self) -> &'static str {
        core::any::type_name::<F>()
    }
}

/// The runner of one kernel test image.
#[derive(Debug)]
pub struct Harness<C: DebugConsole, E: TestExit> {
    console: C,
    exit: E,
    should_panic: bool,
    passed: u32,
    running: bool,
}

impl<C: DebugConsole, E: TestExit> Harness<C, E> {
    /// A runner whose tests must all pass.
    pub const fn new(console: C, exit: E) -> Self {
        Harness {
            console,
            exit,
            should_panic: false,
            passed: 0,
            running: false,
        }
    }

    /// A runner for an image whose one test is expected to panic.
    pub const fn expecting_panic(console: C, exit: E) -> Self {
        Harness {
            console,
            exit,
            should_panic: true,
            passed: 0,
            running: false,
        }
    }

    /// The console, for a test image that writes its own lines.
    pub const fn console(&mut self) -> &mut C {
        &mut self.console
    }

    /// The number of tests that have passed.
    pub const fn passed(&self) -> u32 {
        self.passed
    }

    /// Runs every test, writes the summary, and requests the exit. The
    /// caller halts afterwards; this function returns so that the same
    /// sequence runs in a host test.
    pub fn run(&mut self, tests: &[&dyn Testable]) {
        for test in tests {
            self.begin(test.name());
            test.run();
            self.running = false;
            protocol::passed(&mut self.console);
            self.passed = self.passed.saturating_add(1);
        }
        if self.should_panic {
            protocol::start(&mut self.console, "expected panic");
            protocol::failed(&mut self.console, format_args!("no panic"));
            protocol::summary(&mut self.console, self.passed, 1);
            self.exit.exit(ExitStatus::Failure);
            return;
        }
        protocol::summary(&mut self.console, self.passed, 0);
        self.exit.exit(ExitStatus::Success);
    }

    /// Opens the line of a test, for an image that runs its tests itself
    /// instead of handing a list to [`Harness::run`].
    pub fn begin(&mut self, name: &str) {
        protocol::start(&mut self.console, name);
        self.running = true;
    }

    /// Closes the line of a test that passed.
    pub fn end(&mut self) {
        if self.running {
            protocol::passed(&mut self.console);
            self.passed = self.passed.saturating_add(1);
            self.running = false;
        }
    }

    /// Reports that the running test failed with `message` and requests
    /// the exit. An image that expects a panic reports a pass instead.
    pub fn fail(&mut self, message: fmt::Arguments<'_>) {
        if !self.running {
            protocol::start(&mut self.console, "outside a test");
        }
        self.running = false;
        if self.should_panic {
            protocol::passed(&mut self.console);
            self.passed = self.passed.saturating_add(1);
            protocol::summary(&mut self.console, self.passed, 0);
            self.exit.exit(ExitStatus::Success);
            return;
        }
        protocol::failed(&mut self.console, message);
        protocol::summary(&mut self.console, self.passed, 1);
        self.exit.exit(ExitStatus::Failure);
    }

    /// Reports a failure that names where it happened, if the caller knows.
    pub fn fail_at(&mut self, message: fmt::Arguments<'_>, location: Option<&Location<'_>>) {
        match location {
            Some(location) => self.fail(format_args!("{message} at {location}")),
            None => self.fail(message),
        }
    }

    /// The panic handler of a test image reports the panic through
    /// [`Harness::fail_at`].
    pub fn on_panic(&mut self, info: &PanicInfo<'_>) {
        self.fail_at(format_args!("{}", info.message()), info.location());
    }
}
