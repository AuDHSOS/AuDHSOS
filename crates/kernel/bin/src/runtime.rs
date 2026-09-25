// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Processor tokens at the architecture-neutral kernel boundary.
use core::sync::atomic::Ordering;
use kernel_hal_x86_64::apic::Command;
use kernel_hal_x86_64::{
    instructions::InterruptGuard,
    processor::{IDENTIFIERS, KernelToken, with_local},
};

pub(crate) fn with_machine<R>(
    body: impl FnOnce(&mut kernel_core::machine::Machine) -> R,
) -> Option<R> {
    let _guard = InterruptGuard::new();
    #[cfg(test)]
    let token = measured::Token::new();
    #[cfg(not(test))]
    let token = KernelToken;
    let (answer, pending) = kernel_core::machine::with_machine_on(&token, |machine| {
        #[cfg(test)]
        token.acquired();
        let answer = body(machine);
        (answer, machine.scheduler.take_pending())
    })?;
    for (cpu, id) in IDENTIFIERS.iter().enumerate() {
        if pending & (1u16 << cpu) != 0 {
            let destination = u8::try_from(id.load(Ordering::Relaxed))
                .unwrap_or_else(|_| kernel_hal_x86_64::entry::fail(b"invalid online APIC\n"));
            assert_eq!(
                with_local(|local| local.send(Command::fixed(
                    destination,
                    kernel_hal_x86_64::vectors::RESCHEDULE
                ))),
                Some(true),
                "reschedule IPI failed"
            );
        }
    }
    Some(answer)
}

pub(crate) fn with_memory<R>(
    body: impl FnOnce(&mut kernel_core::memory::KernelMemory) -> R,
) -> Option<R> {
    let _guard = InterruptGuard::new();
    kernel_core::memory::with_memory_on(&KernelToken, body)
}

#[cfg(test)]
pub(crate) mod measured {
    use audhsos_sync::ExclusiveToken;
    use core::{
        cell::Cell,
        sync::atomic::{AtomicU64, Ordering},
    };
    use kernel_hal_x86_64::{
        instructions::read_tsc,
        processor::{self, KernelToken},
    };

    pub(crate) static WAIT_TICKS: [AtomicU64; processor::CPUS] =
        [const { AtomicU64::new(0) }; processor::CPUS];
    pub(crate) struct Token {
        since: Cell<Option<u64>>,
    }
    impl Token {
        pub(crate) const fn new() -> Self {
            Self {
                since: Cell::new(None),
            }
        }
        pub(crate) fn acquired(&self) {
            if let Some(start) = self.since.take() {
                let elapsed = read_tsc().wrapping_sub(start);
                processor::slot(
                    &WAIT_TICKS,
                    usize::from(processor::processor().unwrap_or(0)),
                )
                .fetch_add(elapsed, Ordering::Relaxed);
            }
        }
    }
    impl ExclusiveToken for Token {
        fn owner(&self) -> u32 {
            KernelToken.owner()
        }
        fn wait(&self) {
            if self.since.get().is_none() {
                self.since.set(Some(read_tsc()));
            }
            KernelToken.wait();
        }
    }
}
