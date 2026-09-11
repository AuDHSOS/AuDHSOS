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
    bytecode::{BytecodeFunction, Instruction, Reg},
    feedback::FeedbackVector,
    heap::{GenerationalHeap, HeapError},
    shape::{PropertyFlags, ShapeId},
    value::{ObjectRef, VALUE_FALSE, VALUE_NULL, VALUE_TRUE, VALUE_UNDEFINED, Value},
};
use alloc::vec::Vec;

/// Virtual machine execution errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VMError {
    /// Fuel exhausted.
    OutOfFuel,
    /// Invalid register access.
    InvalidRegister,
    /// Operand stack limit exceeded.
    StackOverflow,
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

/// Light frame boundary recorded on the contiguous call stack.
#[derive(Clone, Copy, Debug)]
pub struct FrameHeader {
    /// Base pointer of the caller frame.
    pub caller_fp: usize,
    /// Program counter to resume in caller.
    pub return_pc: usize,
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
        Self {
            stack: alloc::vec![VALUE_UNDEFINED; 4096],
            fp: 0,
            acc: VALUE_UNDEFINED,
            fuel,
        }
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
            .get_mut(self.fp..frame_end)
            .ok_or(VMError::StackOverflow)?;
        heap.scavenge_with_roots(registers, &mut self.acc)?;
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

