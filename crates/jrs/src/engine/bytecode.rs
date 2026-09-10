// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Register-based Bytecode Instruction Set.
//!
//! Instructions operate on a register file (r0..rn) and an explicit
//! accumulator register (acc). This mirrors modern production engines (Ignition)
//! and eliminates the stack push/pop dispatch overhead.

use super::value::{StringRef, Value};
use alloc::vec::Vec;

/// Virtual register index inside a function's call frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reg(pub u16);

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
}
