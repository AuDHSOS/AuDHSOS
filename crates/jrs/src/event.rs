// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Runtime-owned event state; dispatch semantics live in audhsos-event-target.
use crate::Value;
pub(crate) struct Event {
    pub(crate) state: audhsos_event_target::Event,
    pub(crate) target: Value,
    pub(crate) current: Value,
    pub(crate) timestamp: f64,
    pub(crate) detail: Option<Value>,
    pub(crate) trusted: bool,
}
#[derive(Clone, Copy)]
pub(crate) enum Field {
    Detail,
    Type,
    Target,
    Current,
    Phase,
    Bubbles,
    Cancelable,
    Composed,
    Canceled,
    CancelBubble,
    ReturnValue,
    Trusted,
    Timestamp,
}

#[derive(Clone, Copy)]
pub(crate) enum ExceptionField {
    Name,
    Message,
    Code,
}
pub(crate) struct DomException {
    pub(crate) name: alloc::rc::Rc<[u16]>,
    pub(crate) message: alloc::rc::Rc<[u16]>,
}

#[derive(Clone, Copy)]
pub(crate) enum AbortField {
    Signal,
    Aborted,
    Reason,
    Handler,
}

pub(crate) struct AbortSignal {
    pub(crate) reason: Value,
    pub(crate) dependent: bool,
    // Weak generation-checked graph edges; neither direction is an ordinary root.
    pub(crate) sources: alloc::vec::Vec<crate::heap::Handle>,
    pub(crate) dependents: alloc::vec::Vec<crate::heap::Handle>,
    pub(crate) removals: alloc::vec::Vec<(Value, u64)>,
    pub(crate) handler: Value,
    pub(crate) handler_id: Option<u64>,
}
impl AbortSignal {
    pub(crate) const fn new() -> Self {
        Self {
            reason: Value::Undefined,
            dependent: false,
            sources: alloc::vec::Vec::new(),
            dependents: alloc::vec::Vec::new(),
            removals: alloc::vec::Vec::new(),
            handler: Value::Null,
            handler_id: None,
        }
    }
    pub(crate) const fn aborted(&self) -> bool {
        !matches!(self.reason, Value::Undefined)
    }
}
