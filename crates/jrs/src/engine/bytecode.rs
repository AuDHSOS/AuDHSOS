// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Register-based Bytecode Instruction Set.
//!
//! Instructions operate on a register file (r0..rn) and an explicit
//! accumulator register (acc). This mirrors modern production engines (Ignition)
//! and eliminates the stack push/pop dispatch overhead.

use super::value::Value;
use alloc::{collections::VecDeque, vec::Vec};

/// Virtual register index inside a function's call frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reg(pub u16);

/// Structural verification failure in register bytecode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationError {
    /// The function contains no instruction.
    EmptyFunction,
    /// The parameter window does not fit in the register frame.
    ParametersExceedRegisters,
    /// The active binding prefix does not fit in the register frame.
    BindingsExceedRegisters,
    /// An instruction addresses a register outside the frame.
    RegisterOutOfBounds {
        /// Instruction offset.
        pc: usize,
        /// Invalid register.
        register: Reg,
    },
    /// A constant-pool index is outside the pool.
    ConstantOutOfBounds {
        /// Instruction offset.
        pc: usize,
        /// Invalid constant index.
        index: u16,
    },
    /// A UTF-16 string constant index is outside the string pool.
    StringConstantOutOfBounds {
        /// Instruction offset.
        pc: usize,
        /// Invalid string constant index.
        index: u16,
    },
    /// A generic constant embeds an Agent-local heap identity.
    HeapBoundConstant {
        /// Invalid constant index.
        index: usize,
    },
    /// A feedback slot is outside the function's feedback vector.
    FeedbackOutOfBounds {
        /// Instruction offset.
        pc: usize,
        /// Invalid feedback slot.
        slot: u16,
    },
    /// A Call argument window leaves the register frame.
    CallArgumentsOutOfBounds {
        /// Instruction offset.
        pc: usize,
    },
    /// A closure creation references a function outside the shared code table.
    FunctionOutOfBounds {
        /// Instruction offset.
        pc: usize,
        /// Invalid function index.
        index: u32,
    },
    /// An entry in the shared flat function table carries an unreachable nested table.
    NestedFunctionTable {
        /// Invalid top-level function index.
        index: u32,
    },
    /// A relative branch target is outside the instruction array.
    JumpOutOfBounds {
        /// Instruction offset.
        pc: usize,
    },
    /// A reachable control-flow path falls beyond the final instruction.
    ReachableFallthrough {
        /// Instruction offset whose fallthrough leaves the function.
        pc: usize,
    },
}

