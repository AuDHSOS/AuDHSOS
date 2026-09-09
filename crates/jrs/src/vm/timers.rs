// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Optional HTML-style timers: clock supplied by Host, tasks driven by Realm.
use super::Execution;
use crate::{
    Error, Value, bytecode::Builtin, heap::HostBehavior, object::Property, value::FunctionValue,
};
use alloc::{collections::BTreeMap, vec::Vec};
use audhsos_timer_queue::DeadlineQueue;

pub(super) struct Timer {
    pub(super) handler: Value,
    pub(super) arguments: Vec<Value>,
    id: i32,
    delay: u64,
    nesting: u32,
    repeat: bool,
    abort: bool,
}
pub(super) struct Timers {
    pub(super) queue: DeadlineQueue<Timer>,
    pub(super) timeout_function: Option<Value>,
    pub(super) nesting: u32,
    enabled: bool,
    active: BTreeMap<i32, u64>,
    next_id: i32,
    last_clock: u128,
    arguments: usize,
}
impl Timers {
    pub(super) const fn new(limit: usize) -> Self {
        Self {
            queue: DeadlineQueue::new(limit),
            timeout_function: None,
            nesting: 0,
            enabled: false,
            active: BTreeMap::new(),
            next_id: 0,
            last_clock: 0,
            arguments: 0,
        }
    }
}
impl Execution<'_> {
    pub(super) fn timer_clock(&mut self) -> Result<u128, Error> {
        let now = self.host.timer_now()?;
        if now < self.timers.last_clock {
            return Err(Error::Host);
        }
        self.timers.last_clock = now;
        Ok(now)
    }
    pub(super) fn install_timers(&mut self) -> Result<(), Error> {
        self.timer_clock()?;
        if self.timers.enabled {
            return Ok(());
        }
        let global = self.global_object()?;
        for (name, kind, length) in [
            ("setTimeout", Builtin::SetTimeout, 1),
            ("setInterval", Builtin::SetInterval, 1),
            ("clearTimeout", Builtin::ClearTimeout, 0),
            ("clearInterval", Builtin::ClearTimeout, 0),
        ] {
            let function = self.new_host_behavior(HostBehavior::Intrinsic(kind), name, length)?;
            self.define(
                &global,
                Value::string(name).units(),
                Property {
                    enumerable: true,
                    ..Property::data(function)
                },
            )?;
        }
        self.timers.timeout_function = Some(self.new_host_behavior(
            HostBehavior::Intrinsic(Builtin::SignalTimeout),
            "timeout",
            1,
        )?);
        self.timers.enabled = true;
        Ok(())
    }
    pub(super) fn timer_call(
        &mut self,
        kind: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if !matches!(
            kind,
            Builtin::SetTimeout
                | Builtin::SetInterval
                | Builtin::ClearTimeout
                | Builtin::SignalTimeout
        ) {
            return Ok(None);
        }
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = (|| {
            if !self.timers.enabled {
                return Err(Error::Unsupported {
                    feature: "timers not installed",
                });
            }
            if kind == Builtin::SignalTimeout {
                return self.signal_timeout(args);
            }
            let global = self.global_object()?;
            if !matches!(receiver, Value::Null | Value::Undefined) && receiver != &global {
                return Err(Error::Type {
                    message: "timer requires the global receiver",
                });
            }
            if kind == Builtin::ClearTimeout {
                let id = Value::Number(self.numeric(args.first().unwrap_or(&Value::Undefined))?)
                    .to_int32();
                if let Some(token) = self.timers.active.remove(&id)
                    && let Some(timer) = self.timers.queue.remove(token)
                {
                    self.timers.arguments =
                        self.timers.arguments.saturating_sub(timer.arguments.len());
                }
                return Ok(Value::Undefined);
            }
            let input = args.first().ok_or(Error::Type {
                message: "timer requires a handler",
            })?;
            let handler = if matches!(input, Value::Function(_)) {
                input.clone()
            } else {
                Value::String(self.string_units(input)?)
            };
            let delay = Value::Number(self.numeric(args.get(1).unwrap_or(&Value::Undefined))?)
                .to_int32()
                .max(0);
            let delay = u64::try_from(delay).unwrap_or(0);
            let arguments = args.get(2..).unwrap_or(&[]);
            self.check_timer_arguments(arguments.len())?;
            let arguments = arguments.to_vec();
            let id = self.timers.next_id.checked_add(1).ok_or(Error::Limit {
                resource: "timer IDs",
            })?;
            self.timers.next_id = id;
            self.schedule_timer(Timer {
                handler,
                arguments,
                id,
                delay,
                nesting: self.timers.nesting,
                repeat: kind == Builtin::SetInterval,
                abort: false,
            })?;
            Ok(Value::Number(f64::from(id)))
        })();
        self.native_roots.truncate(roots);
        result.map(Some)
    }
    fn signal_timeout(&mut self, args: &[Value]) -> Result<Value, Error> {
        let input = args.first().ok_or(Error::Type {
            message: "timeout requires milliseconds",
        })?;
        let number = self.numeric(input)?;
        let integer = crate::value::integer_or_infinity(number);
        if !number.is_finite() || !(0.0..=9_007_199_254_740_991.0).contains(&integer) {
            return Err(Error::Type {
                message: "timeout milliseconds out of range",
            });
        }
        let delay = milliseconds(integer);
        let signal = self.new_signal()?;
        self.native_roots.push(signal.clone());
        self.schedule_timer(Timer {
            handler: signal.clone(),
            arguments: Vec::new(),
            id: 0,
            delay,
            nesting: 0,
            repeat: false,
            abort: true,
        })?;
        Ok(signal)
    }
    fn schedule_timer(&mut self, mut timer: Timer) -> Result<(), Error> {
        if !timer.abort {
            if timer.nesting > 5 {
                timer.delay = timer.delay.max(4);
            }
            timer.nesting = timer.nesting.saturating_add(1);
        }
        let count = self.check_timer_arguments(timer.arguments.len())?;
        let deadline = self
            .timer_clock()?
            .checked_add(u128::from(timer.delay).saturating_mul(1_000_000))
            .ok_or(Error::Limit {
                resource: "timer deadline",
            })?;
        let id = timer.id;
        let token = self
            .timers
            .queue
            .insert(deadline, timer)
            .map_err(|_| Error::Limit { resource: "timers" })?;
        self.timers.arguments = count;
        if id != 0 {
            self.timers.active.insert(id, token);
        }
        Ok(())
    }
    fn check_timer_arguments(&self, additional: usize) -> Result<usize, Error> {
        self.timers
            .arguments
            .checked_add(additional)
            .filter(|n| *n <= self.limits.stack)
            .ok_or(Error::Limit {
                resource: "timer arguments",
            })
    }
    pub(super) fn run_due_timer(&mut self) -> Result<bool, Error> {
        let now = self.timer_clock()?;
        let Some((token, mut timer)) = self.timers.queue.pop_due(now) else {
            return Ok(false);
        };
        self.timers.arguments = self.timers.arguments.saturating_sub(timer.arguments.len());
        self.charge(1)?;
        self.native_roots.push(timer.handler.clone());
        self.native_roots.extend_from_slice(&timer.arguments);
        self.timers.nesting = if timer.abort { 0 } else { timer.nesting };
        let result = (|| {
            if timer.abort {
                let reason = self.construct_dom_exception(
                    &[
                        Value::string("The operation timed out"),
                        Value::string("TimeoutError"),
                    ],
                    &Value::Function(FunctionValue::native(Builtin::DomException)),
                )?;
                self.signal_abort(&timer.handler, &reason)
            } else if matches!(timer.handler, Value::Function(_)) {
                let global = self.global_object()?;
                self.call_sync(timer.handler.clone(), global, &timer.arguments)
                    .map(|_| ())
            } else {
                self.host.ensure_can_compile_strings()?;
                self.eval_global_source(&timer.handler).map(|_| ())
            }
        })();
        // Language throws do not cancel intervals; resource failures stay fatal.
        let report = match result {
            Ok(()) => Ok(()),
            Err(error) => {
                let reason = self.exception_value(error)?;
                self.retain_result(reason.clone())?;
                let report = self
                    .host
                    .report_exception(&reason)
                    .map(|()| Value::Undefined);
                self.validate_host_result(&report)?;
                report.map(|_| ())
            }
        };
        if timer.id != 0 && self.timers.active.get(&timer.id) == Some(&token) {
            self.timers.active.remove(&timer.id);
            if timer.repeat {
                timer.nesting = self.timers.nesting;
                self.schedule_timer(timer)?;
            }
        }
        report?;
        Ok(true)
    }
}
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated finite integral Web IDL value in [0,2^53-1]"
)]
const fn milliseconds(n: f64) -> u64 {
    n as u64
}
