// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects,
    reason = "Register VM dispatch loop, arithmetic fast-paths and jumps"
)]

//! Register VM Interpreter and Contiguous Call Frame Stack.
//!
//! Executes register-based bytecode using a flat, contiguous execution stack
//! without heap allocations per function call. Implements fast paths for
//! Smi arithmetic and inline cache property access.

use super::{
    bytecode::{BinaryOp, BytecodeFunction, Instruction, Reg, VerificationError},
    context::ContextRef,
    feedback::{BinaryOpFeedback, FeedbackVector, NamedAccessCase},
    heap::{GenerationalHeap, HeapError},
    object::ObjectKind,
    realm::{Intrinsic, Realm},
    shape::{PropertyFlags, ShapeId},
    string::StringError,
    value::{
        ObjectRef, PropertyKey, VALUE_FALSE, VALUE_NAN, VALUE_NULL, VALUE_TRUE, VALUE_UNDEFINED,
        Value,
    },
};
use alloc::vec::Vec;

/// Virtual machine execution errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VMError {
    /// Bytecode failed structural verification before execution.
    InvalidBytecode(VerificationError),
    /// The supplied feedback vector does not match the compiled function.
    InvalidFeedbackVector,
    /// Fuel exhausted.
    OutOfFuel,
    /// Invalid register access.
    InvalidRegister,
    /// Active bytecode calls exceed the configured frame limit.
    CallStackOverflow,
    /// Active parameter/local bindings exceed the configured binding limit.
    BindingStackOverflow,
    /// Operand stack limit exceeded.
    StackOverflow,
    /// A string result exceeds the configured UTF-16 code-unit limit.
    StringLimit,
    /// An object exceeds the configured own-property limit.
    PropertyLimit,
    /// An exception left the outermost frame without a handler (14.15).
    ///
    /// An error the engine itself raised also names its type and message, so
    /// that the embedding sees the error the specification names and not only
    /// the object.
    Thrown(Value, Option<(super::realm::NativeErrorKind, &'static str)>),
    /// Property lookup failed or target is not an object.
    TypeError,
    /// Instruction execution fell off bytecode bounds without Return.
    UnexpectedEnd,
    /// Generational heap invariant or reference failure.
    Heap(HeapError),
}

impl From<HeapError> for VMError {
    fn from(error: HeapError) -> Self {
        Self::Heap(error)
    }
}

impl From<StringError> for VMError {
    fn from(error: StringError) -> Self {
        Self::Heap(HeapError::String(error))
    }
}

/// Bit that marks a call-site target as a native intrinsic rather than an
/// index into the shared bytecode function table.
const NATIVE_CALL_TARGET: u32 = 1 << 31;

/// Operands of one call site.
#[derive(Clone, Copy, Debug)]
struct Call {
    /// The `this` value the callee sees.
    receiver: Value,
    /// Register holding the callable.
    func: Reg,
    /// First argument register.
    arg_start: Reg,
    /// Number of arguments passed.
    arg_count: u16,
    /// Feedback vector slot of the call site.
    slot: u16,
    /// Offset the caller resumes at.
    return_pc: usize,
    /// Bytecode unit active in the caller.
    caller_code_id: Option<u32>,
}

/// Light frame boundary recorded on the contiguous call stack.
#[derive(Clone, Copy, Debug)]
pub struct FrameHeader {
    /// Base pointer of the caller frame.
    pub caller_fp: usize,
    /// Program counter to resume in caller.
    pub return_pc: usize,
    /// Bytecode function active in the caller; `None` denotes the root unit.
    pub caller_code_id: Option<u32>,
    /// Active binding count to restore with the caller.
    pub caller_binding_count: usize,
    /// Lexical heap context to restore with the caller.
    pub caller_context: Option<ContextRef>,
}

/// Contiguous register-based virtual machine executor.
pub struct RegisterVM {
    /// Flat contiguous register stack.
    stack: Vec<Value>,
    /// Current frame pointer (start of active register window).
    fp: usize,
    /// Accumulator register.
    acc: Value,
    /// Fuel remaining for bounded execution.
    pub fuel: u64,
    /// Public operand-stack limit preserved across backend migration.
    operand_stack_limit: usize,
    /// Maximum UTF-16 code units in one materialized string value.
    string_units_limit: usize,
    /// Maximum number of own properties on one object.
    property_limit: usize,
    /// Preallocated call-frame headers; pushes never grow this allocation.
    frames: Vec<FrameHeader>,
    /// Maximum simultaneously active bytecode calls.
    call_frame_limit: usize,
    /// Active parameter/local bindings across all register frames.
    active_binding_count: usize,
    /// Maximum active parameter/local bindings.
    binding_limit: usize,
    /// Lexical heap context of the active frame.
    current_context: Option<ContextRef>,
    /// Reused precise context-root buffer for minor collection.
    context_roots: Vec<Option<ContextRef>>,
}

impl Default for RegisterVM {
    fn default() -> Self {
        Self::new(10_000_000)
    }
}

impl RegisterVM {
    /// Creates a VM with a preallocated contiguous register stack.
    #[must_use]
    pub fn new(fuel: u64) -> Self {
        Self::with_stack_capacity(fuel, 4096)
    }

    /// Creates a VM whose contiguous register stack has an explicit capacity.
    #[must_use]
    pub fn with_stack_capacity(fuel: u64, stack_capacity: usize) -> Self {
        Self::with_limits(fuel, stack_capacity, stack_capacity)
    }

    /// Creates a VM with separate physical register and source operand-stack limits.
    #[must_use]
    pub fn with_limits(fuel: u64, register_capacity: usize, operand_stack_limit: usize) -> Self {
        Self {
            stack: alloc::vec![VALUE_UNDEFINED; register_capacity],
            fp: 0,
            acc: VALUE_UNDEFINED,
            fuel,
            operand_stack_limit,
            string_units_limit: usize::MAX,
            property_limit: usize::MAX,
            frames: Vec::with_capacity(register_capacity),
            call_frame_limit: register_capacity,
            active_binding_count: 0,
            binding_limit: usize::MAX,
            current_context: None,
            context_roots: Vec::with_capacity(register_capacity.saturating_add(1)),
        }
    }

    /// Updates the maximum length of a materialized string value.
    pub(crate) const fn set_string_units_limit(&mut self, limit: usize) {
        self.string_units_limit = limit;
    }

    /// Updates the maximum number of own properties on one object.
    pub(crate) const fn set_property_limit(&mut self, limit: usize) {
        self.property_limit = limit;
    }

    /// Updates the maximum simultaneously active bytecode calls.
    pub(crate) const fn set_call_frame_limit(&mut self, limit: usize) {
        self.call_frame_limit = limit;
    }

    /// Updates the maximum active parameter/local binding count.
    pub(crate) const fn set_binding_limit(&mut self, limit: usize) {
        self.binding_limit = limit;
    }

    /// Reads a register relative to the active frame pointer.
    fn read_reg(&self, reg: Reg) -> Result<Value, VMError> {
        let idx = self.fp.saturating_add(reg.0 as usize);
        self.stack.get(idx).copied().ok_or(VMError::InvalidRegister)
    }

    /// Writes a register relative to the active frame pointer.
    fn write_reg(&mut self, reg: Reg, val: Value) -> Result<(), VMError> {
        let idx = self.fp.saturating_add(reg.0 as usize);
        if let Some(slot) = self.stack.get_mut(idx) {
            *slot = val;
            Ok(())
        } else {
            Err(VMError::InvalidRegister)
        }
    }

    fn collect_young(
        &mut self,
        code: &BytecodeFunction,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let frame_end = self
            .fp
            .checked_add(code.register_count as usize)
            .ok_or(VMError::StackOverflow)?;
        let registers = self
            .stack
            .get_mut(..frame_end)
            .ok_or(VMError::StackOverflow)?;
        self.context_roots.clear();
        self.context_roots.push(self.current_context);
        self.context_roots
            .extend(self.frames.iter().map(|frame| frame.caller_context));
        heap.scavenge_with_roots(registers, &mut self.acc, &mut self.context_roots)?;
        self.current_context = self.context_roots.first().copied().flatten();
        for (frame, context) in self.frames.iter_mut().zip(
            self.context_roots
                .get(1..)
                .unwrap_or_default()
                .iter()
                .copied(),
        ) {
            frame.caller_context = context;
        }
        Ok(())
    }

    fn allocate_object(
        &mut self,
        code: &BytecodeFunction,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        shape: ShapeId,
    ) -> Result<ObjectRef, VMError> {
        loop {
            // The prototype root is re-read after every collection, because a
            // scavenge forwards the intrinsic into the next semispace.
            let prototype = realm.object_prototype(heap)?;
            match heap.allocate_object(shape, prototype) {
                Ok(reference) => return Ok(reference),
                Err(HeapError::NurseryFull) => self.collect_young(code, heap)?,
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn allocate_array(
        &mut self,
        code: &BytecodeFunction,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        length: u32,
    ) -> Result<ObjectRef, VMError> {
        loop {
            match realm.array(heap, length) {
                Ok(reference) => return Ok(reference),
                Err(HeapError::NurseryFull) => self.collect_young(code, heap)?,
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn allocate_function(
        &mut self,
        code: &BytecodeFunction,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        code_id: u32,
        captures_context: bool,
    ) -> Result<ObjectRef, VMError> {
        loop {
            let context = if captures_context {
                Some(self.current_context.ok_or(VMError::InvalidRegister)?)
            } else {
                None
            };
            let prototype = realm.function_prototype(heap)?;
            match heap.allocate_function(code_id, context) {
                Ok(reference) => {
                    heap.set_object_prototype(reference, prototype)?;
                    return Ok(reference);
                }
                Err(HeapError::NurseryFull) => self.collect_young(code, heap)?,
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn allocate_context(
        &mut self,
        code: &BytecodeFunction,
        heap: &mut GenerationalHeap,
        parent: Option<ContextRef>,
        slot_count: u16,
    ) -> Result<ContextRef, VMError> {
        let mut parent = parent;
        loop {
            match heap.allocate_context(parent, usize::from(slot_count)) {
                Ok(context) => return Ok(context),
                Err(HeapError::NurseryFull) => {
                    self.current_context = parent;
                    self.collect_young(code, heap)?;
                    parent = self.current_context;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn allocate_string(
        &self,
        heap: &mut GenerationalHeap,
        units: &[u16],
    ) -> Result<Value, VMError> {
        if units.len() > self.string_units_limit {
            return Err(VMError::StringLimit);
        }
        let latin1 = units
            .iter()
            .copied()
            .map(u8::try_from)
            .collect::<Result<Vec<_>, _>>()
            .ok();
        if let Some(bytes) = latin1 {
            return self.allocate_latin1(heap, &bytes);
        }
        let reference = heap.strings.allocate_utf16(units.to_vec())?;
        Ok(Value::from_string(reference))
    }

    fn allocate_latin1(&self, heap: &mut GenerationalHeap, bytes: &[u8]) -> Result<Value, VMError> {
        if bytes.len() > self.string_units_limit {
            return Err(VMError::StringLimit);
        }
        if bytes.len() <= 5 {
            return Value::from_sso(bytes).ok_or(VMError::StringLimit);
        }
        Ok(Value::from_string(
            heap.strings.allocate_latin1(bytes.to_vec())?,
        ))
    }

    fn type_of(&self, heap: &mut GenerationalHeap) -> Result<Value, VMError> {
        let name: &[u8] = if self.acc.is_undefined() {
            b"undefined"
        } else if self.acc.is_null() {
            b"object"
        } else if self.acc.is_heap_string() {
            heap.strings
                .length_of(self.acc)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            b"string"
        } else if self.acc.is_sso_string() {
            let mut bytes = [0u8; 5];
            self.acc
                .as_sso_string(&mut bytes)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            b"string"
        } else if self.acc.is_symbol() {
            b"symbol"
        } else if self.acc.is_boolean() {
            b"boolean"
        } else if self.acc.is_number() {
            b"number"
        } else if self.acc.is_bigint() {
            b"bigint"
        } else if let Some(reference) = self.acc.as_object() {
            let object = heap
                .get_object(reference)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            if matches!(
                object.kind,
                ObjectKind::Function { .. } | ObjectKind::NativeFunction { .. }
            ) {
                b"function"
            } else {
                b"object"
            }
        } else {
            return Err(VMError::Heap(HeapError::InvalidReference));
        };
        self.allocate_latin1(heap, name)
    }

    fn add(&mut self, rhs: Value, heap: &mut GenerationalHeap) -> Result<(), VMError> {
        if self.acc.is_string() && rhs.is_string() {
            let length = heap
                .strings
                .length_of(self.acc)
                .and_then(|left| {
                    heap.strings
                        .length_of(rhs)
                        .and_then(|right| left.checked_add(right))
                })
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            if length > self.string_units_limit {
                return Err(VMError::StringLimit);
            }
            if length <= 5 {
                let mut units = Vec::with_capacity(length);
                for index in 0..heap.strings.length_of(self.acc).unwrap_or_default() {
                    units.push(
                        heap.strings
                            .char_code_at(self.acc, index)
                            .ok_or(VMError::Heap(HeapError::InvalidReference))?,
                    );
                }
                for index in 0..heap.strings.length_of(rhs).unwrap_or_default() {
                    units.push(
                        heap.strings
                            .char_code_at(rhs, index)
                            .ok_or(VMError::Heap(HeapError::InvalidReference))?,
                    );
                }
                self.acc = self.allocate_string(heap, &units)?;
            } else {
                self.acc = Value::from_string(heap.strings.allocate_cons(self.acc, rhs)?);
            }
            return Ok(());
        }
        if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
            if let Some(result) = a.checked_add(b) {
                self.acc = Value::from_smi(result);
            } else {
                self.acc = Value::from_f64(f64::from(a) + f64::from(b));
            }
        } else if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs)) {
            self.acc = Value::from_f64(a + b);
        } else {
            return Err(VMError::TypeError);
        }
        Ok(())
    }

    fn primitive_binary(
        &mut self,
        op: BinaryOp,
        rhs: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<BinaryOpFeedback, VMError> {
        let observed = if self.acc.as_smi().is_some() && rhs.as_smi().is_some() {
            BinaryOpFeedback::SignedSmallInteger
        } else if self.acc.is_number() && rhs.is_number() {
            BinaryOpFeedback::Number
        } else {
            BinaryOpFeedback::Generic
        };
        if op == BinaryOp::Add && (self.acc.is_string() || rhs.is_string()) {
            let left = self.primitive_string(self.acc, heap)?;
            let right = self.primitive_string(rhs, heap)?;
            self.acc = left;
            self.add(right, heap)?;
            return Ok(observed);
        }
        if matches!(
            op,
            BinaryOp::LessThan
                | BinaryOp::LessThanOrEqual
                | BinaryOp::GreaterThan
                | BinaryOp::GreaterThanOrEqual
        ) && self.acc.is_string()
            && rhs.is_string()
        {
            let left = heap
                .strings
                .to_utf16(self.acc)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let right = heap
                .strings
                .to_utf16(rhs)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            self.acc = Value::from_bool(match op {
                BinaryOp::LessThan => left < right,
                BinaryOp::LessThanOrEqual => left <= right,
                BinaryOp::GreaterThan => left > right,
                BinaryOp::GreaterThanOrEqual => left >= right,
                _ => return Err(VMError::TypeError),
            });
            return Ok(observed);
        }
        let left = primitive_number(self.acc, heap)?;
        let right = primitive_number(rhs, heap)?;
        self.acc = match op {
            BinaryOp::Add => Value::from_f64(left + right),
            BinaryOp::Sub => Value::from_f64(left - right),
            BinaryOp::Mul => Value::from_f64(left * right),
            BinaryOp::Pow => Value::from_f64(audhsos_math::pow(left, right)),
            BinaryOp::Div => Value::from_f64(left / right),
            BinaryOp::Mod => Value::from_f64(left % right),
            BinaryOp::LessThan => Value::from_bool(left < right),
            BinaryOp::LessThanOrEqual => Value::from_bool(left <= right),
            BinaryOp::GreaterThan => Value::from_bool(left > right),
            BinaryOp::GreaterThanOrEqual => Value::from_bool(left >= right),
        };
        if !matches!(
            op,
            BinaryOp::LessThan
                | BinaryOp::LessThanOrEqual
                | BinaryOp::GreaterThan
                | BinaryOp::GreaterThanOrEqual
        ) && observed == BinaryOpFeedback::SignedSmallInteger
            && let Some(integer) = exact_smi(self.acc.as_f64().unwrap_or(f64::NAN))
        {
            self.acc = Value::from_smi(integer);
        }
        Ok(observed)
    }

    fn primitive_string(
        &self,
        value: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<Value, VMError> {
        if value.is_string() {
            return Ok(value);
        }
        let text = if let Some(number) = value.as_f64() {
            crate::number::decimal_string(number)
        } else if let Some(boolean) = value.as_boolean() {
            alloc::string::String::from(if boolean { "true" } else { "false" })
        } else if value.is_null() {
            alloc::string::String::from("null")
        } else if value.is_undefined() {
            alloc::string::String::from("undefined")
        } else {
            return Err(VMError::TypeError);
        };
        self.allocate_string(heap, &text.encode_utf16().collect::<Vec<_>>())
    }

    fn strictly_equals(
        left: Value,
        right: Value,
        heap: &GenerationalHeap,
    ) -> Result<bool, VMError> {
        if left.is_string() && right.is_string() {
            return Ok(heap.strings.equals(left, right)?);
        }
        Ok(left.strictly_equals(right))
    }

    fn loosely_equals(
        mut left: Value,
        mut right: Value,
        heap: &GenerationalHeap,
    ) -> Result<bool, VMError> {
        if left.is_object() && right.is_object() {
            return Ok(left.strictly_equals(right));
        }
        if left.is_object() || right.is_object() || left.is_bigint() || right.is_bigint() {
            return Err(VMError::TypeError);
        }
        if left.is_boolean() {
            left = Value::from_f64(primitive_number(left, heap)?);
        }
        if right.is_boolean() {
            right = Value::from_f64(primitive_number(right, heap)?);
        }
        if left.is_number() && right.is_string() {
            right = Value::from_f64(primitive_number(right, heap)?);
        } else if left.is_string() && right.is_number() {
            left = Value::from_f64(primitive_number(left, heap)?);
        }
        if left.is_null() && right.is_undefined() || left.is_undefined() && right.is_null() {
            return Ok(true);
        }
        Self::strictly_equals(left, right, heap)
    }

    fn to_boolean(value: Value, heap: &GenerationalHeap) -> Result<bool, VMError> {
        if value.is_heap_string() {
            return heap
                .strings
                .length_of(value)
                .map(|length| length != 0)
                .ok_or(VMError::Heap(HeapError::InvalidReference));
        }
        Ok(value.to_boolean())
    }

    /// Enters a call: pushes a contiguous frame for a bytecode function and
    /// returns its code index, or runs a native intrinsic in place and returns
    /// `None`, leaving its value in the accumulator.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a callee that is not callable, and a
    /// resource error when a frame, a binding or fuel is exhausted.
    fn enter_call(
        &mut self,
        code: &BytecodeFunction,
        active_code: &BytecodeFunction,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        call: Call,
    ) -> Result<Option<u32>, VMError> {
        let function = self.read_reg(call.func)?;
        // 13.3.6.1: a callee that is not callable is a TypeError, not a
        // failure of execution.
        let Some(function_ref) = function.as_object() else {
            return Err(type_error(heap, realm, "value is not callable"));
        };
        let kind = heap
            .get_object(function_ref)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .kind
            .clone();
        let (code_id, context) = match kind {
            ObjectKind::Function { code_id, context } => (code_id, context),
            ObjectKind::NativeFunction { id, .. } => {
                let intrinsic = Intrinsic::from_id(id).ok_or(VMError::TypeError)?;
                // A native identifier and a bytecode index are separate
                // namespaces; the call site profile keeps them apart.
                active_feedback
                    .record_call(call.slot, NATIVE_CALL_TARGET | id)
                    .ok_or(VMError::InvalidFeedbackVector)?;
                let value = self.call_intrinsic(intrinsic, call, heap, realm)?;
                self.acc = value;
                return Ok(None);
            }
            _ => return Err(type_error(heap, realm, "value is not callable")),
        };
        let callee = code
            .functions
            .get(code_id as usize)
            .ok_or(VMError::InvalidBytecode(
                VerificationError::FunctionOutOfBounds {
                    pc: call.return_pc.saturating_sub(1),
                    index: code_id,
                },
            ))?;
        if self.frames.len() >= self.call_frame_limit || self.frames.len() == self.frames.capacity()
        {
            return Err(VMError::CallStackOverflow);
        }
        let callee_bindings = self
            .active_binding_count
            .checked_add(usize::from(callee.binding_count))
            .ok_or(VMError::BindingStackOverflow)?;
        if callee_bindings > self.binding_limit {
            return Err(VMError::BindingStackOverflow);
        }
        if callee.entry_stack_requirement > self.operand_stack_limit {
            return Err(VMError::StackOverflow);
        }
        self.fuel = self
            .fuel
            .checked_sub(callee.entry_fuel_cost)
            .ok_or(VMError::OutOfFuel)?;
        let next_frame = self.open_frame(active_code, callee, call, function_ref)?;
        active_feedback
            .record_call(call.slot, code_id)
            .ok_or(VMError::InvalidFeedbackVector)?;
        self.frames.push(FrameHeader {
            caller_fp: self.fp,
            return_pc: call.return_pc,
            caller_code_id: call.caller_code_id,
            caller_binding_count: self.active_binding_count,
            caller_context: self.current_context,
        });
        self.fp = next_frame;
        self.active_binding_count = callee_bindings;
        self.current_context = context;
        if let Some(slot_count) = callee.own_context_slot_count {
            self.current_context =
                Some(self.allocate_context(callee, heap, self.current_context, slot_count)?);
        }
        self.acc = VALUE_UNDEFINED;
        Ok(Some(code_id))
    }

    /// Clears the callee's register window and fills its parameter prefix, the
    /// callee's own Function object and its `this` value.
    fn open_frame(
        &mut self,
        active_code: &BytecodeFunction,
        callee: &BytecodeFunction,
        call: Call,
        function: ObjectRef,
    ) -> Result<usize, VMError> {
        let next_frame = self
            .fp
            .checked_add(active_code.register_count as usize)
            .ok_or(VMError::StackOverflow)?;
        let frame_end = next_frame
            .checked_add(callee.register_count as usize)
            .ok_or(VMError::StackOverflow)?;
        self.stack
            .get_mut(next_frame..frame_end)
            .ok_or(VMError::StackOverflow)?
            .fill(VALUE_UNDEFINED);
        let argument_start = self
            .fp
            .checked_add(call.arg_start.0 as usize)
            .ok_or(VMError::StackOverflow)?;
        for index in 0..usize::from(call.arg_count.min(callee.parameter_count)) {
            let argument = *self
                .stack
                .get(argument_start.saturating_add(index))
                .ok_or(VMError::InvalidRegister)?;
            *self
                .stack
                .get_mut(next_frame.saturating_add(index))
                .ok_or(VMError::StackOverflow)? = argument;
        }
        if let Some(self_register) = callee.self_register {
            *self
                .stack
                .get_mut(next_frame.saturating_add(self_register.0 as usize))
                .ok_or(VMError::StackOverflow)? = Value::from_object(function);
        }
        if let Some(this_register) = callee.this_register {
            *self
                .stack
                .get_mut(next_frame.saturating_add(this_register.0 as usize))
                .ok_or(VMError::StackOverflow)? = call.receiver;
        }
        Ok(next_frame)
    }

    /// Runs one native intrinsic and returns its value.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for the exceptions the intrinsic's algorithm
    /// specifies, and a heap error when a value cannot be materialized.
    fn call_intrinsic(
        &self,
        intrinsic: Intrinsic,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let argument = |vm: &Self, index: u16| -> Result<Value, VMError> {
            if index >= call.arg_count {
                return Ok(VALUE_UNDEFINED);
            }
            let slot = vm
                .fp
                .checked_add(call.arg_start.0 as usize)
                .and_then(|start| start.checked_add(index as usize))
                .ok_or(VMError::InvalidRegister)?;
            vm.stack.get(slot).copied().ok_or(VMError::InvalidRegister)
        };
        match intrinsic {
            // 20.1.3.2: ToPropertyKey first, then ToObject, then HasOwnProperty.
            Intrinsic::ObjectPrototypeHasOwnProperty => {
                let key = argument(self, 0)?;
                let key = property_key(key, heap)?;
                let object = Self::coerce_object(call.receiver, heap, realm)?;
                let own = heap.own_named_flags(object, key)?;
                Ok(Value::from_bool(own.is_some()))
            }
            // 20.1.3.3: a non-Object argument is false without coercing `this`.
            Intrinsic::ObjectPrototypeIsPrototypeOf => {
                let Some(mut value) = argument(self, 0)?.as_object() else {
                    return Ok(VALUE_FALSE);
                };
                let object = Self::coerce_object(call.receiver, heap, realm)?;
                loop {
                    let prototype = heap
                        .get_object(value)
                        .ok_or(VMError::Heap(HeapError::InvalidReference))?
                        .prototype;
                    let Some(prototype) = prototype.as_object() else {
                        return Ok(VALUE_FALSE);
                    };
                    if prototype == object {
                        return Ok(VALUE_TRUE);
                    }
                    value = prototype;
                }
            }
            // 20.1.3.4: an own property that is absent answers false.
            Intrinsic::ObjectPrototypePropertyIsEnumerable => {
                let key = argument(self, 0)?;
                let key = property_key(key, heap)?;
                let object = Self::coerce_object(call.receiver, heap, realm)?;
                let own = heap.own_named_flags(object, key)?;
                Ok(Value::from_bool(own.is_some_and(|flags| flags.enumerable)))
            }
            Intrinsic::ObjectPrototypeToString => self.object_to_string(call.receiver, heap, realm),
            Intrinsic::StringPrototypeCharAt
            | Intrinsic::StringPrototypeCharCodeAt
            | Intrinsic::StringPrototypeIndexOf
            | Intrinsic::StringPrototypeAt
            | Intrinsic::StringPrototypeConcat
            | Intrinsic::StringPrototypeEndsWith
            | Intrinsic::StringPrototypeIncludes
            | Intrinsic::StringPrototypeLastIndexOf
            | Intrinsic::StringPrototypeRepeat
            | Intrinsic::StringPrototypeSlice
            | Intrinsic::StringPrototypeStartsWith
            | Intrinsic::StringPrototypeSubstring
            | Intrinsic::StringPrototypeCodePointAt
            | Intrinsic::StringPrototypePadEnd
            | Intrinsic::StringPrototypePadStart
            | Intrinsic::StringPrototypeTrim
            | Intrinsic::StringPrototypeTrimEnd
            | Intrinsic::StringPrototypeTrimStart => {
                self.call_string_intrinsic(intrinsic, call, heap, realm)
            }
            Intrinsic::ArrayPrototypeValues | Intrinsic::ArrayIteratorPrototypeNext => {
                Self::call_iterator_intrinsic(intrinsic, call, heap, realm)
            }
            Intrinsic::ArrayPrototypeAt
            | Intrinsic::ArrayPrototypeIncludes
            | Intrinsic::ArrayPrototypeIndexOf
            | Intrinsic::ArrayPrototypeLastIndexOf
            | Intrinsic::ArrayPrototypeJoin
            | Intrinsic::ArrayPrototypePop
            | Intrinsic::ArrayPrototypePush => {
                self.call_array_intrinsic(intrinsic, call, heap, realm)
            }
        }
    }

    /// Runs one of the `%Array.prototype%` search methods of 23.1.3.
    ///
    /// Each one reads `length` first and then the indices below it, so a scan
    /// bounded by the index space the Elements store addresses answers the
    /// same: every index above it is absent, which `indexOf` and `lastIndexOf`
    /// skip and `includes` reads as undefined, as it reads the absent index
    /// the bounded scan already visits.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a receiver that is not an Object, and a
    /// heap error when an index name cannot be interned.
    #[expect(
        clippy::too_many_lines,
        reason = "one function keeps each method beside the clause it implements"
    )]
    fn call_array_intrinsic(
        &self,
        intrinsic: Intrinsic,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let argument = |vm: &Self, index: u16| -> Result<Value, VMError> {
            if index >= call.arg_count {
                return Ok(VALUE_UNDEFINED);
            }
            let slot = vm
                .fp
                .checked_add(call.arg_start.0 as usize)
                .and_then(|start| start.checked_add(index as usize))
                .ok_or(VMError::InvalidRegister)?;
            vm.stack.get(slot).copied().ok_or(VMError::InvalidRegister)
        };
        let object = Self::coerce_object(call.receiver, heap, realm)?;
        let length = Self::array_like_length(heap, object)?;
        let search = argument(self, 0)?;
        match intrinsic {
            // 23.1.3.1: an index outside the Array is undefined.
            Intrinsic::ArrayPrototypeAt => {
                let index = absolute_index(integer_argument(search, heap)?, length);
                let index = u32::try_from(index).ok().filter(|_| index < length);
                match index {
                    Some(index) => {
                        Ok(Self::element_at(heap, object, index)?.unwrap_or(VALUE_UNDEFINED))
                    }
                    None => Ok(VALUE_UNDEFINED),
                }
            }
            // 23.1.3.16: SameValueZero, and a missing index reads as undefined.
            Intrinsic::ArrayPrototypeIncludes => {
                let from = integer_argument(argument(self, 1)?, heap)?;
                let start = absolute_index(from, length).clamp(0, length);
                for index in Self::scan_range(start, length) {
                    let element = Self::element_at(heap, object, index)?.unwrap_or(VALUE_UNDEFINED);
                    if same_value_zero(search, element, heap)? {
                        return Ok(VALUE_TRUE);
                    }
                }
                Ok(VALUE_FALSE)
            }
            // 23.1.3.17: IsStrictlyEqual, and a missing index is skipped.
            Intrinsic::ArrayPrototypeIndexOf => {
                let from = integer_argument(argument(self, 1)?, heap)?;
                let start = absolute_index(from, length).clamp(0, length);
                for index in Self::scan_range(start, length) {
                    let Some(element) = Self::element_at(heap, object, index)? else {
                        continue;
                    };
                    if Self::strictly_equals(search, element, heap)? {
                        return Ok(index_value(i64::from(index)));
                    }
                }
                Ok(Value::from_smi(-1))
            }
            // 23.1.3.18: undefined and null contribute the empty String, and
            // the separator defaults to a comma.
            Intrinsic::ArrayPrototypeJoin => {
                let separator = if search.is_undefined() {
                    alloc::vec![0x2C]
                } else {
                    property_name_units(search, heap)?
                };
                let scanned = length.min(i64::from(u32::MAX));
                let mut units: Vec<u16> = Vec::new();
                for index in Self::scan_range(0, scanned) {
                    if index > 0 {
                        units.extend_from_slice(&separator);
                    }
                    let element = Self::element_at(heap, object, index)?.unwrap_or(VALUE_UNDEFINED);
                    if !element.is_undefined() && !element.is_null() {
                        units.extend(property_name_units(element, heap)?);
                    }
                    if units.len() > self.string_units_limit {
                        return Err(VMError::StringLimit);
                    }
                }
                // Every index above that space is absent, so what remains is
                // separators alone, which no string limit admits.
                if length > scanned && !separator.is_empty() {
                    return Err(VMError::StringLimit);
                }
                self.allocate_string(heap, &units)
            }
            // 23.1.3.23: each argument is written at the length reached so
            // far, and the new length is the answer.
            Intrinsic::ArrayPrototypePush => {
                let mut next = length;
                for offset in 0..call.arg_count {
                    let index = u32::try_from(next).map_err(|_| VMError::PropertyLimit)?;
                    heap.set_array_element(object, index, argument(self, offset)?)?;
                    next = next.saturating_add(1);
                }
                Ok(index_value(next))
            }
            // 23.1.3.22: the last element leaves the Array, which is then one
            // shorter; an empty Array only has its length set again.
            Intrinsic::ArrayPrototypePop => {
                let Ok(last) = u32::try_from(length.saturating_sub(1)) else {
                    heap.set_array_length(object, 0)?;
                    return Ok(VALUE_UNDEFINED);
                };
                let element = Self::element_at(heap, object, last)?.unwrap_or(VALUE_UNDEFINED);
                if let Some(elements) = heap.get_object(object).ok_or(VMError::TypeError)?.elements
                {
                    heap.delete_element(elements, last)?;
                }
                heap.set_array_length(object, last)?;
                Ok(element)
            }
            // 23.1.3.20: the same in descending order, from the last index
            // when no second argument is present.
            _ => {
                let from = if call.arg_count > 1 {
                    absolute_index(integer_argument(argument(self, 1)?, heap)?, length)
                        .min(length.saturating_sub(1))
                } else {
                    length.saturating_sub(1)
                };
                for index in Self::scan_range(0, from.saturating_add(1)).rev() {
                    let Some(element) = Self::element_at(heap, object, index)? else {
                        continue;
                    };
                    if Self::strictly_equals(search, element, heap)? {
                        return Ok(index_value(i64::from(index)));
                    }
                }
                Ok(Value::from_smi(-1))
            }
        }
    }

    /// The indices of `start..end` an Elements store can address.
    fn scan_range(start: i64, end: i64) -> core::ops::Range<u32> {
        let bound = |value: i64| u32::try_from(value.max(0)).unwrap_or(u32::MAX);
        bound(start)..bound(end)
    }

    /// `LengthOfArrayLike` of 7.3.18.
    fn array_like_length(heap: &GenerationalHeap, object: ObjectRef) -> Result<i64, VMError> {
        if let Some(length) = heap.array_length(object) {
            return Ok(i64::from(length));
        }
        let Some(name) = heap.strings.lookup_interned_units(&LENGTH_NAME) else {
            return Ok(0);
        };
        let value = heap
            .lookup_named(object, PropertyKey::String(name))?
            .map_or(VALUE_UNDEFINED, |property| property.value);
        // 7.1.20 ToLength clamps into 0..2^53-1; the scan is bounded again by
        // the index space, so the clamp loses no reachable index.
        Ok(integer_argument(value, heap)?.max(0))
    }

    /// `Get(O, ! ToString(𝔽(index)))` of 7.3.2, absent when `HasProperty` is
    /// false: the Elements store answers an index it holds, and a name on the
    /// Prototype Chain answers the rest.
    fn element_at(
        heap: &mut GenerationalHeap,
        object: ObjectRef,
        index: u32,
    ) -> Result<Option<Value>, VMError> {
        let elements = heap.get_object(object).ok_or(VMError::TypeError)?.elements;
        if let Some(elements) = elements
            && let Some(value) = heap
                .get_elements(elements)
                .ok_or(VMError::TypeError)?
                .get(index)
        {
            return Ok(Some(value));
        }
        let key = PropertyKey::String(heap.intern_index(index)?);
        Ok(heap
            .lookup_named(object, key)?
            .map(|property| property.value))
    }

    /// Runs one of the Array iterator intrinsics of 23.1.5.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a receiver that is not one, and a heap
    /// error when the iterator or its result cannot be allocated.
    fn call_iterator_intrinsic(
        intrinsic: Intrinsic,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        match intrinsic {
            // 23.1.3.38: the iterator holds the Array and the next index.
            Intrinsic::ArrayPrototypeValues => {
                let target = Self::coerce_object(call.receiver, heap, realm)?;
                let prototype = realm.array_iterator_prototype(heap)?;
                let shape = heap.shapes.root_shape();
                let iterator = heap.allocate_object(shape, prototype)?;
                heap.set_object_kind(
                    iterator,
                    ObjectKind::ArrayIterator {
                        target: Some(target),
                        index: 0,
                    },
                )?;
                Ok(Value::from_object(iterator))
            }
            // 23.1.5.2.1: the length is read again at every step, and the
            // iterator is exhausted once the index reaches it.
            Intrinsic::ArrayIteratorPrototypeNext => {
                let iterator = call.receiver.as_object().ok_or(VMError::TypeError)?;
                let ObjectKind::ArrayIterator { target, index } =
                    heap.get_object(iterator).ok_or(VMError::TypeError)?.kind
                else {
                    return Err(type_error(
                        heap,
                        realm,
                        "next called on a value that is not an Array Iterator",
                    ));
                };
                let value = target
                    .filter(|target| heap.array_length(*target).is_some_and(|len| index < len))
                    .map(|target| {
                        heap.get_object(target)
                            .and_then(|object| object.elements)
                            .and_then(|elements| heap.get_elements(elements))
                            .and_then(|elements| elements.get(index))
                            .unwrap_or(VALUE_UNDEFINED)
                    });
                let next = match value {
                    Some(_) => ObjectKind::ArrayIterator {
                        target,
                        index: index.saturating_add(1),
                    },
                    None => ObjectKind::ArrayIterator {
                        target: None,
                        index,
                    },
                };
                heap.set_object_kind(iterator, next)?;
                Self::iterator_result(value, heap, realm)
            }
            _ => Err(VMError::InvalidFeedbackVector),
        }
    }

    /// `CreateIterResultObject` of 7.4.14: an ordinary object with an own
    /// `value` and an own `done`.
    fn iterator_result(
        value: Option<Value>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let result = realm.ordinary_object(heap)?;
        let value_key = PropertyKey::String(heap.strings.intern("value")?);
        heap.define_own_named(
            result,
            value_key,
            value.unwrap_or(VALUE_UNDEFINED),
            PropertyFlags::ordinary_data(),
        )?;
        let done_key = PropertyKey::String(heap.strings.intern("done")?);
        heap.define_own_named(
            result,
            done_key,
            Value::from_bool(value.is_none()),
            PropertyFlags::ordinary_data(),
        )?;
        Ok(Value::from_object(result))
    }

    /// `Object.prototype.toString` of 20.1.3.6.
    ///
    /// # Errors
    ///
    /// Returns a heap error when the tag cannot be materialized.
    fn object_to_string(
        &self,
        receiver: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let tag = if receiver.is_undefined() {
            "Undefined"
        } else if receiver.is_null() {
            "Null"
        } else {
            let object = Self::coerce_object(receiver, heap, realm)?;
            match heap.get_object(object).ok_or(VMError::TypeError)?.kind {
                ObjectKind::Array { .. } => "Array",
                ObjectKind::Function { .. } | ObjectKind::NativeFunction { .. } => "Function",
                ObjectKind::Error => "Error",
                ObjectKind::BooleanWrapper(_) => "Boolean",
                ObjectKind::NumberWrapper(_) => "Number",
                ObjectKind::StringWrapper(_) => "String",
                // 23.1.5.2.2 tags the Array Iterator through @@toStringTag, so
                // its builtin tag is the ordinary one.
                ObjectKind::Ordinary | ObjectKind::ArrayIterator { .. } => "Object",
            }
        };
        let mut units: Vec<u16> = "[object ".encode_utf16().collect();
        match receiver
            .as_object()
            .map(|object| {
                heap.lookup_named(object, super::realm::WellKnownSymbol::ToStringTag.key())
            })
            .transpose()?
            .flatten()
            .map(|property| property.value)
            .filter(|value| value.is_string())
            .and_then(|value| heap.strings.to_utf16(value))
        {
            Some(own) => units.extend(own),
            None => units.extend(tag.encode_utf16()),
        }
        units.push(0x5D);
        self.allocate_string(heap, &units)
    }

    /// Runs one `%String.prototype%` method.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a receiver that is not a String.
    #[expect(
        clippy::too_many_lines,
        reason = "one function keeps each method beside the clause it implements"
    )]
    fn call_string_intrinsic(
        &self,
        intrinsic: Intrinsic,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let argument = |vm: &Self, index: u16| -> Result<Value, VMError> {
            if index >= call.arg_count {
                return Ok(VALUE_UNDEFINED);
            }
            let slot = vm
                .fp
                .checked_add(call.arg_start.0 as usize)
                .and_then(|start| start.checked_add(index as usize))
                .ok_or(VMError::InvalidRegister)?;
            vm.stack.get(slot).copied().ok_or(VMError::InvalidRegister)
        };
        let units = Self::receiver_units(call.receiver, heap, realm)?;
        match intrinsic {
            // 22.1.3.1: an index outside the String is the empty String.
            Intrinsic::StringPrototypeCharAt => {
                let position = integer_argument(argument(self, 0)?, heap)?;
                let unit = usize::try_from(position)
                    .ok()
                    .and_then(|position| units.get(position).copied());
                match unit {
                    Some(unit) => self.allocate_string(heap, &[unit]),
                    None => self.allocate_string(heap, &[]),
                }
            }
            // 22.1.3.2: an index outside the String is NaN.
            Intrinsic::StringPrototypeCharCodeAt => {
                let position = integer_argument(argument(self, 0)?, heap)?;
                Ok(usize::try_from(position)
                    .ok()
                    .and_then(|position| units.get(position).copied())
                    .map_or(VALUE_NAN, |unit| Value::from_smi(i32::from(unit))))
            }
            // 22.1.3.9: the search starts at the clamped position and -1 says
            // the String does not occur.
            Intrinsic::StringPrototypeIndexOf => {
                let search = property_name_units(argument(self, 0)?, heap)?;
                let start = integer_argument(argument(self, 1)?, heap)?
                    .clamp(0, i64::try_from(units.len()).unwrap_or(i64::MAX));
                let start = usize::try_from(start).unwrap_or(0);
                let found = (start..=units.len().saturating_sub(search.len()))
                    .filter(|_| search.len() <= units.len())
                    .find(|index| {
                        units.get(*index..index.saturating_add(search.len()))
                            == Some(search.as_slice())
                    });
                Ok(found.map_or(Value::from_smi(-1), |index| {
                    i32::try_from(index).map_or(VALUE_NAN, Value::from_smi)
                }))
            }
            // 22.1.3.1: a negative index counts from the end, and an index
            // outside the String is undefined.
            Intrinsic::StringPrototypeAt => {
                let relative = integer_argument(argument(self, 0)?, heap)?;
                let length = i64::try_from(units.len()).unwrap_or(i64::MAX);
                let index = if relative < 0 {
                    length.saturating_add(relative)
                } else {
                    relative
                };
                let unit = usize::try_from(index)
                    .ok()
                    .and_then(|index| units.get(index).copied());
                match unit {
                    Some(unit) => self.allocate_string(heap, &[unit]),
                    None => Ok(VALUE_UNDEFINED),
                }
            }
            // 22.1.3.5: every argument is appended in order.
            Intrinsic::StringPrototypeConcat => {
                let mut result = units;
                for index in 0..call.arg_count {
                    result.extend(property_name_units(argument(self, index)?, heap)?);
                }
                self.allocate_string(heap, &result)
            }
            // 22.1.3.7: the search ends at the clamped position.
            Intrinsic::StringPrototypeEndsWith => {
                let search = property_name_units(argument(self, 0)?, heap)?;
                let end = match argument(self, 1)? {
                    value if value.is_undefined() => units.len(),
                    value => clamped_index(integer_argument(value, heap)?, units.len()),
                };
                let start = end.checked_sub(search.len());
                Ok(Value::from_bool(start.is_some_and(|start| {
                    units.get(start..end) == Some(search.as_slice())
                })))
            }
            // 22.1.3.8: the search starts at the clamped position.
            Intrinsic::StringPrototypeIncludes => {
                let search = property_name_units(argument(self, 0)?, heap)?;
                let start = clamped_index(integer_argument(argument(self, 1)?, heap)?, units.len());
                Ok(Value::from_bool(
                    find_units(&units, &search, start).is_some(),
                ))
            }
            // 22.1.3.10: the last occurrence at or before the clamped position.
            Intrinsic::StringPrototypeLastIndexOf => {
                let search = property_name_units(argument(self, 0)?, heap)?;
                let position = argument(self, 1)?;
                let last = units.len().saturating_sub(search.len());
                let end = if position.is_undefined() || primitive_number(position, heap)?.is_nan() {
                    last
                } else {
                    clamped_index(integer_argument(position, heap)?, last)
                };
                let found = (0..=end)
                    .rev()
                    .filter(|_| search.len() <= units.len())
                    .find(|index| {
                        units.get(*index..index.saturating_add(search.len()))
                            == Some(search.as_slice())
                    });
                Ok(found.map_or(Value::from_smi(-1), |index| {
                    i32::try_from(index).map_or(VALUE_NAN, Value::from_smi)
                }))
            }
            // 22.1.3.17: a negative or infinite count is a RangeError.
            Intrinsic::StringPrototypeRepeat => {
                // 22.1.3.17 steps 3 and 4: a negative or infinite count is a
                // RangeError, before the String's own length is looked at.
                if primitive_number(argument(self, 0)?, heap)? == f64::INFINITY {
                    return Err(range_error(heap, realm, "repeat count is out of range"));
                }
                // ToIntegerOrInfinity truncates toward zero, so -0.5 is 0 and
                // only a count that is negative after that is out of range.
                let count = integer_argument(argument(self, 0)?, heap)?;
                if count < 0 {
                    return Err(range_error(heap, realm, "repeat count is out of range"));
                }
                if units.is_empty() {
                    return self.allocate_string(heap, &[]);
                }
                let total = usize::try_from(count)
                    .ok()
                    .and_then(|count| count.checked_mul(units.len()))
                    .filter(|total| *total <= self.string_units_limit)
                    .ok_or(VMError::StringLimit)?;
                let mut result = Vec::with_capacity(total);
                for _ in 0..count {
                    result.extend_from_slice(&units);
                }
                self.allocate_string(heap, &result)
            }
            // 22.1.3.22: both ends count from the end when negative, and an
            // inverted range is the empty String.
            Intrinsic::StringPrototypeSlice => {
                let start =
                    relative_index(integer_argument(argument(self, 0)?, heap)?, units.len());
                let end = match argument(self, 1)? {
                    value if value.is_undefined() => units.len(),
                    value => relative_index(integer_argument(value, heap)?, units.len()),
                };
                let slice = units.get(start..end.max(start)).unwrap_or_default();
                self.allocate_string(heap, slice)
            }
            // 22.1.3.24: the search starts at the clamped position.
            Intrinsic::StringPrototypeStartsWith => {
                let search = property_name_units(argument(self, 0)?, heap)?;
                let start = clamped_index(integer_argument(argument(self, 1)?, heap)?, units.len());
                let end = start.saturating_add(search.len());
                Ok(Value::from_bool(
                    units.get(start..end) == Some(search.as_slice()),
                ))
            }
            // 22.1.3.25: both ends are clamped and then ordered.
            Intrinsic::StringPrototypeSubstring => {
                let first = clamped_index(integer_argument(argument(self, 0)?, heap)?, units.len());
                let second = match argument(self, 1)? {
                    value if value.is_undefined() => units.len(),
                    value => clamped_index(integer_argument(value, heap)?, units.len()),
                };
                let slice = units
                    .get(first.min(second)..first.max(second))
                    .unwrap_or_default();
                self.allocate_string(heap, slice)
            }
            // 22.1.3.4: the code point at an index, which pairs a surrogate
            // with the one after it.
            Intrinsic::StringPrototypeCodePointAt => {
                let position = integer_argument(argument(self, 0)?, heap)?;
                let Ok(position) = usize::try_from(position) else {
                    return Ok(VALUE_UNDEFINED);
                };
                let Some(first) = units.get(position).copied() else {
                    return Ok(VALUE_UNDEFINED);
                };
                Ok(Value::from_smi(code_point_at(&units, position, first)))
            }
            // 22.1.3.15 and 22.1.3.16: the filler is repeated and cut to the
            // width the String is short of, and an empty filler pads nothing.
            Intrinsic::StringPrototypePadStart | Intrinsic::StringPrototypePadEnd => {
                let width = integer_argument(argument(self, 0)?, heap)?;
                let width = usize::try_from(width).unwrap_or(0);
                let filler = match argument(self, 1)? {
                    value if value.is_undefined() => alloc::vec![0x20],
                    value => property_name_units(value, heap)?,
                };
                if width <= units.len() || filler.is_empty() {
                    return self.allocate_string(heap, &units);
                }
                if width > self.string_units_limit {
                    return Err(VMError::StringLimit);
                }
                let missing = width.saturating_sub(units.len());
                let mut pad = Vec::with_capacity(missing);
                while pad.len() < missing {
                    let take = missing.saturating_sub(pad.len()).min(filler.len());
                    pad.extend_from_slice(filler.get(..take).unwrap_or_default());
                }
                let mut result = Vec::with_capacity(width);
                if intrinsic == Intrinsic::StringPrototypePadStart {
                    result.extend_from_slice(&pad);
                    result.extend_from_slice(&units);
                } else {
                    result.extend_from_slice(&units);
                    result.extend_from_slice(&pad);
                }
                self.allocate_string(heap, &result)
            }
            // 22.1.3.32 to 22.1.3.34: TrimString removes the white space and
            // line terminators of 11.2 and 11.3 from the named ends.
            Intrinsic::StringPrototypeTrim
            | Intrinsic::StringPrototypeTrimStart
            | Intrinsic::StringPrototypeTrimEnd => {
                let start = if intrinsic == Intrinsic::StringPrototypeTrimEnd {
                    0
                } else {
                    units.iter().take_while(|unit| trimmable(**unit)).count()
                };
                let end = if intrinsic == Intrinsic::StringPrototypeTrimStart {
                    units.len()
                } else {
                    units.len()
                        - units
                            .iter()
                            .rev()
                            .take_while(|unit| trimmable(**unit))
                            .count()
                };
                let trimmed = units.get(start..end.max(start)).unwrap_or_default();
                self.allocate_string(heap, trimmed)
            }
            _ => Err(VMError::InvalidFeedbackVector),
        }
    }

    /// The code units of a String receiver.
    ///
    /// Every `%String.prototype%` method begins with `RequireObjectCoercible`
    /// and `ToString` of the `this` value (22.1.3). The lowering only emits a
    /// call on a String, so this refuses anything else rather than guessing.
    fn receiver_units(
        receiver: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Vec<u16>, VMError> {
        if !receiver.is_string() {
            return Err(type_error(
                heap,
                realm,
                "String method called on a value that is not a String",
            ));
        }
        heap.strings
            .to_utf16(receiver)
            .ok_or(VMError::Heap(HeapError::InvalidReference))
    }

    /// The value a String answers for one property name.
    ///
    /// 10.4.3 describes the String exotic object `ToObject` produces: its own
    /// properties are `"length"` and the index of every code unit. Anything
    /// else is resolved on `%String.prototype%`, which carries no method yet,
    /// so it is undefined.
    fn string_member(
        &self,
        value: Value,
        name: PropertyKey,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let length = heap
            .strings
            .length_of(value)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        let units = name
            .as_string()
            .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
            .unwrap_or_default();
        if units == LENGTH_NAME {
            let length = i32::try_from(length).map_err(|_| VMError::StringLimit)?;
            return Ok(Value::from_smi(length));
        }
        let Some(index) = string_index(&units) else {
            // Anything that is not an index is resolved on %String.prototype%.
            let prototype = realm
                .string_prototype(heap)?
                .as_object()
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            return Ok(heap
                .lookup_named(prototype, name)?
                .map_or(VALUE_UNDEFINED, |property| property.value));
        };
        let Some(unit) = usize::try_from(index)
            .ok()
            .filter(|index| *index < length)
            .and_then(|index| heap.strings.char_code_at(value, index))
        else {
            return Ok(VALUE_UNDEFINED);
        };
        self.allocate_string(heap, &[unit])
    }

    /// `ToObject` of 7.1.18: an Object is itself, a primitive is boxed, and
    /// undefined and null throw a `TypeError`.
    fn coerce_object(
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<ObjectRef, VMError> {
        if let Some(object) = value.as_object() {
            return Ok(object);
        }
        let kind = if let Some(boolean) = value.as_boolean() {
            ObjectKind::BooleanWrapper(boolean)
        } else if let Some(number) = value.as_f64() {
            ObjectKind::NumberWrapper(number)
        } else if value.is_string() {
            ObjectKind::StringWrapper(value)
        } else {
            return Err(type_error(heap, realm, "cannot box null or undefined"));
        };
        let prototype = realm.object_prototype(heap)?;
        let shape = heap.shapes.root_shape();
        let boxed = heap.allocate_object(shape, prototype)?;
        heap.set_object_kind(boxed, kind)?;
        Ok(boxed)
    }

    /// Advances an iterator by one step (7.4.8) and unpacks its result.
    ///
    /// The two registers at `state` hold the iterator and the value a step
    /// produced. An iterator whose `next` is not a native intrinsic is refused:
    /// calling a bytecode `next` from here needs a frame this operation does
    /// not open.
    fn iterator_next(
        &mut self,
        state: Reg,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let value_slot = Reg(state.0.checked_add(1).ok_or(VMError::InvalidRegister)?);
        let iterator = self.read_reg(state)?;
        let Some(reference) = iterator.as_object() else {
            return Err(type_error(heap, realm, "iterator is not an object"));
        };
        let next_key = PropertyKey::String(heap.strings.intern("next")?);
        let next = heap
            .lookup_named(reference, next_key)?
            .map(|property| property.value)
            .and_then(Value::as_object)
            .and_then(|next| heap.get_object(next))
            .map(|next| next.kind.clone());
        let Some(ObjectKind::NativeFunction { id, .. }) = next else {
            return Err(type_error(heap, realm, "iterator next is not callable"));
        };
        let intrinsic = Intrinsic::from_id(id).ok_or(VMError::TypeError)?;
        let result = self.call_intrinsic(
            intrinsic,
            Call {
                receiver: iterator,
                func: state,
                arg_start: state,
                arg_count: 0,
                slot: 0,
                return_pc: 0,
                caller_code_id: None,
            },
            heap,
            realm,
        )?;
        let result = result
            .as_object()
            .ok_or_else(|| type_error(heap, realm, "iterator result is not an object"))?;
        let done_key = PropertyKey::String(heap.strings.intern("done")?);
        let done = heap
            .lookup_named(result, done_key)?
            .is_some_and(|property| property.value.to_boolean());
        let value_key = PropertyKey::String(heap.strings.intern("value")?);
        let value = heap
            .lookup_named(result, value_key)?
            .map_or(VALUE_UNDEFINED, |property| property.value);
        self.write_reg(value_slot, if done { VALUE_UNDEFINED } else { value })?;
        Ok(Value::from_bool(!done))
    }

    /// Produces the next key of a for-in enumeration, or undefined when the
    /// Prototype Chain is spent (14.7.5.9).
    ///
    /// The four registers at `state` hold the object currently enumerated, that
    /// level's own-key Array, the index reached in it, and the object recording
    /// the keys already visited. An own key is recorded as visited even when it
    /// is not enumerable, so that it shadows the same key on a prototype, and
    /// the attributes are read again at this point, so that a property deleted
    /// during the enumeration is not visited.
    fn for_in_next(
        &mut self,
        code: &BytecodeFunction,
        state: Reg,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let object_slot = state;
        let keys_slot = Reg(state.0.checked_add(1).ok_or(VMError::InvalidRegister)?);
        let index_slot = Reg(state.0.checked_add(2).ok_or(VMError::InvalidRegister)?);
        let visited_slot = Reg(state.0.checked_add(3).ok_or(VMError::InvalidRegister)?);
        loop {
            let Some(object) = self.read_reg(object_slot)?.as_object() else {
                return Ok(VALUE_UNDEFINED);
            };
            let keys = if let Some(keys) = self.read_reg(keys_slot)?.as_object() {
                keys
            } else {
                // 14.7.5.9 enumerates String keys only, so the Symbol keys of
                // 10.1.11.1 are dropped before the level's Array is sized.
                let own: Vec<_> = heap
                    .own_keys(object)?
                    .into_iter()
                    .filter_map(|(name, _)| name.as_string())
                    .collect();
                let length = u32::try_from(own.len()).map_err(|_| VMError::PropertyLimit)?;
                let keys = self.allocate_array(code, heap, realm, length)?;
                for (index, name) in own.into_iter().enumerate() {
                    let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
                    heap.set_array_element(keys, index, Value::from_string(name))?;
                }
                self.write_reg(keys_slot, Value::from_object(keys))?;
                self.write_reg(index_slot, Value::from_smi(0))?;
                keys
            };
            let length = heap.array_length(keys).ok_or(VMError::TypeError)?;
            loop {
                let index = self
                    .read_reg(index_slot)?
                    .as_smi()
                    .ok_or(VMError::TypeError)?;
                let index = u32::try_from(index).map_err(|_| VMError::TypeError)?;
                if index >= length {
                    break;
                }
                self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                self.write_reg(
                    index_slot,
                    Value::from_smi(i32::try_from(index.saturating_add(1)).unwrap_or(i32::MAX)),
                )?;
                let elements = heap.get_object(keys).ok_or(VMError::TypeError)?.elements;
                let key = elements
                    .and_then(|elements| heap.get_elements(elements))
                    .and_then(|elements| elements.get(index))
                    .ok_or(VMError::TypeError)?;
                let name = PropertyKey::String(key.as_heap_string().ok_or(VMError::TypeError)?);
                let visited = self
                    .read_reg(visited_slot)?
                    .as_object()
                    .ok_or(VMError::TypeError)?;
                let visited_shape = heap.get_object(visited).ok_or(VMError::TypeError)?.shape_id;
                if heap.shapes.lookup(visited_shape, name).is_some() {
                    continue;
                }
                let Some(flags) = heap.own_named_flags(object, name)? else {
                    continue;
                };
                if heap.own_property_count(visited).unwrap_or(usize::MAX) >= self.property_limit {
                    return Err(VMError::PropertyLimit);
                }
                heap.define_own_named(visited, name, VALUE_TRUE, PropertyFlags::ordinary_data())?;
                if flags.enumerable {
                    return Ok(key);
                }
            }
            let prototype = heap.get_object(object).ok_or(VMError::TypeError)?.prototype;
            self.write_reg(object_slot, prototype)?;
            self.write_reg(keys_slot, VALUE_UNDEFINED)?;
        }
    }

    /// Transfers control to the innermost handler protecting the throwing
    /// instruction, unwinding call frames until one is found (14.15).
    ///
    /// `pc` is the offset of the instruction that threw, not the next one.
    fn unwind(
        &mut self,
        code: &BytecodeFunction,
        mut pc: usize,
        mut current_code_id: Option<u32>,
        value: Value,
        native: Option<(super::realm::NativeErrorKind, &'static str)>,
    ) -> Result<(usize, Option<u32>), VMError> {
        loop {
            let active_code = code_unit(code, current_code_id).ok_or(VMError::InvalidBytecode(
                VerificationError::FunctionOutOfBounds {
                    pc,
                    index: current_code_id.unwrap_or(u32::MAX),
                },
            ))?;
            if let Some(handler) = active_code.find_handler(pc) {
                self.write_reg(handler.exception, value)?;
                self.acc = VALUE_UNDEFINED;
                return Ok((handler.handler_pc as usize, current_code_id));
            }
            let Some(frame) = self.frames.pop() else {
                self.fp = 0;
                self.active_binding_count = 0;
                self.current_context = None;
                return Err(VMError::Thrown(value, native));
            };
            self.fp = frame.caller_fp;
            self.active_binding_count = frame.caller_binding_count;
            self.current_context = frame.caller_context;
            current_code_id = frame.caller_code_id;
            // The saved offset resumes after the call, so the protected
            // instruction is the call itself.
            pc = frame.return_pc.saturating_sub(1);
        }
    }

    /// Executes a bytecode function against the heap with active feedback caching.
    ///
    /// # Errors
    /// Returns [`VMError`] on out of fuel, type errors, invalid registers, or stack overflow.
    pub fn run(
        &mut self,
        code: &BytecodeFunction,
        feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        self.run_with_arguments(code, &[], feedback, heap, realm)
    }

    /// Executes bytecode after copying supplied values into the formal-parameter
    /// prefix of the contiguous register frame.
    ///
    /// Missing parameters remain `undefined`; extra arguments are ignored by
    /// this low-level entry point. Values in parameter registers participate in
    /// precise Nursery forwarding at allocation Safe Points.
    ///
    /// # Errors
    ///
    /// Returns [`VMError`] on invalid bytecode, resource exhaustion, invalid
    /// heap references, or an operation unsupported by the current bytecode.
    pub fn run_with_arguments(
        &mut self,
        code: &BytecodeFunction,
        arguments: &[Value],
        feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        self.fp = 0;
        self.frames.clear();
        self.current_context = None;
        code.verify().map_err(VMError::InvalidBytecode)?;
        if !feedback.matches_code(code) {
            return Err(VMError::InvalidFeedbackVector);
        }
        if code.entry_stack_requirement > self.operand_stack_limit {
            return Err(VMError::StackOverflow);
        }
        self.active_binding_count = usize::from(code.binding_count);
        if self.active_binding_count > self.binding_limit {
            return Err(VMError::BindingStackOverflow);
        }
        self.fuel = self
            .fuel
            .checked_sub(code.entry_fuel_cost)
            .ok_or(VMError::OutOfFuel)?;
        let mut pc: usize = 0;
        let mut current_code_id = None;
        let frame_end = self
            .fp
            .checked_add(code.register_count as usize)
            .ok_or(VMError::StackOverflow)?;
        self.stack
            .get_mut(self.fp..frame_end)
            .ok_or(VMError::StackOverflow)?
            .fill(VALUE_UNDEFINED);
        for (index, value) in arguments
            .iter()
            .copied()
            .take(usize::from(code.parameter_count))
            .enumerate()
        {
            let slot = self
                .stack
                .get_mut(self.fp.saturating_add(index))
                .ok_or(VMError::StackOverflow)?;
            *slot = value;
        }
        self.acc = VALUE_UNDEFINED;
        if let Some(slot_count) = code.own_context_slot_count {
            self.current_context = Some(self.allocate_context(code, heap, None, slot_count)?);
        }

        loop {
            match self.step(code, feedback, heap, realm, &mut pc, &mut current_code_id) {
                Ok(None) => {}
                Ok(Some(value)) => return Ok(value),
                // 14.15: a thrown value looks for a handler from the throwing
                // instruction outwards before it leaves the outermost frame.
                Err(VMError::Thrown(value, native)) => {
                    let (next_pc, next_code_id) =
                        self.unwind(code, pc.saturating_sub(1), current_code_id, value, native)?;
                    pc = next_pc;
                    current_code_id = next_code_id;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Executes one instruction, returning the function's value when it returns.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a value the caller has to unwind to a
    /// handler, and any other [`VMError`] for a failure execution cannot catch.
    #[expect(clippy::too_many_lines, reason = "central bytecode dispatch")]
    fn step(
        &mut self,
        code: &BytecodeFunction,
        feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        pc_out: &mut usize,
        code_id_out: &mut Option<u32>,
    ) -> Result<Option<Value>, VMError> {
        let mut pc = *pc_out;
        let mut current_code_id = *code_id_out;
        let outcome = (|| -> Result<Option<Value>, VMError> {
            let active_code = code_unit(code, current_code_id).ok_or(VMError::InvalidBytecode(
                VerificationError::FunctionOutOfBounds {
                    pc,
                    index: current_code_id.unwrap_or(u32::MAX),
                },
            ))?;
            let Some(&inst) = active_code.instructions.get(pc) else {
                return Err(VMError::UnexpectedEnd);
            };
            pc = pc.saturating_add(1);
            let active_feedback = feedback_unit_mut(feedback, current_code_id)
                .ok_or(VMError::InvalidFeedbackVector)?;

            match inst {
                Instruction::LdaSmi(val) => {
                    self.acc = Value::from_smi(val);
                }
                Instruction::LdaConstant(idx) => {
                    self.acc = active_code
                        .constants
                        .get(idx as usize)
                        .copied()
                        .ok_or(VMError::InvalidRegister)?;
                }
                Instruction::LdaString(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    self.acc = self.allocate_string(heap, units)?;
                }
                Instruction::LdaUndefined | Instruction::ToUndefined => {
                    self.acc = VALUE_UNDEFINED;
                }
                Instruction::LdaNull => {
                    self.acc = VALUE_NULL;
                }
                Instruction::LdaTrue => {
                    self.acc = VALUE_TRUE;
                }
                Instruction::LdaFalse => {
                    self.acc = VALUE_FALSE;
                }
                Instruction::Negate => {
                    let number = numeric_value(self.acc).ok_or(VMError::TypeError)?;
                    self.acc = Value::from_f64(-number);
                }
                Instruction::LogicalNot => {
                    self.acc = Value::from_bool(!Self::to_boolean(self.acc, heap)?);
                }
                Instruction::ToNumber => {
                    self.acc = Value::from_f64(primitive_number(self.acc, heap)?);
                }
                Instruction::BitNot => {
                    let number = self.acc.as_f64().ok_or(VMError::TypeError)?;
                    self.acc = Value::from_smi(!number_to_i32(number));
                }
                Instruction::TypeOf => {
                    self.acc = self.type_of(heap)?;
                }
                Instruction::Ldar(reg) => {
                    self.acc = self.read_reg(reg)?;
                }
                Instruction::Star(reg) => {
                    self.write_reg(reg, self.acc)?;
                }
                Instruction::Mov { src, dst } => {
                    let val = self.read_reg(src)?;
                    self.write_reg(dst, val)?;
                }
                Instruction::LoadContext { depth, slot } => {
                    self.acc = heap
                        .context_slot(
                            self.current_context.ok_or(VMError::InvalidRegister)?,
                            depth,
                            slot,
                        )
                        .ok_or(VMError::InvalidRegister)?;
                }
                Instruction::StoreContext { depth, slot } => {
                    heap.set_context_slot(
                        self.current_context.ok_or(VMError::InvalidRegister)?,
                        depth,
                        slot,
                        self.acc,
                    )?;
                }
                Instruction::Add(reg) => {
                    let rhs = self.read_reg(reg)?;
                    self.add(rhs, heap)?;
                }
                Instruction::Sub(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        if let Some(res) = a.checked_sub(b) {
                            self.acc = Value::from_smi(res);
                        } else {
                            self.acc = Value::from_f64(f64::from(a) - f64::from(b));
                        }
                    } else if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs))
                    {
                        self.acc = Value::from_f64(a - b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Mul(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        if let Some(res) = a.checked_mul(b) {
                            // 6.1.6.1.4: the sign of a product is the sign of
                            // the operands, so a zero product of operands with
                            // different signs is -0, which no Smi can hold.
                            self.acc = if res == 0 && (a < 0) != (b < 0) {
                                Value::from_f64(-0.0)
                            } else {
                                Value::from_smi(res)
                            };
                        } else {
                            self.acc = Value::from_f64(f64::from(a) * f64::from(b));
                        }
                    } else if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs))
                    {
                        self.acc = Value::from_f64(a * b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Pow(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(left), Some(right)) = (numeric_value(self.acc), numeric_value(rhs))
                    {
                        self.acc = Value::from_f64(audhsos_math::pow(left, right));
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Div(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs)) {
                        self.acc = Value::from_f64(a / b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Mod(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs)) {
                        self.acc = Value::from_f64(a % b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Binary { op, rhs, slot } => {
                    let rhs = self.read_reg(rhs)?;
                    let observed = self.primitive_binary(op, rhs, heap)?;
                    active_feedback
                        .record_binary(slot, observed)
                        .ok_or(VMError::InvalidFeedbackVector)?;
                }
                Instruction::BitAnd(reg) => {
                    let rhs = self.read_reg(reg)?;
                    let left = number_to_i32(primitive_number(self.acc, heap)?);
                    let right = number_to_i32(primitive_number(rhs, heap)?);
                    self.acc = Value::from_smi(left & right);
                }
                Instruction::BitOr(reg) => {
                    let rhs = self.read_reg(reg)?;
                    let left = number_to_i32(primitive_number(self.acc, heap)?);
                    let right = number_to_i32(primitive_number(rhs, heap)?);
                    self.acc = Value::from_smi(left | right);
                }
                Instruction::BitXor(reg) => {
                    let rhs = self.read_reg(reg)?;
                    let left = number_to_i32(primitive_number(self.acc, heap)?);
                    let right = number_to_i32(primitive_number(rhs, heap)?);
                    self.acc = Value::from_smi(left ^ right);
                }
                Instruction::Shl(reg) => {
                    let rhs = self.read_reg(reg)?;
                    let left = number_to_i32(primitive_number(self.acc, heap)?);
                    let shift = crate::value::number_uint32(primitive_number(rhs, heap)?) & 0x1F;
                    self.acc = Value::from_smi(left.wrapping_shl(shift));
                }
                Instruction::Shr(reg) => {
                    let rhs = self.read_reg(reg)?;
                    let left = number_to_i32(primitive_number(self.acc, heap)?);
                    let shift = crate::value::number_uint32(primitive_number(rhs, heap)?) & 0x1F;
                    self.acc = Value::from_smi(left.wrapping_shr(shift));
                }
                Instruction::Ushr(reg) => {
                    let rhs = self.read_reg(reg)?;
                    let left = crate::value::number_uint32(primitive_number(self.acc, heap)?);
                    let shift = crate::value::number_uint32(primitive_number(rhs, heap)?) & 0x1F;
                    self.acc = Value::from_f64(f64::from(left.wrapping_shr(shift)));
                }
                Instruction::TestEqual(reg) => {
                    let rhs = self.read_reg(reg)?;
                    self.acc = Value::from_bool(Self::loosely_equals(self.acc, rhs, heap)?);
                }
                Instruction::TestStrictEqual(reg) => {
                    let rhs = self.read_reg(reg)?;
                    self.acc = Value::from_bool(Self::strictly_equals(self.acc, rhs, heap)?);
                }
                Instruction::TestLessThan(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs)) {
                        self.acc = Value::from_bool(a < b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::TestLessThanOrEqual(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs)) {
                        self.acc = Value::from_bool(a <= b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::TestGreaterThan(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs)) {
                        self.acc = Value::from_bool(a > b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::TestGreaterThanOrEqual(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (numeric_value(self.acc), numeric_value(rhs)) {
                        self.acc = Value::from_bool(a >= b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Jump(offset) => {
                    if offset < 0 {
                        self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                    }
                    pc = (pc as isize + offset as isize) as usize;
                }
                Instruction::JumpIfTrue(offset) => {
                    if Self::to_boolean(self.acc, heap)? {
                        if offset < 0 {
                            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                        }
                        pc = (pc as isize + offset as isize) as usize;
                    }
                }
                Instruction::JumpIfFalse(offset) => {
                    if !Self::to_boolean(self.acc, heap)? {
                        if offset < 0 {
                            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                        }
                        pc = (pc as isize + offset as isize) as usize;
                    }
                }
                Instruction::JumpIfNotNullish(offset) => {
                    if !self.acc.is_null_or_undefined() {
                        if offset < 0 {
                            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                        }
                        pc = (pc as isize + offset as isize) as usize;
                    }
                }
                Instruction::JumpIfNotUndefined(offset) => {
                    if !self.acc.is_undefined() {
                        if offset < 0 {
                            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                        }
                        pc = (pc as isize + offset as isize) as usize;
                    }
                }
                Instruction::GetNamed { obj, name, slot } => {
                    let name = active_code
                        .string_constants
                        .get(name as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(name)?);
                    let target = self.read_reg(obj)?;
                    // 10.4.3: a String answers "length" and its indices from the
                    // exotic object ToObject would produce, without producing it.
                    if target.is_string() {
                        self.acc = self.string_member(target, name, heap, realm)?;
                        return Ok(None);
                    }
                    let Some(oref) = target.as_object() else {
                        return Err(type_error(
                            heap,
                            realm,
                            "property access on a value that is not an object",
                        ));
                    };
                    let shape_id = heap.get_object(oref).ok_or(VMError::TypeError)?.shape_id;
                    let prototype_epoch = heap.shapes.prototype_epoch();

                    // Inline cache check
                    let cached = active_feedback
                        .get_named_ic(slot)
                        .and_then(|ic| ic.try_get(name, shape_id, prototype_epoch));

                    if let Some(case) = cached
                        && let Some(value) = heap.load_cached_named(
                            oref,
                            case.receiver_shape,
                            case.holder_depth,
                            case.holder_shape,
                            case.slot,
                            case.prototype_epoch,
                        )?
                    {
                        self.acc = value;
                        return Ok(None);
                    }

                    if let Some(property) = heap.lookup_named(oref, name)? {
                        if let Some(ic) = active_feedback.get_named_ic_mut(slot) {
                            ic.record(NamedAccessCase {
                                name,
                                receiver_shape: property.receiver_shape,
                                holder_depth: property.holder_depth,
                                holder_shape: property.holder_shape,
                                slot: property.slot,
                                prototype_epoch,
                            });
                        }
                        self.acc = property.value;
                    } else {
                        self.acc = VALUE_UNDEFINED;
                    }
                }
                Instruction::SetNamed { obj, name, slot } => {
                    let name = active_code
                        .string_constants
                        .get(name as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(name)?);
                    let target = self.read_reg(obj)?;
                    let oref = target.as_object().ok_or(VMError::TypeError)?;
                    let current_shape = heap.get_object(oref).ok_or(VMError::TypeError)?.shape_id;
                    let prototype_epoch = heap.shapes.prototype_epoch();

                    let cached = active_feedback
                        .get_named_ic(slot)
                        .and_then(|ic| ic.try_get(name, current_shape, prototype_epoch));
                    if let Some(case) = cached
                        && case.holder_depth == 0
                        && case.holder_shape == current_shape
                    {
                        heap.set_object_slot(oref, case.slot, self.acc)?;
                        return Ok(None);
                    }

                    // Check if property exists in current shape
                    if let Some(loc) = heap.shapes.lookup(current_shape, name) {
                        let val = self.acc;
                        heap.set_object_slot(oref, loc.slot_offset, val)?;
                        if let Some(ic) = active_feedback.get_named_ic_mut(slot) {
                            ic.record(NamedAccessCase {
                                name,
                                receiver_shape: current_shape,
                                holder_depth: 0,
                                holder_shape: current_shape,
                                slot: loc.slot_offset,
                                prototype_epoch: None,
                            });
                        }
                    } else {
                        if heap.own_property_count(oref).unwrap_or(usize::MAX)
                            >= self.property_limit
                        {
                            return Err(VMError::PropertyLimit);
                        }
                        // Transition to new Shape
                        let (new_shape, slot_idx) = heap.shapes.transition(
                            current_shape,
                            name,
                            PropertyFlags::ordinary_data(),
                        );
                        let val = self.acc;
                        heap.set_object_shape(oref, new_shape)?;
                        heap.set_object_slot(oref, slot_idx, val)?;
                        if let Some(ic) = active_feedback.get_named_ic_mut(slot) {
                            ic.record(NamedAccessCase {
                                name,
                                receiver_shape: new_shape,
                                holder_depth: 0,
                                holder_shape: new_shape,
                                slot: slot_idx,
                                prototype_epoch: None,
                            });
                        }
                    }
                }
                Instruction::GetByValue { obj, key, slot } => {
                    let target = self.read_reg(obj)?;
                    if target.is_string() {
                        let key = self.read_reg(key)?;
                        let name = property_key(key, heap)?;
                        self.acc = self.string_member(target, name, heap, realm)?;
                        return Ok(None);
                    }
                    let Some(oref) = target.as_object() else {
                        return Err(type_error(
                            heap,
                            realm,
                            "property access on a value that is not an object",
                        ));
                    };
                    let js_obj = heap.get_object(oref).ok_or(VMError::TypeError)?;
                    let key_val = self.read_reg(key)?;

                    if let (Some(idx), Some(eref)) = (array_index(key_val, heap)?, js_obj.elements)
                    {
                        let elem = heap.get_elements(eref).ok_or(VMError::TypeError)?;
                        self.acc = elem.get(idx).unwrap_or(VALUE_UNDEFINED);
                    } else {
                        let name = property_name_units(key_val, heap)?;
                        if name.as_slice() == [0x6C, 0x65, 0x6E, 0x67, 0x74, 0x68]
                            && let Some(length) = heap.array_length(oref)
                        {
                            self.acc = i32::try_from(length).map_or_else(
                                |_| Value::from_f64(f64::from(length)),
                                Value::from_smi,
                            );
                            return Ok(None);
                        }
                        let Some(name) = heap
                            .strings
                            .lookup_interned_units(&name)
                            .map(PropertyKey::String)
                        else {
                            self.acc = VALUE_UNDEFINED;
                            return Ok(None);
                        };
                        let shape_id = heap.get_object(oref).ok_or(VMError::TypeError)?.shape_id;
                        let prototype_epoch = heap.shapes.prototype_epoch();
                        let cached = active_feedback
                            .get_named_ic(slot)
                            .and_then(|ic| ic.try_get(name, shape_id, prototype_epoch));
                        if let Some(case) = cached
                            && let Some(value) = heap.load_cached_named(
                                oref,
                                case.receiver_shape,
                                case.holder_depth,
                                case.holder_shape,
                                case.slot,
                                case.prototype_epoch,
                            )?
                        {
                            self.acc = value;
                            return Ok(None);
                        }
                        if let Some(property) = heap.lookup_named(oref, name)? {
                            if let Some(ic) = active_feedback.get_named_ic_mut(slot) {
                                ic.record(NamedAccessCase {
                                    name,
                                    receiver_shape: property.receiver_shape,
                                    holder_depth: property.holder_depth,
                                    holder_shape: property.holder_shape,
                                    slot: property.slot,
                                    prototype_epoch,
                                });
                            }
                            self.acc = property.value;
                        } else {
                            self.acc = VALUE_UNDEFINED;
                        }
                    }
                }
                Instruction::SetByValue { obj, key, slot } => {
                    let target = self.read_reg(obj)?;
                    let oref = target.as_object().ok_or(VMError::TypeError)?;
                    let js_obj = heap.get_object(oref).ok_or(VMError::TypeError)?;
                    let key_val = self.read_reg(key)?;
                    let val = self.acc;

                    if js_obj.elements.is_some()
                        && let Some(index) = array_index(key_val, heap)?
                    {
                        let elements_reference = js_obj.elements.ok_or(VMError::TypeError)?;
                        let elements = heap
                            .get_elements(elements_reference)
                            .ok_or(VMError::TypeError)?;
                        if elements.get(index).is_none()
                            && heap.own_property_count(oref).unwrap_or(usize::MAX)
                                >= self.property_limit
                        {
                            return Err(VMError::PropertyLimit);
                        }
                        heap.set_array_element(oref, index, val)?;
                    } else {
                        let name_units = property_name_units(key_val, heap)?;
                        let name =
                            if let Some(name) = heap.strings.lookup_interned_units(&name_units) {
                                PropertyKey::String(name)
                            } else {
                                if heap.own_property_count(oref).unwrap_or(usize::MAX)
                                    >= self.property_limit
                                {
                                    return Err(VMError::PropertyLimit);
                                }
                                PropertyKey::String(heap.strings.intern_units(&name_units)?)
                            };
                        let current_shape =
                            heap.get_object(oref).ok_or(VMError::TypeError)?.shape_id;
                        let prototype_epoch = heap.shapes.prototype_epoch();
                        let cached = active_feedback
                            .get_named_ic(slot)
                            .and_then(|ic| ic.try_get(name, current_shape, prototype_epoch));
                        if let Some(case) = cached
                            && case.holder_depth == 0
                            && case.holder_shape == current_shape
                        {
                            heap.set_object_slot(oref, case.slot, val)?;
                            return Ok(None);
                        }
                        if let Some(location) = heap.shapes.lookup(current_shape, name) {
                            heap.set_object_slot(oref, location.slot_offset, val)?;
                            if let Some(ic) = active_feedback.get_named_ic_mut(slot) {
                                ic.record(NamedAccessCase {
                                    name,
                                    receiver_shape: current_shape,
                                    holder_depth: 0,
                                    holder_shape: current_shape,
                                    slot: location.slot_offset,
                                    prototype_epoch: None,
                                });
                            }
                        } else {
                            if heap.own_property_count(oref).unwrap_or(usize::MAX)
                                >= self.property_limit
                            {
                                return Err(VMError::PropertyLimit);
                            }
                            let (new_shape, property_slot) = heap.shapes.transition(
                                current_shape,
                                name,
                                PropertyFlags::ordinary_data(),
                            );
                            heap.set_object_shape(oref, new_shape)?;
                            heap.set_object_slot(oref, property_slot, val)?;
                            if let Some(ic) = active_feedback.get_named_ic_mut(slot) {
                                ic.record(NamedAccessCase {
                                    name,
                                    receiver_shape: new_shape,
                                    holder_depth: 0,
                                    holder_shape: new_shape,
                                    slot: property_slot,
                                    prototype_epoch: None,
                                });
                            }
                        }
                    }
                }
                Instruction::GetArrayLength { obj } => {
                    let target = self.read_reg(obj)?;
                    let object = target.as_object().ok_or(VMError::TypeError)?;
                    let length = heap.array_length(object).ok_or(VMError::TypeError)?;
                    self.acc = i32::try_from(length)
                        .map_or_else(|_| Value::from_f64(f64::from(length)), Value::from_smi);
                }
                Instruction::CreateObject => {
                    let root_shape = heap.shapes.root_shape();
                    let oref = self.allocate_object(active_code, heap, realm, root_shape)?;
                    self.acc = Value::from_object(oref);
                }
                Instruction::CreateArray(length) => {
                    if self.property_limit == 0 {
                        return Err(VMError::PropertyLimit);
                    }
                    let oref = self.allocate_array(active_code, heap, realm, length)?;
                    self.acc = Value::from_object(oref);
                }
                Instruction::CreateClosure(code_id) => {
                    let target =
                        code.functions
                            .get(code_id as usize)
                            .ok_or(VMError::InvalidBytecode(
                                VerificationError::FunctionOutOfBounds {
                                    pc: pc.saturating_sub(1),
                                    index: code_id,
                                },
                            ))?;
                    let captures_context = !target.outer_context_slot_counts.is_empty();
                    let function = self.allocate_function(
                        active_code,
                        heap,
                        realm,
                        code_id,
                        captures_context,
                    )?;
                    self.acc = Value::from_object(function);
                }
                Instruction::Call {
                    func,
                    arg_start,
                    arg_count,
                    slot,
                } => {
                    if let Some(code_id) = self.enter_call(
                        code,
                        active_code,
                        active_feedback,
                        heap,
                        realm,
                        Call {
                            receiver: VALUE_UNDEFINED,
                            func,
                            arg_start,
                            arg_count,
                            slot,
                            return_pc: pc,
                            caller_code_id: current_code_id,
                        },
                    )? {
                        current_code_id = Some(code_id);
                        pc = 0;
                    }
                }
                Instruction::CallMethod {
                    receiver,
                    func,
                    arg_start,
                    arg_count,
                    slot,
                } => {
                    let receiver = self.read_reg(receiver)?;
                    if let Some(code_id) = self.enter_call(
                        code,
                        active_code,
                        active_feedback,
                        heap,
                        realm,
                        Call {
                            receiver,
                            func,
                            arg_start,
                            arg_count,
                            slot,
                            return_pc: pc,
                            caller_code_id: current_code_id,
                        },
                    )? {
                        current_code_id = Some(code_id);
                        pc = 0;
                    }
                }
                Instruction::IteratorNext { state } => {
                    self.acc = self.iterator_next(state, heap, realm)?;
                }
                Instruction::ForInNext { state } => {
                    self.acc = self.for_in_next(active_code, state, heap, realm)?;
                }
                Instruction::Throw => return Err(VMError::Thrown(self.acc, None)),
                Instruction::Return => {
                    if let Some(frame) = self.frames.pop() {
                        self.fp = frame.caller_fp;
                        pc = frame.return_pc;
                        current_code_id = frame.caller_code_id;
                        self.active_binding_count = frame.caller_binding_count;
                        self.current_context = frame.caller_context;
                    } else {
                        self.fp = 0;
                        self.active_binding_count = 0;
                        self.current_context = None;
                        return Ok(Some(self.acc));
                    }
                }
            }
            Ok(None)
        })();
        *pc_out = pc;
        *code_id_out = current_code_id;
        outcome
    }
}

fn number_to_i32(number: f64) -> i32 {
    i32::from_ne_bytes(crate::value::number_uint32(number).to_ne_bytes())
}

fn code_unit(root: &BytecodeFunction, code_id: Option<u32>) -> Option<&BytecodeFunction> {
    code_id.map_or(Some(root), |code_id| root.functions.get(code_id as usize))
}

fn feedback_unit_mut(
    root: &mut FeedbackVector,
    code_id: Option<u32>,
) -> Option<&mut FeedbackVector> {
    if let Some(code_id) = code_id {
        root.function_mut(code_id)
    } else {
        Some(root)
    }
}

fn numeric_value(value: Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.is_undefined().then_some(f64::NAN))
}

fn primitive_number(value: Value, heap: &GenerationalHeap) -> Result<f64, VMError> {
    if let Some(number) = value.as_f64() {
        return Ok(number);
    }
    if value.is_undefined() {
        return Ok(f64::NAN);
    }
    if value.is_null() {
        return Ok(0.0);
    }
    if let Some(boolean) = value.as_boolean() {
        return Ok(f64::from(u8::from(boolean)));
    }
    if value.is_string() {
        let text = heap
            .strings
            .to_rust_string(value)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        return Ok(crate::value::string_number(&text));
    }
    Err(VMError::TypeError)
}

fn exact_smi(number: f64) -> Option<i32> {
    if !number.is_finite() || number < f64::from(i32::MIN) || number > f64::from(i32::MAX) {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "finite Number was bounded to the signed 32-bit range"
    )]
    let integer = number as i32;
    #[expect(clippy::float_cmp, reason = "Smi conversion requires exact integers")]
    (f64::from(integer) == number && !(number == 0.0 && number.is_sign_negative()))
        .then_some(integer)
}

fn array_index(value: Value, heap: &GenerationalHeap) -> Result<Option<u32>, VMError> {
    if let Some(index) = value.as_smi() {
        return Ok(u32::try_from(index).ok());
    }
    if let Some(number) = value.as_f64() {
        if number == 0.0 {
            return Ok(Some(0));
        }
        if !number.is_finite() || number < 0.0 || number >= f64::from(u32::MAX) {
            return Ok(None);
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "finite integral value was bounded to the Array-index range"
        )]
        let index = number as u32;
        #[expect(
            clippy::float_cmp,
            reason = "Array indices require exact integral binary64 values"
        )]
        return Ok((f64::from(index) == number).then_some(index));
    }
    if !value.is_string() {
        return Ok(None);
    }
    let length = heap
        .strings
        .length_of(value)
        .ok_or(VMError::Heap(HeapError::InvalidReference))?;
    if length == 0 || length > 10 {
        return Ok(None);
    }
    let first = heap
        .strings
        .char_code_at(value, 0)
        .ok_or(VMError::Heap(HeapError::InvalidReference))?;
    if length > 1 && first == u16::from(b'0') {
        return Ok(None);
    }
    let mut index = 0u32;
    for position in 0..length {
        let unit = heap
            .strings
            .char_code_at(value, position)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        let Some(digit) = unit
            .checked_sub(u16::from(b'0'))
            .filter(|digit| *digit < 10)
        else {
            return Ok(None);
        };
        let Some(next) = index
            .checked_mul(10)
            .and_then(|index| index.checked_add(u32::from(digit)))
        else {
            return Ok(None);
        };
        index = next;
    }
    Ok((index != u32::MAX).then_some(index))
}

/// `ToPropertyKey` of 7.1.19 for the primitive values the engine carries.
///
/// An Object key needs `ToPrimitive`, which needs callable `valueOf` and
/// `toString` intrinsics; until those exist such a key is refused.
fn property_key(value: Value, heap: &mut GenerationalHeap) -> Result<PropertyKey, VMError> {
    if let Some(symbol) = value.as_symbol() {
        return Ok(PropertyKey::Symbol(symbol));
    }
    if let Some(name) = value.as_heap_string() {
        return Ok(PropertyKey::String(name));
    }
    let units = property_name_units(value, heap)?;
    Ok(PropertyKey::String(heap.strings.intern_units(&units)?))
}

/// Creates a `TypeError` of the Realm and raises it as an exception.
fn type_error(heap: &mut GenerationalHeap, realm: &Realm, message: &'static str) -> VMError {
    raise(
        heap,
        realm,
        super::realm::NativeErrorKind::TypeError,
        message,
    )
}

/// `ToAbsoluteIndex` of 7.1.25: a negative position counts from the end.
const fn absolute_index(position: i64, length: i64) -> i64 {
    if position < 0 {
        length.saturating_add(position)
    } else {
        position
    }
}

/// `SameValueZero` of 7.2.10: strict equality, except that NaN matches NaN.
fn same_value_zero(left: Value, right: Value, heap: &GenerationalHeap) -> Result<bool, VMError> {
    if left.as_f64().is_some_and(f64::is_nan) && right.as_f64().is_some_and(f64::is_nan) {
        return Ok(true);
    }
    RegisterVM::strictly_equals(left, right, heap)
}

/// An index as the Number the Array search methods of 23.1.3 answer with.
fn index_value(index: i64) -> Value {
    i32::try_from(index).map_or_else(
        |_| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "an index is below 2^53, which binary64 represents exactly"
            )]
            Value::from_f64(index as f64)
        },
        Value::from_smi,
    )
}

/// A position clamped into `0..=length`, as the String methods of 22.1.3 do
/// before they index.
fn clamped_index(position: i64, length: usize) -> usize {
    usize::try_from(position.max(0))
        .unwrap_or(usize::MAX)
        .min(length)
}

/// A relative index: a negative one counts from the end, and both ends are
/// clamped into `0..=length`.
fn relative_index(position: i64, length: usize) -> usize {
    let length_signed = i64::try_from(length).unwrap_or(i64::MAX);
    let index = if position < 0 {
        length_signed.saturating_add(position)
    } else {
        position
    };
    clamped_index(index, length)
}

/// The first index at or after `start` where `search` occurs.
fn find_units(units: &[u16], search: &[u16], start: usize) -> Option<usize> {
    if search.len() > units.len() {
        return None;
    }
    (start..=units.len().saturating_sub(search.len()))
        .find(|index| units.get(*index..index.saturating_add(search.len())) == Some(search))
}

/// Creates a `RangeError` of the Realm and raises it as an exception.
fn range_error(heap: &mut GenerationalHeap, realm: &Realm, message: &'static str) -> VMError {
    raise(
        heap,
        realm,
        super::realm::NativeErrorKind::RangeError,
        message,
    )
}

/// Creates an error of the Realm and raises it as an exception.
fn raise(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    kind: super::realm::NativeErrorKind,
    message: &'static str,
) -> VMError {
    match realm.create_native_error(heap, kind, message) {
        Ok(error) => VMError::Thrown(Value::from_object(error), Some((kind, message))),
        Err(error) => VMError::Heap(error),
    }
}

/// The code point at one index, pairing a leading surrogate with the trailing
/// one after it (11.1.4).
fn code_point_at(units: &[u16], position: usize, first: u16) -> i32 {
    let leading = (0xD800..0xDC00).contains(&first);
    let trailing = position
        .checked_add(1)
        .and_then(|next| units.get(next).copied())
        .filter(|unit| (0xDC00..0xE000).contains(unit));
    match (leading, trailing) {
        (true, Some(second)) => {
            let high = i32::from(first)
                .saturating_sub(0xD800)
                .saturating_mul(0x400);
            high.saturating_add(i32::from(second).saturating_sub(0xDC00))
                .saturating_add(0x1_0000)
        }
        _ => i32::from(first),
    }
}

/// Whether one code unit is trimmed from a String's ends: the white space of
/// 11.2 or the line terminators of 11.3.
fn trimmable(unit: u16) -> bool {
    char::from_u32(u32::from(unit))
        .is_some_and(|ch| crate::value::whitespace(ch) || crate::value::line_terminator(ch))
}

/// The largest integer a binary64 represents exactly, which bounds every index.
const INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;

/// `ToIntegerOrInfinity` of 7.1.5 for a primitive argument, clamped to the
/// range an index can occupy.
fn integer_argument(value: Value, heap: &GenerationalHeap) -> Result<i64, VMError> {
    let number = primitive_number(value, heap)?;
    if number.is_nan() {
        return Ok(0);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the value is clamped into the index range before truncation"
    )]
    Ok(number.clamp(-INTEGER_LIMIT, INTEGER_LIMIT) as i64)
}

/// UTF-16 code units of the property name `"length"`.
const LENGTH_NAME: [u16; 6] = [0x6C, 0x65, 0x6E, 0x67, 0x74, 0x68];

/// The canonical index a property name denotes, if it denotes one.
fn string_index(units: &[u16]) -> Option<u32> {
    let digit = |unit: u16| {
        (0x30..=0x39)
            .contains(&unit)
            .then(|| unit.wrapping_sub(0x30))
    };
    let (first, rest) = units.split_first()?;
    if digit(*first).is_none() || (*first == 0x30 && !rest.is_empty()) || units.len() > 10 {
        return None;
    }
    let mut value: u32 = 0;
    for unit in units {
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(digit(*unit)?))?;
    }
    Some(value)
}

fn property_name_units(value: Value, heap: &GenerationalHeap) -> Result<Vec<u16>, VMError> {
    if value.is_string() {
        return heap
            .strings
            .to_utf16(value)
            .ok_or(VMError::Heap(HeapError::InvalidReference));
    }
    if let Some(number) = value.as_f64() {
        return Ok(crate::number::decimal_string(number)
            .encode_utf16()
            .collect());
    }
    if let Some(boolean) = value.as_boolean() {
        return Ok(if boolean { "true" } else { "false" }
            .encode_utf16()
            .collect());
    }
    if value.is_null() {
        return Ok("null".encode_utf16().collect());
    }
    if value.is_undefined() {
        return Ok("undefined".encode_utf16().collect());
    }
    Err(VMError::TypeError)
}

#[cfg(test)]
mod tests {
    use super::super::bytecode::FeedbackKind;
    use super::super::feedback::CallIC;
    use super::*;

    #[test]
    fn vm_register_arithmetic_loop_sum() {
        // let mut sum = 0;
        // for (let i = 0; i < 10; i++) { sum += i; }
        // return sum;
        let mut code = BytecodeFunction::new(4, 0); // r0 = sum, r1 = i, r2 = 10, r3 = temp
        let r_sum = Reg(0);
        let r_i = Reg(1);
        let r_limit = Reg(2);
        let r_temp = Reg(3);
        let c_one = code.add_constant(Value::from_smi(1));

        // sum = 0
        code.emit(Instruction::LdaSmi(0));
        code.emit(Instruction::Star(r_sum));
        // i = 0
        code.emit(Instruction::LdaSmi(0));
        code.emit(Instruction::Star(r_i));
        // limit = 10
        code.emit(Instruction::LdaSmi(10));
        code.emit(Instruction::Star(r_limit));

        // Loop header (offset = 6): if !(i < limit) goto exit
        code.emit(Instruction::Ldar(r_i));
        code.emit(Instruction::TestLessThan(r_limit));
        // Jump to exit (+9 instructions ahead) if false
        code.emit(Instruction::JumpIfFalse(9));

        // sum = sum + i
        code.emit(Instruction::Ldar(r_sum));
        code.emit(Instruction::Add(r_i));
        code.emit(Instruction::Star(r_sum));

        // i = i + 1
        code.emit(Instruction::LdaConstant(c_one));
        code.emit(Instruction::Star(r_temp));
        code.emit(Instruction::Ldar(r_i));
        code.emit(Instruction::Add(r_temp));
        code.emit(Instruction::Star(r_i));

        // Jump back to loop header
        code.emit(Instruction::Jump(-12));

        // Exit: return sum
        code.emit(Instruction::Ldar(r_sum));
        code.emit(Instruction::Return);

        let mut vm = RegisterVM::new(100_000);
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&code);

        let res = vm.run(&code, &mut feedback, &mut heap, &realm).unwrap();
        assert_eq!(res.as_smi(), Some(45));
    }

    #[test]
    fn undefined_branch_distinguishes_null_and_undefined() {
        for (value, expected) in [
            (Instruction::LdaUndefined, Value::from_smi(42)),
            (Instruction::LdaNull, VALUE_NULL),
        ] {
            let mut code = BytecodeFunction::new(0, 0);
            code.emit(value);
            code.emit(Instruction::JumpIfNotUndefined(1));
            code.emit(Instruction::LdaSmi(42));
            code.emit(Instruction::Return);
            let mut feedback = FeedbackVector::for_code(&code);
            let mut heap = GenerationalHeap::default();
            let realm = Realm::new(&mut heap).unwrap();
            assert_eq!(
                RegisterVM::default().run(&code, &mut feedback, &mut heap, &realm),
                Ok(expected)
            );
        }
    }

    #[test]
    fn vm_named_property_access_and_ic_monomorphism() {
        // o = {}
        // o.x = 42
        // return o.x
        let mut code = BytecodeFunction::new(2, 0);
        let r_obj = Reg(0);
        let s_slot = code.allocate_feedback_slot(FeedbackKind::NamedAccess);
        let g_slot = code.allocate_feedback_slot(FeedbackKind::NamedAccess);

        let mut heap = GenerationalHeap::new();

        let realm = Realm::new(&mut heap).unwrap();
        let prop_x = code.add_string_constant("x".encode_utf16().collect());

        code.emit(Instruction::CreateObject);
        code.emit(Instruction::Star(r_obj));

        code.emit(Instruction::LdaSmi(42));
        code.emit(Instruction::SetNamed {
            obj: r_obj,
            name: prop_x,
            slot: s_slot,
        });

        code.emit(Instruction::GetNamed {
            obj: r_obj,
            name: prop_x,
            slot: g_slot,
        });
        code.emit(Instruction::Return);

        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(10_000);

        let res = vm.run(&code, &mut feedback, &mut heap, &realm).unwrap();
        assert_eq!(res.as_smi(), Some(42));

        // Verification of IC feedback: slot g_slot must be monomorphic!
        let ic = feedback.get_named_ic(g_slot).unwrap();
        assert!(matches!(
            ic,
            super::super::feedback::NamedAccessIC::Monomorphic(_)
        ));
    }

    #[test]
    fn allocation_safe_point_forwards_live_registers_and_promotes_survivors() {
        let mut code = BytecodeFunction::new(2, 0);
        code.emit(Instruction::CreateObject);
        code.emit(Instruction::Star(Reg(0)));
        code.emit(Instruction::CreateObject);
        code.emit(Instruction::Star(Reg(1)));
        code.emit(Instruction::Ldar(Reg(0)));
        code.emit(Instruction::Return);

        let mut heap = GenerationalHeap::with_nursery_capacity(1);

        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::new(0);
        let mut vm = RegisterVM::new(100);

        let result = vm.run(&code, &mut feedback, &mut heap, &realm).unwrap();
        let first = result.as_object().unwrap();
        assert!(first.is_old());
        assert!(heap.get_object(first).is_some());
    }

    #[test]
    fn run_rejects_unverified_code_and_mismatched_feedback() {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let mut vm = RegisterVM::new(100);
        let mut malformed = BytecodeFunction::new(0, 0);
        malformed.emit(Instruction::Ldar(Reg(0)));
        malformed.emit(Instruction::Return);
        let mut feedback = FeedbackVector::new(0);
        assert_eq!(
            vm.run(&malformed, &mut feedback, &mut heap, &realm),
            Err(VMError::InvalidBytecode(
                VerificationError::RegisterOutOfBounds {
                    pc: 0,
                    register: Reg(0)
                }
            ))
        );

        let mut valid = BytecodeFunction::new(0, 0);
        valid.feedback_slots.push(FeedbackKind::NamedAccess);
        valid.emit(Instruction::Return);
        assert_eq!(
            vm.run(&valid, &mut feedback, &mut heap, &realm),
            Err(VMError::InvalidFeedbackVector)
        );
    }

    #[test]
    fn primitive_to_number_rejects_symbols_bigints_and_objects() {
        let mut code = BytecodeFunction::new(1, 1);
        code.emit(Instruction::Ldar(Reg(0)));
        code.emit(Instruction::ToNumber);
        code.emit(Instruction::Return);
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let object = heap
            .allocate_object(heap.shapes.root_shape(), VALUE_NULL)
            .unwrap();

        for value in [
            Value::from_symbol(super::super::value::SymbolRef(0)),
            Value::from_bigint(super::super::value::BigIntRef(0)),
            Value::from_object(object),
        ] {
            let mut feedback = FeedbackVector::for_code(&code);
            let mut vm = RegisterVM::new(100);
            assert_eq!(
                vm.run_with_arguments(&code, &[value], &mut feedback, &mut heap, &realm),
                Err(VMError::TypeError)
            );
        }
    }

    #[test]
    fn primitive_loose_equality_rejects_objects_and_bigints() {
        let mut code = BytecodeFunction::new(2, 2);
        code.emit(Instruction::Ldar(Reg(0)));
        code.emit(Instruction::TestEqual(Reg(1)));
        code.emit(Instruction::Return);
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let object = heap
            .allocate_object(heap.shapes.root_shape(), VALUE_NULL)
            .unwrap();

        for value in [
            Value::from_bigint(super::super::value::BigIntRef(0)),
            Value::from_object(object),
        ] {
            let mut feedback = FeedbackVector::for_code(&code);
            let mut vm = RegisterVM::new(100);
            assert_eq!(
                vm.run_with_arguments(
                    &code,
                    &[VALUE_UNDEFINED, value],
                    &mut feedback,
                    &mut heap,
                    &realm
                ),
                Err(VMError::TypeError)
            );
        }

        let first = Value::from_symbol(super::super::value::SymbolRef(1));
        let second = Value::from_symbol(super::super::value::SymbolRef(2));
        for (right, expected) in [(first, true), (second, false)] {
            let mut feedback = FeedbackVector::for_code(&code);
            let mut vm = RegisterVM::new(100);
            assert_eq!(
                vm.run_with_arguments(&code, &[first, right], &mut feedback, &mut heap, &realm),
                Ok(Value::from_bool(expected))
            );
        }
    }

    #[test]
    fn array_index_classification_matches_array_property_boundaries() {
        let mut heap = GenerationalHeap::new();
        assert_eq!(array_index(Value::from_smi(0), &heap), Ok(Some(0)));
        assert_eq!(array_index(Value::from_smi(-1), &heap), Ok(None));
        assert_eq!(array_index(Value::from_f64(-0.0), &heap), Ok(Some(0)));
        assert_eq!(
            array_index(Value::from_f64(2_147_483_648.0), &heap),
            Ok(Some(2_147_483_648))
        );
        assert_eq!(array_index(Value::from_f64(1.5), &heap), Ok(None));
        assert_eq!(array_index(Value::from_f64(f64::NAN), &heap), Ok(None));
        assert_eq!(
            array_index(Value::from_f64(4_294_967_294.0), &heap),
            Ok(Some(4_294_967_294))
        );
        assert_eq!(
            array_index(Value::from_f64(4_294_967_295.0), &heap),
            Ok(None)
        );
        let zero = Value::from_string(heap.strings.allocate_str("0").unwrap());
        let leading_zero = Value::from_string(heap.strings.allocate_str("01").unwrap());
        let maximum = Value::from_string(heap.strings.allocate_str("4294967294").unwrap());
        let too_large = Value::from_string(heap.strings.allocate_str("4294967295").unwrap());
        assert_eq!(array_index(zero, &heap), Ok(Some(0)));
        assert_eq!(array_index(leading_zero, &heap), Ok(None));
        assert_eq!(array_index(maximum, &heap), Ok(Some(4_294_967_294)));
        assert_eq!(array_index(too_large, &heap), Ok(None));
    }

    #[test]
    fn dynamic_array_stores_obey_the_property_limit() {
        let mut code = BytecodeFunction::new(2, 0);
        code.feedback_slots = alloc::vec![FeedbackKind::NamedAccess; 2];
        code.emit(Instruction::CreateArray(0));
        code.emit(Instruction::Star(Reg(0)));
        code.emit(Instruction::LdaSmi(0));
        code.emit(Instruction::Star(Reg(1)));
        code.emit(Instruction::LdaSmi(1));
        code.emit(Instruction::SetByValue {
            obj: Reg(0),
            key: Reg(1),
            slot: 0,
        });
        code.emit(Instruction::LdaSmi(1));
        code.emit(Instruction::Star(Reg(1)));
        code.emit(Instruction::LdaSmi(2));
        code.emit(Instruction::SetByValue {
            obj: Reg(0),
            key: Reg(1),
            slot: 1,
        });
        code.emit(Instruction::Return);

        let mut heap = GenerationalHeap::new();

        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(100);
        vm.set_property_limit(2);
        assert_eq!(
            vm.run(&code, &mut feedback, &mut heap, &realm),
            Err(VMError::PropertyLimit)
        );
    }

    #[test]
    fn keyed_named_access_guards_the_property_atom() {
        let mut code = BytecodeFunction::new(2, 0);
        code.feedback_slots = alloc::vec![FeedbackKind::NamedAccess; 2];
        code.emit(Instruction::CreateArray(0));
        code.emit(Instruction::Star(Reg(0)));
        code.emit(Instruction::LdaSmi(-1));
        code.emit(Instruction::Star(Reg(1)));
        code.emit(Instruction::LdaSmi(7));
        code.emit(Instruction::SetByValue {
            obj: Reg(0),
            key: Reg(1),
            slot: 0,
        });
        code.emit(Instruction::LdaSmi(-2));
        code.emit(Instruction::Star(Reg(1)));
        code.emit(Instruction::LdaSmi(8));
        code.emit(Instruction::SetByValue {
            obj: Reg(0),
            key: Reg(1),
            slot: 0,
        });
        code.emit(Instruction::LdaSmi(-1));
        code.emit(Instruction::Star(Reg(1)));
        code.emit(Instruction::GetByValue {
            obj: Reg(0),
            key: Reg(1),
            slot: 1,
        });
        code.emit(Instruction::Return);

        let mut heap = GenerationalHeap::new();

        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(100);
        vm.set_property_limit(3);
        assert_eq!(
            vm.run(&code, &mut feedback, &mut heap, &realm),
            Ok(Value::from_smi(7))
        );
        assert!(matches!(
            feedback.get_named_ic(0),
            Some(super::super::feedback::NamedAccessIC::Polymorphic(cases))
                if cases.len() == 2 && cases[0].name != cases[1].name
        ));
    }

    #[test]
    fn bytecode_calls_use_contiguous_frames_and_record_targets() {
        let mut add = BytecodeFunction::new(2, 2);
        add.emit(Instruction::Ldar(Reg(0)));
        add.emit(Instruction::Add(Reg(1)));
        add.emit(Instruction::Return);

        let mut root = BytecodeFunction::new(3, 0);
        root.functions.push(add);
        root.feedback_slots.push(FeedbackKind::Call);
        root.emit(Instruction::CreateClosure(0));
        root.emit(Instruction::Star(Reg(0)));
        root.emit(Instruction::LdaSmi(20));
        root.emit(Instruction::Star(Reg(1)));
        root.emit(Instruction::LdaSmi(22));
        root.emit(Instruction::Star(Reg(2)));
        root.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(1),
            arg_count: 2,
            slot: 0,
        });
        root.emit(Instruction::Return);

        let mut heap = GenerationalHeap::new();

        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::with_stack_capacity(100, 8);
        let frame_capacity = vm.frames.capacity();
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap, &realm),
            Ok(Value::from_smi(42))
        );
        assert_eq!(vm.frames.capacity(), frame_capacity);
        assert!(vm.frames.is_empty());
        assert_eq!(feedback.get_call_ic(0), Some(&CallIC::Monomorphic(0)));
    }

    #[test]
    fn nested_calls_obey_frame_limits_and_vm_recovers() {
        let mut recursive = BytecodeFunction::new(1, 0);
        recursive.feedback_slots.push(FeedbackKind::Call);
        recursive.emit(Instruction::CreateClosure(0));
        recursive.emit(Instruction::Star(Reg(0)));
        recursive.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(0),
            arg_count: 0,
            slot: 0,
        });
        recursive.emit(Instruction::Return);

        let mut root = BytecodeFunction::new(1, 0);
        root.functions.push(recursive);
        root.feedback_slots.push(FeedbackKind::Call);
        root.emit(Instruction::CreateClosure(0));
        root.emit(Instruction::Star(Reg(0)));
        root.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(0),
            arg_count: 0,
            slot: 0,
        });
        root.emit(Instruction::Return);

        let mut heap = GenerationalHeap::new();

        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::with_stack_capacity(100, 8);
        vm.set_call_frame_limit(2);
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap, &realm),
            Err(VMError::CallStackOverflow)
        );

        let mut value = BytecodeFunction::new(0, 0);
        value.emit(Instruction::LdaSmi(7));
        value.emit(Instruction::Return);
        let mut feedback = FeedbackVector::for_code(&value);
        assert_eq!(
            vm.run(&value, &mut feedback, &mut heap, &realm),
            Ok(Value::from_smi(7))
        );
        assert!(vm.frames.is_empty());
    }

    #[test]
    fn callee_scavenge_forwards_caller_registers() {
        let mut allocate = BytecodeFunction::new(0, 0);
        allocate.emit(Instruction::CreateObject);
        allocate.emit(Instruction::Return);

        let mut root = BytecodeFunction::new(1, 0);
        root.functions.push(allocate);
        root.feedback_slots = alloc::vec![FeedbackKind::Call; 2];
        root.emit(Instruction::CreateClosure(0));
        root.emit(Instruction::Star(Reg(0)));
        root.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(0),
            arg_count: 0,
            slot: 0,
        });
        root.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(0),
            arg_count: 0,
            slot: 1,
        });
        root.emit(Instruction::Return);

        let mut heap = GenerationalHeap::with_nursery_capacity(1);

        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::with_stack_capacity(100, 8);
        let result = vm.run(&root, &mut feedback, &mut heap, &realm).unwrap();
        assert!(result.is_object());
        assert!(heap.get_object(result.as_object().unwrap()).is_some());
        assert_eq!(feedback.get_call_ic(0), Some(&CallIC::Monomorphic(0)));
        assert_eq!(feedback.get_call_ic(1), Some(&CallIC::Monomorphic(0)));
    }

    #[test]
    fn binary_feedback_widens_from_smi_to_number_to_generic() {
        let mut binary = BytecodeFunction::new(2, 2);
        let binary_slot = binary.allocate_feedback_slot(FeedbackKind::BinaryOp);
        binary.emit(Instruction::Ldar(Reg(0)));
        binary.emit(Instruction::Binary {
            op: BinaryOp::Add,
            rhs: Reg(1),
            slot: binary_slot,
        });
        binary.emit(Instruction::Return);

        let mut root = BytecodeFunction::new(3, 0);
        root.functions.push(binary);
        let call_slot = root.allocate_feedback_slot(FeedbackKind::Call);
        root.emit(Instruction::CreateClosure(0));
        root.emit(Instruction::Star(Reg(0)));
        root.emit(Instruction::LdaSmi(20));
        root.emit(Instruction::Star(Reg(1)));
        root.emit(Instruction::LdaSmi(22));
        root.emit(Instruction::Star(Reg(2)));
        root.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(1),
            arg_count: 2,
            slot: call_slot,
        });
        root.emit(Instruction::Return);

        let mut heap = GenerationalHeap::new();

        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::new(100);
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap, &realm),
            Ok(Value::from_smi(42))
        );
        assert_eq!(
            feedback.function(0).and_then(|vector| vector.get_binary(0)),
            Some(BinaryOpFeedback::SignedSmallInteger)
        );

        root.instructions[2] = Instruction::LdaConstant(root.add_constant(Value::from_f64(0.5)));
        vm.fuel = 100;
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap, &realm),
            Ok(Value::from_f64(22.5))
        );
        assert_eq!(
            feedback.function(0).and_then(|vector| vector.get_binary(0)),
            Some(BinaryOpFeedback::Number)
        );

        root.string_constants.push("x".encode_utf16().collect());
        root.instructions[2] = Instruction::LdaString(0);
        vm.fuel = 100;
        let value = vm.run(&root, &mut feedback, &mut heap, &realm).unwrap();
        assert_eq!(heap.strings.to_rust_string(value).as_deref(), Some("x22"));
        assert_eq!(
            feedback.function(0).and_then(|vector| vector.get_binary(0)),
            Some(BinaryOpFeedback::Generic)
        );
    }

    #[test]
    fn closure_context_load_store_and_safe_points_preserve_binding_identity() {
        let mut increment = BytecodeFunction::new(2, 0);
        increment.outer_context_slot_counts.push(1);
        increment.emit(Instruction::LoadContext { depth: 0, slot: 0 });
        increment.emit(Instruction::Star(Reg(0)));
        increment.emit(Instruction::LdaSmi(1));
        increment.emit(Instruction::Star(Reg(1)));
        increment.emit(Instruction::Ldar(Reg(0)));
        increment.emit(Instruction::Add(Reg(1)));
        increment.emit(Instruction::StoreContext { depth: 0, slot: 0 });
        increment.emit(Instruction::Return);

        let mut root = BytecodeFunction::new(2, 0);
        root.own_context_slot_count = Some(1);
        root.functions.push(increment);
        let first_call = root.allocate_feedback_slot(FeedbackKind::Call);
        let second_call = root.allocate_feedback_slot(FeedbackKind::Call);
        root.emit(Instruction::LdaSmi(40));
        root.emit(Instruction::StoreContext { depth: 0, slot: 0 });
        root.emit(Instruction::CreateClosure(0));
        root.emit(Instruction::Star(Reg(0)));
        root.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(1),
            arg_count: 0,
            slot: first_call,
        });
        root.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(1),
            arg_count: 0,
            slot: second_call,
        });
        root.emit(Instruction::Return);

        let mut heap = GenerationalHeap::with_nursery_capacity(1);

        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::new(100);
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap, &realm),
            Ok(Value::from_smi(42))
        );
    }

    #[test]
    fn native_intrinsic_calls_receive_the_method_receiver() {
        // hasOwnProperty.call({ a: 1 }, "a"), with the intrinsic passed in as a
        // parameter because it is not yet a property of %Object.prototype%.
        let mut code = BytecodeFunction::new(4, 1);
        let method = Reg(0);
        let object = Reg(1);
        let key = Reg(2);
        let receiver = Reg(3);
        let name = code.add_string_constant(alloc::vec![0x61]);
        let set_name = code.allocate_feedback_slot(FeedbackKind::NamedAccess);
        let call = code.allocate_feedback_slot(FeedbackKind::Call);

        code.emit(Instruction::CreateObject);
        code.emit(Instruction::Star(object));
        code.emit(Instruction::LdaSmi(1));
        code.emit(Instruction::SetNamed {
            obj: object,
            name,
            slot: set_name,
        });
        code.emit(Instruction::LdaString(name));
        code.emit(Instruction::Star(key));
        code.emit(Instruction::Ldar(object));
        code.emit(Instruction::Star(receiver));
        code.emit(Instruction::CallMethod {
            receiver,
            func: method,
            arg_start: key,
            arg_count: 1,
            slot: call,
        });
        code.emit(Instruction::Return);

        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let has_own = realm
            .intrinsic(&heap, Intrinsic::ObjectPrototypeHasOwnProperty)
            .unwrap();
        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(1000);
        assert_eq!(
            vm.run_with_arguments(&code, &[has_own], &mut feedback, &mut heap, &realm),
            Ok(VALUE_TRUE)
        );

        // The same call for a key the object does not own.
        let missing = code.add_string_constant(alloc::vec![0x62]);
        let mut absent = code.clone();
        absent.instructions[5] = Instruction::LdaString(missing);
        let mut feedback = FeedbackVector::for_code(&absent);
        let mut vm = RegisterVM::new(1000);
        assert_eq!(
            vm.run_with_arguments(&absent, &[has_own], &mut feedback, &mut heap, &realm),
            Ok(VALUE_FALSE)
        );
    }

    #[test]
    fn a_native_intrinsic_boxes_a_primitive_receiver_and_refuses_a_nullish_one() {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();

        // 7.1.18: a primitive receiver is boxed, undefined and null are not.
        let boxed = RegisterVM::coerce_object(Value::from_smi(1), &mut heap, &realm).unwrap();
        assert!(matches!(
            heap.get_object(boxed).unwrap().kind,
            ObjectKind::NumberWrapper(_)
        ));
        for nullish in [VALUE_UNDEFINED, VALUE_NULL] {
            let error = RegisterVM::coerce_object(nullish, &mut heap, &realm).unwrap_err();
            let VMError::Thrown(value, _) = error else {
                panic!("expected a thrown TypeError, got {error:?}");
            };
            let thrown = value.as_object().unwrap();
            let name = PropertyKey::String(heap.strings.intern("name").unwrap());
            let found = heap.lookup_named(thrown, name).unwrap().unwrap();
            assert_eq!(
                heap.strings.to_rust_string(found.value).unwrap(),
                "TypeError"
            );
        }
    }

    #[test]
    fn a_property_key_is_the_canonical_string_of_a_primitive() {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        for (value, expected) in [
            (Value::from_smi(12), "12"),
            (VALUE_TRUE, "true"),
            (VALUE_NULL, "null"),
            (VALUE_UNDEFINED, "undefined"),
        ] {
            let key = property_key(value, &mut heap).unwrap();
            assert_eq!(
                heap.strings.to_rust_string(key.to_value()).unwrap(),
                expected
            );
        }
        // An Object key needs ToPrimitive, which the engine cannot run yet.
        let object = realm.ordinary_object(&mut heap).unwrap();
        assert_eq!(
            property_key(Value::from_object(object), &mut heap),
            Err(VMError::TypeError)
        );
    }

    #[test]
    fn calling_a_value_that_is_not_callable_throws_a_type_error() {
        // 13.3.6.1: the callee is checked before the frame is entered.
        let mut code = BytecodeFunction::new(2, 0);
        let callee = Reg(0);
        let argument = Reg(1);
        let slot = code.allocate_feedback_slot(FeedbackKind::Call);
        code.emit(Instruction::LdaSmi(1));
        code.emit(Instruction::Star(callee));
        code.emit(Instruction::LdaUndefined);
        code.emit(Instruction::Star(argument));
        code.emit(Instruction::Call {
            func: callee,
            arg_start: argument,
            arg_count: 0,
            slot,
        });
        code.emit(Instruction::Return);

        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(1000);
        let error = vm
            .run(&code, &mut feedback, &mut heap, &realm)
            .expect_err("a Smi is not callable");
        let VMError::Thrown(value, _) = error else {
            panic!("expected a thrown TypeError, got {error:?}");
        };
        let thrown = value.as_object().unwrap();
        assert_eq!(
            realm.native_error_kind(&heap, thrown),
            Some(super::super::realm::NativeErrorKind::TypeError)
        );
    }

    #[test]
    fn a_thrown_type_error_reaches_a_handler_of_the_throwing_function() {
        // The unwinder treats an error the VM raises like any other value.
        let mut code = BytecodeFunction::new(3, 0);
        let callee = Reg(0);
        let argument = Reg(1);
        let caught = Reg(2);
        let slot = code.allocate_feedback_slot(FeedbackKind::Call);
        code.emit(Instruction::LdaSmi(1));
        code.emit(Instruction::Star(callee));
        code.emit(Instruction::LdaUndefined);
        code.emit(Instruction::Star(argument));
        let start = code.instructions.len();
        code.emit(Instruction::Call {
            func: callee,
            arg_start: argument,
            arg_count: 0,
            slot,
        });
        let end = code.instructions.len();
        let handler = code.instructions.len();
        code.emit(Instruction::LdaTrue);
        code.emit(Instruction::Return);
        code.handlers
            .push(super::super::bytecode::ExceptionHandler {
                start_pc: u32::try_from(start).unwrap(),
                end_pc: u32::try_from(end).unwrap(),
                handler_pc: u32::try_from(handler).unwrap(),
                exception: caught,
            });

        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(1000);
        assert_eq!(
            vm.run(&code, &mut feedback, &mut heap, &realm),
            Ok(VALUE_TRUE)
        );
    }

    #[test]
    fn an_array_iterator_yields_every_element_and_then_reports_done() {
        // 23.1.3.38 and 23.1.5.2.1, driven by IteratorNext over two registers.
        let mut code = BytecodeFunction::new(4, 0);
        let array = Reg(0);
        let method = Reg(1);
        let iterator = Reg(2);
        let value = Reg(3);
        let values = code.add_string_constant("values".encode_utf16().collect());
        let get = code.allocate_feedback_slot(FeedbackKind::NamedAccess);
        let call = code.allocate_feedback_slot(FeedbackKind::Call);

        code.emit(Instruction::CreateArray(0));
        code.emit(Instruction::Star(array));
        code.emit(Instruction::LdaSmi(7));
        code.emit(Instruction::Star(value));
        code.emit(Instruction::LdaSmi(0));
        code.emit(Instruction::Star(method));
        code.emit(Instruction::Ldar(value));
        code.emit(Instruction::SetByValue {
            obj: array,
            key: method,
            slot: get,
        });
        code.emit(Instruction::GetNamed {
            obj: array,
            name: values,
            slot: get,
        });
        code.emit(Instruction::Star(method));
        code.emit(Instruction::CallMethod {
            receiver: array,
            func: method,
            arg_start: method,
            arg_count: 0,
            slot: call,
        });
        code.emit(Instruction::Star(iterator));
        code.emit(Instruction::LdaUndefined);
        code.emit(Instruction::Star(value));
        code.emit(Instruction::IteratorNext { state: iterator });
        code.emit(Instruction::JumpIfFalse(2));
        code.emit(Instruction::Ldar(value));
        code.emit(Instruction::Return);
        code.emit(Instruction::LdaNull);
        code.emit(Instruction::Return);

        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(1000);
        assert_eq!(
            vm.run(&code, &mut feedback, &mut heap, &realm),
            Ok(Value::from_smi(7))
        );
    }

    #[test]
    fn iterator_next_refuses_what_it_cannot_step() {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let mut vm = RegisterVM::with_limits(1000, 8, 8);

        // A primitive is not an iterator.
        vm.write_reg(Reg(0), Value::from_smi(1)).unwrap();
        assert!(matches!(
            vm.iterator_next(Reg(0), &mut heap, &realm),
            Err(VMError::Thrown(
                _,
                Some((super::super::realm::NativeErrorKind::TypeError, _))
            ))
        ));

        // An object without a callable `next` is not one either.
        let plain = realm.ordinary_object(&mut heap).unwrap();
        vm.write_reg(Reg(0), Value::from_object(plain)).unwrap();
        assert!(matches!(
            vm.iterator_next(Reg(0), &mut heap, &realm),
            Err(VMError::Thrown(
                _,
                Some((super::super::realm::NativeErrorKind::TypeError, _))
            ))
        ));

        // 23.1.5.2.1 refuses a receiver that is not an Array Iterator.
        let next = realm
            .intrinsic(&heap, Intrinsic::ArrayIteratorPrototypeNext)
            .unwrap();
        let key = super::super::value::PropertyKey::String(heap.strings.intern("next").unwrap());
        heap.define_own_named(plain, key, next, PropertyFlags::ordinary_data())
            .unwrap();
        assert!(matches!(
            vm.iterator_next(Reg(0), &mut heap, &realm),
            Err(VMError::Thrown(
                _,
                Some((super::super::realm::NativeErrorKind::TypeError, _))
            ))
        ));
    }

    #[test]
    fn an_intrinsic_group_refuses_an_intrinsic_of_another_group() {
        // Each group's dispatch is exhaustive over its own members; a member of
        // another group is not one of them.
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let vm = RegisterVM::new(100);
        let call = Call {
            receiver: VALUE_UNDEFINED,
            func: Reg(0),
            arg_start: Reg(0),
            arg_count: 0,
            slot: 0,
            return_pc: 0,
            caller_code_id: None,
        };
        assert_eq!(
            RegisterVM::call_iterator_intrinsic(
                Intrinsic::ObjectPrototypeToString,
                call,
                &mut heap,
                &realm
            ),
            Err(VMError::InvalidFeedbackVector)
        );
        let string_call = Call {
            receiver: vm.allocate_string(&mut heap, &[0x61]).unwrap(),
            ..call
        };
        assert_eq!(
            vm.call_string_intrinsic(
                Intrinsic::ObjectPrototypeToString,
                string_call,
                &mut heap,
                &realm
            ),
            Err(VMError::InvalidFeedbackVector)
        );
    }
}