/// Register-based bytecode instruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Instruction {
    /// `acc = Smi(value)`
    LdaSmi(i32),
    /// `acc = constants[index]`
    LdaConstant(u16),
    /// `acc = strings[index]`, materialized in the current Agent's string arena.
    LdaString(u16),
    /// `acc = undefined`
    LdaUndefined,
    /// `acc = null`
    LdaNull,
    /// `acc = true`
    LdaTrue,
    /// `acc = false`
    LdaFalse,
    /// `acc = -acc`
    Negate,
    /// `acc = !ToBoolean(acc)`
    LogicalNot,
    /// `acc = undefined`, after evaluating its operand.
    ToUndefined,
    /// `acc = ~ToInt32(acc)` for an already numeric primitive.
    BitNot,
    /// `acc = reg`
    Ldar(Reg),
    /// `reg = acc`
    Star(Reg),
    /// `dst = src`
    Mov {
        /// Source register.
        src: Reg,
        /// Destination register.
        dst: Reg,
    },
    /// `acc = acc + reg`
    Add(Reg),
    /// `acc = acc - reg`
    Sub(Reg),
    /// `acc = acc * reg`
    Mul(Reg),
    /// `acc = acc / reg`
    Div(Reg),
    /// `acc = acc % reg`
    Mod(Reg),
    /// `acc = acc & reg`
    BitAnd(Reg),
    /// `acc = acc | reg`
    BitOr(Reg),
    /// `acc = acc ^ reg`
    BitXor(Reg),
    /// `acc = acc << reg`
    Shl(Reg),
    /// `acc = acc >> reg`
    Shr(Reg),
    /// `acc = (acc == reg)`
    TestEqual(Reg),
    /// `acc = (acc === reg)`
    TestStrictEqual(Reg),
    /// `acc = (acc < reg)`
    TestLessThan(Reg),
    /// `acc = (acc <= reg)`
    TestLessThanOrEqual(Reg),
    /// `acc = (acc > reg)`
    TestGreaterThan(Reg),
    /// `acc = (acc >= reg)`
    TestGreaterThanOrEqual(Reg),
    /// Unconditional jump by relative instruction offset.
    Jump(i32),
    /// Jump if `acc` is truthy.
    JumpIfTrue(i32),
    /// Jump if `acc` is falsy.
    JumpIfFalse(i32),
    /// Load named property: `acc = obj_reg[name]` (uses feedback slot).
    GetNamed {
        /// Object register.
        obj: Reg,
        /// Property-name index in the heap-independent UTF-16 constant pool.
        name: u16,
        /// Feedback vector slot for inline caching.
        slot: u16,
    },
    /// Store named property: `obj_reg[name] = acc` (uses feedback slot).
    SetNamed {
        /// Object register.
        obj: Reg,
        /// Property-name index in the heap-independent UTF-16 constant pool.
        name: u16,
        /// Feedback vector slot for inline caching.
        slot: u16,
    },
    /// Load indexed element: `acc = obj_reg[key_reg]` (uses feedback slot).
    GetByValue {
        /// Object register.
        obj: Reg,
        /// Key/index register.
        key: Reg,
        /// Feedback vector slot for inline caching.
        slot: u16,
    },
    /// Store indexed element: `obj_reg[key_reg] = acc` (uses feedback slot).
    SetByValue {
        /// Object register.
        obj: Reg,
        /// Key/index register.
        key: Reg,
        /// Feedback vector slot for inline caching.
        slot: u16,
    },
    /// Loads an Array exotic object's `length` data property.
    GetArrayLength {
        /// Array object register.
        obj: Reg,
    },
    /// Creates an empty object `{}` in `acc`.
    CreateObject,
    /// Creates an empty array `[]` in `acc` with initial capacity.
    CreateArray(u32),
    /// Creates a callable closure for one entry in the shared function table.
    CreateClosure(u32),
    /// Call function: `acc = func(arg_start..arg_start + count)` (uses feedback slot).
    Call {
        /// Callable function register.
        func: Reg,
        /// First argument register.
        arg_start: Reg,
        /// Number of arguments passed.
        arg_count: u16,
        /// Feedback vector slot for call target caching.
        slot: u16,
    },
    /// Return `acc` to caller.
    Return,
}

/// Compiled bytecode unit for a function or top-level script.
#[derive(Clone, Debug)]
pub struct BytecodeFunction {
    /// Sequence of bytecode instructions.
    pub instructions: Vec<Instruction>,
    /// Constant pool referenced by `LdaConstant`.
    pub constants: Vec<Value>,
    /// Heap-independent UTF-16 constants referenced by `LdaString`.
    pub string_constants: Vec<Vec<u16>>,
    /// Heap-independent nested function code addressed by `CreateClosure`.
    pub functions: Vec<BytecodeFunction>,
    /// Number of local registers required in the stack frame.
    pub register_count: u16,
    /// Number of formal parameters expected.
    pub parameter_count: u16,
    /// Number of parameter/local binding registers charged to the active binding budget.
    pub binding_count: u16,
    /// Number of feedback vector slots allocated for inline caches.
    pub feedback_slot_count: u16,
    /// Fuel charged once in the function prologue.
    pub entry_fuel_cost: u64,
    /// Legacy operand-stack capacity required by the selected source program.
    pub entry_stack_requirement: usize,
}

impl BytecodeFunction {
    /// Creates a new empty bytecode function.
    #[must_use]
    pub const fn new(register_count: u16, parameter_count: u16) -> Self {
        Self {
            instructions: Vec::new(),
            constants: Vec::new(),
            string_constants: Vec::new(),
            functions: Vec::new(),
            register_count,
            parameter_count,
            binding_count: parameter_count,
            feedback_slot_count: 0,
            entry_fuel_cost: 1,
            entry_stack_requirement: 0,
        }
    }

    /// Emits an instruction and returns its program counter offset.
    pub fn emit(&mut self, inst: Instruction) -> usize {
        let pc = self.instructions.len();
        self.instructions.push(inst);
        pc
    }

