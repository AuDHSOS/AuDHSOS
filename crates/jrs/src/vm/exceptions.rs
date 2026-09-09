// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Explicit completion unwinding: finally runs on return, throw and loop exit.
//! Resource and host failures are embedding failures, not catchable exceptions.

use super::Execution;
use crate::{Error, Value, heap::Heap};

pub(super) enum Completion {
    Normal(Value),
    Return(Value),
    Throw(Error),
    Jump(usize),
}

pub(super) struct Handler {
    start: usize,
    catch: Option<usize>,
    finally: Option<usize>,
    end: usize,
    stack: usize,
    phase: Phase,
    pending: Option<Completion>,
    pub(super) iterator: Option<usize>,
}

#[derive(PartialEq, Eq)]
enum Phase {
    Try,
    Catch,
    Finally,
}

impl Handler {
    pub(super) const fn rebase(&mut self, old: usize, new: usize) {
        self.stack = self.stack.saturating_sub(old).saturating_add(new);
    }
    pub(super) fn trace_handles(&self, work: &mut alloc::vec::Vec<crate::heap::Handle>) {
        if let Some(
            Completion::Normal(value)
            | Completion::Return(value)
            | Completion::Throw(Error::Thrown { value }),
        ) = &self.pending
        {
            work.extend(value.heap_handle());
        }
    }
    pub(super) const fn new(
        start: usize,
        catch: Option<usize>,
        finally: Option<usize>,
        end: usize,
        stack: usize,
    ) -> Self {
        Self {
            start,
            catch,
            finally,
            end,
            stack,
            phase: Phase::Try,
            pending: None,
            iterator: None,
        }
    }
    pub(super) fn trace(&self, heap: &mut Heap) {
        if let Some(
            Completion::Normal(value)
            | Completion::Return(value)
            | Completion::Throw(Error::Thrown { value }),
        ) = &self.pending
        {
            heap.mark_value(value);
        }
    }
}

impl Execution<'_> {
    pub(super) fn catchable(&self, error: &Error) -> bool {
        matches!(
            error,
            Error::Thrown { .. }
                | Error::Type { .. }
                | Error::Reference { .. }
                | Error::Range { .. }
                | Error::Syntax { .. }
        ) && self
            .frames
            .iter()
            .any(|frame| !frame.handlers.is_empty() || frame.async_promise.is_some())
    }

    pub(super) fn end_try(&mut self) -> Result<usize, Error> {
        let frame = self.frames.last_mut().ok_or(Error::InvalidBytecode)?;
        let handler = frame.handlers.last_mut().ok_or(Error::InvalidBytecode)?;
        if let Some(finally) = handler.finally {
            handler.pending = Some(Completion::Normal(frame.result.clone()));
            handler.phase = Phase::Finally;
            frame.result = Value::Undefined;
            Ok(finally)
        } else {
            let end = handler.end;
            frame.handlers.pop();
            Ok(end)
        }
    }
    pub(super) fn end_finally(&mut self, pc: &mut usize) -> Result<(), Error> {
        let frame = self.frames.last_mut().ok_or(Error::InvalidBytecode)?;
        let handler = frame.handlers.pop().ok_or(Error::InvalidBytecode)?;
        let completion = handler.pending.ok_or(Error::InvalidBytecode)?;
        if let Completion::Normal(value) = completion {
            frame.result = value;
            *pc = handler.end;
            Ok(())
        } else {
            self.abrupt(completion, pc)
        }
    }

    pub(super) fn abrupt(&mut self, completion: Completion, pc: &mut usize) -> Result<(), Error> {
        self.abrupt_until(completion, pc, self.unwind_boundary)
    }

    pub(super) fn abrupt_until(
        &mut self,
        mut completion: Completion,
        pc: &mut usize,
        boundary: usize,
    ) -> Result<(), Error> {
        loop {
            if self.frames.len() <= boundary {
                return Err(match completion {
                    Completion::Throw(error) => error,
                    _ => Error::InvalidBytecode,
                });
            }
            let frame = self.frames.last_mut().ok_or(Error::InvalidBytecode)?;
            if let Some(handler) = frame.handlers.last_mut() {
                if let Completion::Jump(target) = &completion
                    && *target >= handler.start
                    && *target < handler.end
                {
                    *pc = *target;
                    return Ok(());
                }
                if let Some(id) = handler.iterator {
                    frame.handlers.pop();
                    completion = self.close_iteration_completion(id, completion)?;
                    continue;
                }
                if handler.phase == Phase::Finally {
                    frame.handlers.pop();
                    continue;
                }
                self.stack.truncate(handler.stack);
                if let Completion::Throw(error) = &completion
                    && handler.phase == Phase::Try
                    && let Some(catch) = handler.catch
                {
                    handler.phase = Phase::Catch;
                    frame.result = Value::Undefined;
                    *pc = catch;
                    let value = match error {
                        Error::Thrown { value } => value.clone(),
                        _ => self.error_object(error)?,
                    };
                    self.push(value)?;
                    return Ok(());
                }
                if let Some(finally) = handler.finally {
                    handler.phase = Phase::Finally;
                    handler.pending = Some(completion);
                    frame.result = Value::Undefined;
                    *pc = finally;
                    return Ok(());
                }
                frame.handlers.pop();
                continue;
            }
            match completion {
                Completion::Jump(target) => {
                    *pc = target;
                    return Ok(());
                }
                Completion::Normal(_) => return Err(Error::InvalidBytecode),
                Completion::Return(_) | Completion::Throw(_) => {}
            }
            if frame.code.is_none() {
                return Err(match completion {
                    Completion::Throw(error) => error,
                    _ => Error::InvalidBytecode,
                });
            }
            completion = self.constructor_completion(completion)?;
            let frame = self.frames.pop().ok_or(Error::InvalidBytecode)?;
            self.binding_slots = self.binding_slots.saturating_sub(frame.locals.len());
            self.stack.truncate(frame.base);
            *pc = self.frames.last().ok_or(Error::InvalidBytecode)?.pc;
            if let Some(promise) = frame.async_promise {
                self.push(promise.clone())?;
                return match completion {
                    Completion::Return(value) => self.resolve_promise(&promise, value),
                    Completion::Throw(error) => {
                        let reason = self.exception_value(error)?;
                        self.settle(&promise, reason, true)
                    }
                    _ => Err(Error::InvalidBytecode),
                };
            }
            if let Completion::Return(value) = completion {
                let value = if frame.constructing
                    && !matches!(value, Value::Object(_) | Value::Function(_))
                {
                    frame.this_value
                } else {
                    value
                };
                self.push(value)?;
                return Ok(());
            }
        }
    }
    fn constructor_completion(&self, completion: Completion) -> Result<Completion, Error> {
        let frame = self.frames.last().ok_or(Error::InvalidBytecode)?;
        if let Completion::Return(value) = &completion
            && frame.constructing
            && !matches!(value, Value::Object(_) | Value::Function(_))
        {
            if frame.code.as_ref().is_some_and(|code| {
                code.constructor_kind == crate::parser::ConstructorKind::DerivedClass
            }) && !matches!(value, Value::Undefined)
            {
                return Ok(Completion::Throw(Error::Type {
                    message: "derived constructor returned primitive",
                }));
            }
            return Ok(match self.frame_this(frame) {
                Ok(value) => Completion::Return(value),
                Err(error) => Completion::Throw(error),
            });
        }
        Ok(completion)
    }
}
