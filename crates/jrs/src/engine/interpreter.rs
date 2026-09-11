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
    shape::{PropertyFlags, ShapeId},
    string::StringError,
    value::{ObjectRef, VALUE_FALSE, VALUE_NULL, VALUE_TRUE, VALUE_UNDEFINED, Value},
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
        shape: ShapeId,
    ) -> Result<ObjectRef, VMError> {
        loop {
            match heap.allocate_object(shape, VALUE_NULL) {
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
        length: u32,
    ) -> Result<ObjectRef, VMError> {
        loop {
            match heap.allocate_array(length) {
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
        code_id: u32,
        captures_context: bool,
    ) -> Result<ObjectRef, VMError> {
        loop {
            let context = if captures_context {
                Some(self.current_context.ok_or(VMError::InvalidRegister)?)
            } else {
                None
            };
            match heap.allocate_function(code_id, context) {
                Ok(reference) => return Ok(reference),
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

    /// Executes a bytecode function against the heap with active feedback caching.
    ///
    /// # Errors
    /// Returns [`VMError`] on out of fuel, type errors, invalid registers, or stack overflow.
    pub fn run(
        &mut self,
        code: &BytecodeFunction,
        feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
    ) -> Result<Value, VMError> {
        self.run_with_arguments(code, &[], feedback, heap)
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
    #[expect(clippy::too_many_lines, reason = "central bytecode dispatch loop")]
    pub fn run_with_arguments(
        &mut self,
        code: &BytecodeFunction,
        arguments: &[Value],
        feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
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
                            self.acc = Value::from_smi(res);
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
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        self.acc = Value::from_smi(a & b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::BitOr(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        self.acc = Value::from_smi(a | b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::BitXor(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        self.acc = Value::from_smi(a ^ b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Shl(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        #[expect(clippy::as_conversions, reason = "shift count masked")]
                        let shift = (b as u32) & 0x1F;
                        self.acc = Value::from_smi(a << shift);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Shr(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        #[expect(clippy::as_conversions, reason = "shift count masked")]
                        let shift = (b as u32) & 0x1F;
                        self.acc = Value::from_smi(a >> shift);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::TestEqual(reg) | Instruction::TestStrictEqual(reg) => {
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
                Instruction::GetNamed { obj, name, slot } => {
                    let name = active_code
                        .string_constants
                        .get(name as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = heap.strings.intern_units(name)?;
                    let target = self.read_reg(obj)?;
                    let oref = target.as_object().ok_or(VMError::TypeError)?;
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
                        continue;
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
                    let name = heap.strings.intern_units(name)?;
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
                        continue;
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
                    let oref = target.as_object().ok_or(VMError::TypeError)?;
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
                            continue;
                        }
                        let Some(name) = heap.strings.lookup_interned_units(&name) else {
                            self.acc = VALUE_UNDEFINED;
                            continue;
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
                            continue;
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
                                name
                            } else {
                                if heap.own_property_count(oref).unwrap_or(usize::MAX)
                                    >= self.property_limit
                                {
                                    return Err(VMError::PropertyLimit);
                                }
                                heap.strings.intern_units(&name_units)?
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
                            continue;
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
                    let oref = self.allocate_object(active_code, heap, root_shape)?;
                    self.acc = Value::from_object(oref);
                }
                Instruction::CreateArray(length) => {
                    if self.property_limit == 0 {
                        return Err(VMError::PropertyLimit);
                    }
                    let oref = self.allocate_array(active_code, heap, length)?;
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
                    let function =
                        self.allocate_function(active_code, heap, code_id, captures_context)?;
                    self.acc = Value::from_object(function);
                }
                Instruction::Call {
                    func,
                    arg_start,
                    arg_count,
                    slot,
                } => {
                    let function = self.read_reg(func)?;
                    let function_ref = function.as_object().ok_or(VMError::TypeError)?;
                    let function = heap.get_object(function_ref).ok_or(VMError::TypeError)?;
                    let super::object::ObjectKind::Function { code_id, context } = function.kind
                    else {
                        return Err(VMError::TypeError);
                    };
                    let callee =
                        code.functions
                            .get(code_id as usize)
                            .ok_or(VMError::InvalidBytecode(
                                VerificationError::FunctionOutOfBounds {
                                    pc: pc.saturating_sub(1),
                                    index: code_id,
                                },
                            ))?;
                    if self.frames.len() >= self.call_frame_limit
                        || self.frames.len() == self.frames.capacity()
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
                        .checked_add(arg_start.0 as usize)
                        .ok_or(VMError::StackOverflow)?;
                    for index in 0..usize::from(arg_count.min(callee.parameter_count)) {
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
                            .ok_or(VMError::StackOverflow)? = Value::from_object(function_ref);
                    }
                    active_feedback
                        .record_call(slot, code_id)
                        .ok_or(VMError::InvalidFeedbackVector)?;
                    let caller_context = self.current_context;
                    self.frames.push(FrameHeader {
                        caller_fp: self.fp,
                        return_pc: pc,
                        caller_code_id: current_code_id,
                        caller_binding_count: self.active_binding_count,
                        caller_context,
                    });
                    self.fp = next_frame;
                    self.active_binding_count = callee_bindings;
                    current_code_id = Some(code_id);
                    self.current_context = context;
                    self.current_context = if let Some(slot_count) = callee.own_context_slot_count {
                        Some(self.allocate_context(
                            callee,
                            heap,
                            self.current_context,
                            slot_count,
                        )?)
                    } else {
                        self.current_context
                    };
                    pc = 0;
                    self.acc = VALUE_UNDEFINED;
                }
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
                        return Ok(self.acc);
                    }
                }
            }
        }
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
        let mut feedback = FeedbackVector::for_code(&code);

        let res = vm.run(&code, &mut feedback, &mut heap).unwrap();
        assert_eq!(res.as_smi(), Some(45));
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

        let res = vm.run(&code, &mut feedback, &mut heap).unwrap();
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
        let mut feedback = FeedbackVector::new(0);
        let mut vm = RegisterVM::new(100);

        let result = vm.run(&code, &mut feedback, &mut heap).unwrap();
        let first = result.as_object().unwrap();
        assert!(first.is_old());
        assert!(heap.get_object(first).is_some());
    }

    #[test]
    fn run_rejects_unverified_code_and_mismatched_feedback() {
        let mut heap = GenerationalHeap::new();
        let mut vm = RegisterVM::new(100);
        let mut malformed = BytecodeFunction::new(0, 0);
        malformed.emit(Instruction::Ldar(Reg(0)));
        malformed.emit(Instruction::Return);
        let mut feedback = FeedbackVector::new(0);
        assert_eq!(
            vm.run(&malformed, &mut feedback, &mut heap),
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
            vm.run(&valid, &mut feedback, &mut heap),
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
                vm.run_with_arguments(&code, &[value], &mut feedback, &mut heap),
                Err(VMError::TypeError)
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
        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(100);
        vm.set_property_limit(2);
        assert_eq!(
            vm.run(&code, &mut feedback, &mut heap),
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
        let mut feedback = FeedbackVector::for_code(&code);
        let mut vm = RegisterVM::new(100);
        vm.set_property_limit(3);
        assert_eq!(
            vm.run(&code, &mut feedback, &mut heap),
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
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::with_stack_capacity(100, 8);
        let frame_capacity = vm.frames.capacity();
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap),
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
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::with_stack_capacity(100, 8);
        vm.set_call_frame_limit(2);
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap),
            Err(VMError::CallStackOverflow)
        );

        let mut value = BytecodeFunction::new(0, 0);
        value.emit(Instruction::LdaSmi(7));
        value.emit(Instruction::Return);
        let mut feedback = FeedbackVector::for_code(&value);
        assert_eq!(
            vm.run(&value, &mut feedback, &mut heap),
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
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::with_stack_capacity(100, 8);
        let result = vm.run(&root, &mut feedback, &mut heap).unwrap();
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
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::new(100);
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap),
            Ok(Value::from_smi(42))
        );
        assert_eq!(
            feedback.function(0).and_then(|vector| vector.get_binary(0)),
            Some(BinaryOpFeedback::SignedSmallInteger)
        );

        root.instructions[2] = Instruction::LdaConstant(root.add_constant(Value::from_f64(0.5)));
        vm.fuel = 100;
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap),
            Ok(Value::from_f64(22.5))
        );
        assert_eq!(
            feedback.function(0).and_then(|vector| vector.get_binary(0)),
            Some(BinaryOpFeedback::Number)
        );

        root.string_constants.push("x".encode_utf16().collect());
        root.instructions[2] = Instruction::LdaString(0);
        vm.fuel = 100;
        let value = vm.run(&root, &mut feedback, &mut heap).unwrap();
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
        let mut feedback = FeedbackVector::for_code(&root);
        let mut vm = RegisterVM::new(100);
        assert_eq!(
            vm.run(&root, &mut feedback, &mut heap),
            Ok(Value::from_smi(42))
        );
    }
}