    /// Adds a constant to the constant pool, returning its index.
    pub fn add_constant(&mut self, val: Value) -> u16 {
        let idx = self.constants.len();
        self.constants.push(val);
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "constant pool size fits in u16"
        )]
        (idx as u16)
    }

    /// Adds a heap-independent UTF-16 string constant, returning its index.
    pub fn add_string_constant(&mut self, units: Vec<u16>) -> u16 {
        let index = self.string_constants.len();
        self.string_constants.push(units);
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "constant pool size is verified before generated bytecode is published"
        )]
        (index as u16)
    }

    /// Allocates an inline cache slot in the feedback vector.
    pub const fn allocate_feedback_slot(&mut self) -> u16 {
        let slot = self.feedback_slot_count;
        self.feedback_slot_count = self.feedback_slot_count.saturating_add(1);
        slot
    }

    /// Verifies operand bounds and reachable control flow.
    ///
    /// # Errors
    ///
    /// Returns a [`VerificationError`] for malformed bytecode. Every instruction
    /// is checked, including unreachable instructions, before the control-flow
    /// traversal begins.
    pub fn verify(&self) -> Result<(), VerificationError> {
        self.verify_unit(&self.functions)?;
        for (index, function) in self.functions.iter().enumerate() {
            if !function.functions.is_empty() {
                return Err(VerificationError::NestedFunctionTable {
                    index: u32::try_from(index).unwrap_or(u32::MAX),
                });
            }
            function.verify_unit(&self.functions)?;
        }
        Ok(())
    }

    fn verify_unit(&self, functions: &[Self]) -> Result<(), VerificationError> {
        if self.instructions.is_empty() {
            return Err(VerificationError::EmptyFunction);
        }
        if self.parameter_count > self.register_count {
            return Err(VerificationError::ParametersExceedRegisters);
        }
        if self.binding_count > self.register_count || self.parameter_count > self.binding_count {
            return Err(VerificationError::BindingsExceedRegisters);
        }
        for (index, constant) in self.constants.iter().enumerate() {
            if constant.is_object()
                || constant.is_heap_string()
                || constant.is_symbol()
                || constant.is_bigint()
            {
                return Err(VerificationError::HeapBoundConstant { index });
            }
        }
        for (pc, instruction) in self.instructions.iter().enumerate() {
            self.verify_instruction(pc, *instruction, functions)?;
        }

        let mut seen = alloc::vec![false; self.instructions.len()];
        let mut work = VecDeque::from([0usize]);
        while let Some(pc) = work.pop_front() {
            if seen.get(pc).copied().unwrap_or(false) {
                continue;
            }
            *seen
                .get_mut(pc)
                .ok_or(VerificationError::ReachableFallthrough { pc })? = true;
            let instruction = *self
                .instructions
                .get(pc)
                .ok_or(VerificationError::ReachableFallthrough { pc })?;
            match instruction {
                Instruction::Return => {}
                Instruction::Jump(offset) => {
                    work.push_back(self.jump_target(pc, offset)?);
                }
                Instruction::JumpIfTrue(offset) | Instruction::JumpIfFalse(offset) => {
                    work.push_back(self.jump_target(pc, offset)?);
                    work.push_back(self.fallthrough(pc)?);
                }
                _ => work.push_back(self.fallthrough(pc)?),
            }
        }
        Ok(())
    }

    fn verify_instruction(
        &self,
        pc: usize,
        instruction: Instruction,
        functions: &[Self],
    ) -> Result<(), VerificationError> {
        let register = match instruction {
            Instruction::Ldar(register)
            | Instruction::Star(register)
            | Instruction::Add(register)
            | Instruction::Sub(register)
            | Instruction::Mul(register)
            | Instruction::Div(register)
            | Instruction::Mod(register)
            | Instruction::BitAnd(register)
            | Instruction::BitOr(register)
            | Instruction::BitXor(register)
            | Instruction::Shl(register)
            | Instruction::Shr(register)
            | Instruction::TestEqual(register)
            | Instruction::TestStrictEqual(register)
            | Instruction::TestLessThan(register)
            | Instruction::TestLessThanOrEqual(register)
            | Instruction::TestGreaterThan(register)
            | Instruction::TestGreaterThanOrEqual(register) => Some(register),
            Instruction::Mov { src, dst } => {
                self.verify_register(pc, src)?;
                Some(dst)
            }
            Instruction::GetNamed { obj, name, slot }
            | Instruction::SetNamed { obj, name, slot } => {
                self.verify_string_constant(pc, name)?;
                self.verify_feedback(pc, slot)?;
                Some(obj)
            }
            Instruction::GetByValue { obj, key, slot, .. }
            | Instruction::SetByValue { obj, key, slot, .. } => {
                self.verify_register(pc, obj)?;
                self.verify_feedback(pc, slot)?;
                Some(key)
            }
            Instruction::GetArrayLength { obj } => Some(obj),
            Instruction::Call {
                func,
                arg_start,
                arg_count,
                slot,
            } => {
                self.verify_register(pc, func)?;
                self.verify_register(pc, arg_start)?;
                self.verify_feedback(pc, slot)?;
                let end = u32::from(arg_start.0).saturating_add(u32::from(arg_count));
                if end > u32::from(self.register_count) {
                    return Err(VerificationError::CallArgumentsOutOfBounds { pc });
                }
                None
            }
            Instruction::LdaConstant(index) => {
                if usize::from(index) >= self.constants.len() {
                    return Err(VerificationError::ConstantOutOfBounds { pc, index });
                }
                None
            }
            Instruction::LdaString(index) => {
                if usize::from(index) >= self.string_constants.len() {
                    return Err(VerificationError::StringConstantOutOfBounds { pc, index });
                }
                None
            }
            Instruction::CreateClosure(index) => {
                if usize::try_from(index)
                    .ok()
                    .as_ref()
                    .is_none_or(|index| *index >= functions.len())
                {
                    return Err(VerificationError::FunctionOutOfBounds { pc, index });
                }
                None
            }
            Instruction::Jump(offset)
            | Instruction::JumpIfTrue(offset)
            | Instruction::JumpIfFalse(offset) => {
                self.jump_target(pc, offset)?;
                None
            }
            Instruction::LdaSmi(_)
            | Instruction::Negate
            | Instruction::LogicalNot
            | Instruction::ToUndefined
            | Instruction::BitNot
            | Instruction::LdaUndefined
            | Instruction::LdaNull
            | Instruction::LdaTrue
            | Instruction::LdaFalse
            | Instruction::CreateObject
            | Instruction::CreateArray(_)
            | Instruction::Return => None,
        };
        if let Some(register) = register {
            self.verify_register(pc, register)?;
        }
        Ok(())
    }

    const fn verify_register(&self, pc: usize, register: Reg) -> Result<(), VerificationError> {
        if register.0 >= self.register_count {
            Err(VerificationError::RegisterOutOfBounds { pc, register })
        } else {
            Ok(())
        }
    }

    const fn verify_feedback(&self, pc: usize, slot: u16) -> Result<(), VerificationError> {
        if slot >= self.feedback_slot_count {
            Err(VerificationError::FeedbackOutOfBounds { pc, slot })
        } else {
            Ok(())
        }
    }

    fn verify_string_constant(&self, pc: usize, index: u16) -> Result<(), VerificationError> {
        if usize::from(index) >= self.string_constants.len() {
            Err(VerificationError::StringConstantOutOfBounds { pc, index })
        } else {
            Ok(())
        }
    }

    fn jump_target(&self, pc: usize, offset: i32) -> Result<usize, VerificationError> {
        let next = pc
            .checked_add(1)
            .and_then(|value| isize::try_from(value).ok())
            .ok_or(VerificationError::JumpOutOfBounds { pc })?;
        let offset =
            isize::try_from(offset).map_err(|_| VerificationError::JumpOutOfBounds { pc })?;
        let target = next
            .checked_add(offset)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|target| *target < self.instructions.len())
            .ok_or(VerificationError::JumpOutOfBounds { pc })?;
        Ok(target)
    }

    const fn fallthrough(&self, pc: usize) -> Result<usize, VerificationError> {
        let next = pc.saturating_add(1);
        if next < self.instructions.len() {
            Ok(next)
        } else {
            Err(VerificationError::ReachableFallthrough { pc })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_accepts_a_bounded_loop_with_return() {
        let mut function = BytecodeFunction::new(1, 0);
        function.emit(Instruction::LdaTrue);
        function.emit(Instruction::JumpIfFalse(1));
        function.emit(Instruction::Jump(-2));
        function.emit(Instruction::Return);
        assert_eq!(function.verify(), Ok(()));
    }

    #[test]
    fn verifier_rejects_invalid_frame_and_operand_bounds() {
        assert_eq!(
            BytecodeFunction::new(0, 0).verify(),
            Err(VerificationError::EmptyFunction)
        );
        let mut parameters = BytecodeFunction::new(0, 1);
        parameters.emit(Instruction::Return);
        assert_eq!(
            parameters.verify(),
            Err(VerificationError::ParametersExceedRegisters)
        );

        let mut bindings = BytecodeFunction::new(1, 0);
        bindings.binding_count = 2;
        bindings.emit(Instruction::Return);
        assert_eq!(
            bindings.verify(),
            Err(VerificationError::BindingsExceedRegisters)
        );

        let mut register = BytecodeFunction::new(1, 0);
        register.emit(Instruction::Ldar(Reg(1)));
        register.emit(Instruction::Return);
        assert_eq!(
            register.verify(),
            Err(VerificationError::RegisterOutOfBounds {
                pc: 0,
                register: Reg(1)
            })
        );

        let mut constant = BytecodeFunction::new(0, 0);
        constant.emit(Instruction::LdaConstant(0));
        constant.emit(Instruction::Return);
        assert_eq!(
            constant.verify(),
            Err(VerificationError::ConstantOutOfBounds { pc: 0, index: 0 })
        );

        let mut string = BytecodeFunction::new(0, 0);
        string.emit(Instruction::LdaString(0));
        string.emit(Instruction::Return);
        assert_eq!(
            string.verify(),
            Err(VerificationError::StringConstantOutOfBounds { pc: 0, index: 0 })
        );

        let mut heap_bound = BytecodeFunction::new(0, 0);
        heap_bound.constants.push(Value::from_string(
            super::super::value::StringRef::from_parts(0, 0),
        ));
        heap_bound.emit(Instruction::LdaConstant(0));
        heap_bound.emit(Instruction::Return);
        assert_eq!(
            heap_bound.verify(),
            Err(VerificationError::HeapBoundConstant { index: 0 })
        );
    }

    #[test]
    fn verifier_rejects_invalid_feedback_calls_and_jumps() {
        let mut feedback = BytecodeFunction::new(1, 0);
        let name = feedback.add_string_constant("x".encode_utf16().collect());
        feedback.emit(Instruction::GetNamed {
            obj: Reg(0),
            name,
            slot: 0,
        });
        feedback.emit(Instruction::Return);
        assert_eq!(
            feedback.verify(),
            Err(VerificationError::FeedbackOutOfBounds { pc: 0, slot: 0 })
        );

        let mut call = BytecodeFunction::new(2, 0);
        call.feedback_slot_count = 1;
        call.emit(Instruction::Call {
            func: Reg(0),
            arg_start: Reg(1),
            arg_count: 2,
            slot: 0,
        });
        call.emit(Instruction::Return);
        assert_eq!(
            call.verify(),
            Err(VerificationError::CallArgumentsOutOfBounds { pc: 0 })
        );

        let mut jump = BytecodeFunction::new(0, 0);
        jump.emit(Instruction::Jump(1));
        jump.emit(Instruction::Return);
        assert_eq!(
            jump.verify(),
            Err(VerificationError::JumpOutOfBounds { pc: 0 })
        );

        let mut closure = BytecodeFunction::new(0, 0);
        closure.emit(Instruction::CreateClosure(0));
        closure.emit(Instruction::Return);
        assert_eq!(
            closure.verify(),
            Err(VerificationError::FunctionOutOfBounds { pc: 0, index: 0 })
        );

        let mut root = BytecodeFunction::new(0, 0);
        root.emit(Instruction::Return);
        let mut nested = BytecodeFunction::new(0, 0);
        nested.emit(Instruction::Return);
        let mut unreachable = BytecodeFunction::new(0, 0);
        unreachable.emit(Instruction::Return);
        nested.functions.push(unreachable);
        root.functions.push(nested);
        assert_eq!(
            root.verify(),
            Err(VerificationError::NestedFunctionTable { index: 0 })
        );
    }

    #[test]
    fn verifier_rejects_reachable_fallthrough_but_allows_unreachable_tail() {
        let mut fallthrough = BytecodeFunction::new(0, 0);
        fallthrough.emit(Instruction::LdaUndefined);
        assert_eq!(
            fallthrough.verify(),
            Err(VerificationError::ReachableFallthrough { pc: 0 })
        );

        let mut unreachable_tail = BytecodeFunction::new(0, 0);
        unreachable_tail.emit(Instruction::Return);
        unreachable_tail.emit(Instruction::LdaUndefined);
        assert_eq!(unreachable_tail.verify(), Ok(()));
    }
}