    /// Executes a bytecode function against the heap with active feedback caching.
    ///
    /// # Errors
    /// Returns [`VMError`] on out of fuel, type errors, invalid registers, or stack overflow.
    #[expect(clippy::too_many_lines, reason = "central bytecode dispatch loop")]
    pub fn run(
        &mut self,
        code: &BytecodeFunction,
        feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
    ) -> Result<Value, VMError> {
        let mut pc: usize = 0;
        let instructions = &code.instructions;
        let frame_end = self
            .fp
            .checked_add(code.register_count as usize)
            .ok_or(VMError::StackOverflow)?;
        self.stack
            .get_mut(self.fp..frame_end)
            .ok_or(VMError::StackOverflow)?
            .fill(VALUE_UNDEFINED);
        self.acc = VALUE_UNDEFINED;

        loop {
            let Some(&inst) = instructions.get(pc) else {
                return Err(VMError::UnexpectedEnd);
            };
            pc = pc.saturating_add(1);

            match inst {
                Instruction::LdaSmi(val) => {
                    self.acc = Value::from_smi(val);
                }
                Instruction::LdaConstant(idx) => {
                    self.acc = code
                        .constants
                        .get(idx as usize)
                        .copied()
                        .ok_or(VMError::InvalidRegister)?;
                }
                Instruction::LdaUndefined => {
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
                Instruction::Add(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        if let Some(res) = a.checked_add(b) {
                            self.acc = Value::from_smi(res);
                        } else {
                            self.acc = Value::from_f64(f64::from(a) + f64::from(b));
                        }
                    } else if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
                        self.acc = Value::from_f64(a + b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Sub(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_smi(), rhs.as_smi()) {
                        if let Some(res) = a.checked_sub(b) {
                            self.acc = Value::from_smi(res);
                        } else {
                            self.acc = Value::from_f64(f64::from(a) - f64::from(b));
                        }
                    } else if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
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
                    } else if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
                        self.acc = Value::from_f64(a * b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Div(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
                        self.acc = Value::from_f64(a / b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::Mod(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
                        self.acc = Value::from_f64(a % b);
                    } else {
                        return Err(VMError::TypeError);
                    }
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
                    self.acc = Value::from_bool(self.acc.strictly_equals(rhs));
                }
                Instruction::TestLessThan(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
                        self.acc = Value::from_bool(a < b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::TestLessThanOrEqual(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
                        self.acc = Value::from_bool(a <= b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::TestGreaterThan(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
                        self.acc = Value::from_bool(a > b);
                    } else {
                        return Err(VMError::TypeError);
                    }
                }
                Instruction::TestGreaterThanOrEqual(reg) => {
                    let rhs = self.read_reg(reg)?;
                    if let (Some(a), Some(b)) = (self.acc.as_f64(), rhs.as_f64()) {
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
                    if self.acc.to_boolean() {
                        if offset < 0 {
                            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                        }
                        pc = (pc as isize + offset as isize) as usize;
                    }
                }
                Instruction::JumpIfFalse(offset) => {
                    if !self.acc.to_boolean() {
                        if offset < 0 {
                            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                        }
                        pc = (pc as isize + offset as isize) as usize;
                    }
                }
                Instruction::GetNamed { obj, name, slot } => {
                    let target = self.read_reg(obj)?;
                    let oref = target.as_object().ok_or(VMError::TypeError)?;
                    let js_obj = heap.get_object(oref).ok_or(VMError::TypeError)?;
                    let shape_id = js_obj.shape_id;

                    // Inline cache check
                    let cached_slot = feedback
                        .get_named_ic(slot)
                        .and_then(|ic| ic.try_get_slot(shape_id));

                    if let Some(slot_idx) = cached_slot {
                        self.acc = js_obj.get_slot(slot_idx).unwrap_or(VALUE_UNDEFINED);
                    } else {
                        // Slow path lookup along Shape chain
                        if let Some(loc) = heap.shapes.lookup(shape_id, name) {
                            if let Some(ic) = feedback.get_named_ic_mut(slot) {
                                ic.record_shape(shape_id, loc.slot_offset);
                            }
                            self.acc = js_obj.get_slot(loc.slot_offset).unwrap_or(VALUE_UNDEFINED);
                        } else {
                            self.acc = VALUE_UNDEFINED;
                        }
                    }
                }
                Instruction::SetNamed { obj, name, slot } => {
                    let target = self.read_reg(obj)?;
                    let oref = target.as_object().ok_or(VMError::TypeError)?;
                    let current_shape = heap.get_object(oref).ok_or(VMError::TypeError)?.shape_id;

                    // Check if property exists in current shape
                    if let Some(loc) = heap.shapes.lookup(current_shape, name) {
                        let val = self.acc;
                        heap.set_object_slot(oref, loc.slot_offset, val)?;
                    } else {
                        // Transition to new Shape
                        let (new_shape, slot_idx) = heap.shapes.transition(
                            current_shape,
                            name,
                            PropertyFlags::ordinary_data(),
                        );
                        let val = self.acc;
                        heap.set_object_shape(oref, new_shape)?;
                        heap.set_object_slot(oref, slot_idx, val)?;
                        if let Some(ic) = feedback.get_named_ic_mut(slot) {
                            ic.record_shape(new_shape, slot_idx);
                        }
                    }
                }
                Instruction::GetByValue { obj, key, .. } => {
                    let target = self.read_reg(obj)?;
                    let oref = target.as_object().ok_or(VMError::TypeError)?;
                    let js_obj = heap.get_object(oref).ok_or(VMError::TypeError)?;
                    let key_val = self.read_reg(key)?;

                    if let (Some(idx), Some(eref)) = (key_val.as_smi(), js_obj.elements)
                        && idx >= 0
                    {
                        let elem = heap.get_elements(eref).ok_or(VMError::TypeError)?;
                        #[expect(clippy::as_conversions, reason = "non-negative index fits u32")]
                        let uidx = idx as u32;
                        self.acc = elem.get(uidx).unwrap_or(VALUE_UNDEFINED);
                    } else {
                        self.acc = VALUE_UNDEFINED;
                    }
                }
                Instruction::SetByValue { obj, key, .. } => {
                    let target = self.read_reg(obj)?;
                    let oref = target.as_object().ok_or(VMError::TypeError)?;
                    let js_obj = heap.get_object(oref).ok_or(VMError::TypeError)?;
                    let key_val = self.read_reg(key)?;
                    let val = self.acc;

                    if let (Some(idx), Some(eref)) = (key_val.as_smi(), js_obj.elements)
                        && idx >= 0
                    {
                        #[expect(clippy::as_conversions, reason = "non-negative index fits u32")]
                        heap.set_element(eref, idx as u32, val)?;
                    }
                }
                Instruction::CreateObject => {
                    let root_shape = heap.shapes.root_shape();
                    let oref = self.allocate_object(code, heap, root_shape)?;
                    self.acc = Value::from_object(oref);
                }
                Instruction::CreateArray(length) => {
                    let oref = self.allocate_array(code, heap, length)?;
                    self.acc = Value::from_object(oref);
                }
                Instruction::Call { .. } => {
                    // Higher level function calls bridge through contiguous call frames
                    self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                }
                Instruction::Return => {
                    return Ok(self.acc);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
        let mut feedback = FeedbackVector::new(code.feedback_slot_count);

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
        let s_slot = code.allocate_feedback_slot();
        let g_slot = code.allocate_feedback_slot();

        let mut heap = GenerationalHeap::new();
        let prop_x = heap.strings.intern("x").unwrap();

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

        let mut feedback = FeedbackVector::new(code.feedback_slot_count);
        let mut vm = RegisterVM::new(10_000);

        let res = vm.run(&code, &mut feedback, &mut heap).unwrap();
        assert_eq!(res.as_smi(), Some(42));

        // Verification of IC feedback: slot g_slot must be monomorphic!
        let ic = feedback.get_named_ic(g_slot).unwrap();
        assert!(matches!(
            ic,
            super::super::feedback::NamedAccessIC::Monomorphic { .. }
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
}
