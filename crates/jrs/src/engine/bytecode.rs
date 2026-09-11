// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Register-based Bytecode Instruction Set.
//!
//! Instructions operate on a register file (r0..rn) and an explicit
//! accumulator register (acc). This mirrors modern production engines (Ignition)
//! and eliminates the stack push/pop dispatch overhead.

use super::value::{StringRef, Value};
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
    /// `acc = undefined`
    LdaUndefined,
    /// `acc = null`
    LdaNull,
    /// `acc = true`
    LdaTrue,
    /// `acc = false`
    LdaFalse,
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
        /// Property name identifier.
        name: StringRef,
        /// Feedback vector slot for inline caching.
        slot: u16,
    },
    /// Store named property: `obj_reg[name] = acc` (uses feedback slot).
    SetNamed {
        /// Object register.
        obj: Reg,
        /// Property name identifier.
        name: StringRef,
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
    /// Creates an empty object `{}` in `acc`.
    CreateObject,
    /// Creates an empty array `[]` in `acc` with initial capacity.
    CreateArray(u32),
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
    /// Number of local registers required in the stack frame.
    pub register_count: u16,
    /// Number of formal parameters expected.
    pub parameter_count: u16,
    /// Number of feedback vector slots allocated for inline caches.
    pub feedback_slot_count: u16,
}

impl BytecodeFunction {
    /// Creates a new empty bytecode function.
    #[must_use]
    pub const fn new(register_count: u16, parameter_count: u16) -> Self {
        Self {
            instructions: Vec::new(),
            constants: Vec::new(),
            register_count,
            parameter_count,
            feedback_slot_count: 0,
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
        if self.instructions.is_empty() {
            return Err(VerificationError::EmptyFunction);
        }
        if self.parameter_count > self.register_count {
            return Err(VerificationError::ParametersExceedRegisters);
        }
        for (pc, instruction) in self.instructions.iter().enumerate() {
            self.verify_instruction(pc, *instruction)?;
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
            Instruction::GetNamed { obj, slot, .. } | Instruction::SetNamed { obj, slot, .. } => {
                self.verify_feedback(pc, slot)?;
                Some(obj)
            }
            Instruction::GetByValue { obj, key, slot, .. }
            | Instruction::SetByValue { obj, key, slot, .. } => {
                self.verify_register(pc, obj)?;
                self.verify_feedback(pc, slot)?;
                Some(key)
            }
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
            Instruction::Jump(offset)
            | Instruction::JumpIfTrue(offset)
            | Instruction::JumpIfFalse(offset) => {
                self.jump_target(pc, offset)?;
                None
            }
            Instruction::LdaSmi(_)
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
    }

    #[test]
    fn verifier_rejects_invalid_feedback_calls_and_jumps() {
        let mut feedback = BytecodeFunction::new(1, 0);
        feedback.emit(Instruction::GetNamed {
            obj: Reg(0),
            name: StringRef::from_parts(0, 0),
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
