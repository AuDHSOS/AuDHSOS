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
    elements::ElementsRef,
    feedback::{BinaryOpFeedback, FeedbackVector, NamedAccessCase},
    heap::{GenerationalHeap, HeapError, NamedProperty, Root},
    object::{ArrayIterationKind, ObjectKind},
    promise,
    realm::{Intrinsic, Realm},
    shape::{PropertyFlags, ShapeId},
    string::StringError,
    value::{
        ObjectRef, PropertyKey, StringRef, SymbolRef, VALUE_FALSE, VALUE_NAN, VALUE_NULL,
        VALUE_TRUE, VALUE_UNDEFINED, VALUE_UNINITIALIZED, Value,
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
    /// An algorithm reached a step this engine does not implement. It is a gap
    /// in the migration, never an answer a program gave.
    Unsupported(&'static str),
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

/// Bit that marks a call-site target as a native intrinsic rather than a
/// function of a code unit.
const NATIVE_CALL_TARGET: u64 = 1 << 63;

/// Names a bytecode function at a call site: the unit it belongs to and its
/// index in that unit. The same index in two units is two functions.
const fn bytecode_call_target(unit: u32, code_id: u32) -> u64 {
    ((unit as u64) << 32) | code_id as u64
}

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
    /// Set when an operation opened this call and waits for its answer.
    resume: Option<Resume>,
    /// Set when 7.3.15 opened this call; see [`FrameHeader::construct`].
    construct: Option<Reg>,
}

/// What the engine writes after the name of an unresolvable binding.
///
/// The embedding takes the name back off the message, so that a
/// `ReferenceError` of this engine reaches it in the same shape as one of the
/// stack backend rather than as a thrown object it cannot classify.
pub const UNRESOLVABLE_SUFFIX: &str = " is not initialized or defined";

/// How many indices a walk of 23.1.3 may pass over for one unit of fuel.
///
/// An index the object does not have costs a lookup and no more, so it is
/// charged in batches; what this bounds is the walk of a `length` no object
/// could fill.
const HOLES_PER_FUEL_UNIT: u32 = 4;

/// The hint 7.1.1 passes to `@@toPrimitive` for an operation that names none.
const DEFAULT_HINT: [u16; 7] = [0x64, 0x65, 0x66, 0x61, 0x75, 0x6C, 0x74];

/// The `"string"` hint 7.1.17 passes to 7.1.1.
const STRING_HINT: [u16; 6] = [0x73, 0x74, 0x72, 0x69, 0x6E, 0x67];

/// The text `"number"`, which 7.1.4 passes `@@toPrimitive`.
const NUMBER_HINT: [u16; 6] = [0x6E, 0x75, 0x6D, 0x62, 0x65, 0x72];

/// The text 25.5.2.4 gives null and a Number that is not finite.
const NULL_UNITS: [u16; 4] = [0x6E, 0x75, 0x6C, 0x6C];

/// A walk of 23.1.3 converts nothing before it begins.
const CONVERTING_NOTHING: u8 = 0;
/// It is converting the `length` 7.1.20 reads.
const CONVERTING_LENGTH: u8 = 1;
/// It is converting the index 7.1.5 makes of what a scan starts from.
const CONVERTING_FROM: u8 = 2;

/// The state of one walk of 23.1.3, read out of the object that holds it.
///
/// A plain record, so the walk can be reasoned about in one place; it is
/// written back after every step, because the object is what the collector
/// traces and a value held here would not survive a call.
struct ArrayWalk {
    intrinsic: Intrinsic,
    target: Value,
    callback: Value,
    receiver: Value,
    output: Value,
    element: Value,
    element_index: u32,
    index: u32,
    length: u32,
    started: bool,
    /// Whether the call in flight is the getter of an accessor element
    /// rather than the callback of the clause.
    getter: bool,
    /// Which value the walk is still converting before it begins.
    converting: u8,
    /// Which method 7.1.1 has already asked for the value in flight.
    convert_step: u8,
}

impl ArrayWalk {
    /// Whether this clause walks from the end (23.1.3.25) or from the start.
    const fn backwards(&self) -> bool {
        matches!(
            self.intrinsic,
            Intrinsic::ArrayPrototypeReduceRight
                | Intrinsic::ArrayPrototypeFindLast
                | Intrinsic::ArrayPrototypeFindLastIndex
                | Intrinsic::ArrayPrototypeLastIndexOf
        )
    }

    /// The next index to ask about, or none when the walk is done.
    const fn next_index(&self) -> Option<u32> {
        if self.backwards() {
            if self.index == 0 {
                return None;
            }
            return Some(self.index.saturating_sub(1));
        }
        if self.index >= self.length {
            return None;
        }
        Some(self.index)
    }

    /// Moves past the index just taken.
    const fn advance(&mut self, element_index: u32) {
        self.index = if self.backwards() {
            element_index
        } else {
            element_index.saturating_add(1)
        };
    }

    /// How many arguments 23.1.3 gives this callback: four where an
    /// accumulator goes in front of the element, three otherwise.
    const fn callback_arity(&self) -> u16 {
        match self.intrinsic {
            Intrinsic::ArrayPrototypeReduce | Intrinsic::ArrayPrototypeReduceRight => 4,
            // 23.1.2.1 step 5.f.iii passes the element and its index alone.
            Intrinsic::ArrayFrom => 2,
            _ => 3,
        }
    }

    /// What the clause answers once every element has been asked about.
    const fn answer(&self) -> Value {
        match self.intrinsic {
            // 23.1.3.15 answers undefined, 23.1.3.6 true and 23.1.3.29 false.
            Intrinsic::ArrayPrototypeForEach
            | Intrinsic::ArrayPrototypeFind
            | Intrinsic::ArrayPrototypeFindLast => VALUE_UNDEFINED,
            Intrinsic::ArrayPrototypeEvery => VALUE_TRUE,
            // 23.1.3.16 answers false too.
            Intrinsic::ArrayPrototypeSome | Intrinsic::ArrayPrototypeIncludes => VALUE_FALSE,
            // 23.1.3.10 answers minus one; 23.1.3.9 answers undefined, as
            // 23.1.3.15 does.
            Intrinsic::ArrayPrototypeFindIndex
            | Intrinsic::ArrayPrototypeFindLastIndex
            // 23.1.3.17 and 23.1.3.20 answer minus one too, and 23.1.3.16
            // answers false.
            | Intrinsic::ArrayPrototypeIndexOf
            | Intrinsic::ArrayPrototypeLastIndexOf => Value::from_smi(-1),
            // 23.1.3.21 and 23.1.3.8 answer the Array they filled, and
            // 23.1.3.24 the accumulator it carried.
            _ => self.output,
        }
    }
}

/// A Property Descriptor as 6.2.6.5 read it: only the fields it named.
///
/// A field that is `None` is one the descriptor does not have, which 10.1.6.3
/// treats differently from one that is present and undefined.
#[derive(Default)]
struct PartialDescriptor {
    value: Option<Value>,
    writable: Option<bool>,
    get: Option<Value>,
    set: Option<Value>,
    enumerable: Option<bool>,
    configurable: Option<bool>,
}

impl PartialDescriptor {
    /// `IsAccessorDescriptor` of 6.2.6.1.
    const fn is_accessor(&self) -> bool {
        self.get.is_some() || self.set.is_some()
    }

    /// `IsDataDescriptor` of 6.2.6.2.
    const fn is_data(&self) -> bool {
        self.value.is_some() || self.writable.is_some()
    }

    /// `IsGenericDescriptor` of 6.2.6.3.
    const fn is_generic(&self) -> bool {
        !self.is_accessor() && !self.is_data()
    }

    /// Whether the descriptor names no field at all, which 10.1.6.3 step 4
    /// accepts without changing anything.
    const fn is_empty(&self) -> bool {
        self.is_generic() && self.enumerable.is_none() && self.configurable.is_none()
    }
}

/// What a conversion of 7.1.1 reached.
enum Conversion {
    /// The primitive value the operation asked for.
    Done(Value),
    /// A method has to run first; this is the bytecode unit it starts in.
    Suspended(u32),
}

/// Every code unit of a Realm, under the name the Realm gave each.
///
/// A function object names the unit it was compiled with, so a call resolves
/// the function's index in that unit and not in the Script that is running.
/// A Realm never drops a unit, which is what makes the index a name.
#[derive(Clone, Copy)]
pub struct CodeTable<'a> {
    roots: &'a [&'a BytecodeFunction],
}

impl<'a> CodeTable<'a> {
    /// Names the roots of a Realm, in unit order.
    #[must_use]
    pub const fn new(roots: &'a [&'a BytecodeFunction]) -> Self {
        Self { roots }
    }

    /// The root of one unit, or `None` when the Realm has no such unit.
    fn root(self, unit: u32) -> Option<&'a BytecodeFunction> {
        self.roots.get(unit as usize).copied()
    }
}

/// What an instruction works with: every unit of the Realm, and the unit the
/// active frame is executing.
#[derive(Clone, Copy)]
struct CodeUnits<'a> {
    /// Every unit, so a call can reach a function of another Script.
    table: CodeTable<'a>,
    /// The unit of the active frame.
    active: &'a BytecodeFunction,
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
    /// Code unit the caller belongs to, which a call of another Script leaves.
    pub caller_unit: u32,
    /// Set when 7.3.15 opened this call. Names the caller register holding the
    /// object 10.1.13 created, which the return answers when the constructor
    /// answers no object of its own (10.2.2 step 13).
    pub construct: Option<Reg>,
    /// Active binding count to restore with the caller.
    pub caller_binding_count: usize,
    /// Lexical heap context to restore with the caller.
    pub caller_context: Option<ContextRef>,
    /// Set when this call was opened by an operation rather than by a call
    /// instruction, and says what the operation does with the answer.
    pub resume: Option<Resume>,
    /// Where the arguments of this call sit in the caller frame, which 10.4.4
    /// needs after the frame has been entered. Only registers are held, never
    /// values: the collector sees registers, not the fields of a header.
    pub arguments: Option<FrameArguments>,
}

/// How far a property question reaches (10.1.5 and 10.1.8).
///
/// An own query stops at the object; a read walks the Prototype Chain, so a
/// name a Prototype owns and this Realm has not built is a gap there and not
/// on the object.
#[derive(Clone, Copy)]
enum Reach {
    /// `[[GetOwnProperty]]` of 10.1.5.
    Own,
    /// `[[Get]]` of 10.1.8.
    Chain,
}

/// The registers of the caller frame that 10.4.4 reads to build an arguments
/// object: the arguments themselves and the callable the call named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameArguments {
    /// First argument register of the caller frame.
    pub start: Reg,
    /// Number of arguments the call passed.
    pub count: u16,
    /// Caller register holding the Function object, which 10.4.4 makes
    /// `callee`.
    pub callee: Reg,
}

/// What an operation that called user code does when the call returns.
///
/// Only a register of the caller frame is held, never a value: the collector
/// sees registers, and it does not see the fields of a frame header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    /// A conversion of 7.1.1 asked a method of the object for a primitive.
    Primitive {
        /// Register of the caller frame the answer is written to.
        register: Reg,
        /// The method of 7.1.1 whose answer this is.
        step: PrimitiveStep,
        /// The hint 7.1.1 was given, which decides the order of the methods.
        hint: PrimitiveHint,
        /// Root holding the value the instruction still has to write, where
        /// the conversion is of the key of a write: the accumulator holds it
        /// and the frame this opens would take that place.
        held: Option<Root>,
    },
    /// A native operation asked the Script for a primitive argument.
    ///
    /// The native has no frame, and the caller's registers still hold the
    /// arguments it was called with, so the operation runs again from the
    /// beginning once the argument they name is a primitive. Only an
    /// operation that converts before it changes anything asks this way.
    Coercion {
        /// The intrinsic that asked, by [`Intrinsic::id`].
        intrinsic: u32,
        /// Root holding the `this` value of the call.
        receiver: Root,
        /// The argument register the primitive is written to.
        register: Reg,
        /// First argument register of the call.
        arg_start: Reg,
        /// Number of arguments the call passed.
        arg_count: u16,
        /// Register `new` keeps its object in, when this was a construct.
        construct: Option<Reg>,
        /// The method of 7.1.1 whose answer this is.
        step: PrimitiveStep,
        /// The hint 7.1.1 was given.
        hint: PrimitiveHint,
        /// Set where 22.1.3 converts the `this` value rather than an argument,
        /// which has no register of the caller to answer into: the root the
        /// receiver travels in takes the primitive instead.
        of_receiver: bool,
    },
    /// 6.2.6.5 is reading the fields of a descriptor, and one of them is a
    /// getter of the Script.
    ///
    /// Each field it has read waits in an object of its own, so every getter
    /// runs once and the clause reads the copy rather than the descriptor.
    Descriptor {
        /// The intrinsic that asked, by [`Intrinsic::id`].
        intrinsic: u32,
        /// Root holding the `this` value of the call.
        receiver: Root,
        /// Root holding what the clause reads instead of the argument: the
        /// copy of the descriptor, or the object that holds one copy per key.
        copy: Root,
        /// Root holding the keys 7.3.25 step 2 listed, or undefined where the
        /// clause reads one descriptor.
        keys: Root,
        /// Root holding the descriptor whose fields are being read.
        pending: Root,
        /// Root holding the object those fields are copied into.
        target: Root,
        /// How many of the keys 7.3.25 lists have been read.
        key_index: u32,
        /// How many of the six fields 6.2.6.5 names have been read;
        /// `DESCRIPTOR_KEY` and `DESCRIPTOR_VALUE` name the two steps of
        /// 7.3.25 that stand before them.
        field: u8,
        /// The argument register the descriptor came in and the copy goes to.
        register: Reg,
        /// First argument register of the call.
        arg_start: Reg,
        /// Number of arguments the call passed.
        arg_count: u16,
        /// Register `new` keeps its object in, when this was a construct.
        construct: Option<Reg>,
    },
    /// A method of 23.1.3 called the callback for one element.
    ///
    /// The walk keeps what it has reached in an object of the heap rather
    /// than in a register, because it has no frame of its own and the
    /// registers of the caller belong to the Script. Only the root is held
    /// here, so the collector moves the state freely.
    Iteration {
        /// Root naming the [`ObjectKind::ArrayIteration`] state.
        state: Root,
    },
    /// The arguments of the call are a List and not registers of the caller.
    ///
    /// 20.2.3.1 and 28.1.1 make one out of an array-like, which no frame of
    /// the caller holds, so the call carries the Array it was made into and
    /// the frame takes its parameters from there.
    Spread {
        /// Root naming the Array the arguments were collected into.
        arguments: Root,
    },
    /// `[[Get]]` of 10.1.8.1 called the getter of an accessor property.
    ///
    /// The getter answers into the accumulator, which is where every
    /// instruction that reads a property leaves the value, so the read is
    /// finished the moment the call returns.
    Getter,
    /// A clause of 23.1.3 called the getter of an accessor `length`.
    ///
    /// 7.3.18 reads the length once, before the clause does anything else, so
    /// the operation starts again with the length it answered rather than
    /// reading it a second time.
    Length {
        /// The intrinsic that asked, by [`Intrinsic::id`].
        intrinsic: u32,
        /// Root holding the `this` value of the call.
        receiver: Root,
        /// Root holding the value 7.1.20 is converting, where the read
        /// answered an Object.
        held: Root,
        /// Which method 7.1.1 has already asked: 0 before any, 1 after
        /// `valueOf`, 2 after `toString`.
        step: u8,
        /// First argument register of the call.
        arg_start: Reg,
        /// Number of arguments the call passed.
        arg_count: u16,
        /// Register `new` keeps its object in, when this was a construct.
        construct: Option<Reg>,
    },
    /// A job of 9.5 called its handler.
    ///
    /// The frame stands on no caller: the unit that enqueued the job has
    /// answered, so the return settles the job's capability and the next job
    /// follows, and a value thrown out of the frame stops here.
    Job {
        /// Root holding the List the handler's arguments travel in.
        arguments: Root,
    },
    /// 27.2.3.1 step 6 called the executor of a new Promise.
    ///
    /// The constructor answers the promise whatever the executor answers, and
    /// step 7 rejects the promise with a value the executor throws rather than
    /// letting it reach the caller, so this frame catches.
    Executor {
        /// Root holding the promise the constructor answers.
        promise: Root,
        /// Root holding the state the pair of 27.2.1.3 shares, which step 7
        /// rejects through.
        state: Root,
        /// Root holding the List the two resolving functions travel in.
        arguments: Root,
    },
    /// `[[Set]]` of 10.1.9.2 called the setter of an accessor property.
    ///
    /// A setter answers nothing, and 13.15.2 answers the value assigned, so
    /// the value waits in a root of its own until the setter returns.
    Setter {
        /// Root holding the value written.
        value: Root,
    },
}

impl Resume {
    /// The conversion of 7.1.1 this is waiting for, if it is one.
    const fn conversion(self) -> Option<(Reg, PrimitiveStep, PrimitiveHint)> {
        match self {
            Self::Primitive {
                register,
                step,
                hint,
                ..
            }
            | Self::Coercion {
                register,
                step,
                hint,
                ..
            } => Some((register, step, hint)),
            _ => None,
        }
    }

    /// The same conversion, waiting for the next method of 7.1.1.
    const fn with_step(self, next: PrimitiveStep) -> Self {
        match self {
            Self::Primitive {
                register,
                hint,
                held,
                ..
            } => Self::Primitive {
                register,
                step: next,
                hint,
                held,
            },
            Self::Coercion {
                intrinsic,
                receiver,
                register,
                arg_start,
                arg_count,
                construct,
                hint,
                of_receiver,
                ..
            } => Self::Coercion {
                intrinsic,
                receiver,
                register,
                arg_start,
                arg_count,
                construct,
                step: next,
                hint,
                of_receiver,
            },
            other => other,
        }
    }

    /// The List the frame takes its parameters from, for a call that has no
    /// registers of a caller to take them from.
    const fn list(self) -> Option<Root> {
        match self {
            Self::Spread { arguments }
            | Self::Job { arguments }
            | Self::Executor { arguments, .. } => Some(arguments),
            _ => None,
        }
    }

    /// Whether a value thrown out of this frame stops here instead of looking
    /// for a handler of the caller.
    const fn catches(self) -> bool {
        matches!(self, Self::Executor { .. } | Self::Job { .. })
    }
}

/// The hint 7.1.1 takes, which decides the order of the methods 7.1.1.1 asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveHint {
    /// 7.1.1 with no hint, and 7.1.3: `valueOf` before `toString`.
    Default,
    /// 7.1.4: the same order, under the hint `"number"`.
    Number,
    /// 7.1.17: `toString` before `valueOf`.
    String,
}

impl PrimitiveHint {
    /// The text `@@toPrimitive` is passed.
    const fn units(self) -> &'static [u16] {
        match self {
            Self::Default => &DEFAULT_HINT,
            Self::Number => &NUMBER_HINT,
            Self::String => &STRING_HINT,
        }
    }

    /// The method 7.1.1 asks after this one, and none after the last.
    const fn after(self, step: PrimitiveStep) -> Option<PrimitiveStep> {
        match (self, step) {
            (Self::Default | Self::Number, PrimitiveStep::Exotic)
            | (Self::String, PrimitiveStep::ToString) => Some(PrimitiveStep::ValueOf),
            (Self::Default | Self::Number, PrimitiveStep::ValueOf)
            | (Self::String, PrimitiveStep::Exotic) => Some(PrimitiveStep::ToString),
            _ => None,
        }
    }
}

/// Which method of 7.1.1 a conversion has already asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveStep {
    /// `@@toPrimitive` of 7.1.1 step 2, whose answer must be primitive.
    Exotic,
    /// `valueOf` of 7.1.1.1, after which `toString` follows.
    ValueOf,
    /// `toString` of 7.1.1.1, the last method the list holds.
    ToString,
}

/// Contiguous register-based virtual machine executor.
pub struct RegisterVM {
    /// Flat contiguous register stack.
    stack: Vec<Value>,
    /// Current frame pointer (start of active register window).
    fp: usize,
    /// Accumulator register.
    acc: Value,
    /// The code unit being executed, which every function object it creates
    /// carries so that a call resolves against the unit that compiled it.
    unit: u32,
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
    /// How deep a `ToString` of 7.1.17 is nested in methods of this Realm,
    /// which bounds what a cyclic Array spends the Rust stack on.
    conversion_depth: usize,
    /// `[[NewTarget]]` of the call about to be entered (9.4.3).
    ///
    /// It is set where the frame is opened and read where the frame is
    /// entered, with no allocation between, so the collector never sees it.
    pending_new_target: Value,
    /// The source text the run asked the embedding for, which stops it until
    /// the embedding answers.
    pending_source: Option<alloc::rc::Rc<[u16]>>,
    /// Whether that text is a Script 19.2.1 asked to have evaluated, rather
    /// than a unit 20.2.1.1 asked to have compiled.
    pending_script: bool,
    /// Whether that text is a line the embedding was asked to write.
    pending_print: bool,
    /// Where the run continues once that unit exists.
    resume_pc: usize,
    /// The bytecode function the run continues in.
    resume_code_id: Option<u32>,
    /// What the embedding answered for the text it was given.
    compiled_unit: Option<Compiled>,
    /// Root of the job queue of 9.5, the Array of the job records this run
    /// has still to make.
    jobs: Option<Root>,
    /// Root of the job record whose handler holds a frame, which settles when
    /// that frame returns or throws.
    running_job: Option<Root>,
    /// Root of the List a job's arguments travel in, which the frame the job
    /// opens takes its parameters from.
    job_arguments: Option<Root>,
    /// Root of the value the unit answered, held while the queue drains.
    completion: Option<Root>,
    /// How many jobs of the queue have run, which is where the next one is
    /// taken from; the queue is emptied once the index reaches its end.
    job_head: u32,
}

/// What the embedding made of the text the run gave it.
pub enum Compiled {
    /// It wrote the line the run gave it.
    Printed,
    /// The unit it compiled the text into.
    Unit(u32),
    /// How the Script it evaluated ended.
    Evaluated(Result<Value, VMError>),
    /// It took the text for no Script, which 20.2.1.1 step 12 and 19.2.1.1
    /// step 8 each answer with a `SyntaxError`.
    Refused,
}

/// How a run of one unit ended.
pub enum Outcome {
    /// The unit ran to its end and answered this value.
    Done(Value),
    /// 20.2.1.1 compiles a body at run time, which only the embedding that
    /// holds the units of the Realm can do. The run continues where it
    /// stopped once [`RegisterVM::resume_unit`] is given the unit.
    Compile(alloc::rc::Rc<[u16]>),
    /// 19.2.1 evaluates a Script of the same Realm, which the embedding runs
    /// on the heap and Realm this one is using.
    Evaluate(alloc::rc::Rc<[u16]>),
    /// The Script called `print`, which only the embedding can answer: it
    /// writes the line and the call instruction runs again.
    Print(alloc::rc::Rc<[u16]>),
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
            unit: 0,
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
            conversion_depth: 0,
            pending_new_target: VALUE_UNDEFINED,
            pending_source: None,
            pending_script: false,
            pending_print: false,
            resume_pc: 0,
            resume_code_id: None,
            compiled_unit: None,
            jobs: None,
            running_job: None,
            job_arguments: None,
            completion: None,
            job_head: 0,
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
            match heap.allocate_function(self.unit, code_id, context) {
                Ok(reference) => {
                    heap.set_object_prototype(reference, prototype)?;
                    return Ok(reference);
                }
                Err(HeapError::NurseryFull) => self.collect_young(code, heap)?,
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// `OrdinaryCallBindThis` of 10.2.1.2: what the callee sees as `this`.
    ///
    /// A strict function is given the receiver as it stands. A non-strict one
    /// sees the global object where the call had no receiver, and the object
    /// `ToObject` makes of a primitive one.
    fn bind_this(
        receiver: Value,
        strict: bool,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if strict {
            return Ok(receiver);
        }
        if receiver.is_undefined() || receiver.is_null() {
            return Ok(realm.global_environment().this_value(heap)?);
        }
        if receiver.as_object().is_some() {
            return Ok(receiver);
        }
        Ok(Value::from_object(Self::coerce_object(
            receiver, heap, realm,
        )?))
    }

    /// `OrdinaryHasInstance` of 7.3.22, which 13.10.2 reaches because no
    /// `@@hasInstance` exists on any object of this Realm yet.
    ///
    /// Walks the Prototype Chain of the value looking for the constructor's
    /// `prototype`. A right operand that is not an Object is a `TypeError`, one
    /// that is not callable answers false, and a constructor whose `prototype`
    /// is not an Object is a `TypeError`.
    fn ordinary_has_instance(
        constructor: Value,
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<bool, VMError> {
        let Some(function) = constructor.as_object() else {
            return Err(type_error(
                heap,
                realm,
                "right-hand side of instanceof is not an object",
            ));
        };
        // 13.10.2 step 5: an object that is not callable carries no
        // `[[HasInstance]]` of any kind, which is a TypeError and not a false.
        if !Self::is_callable(constructor, heap) {
            return Err(type_error(
                heap,
                realm,
                "right-hand side of instanceof is not callable",
            ));
        }
        let Some(mut current) = value.as_object() else {
            return Ok(false);
        };
        let name = PropertyKey::String(heap.strings.intern("prototype")?);
        let prototype = heap
            .lookup_named(function, name)?
            .map(Self::plain_value)
            .transpose()?
            .unwrap_or(VALUE_UNDEFINED);
        let Some(prototype) = prototype.as_object() else {
            return Err(type_error(
                heap,
                realm,
                "prototype of the right-hand side of instanceof is not an object",
            ));
        };
        loop {
            let next = heap
                .get_object(current)
                .ok_or(VMError::TypeError)?
                .prototype;
            let Some(next) = next.as_object() else {
                return Ok(false);
            };
            if next == prototype {
                return Ok(true);
            }
            current = next;
        }
    }

    /// `OrdinaryCreateFromConstructor` of 10.1.13: the object `new` starts
    /// from, whose Prototype is the constructor's `prototype` when that is an
    /// Object and `%Object.prototype%` otherwise.
    ///
    /// 7.3.15 step 1 refuses a callee without `[[Construct]]`. A function of this
    /// engine has one exactly when 10.2.5 gave it a `prototype`, so a callee
    /// without that property is not a constructor.
    fn ordinary_create_from_constructor(
        &mut self,
        code: &BytecodeFunction,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        func: Reg,
    ) -> Result<ObjectRef, VMError> {
        let Some(function) = self.read_reg(func)?.as_object() else {
            return Err(type_error(heap, realm, "value is not a constructor"));
        };
        let kind = &heap.get_object(function).ok_or(VMError::TypeError)?.kind;
        // 10.4.1.2 constructs the target with the arguments the bind kept and
        // the `newTarget` the call site gave, which this engine has not built.
        if matches!(kind, ObjectKind::BoundFunction { .. }) {
            return Err(VMError::Unsupported("new of a bound function"));
        }
        if !matches!(kind, ObjectKind::Function { .. }) {
            return Err(type_error(heap, realm, "value is not a constructor"));
        }
        let shape = heap.shapes.root_shape();
        let object = self.allocate_object(code, heap, realm, shape)?;
        // A register is a root the collector forwards, so after an allocation
        // that may scavenge this names the same function, and 10.1.13 reads
        // the `prototype` it gives the object from there.
        let function = self.read_reg(func)?.as_object().ok_or(VMError::TypeError)?;
        let name = PropertyKey::String(heap.strings.intern("prototype")?);
        let Some(property) = heap.lookup_named(function, name)? else {
            return Err(type_error(heap, realm, "value is not a constructor"));
        };
        let prototype = Self::plain_value(property)?;
        if prototype.as_object().is_some() {
            heap.set_object_prototype(object, prototype)?;
        }
        Ok(object)
    }

    /// `MakeConstructor` of 10.2.5: gives a function an own `prototype` whose
    /// own `constructor` is the function, both writable and not enumerable.
    ///
    /// The object is allocated first and installed after, so a collection
    /// between the two cannot leave the function holding a forwarded address.
    /// 10.2.5: gives an ordinary function the `prototype` a constructor has.
    ///
    /// The function is the accumulator, not an argument: allocating the
    /// prototype may scavenge, and a reference read before that allocation
    /// does not survive it. The collector follows the accumulator, so the
    /// function is read again on the other side.
    fn make_constructor(
        &mut self,
        code: &BytecodeFunction,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let prototype = self.allocate_object(code, heap, realm, heap.shapes.root_shape())?;
        let function = self.acc.as_object().ok_or(VMError::TypeError)?;
        let constructor = PropertyKey::String(heap.strings.intern("constructor")?);
        heap.define_own_named(
            prototype,
            constructor,
            Value::from_object(function),
            PropertyFlags::constructor_data(),
        )?;
        let name = PropertyKey::String(heap.strings.intern("prototype")?);
        heap.define_own_named(
            function,
            name,
            Value::from_object(prototype),
            PropertyFlags::constructor_data(),
        )?;
        Ok(())
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
            // 13.5.3 answers "function" for everything 7.2.3 calls callable,
            // which 10.4.1 makes a bound function one of.
            if matches!(
                object.kind,
                ObjectKind::Function { .. }
                    | ObjectKind::NativeFunction { .. }
                    | ObjectKind::BoundFunction { .. }
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
        realm: &Realm,
    ) -> Result<BinaryOpFeedback, VMError> {
        let observed = if self.acc.as_smi().is_some() && rhs.as_smi().is_some() {
            BinaryOpFeedback::SignedSmallInteger
        } else if self.acc.is_number() && rhs.is_number() {
            BinaryOpFeedback::Number
        } else {
            BinaryOpFeedback::Generic
        };
        if op == BinaryOp::Add && (self.acc.is_string() || rhs.is_string()) {
            let left = self.primitive_string(self.acc, heap, realm)?;
            let right = self.primitive_string(rhs, heap, realm)?;
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
        // 13.11.1 is 7.2.14, which compares two primitives and never a
        // number it made of a String against another String.
        if op == BinaryOp::Equals {
            self.acc = Value::from_bool(Self::loosely_equals(self.acc, rhs, heap)?);
            return Ok(observed);
        }
        let left = primitive_number(self.acc, heap)?;
        let right = primitive_number(rhs, heap)?;
        // 13.12 and 13.9 read both operands as integers of 32 bits.
        if let Some(bits) = Self::integer_binary(op, left, right) {
            self.acc = bits;
            return Ok(observed);
        }
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
            // The integer operators answered above, and 13.11.1 before them.
            _ => return Err(VMError::TypeError),
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

    /// The operators of 13.12 and 13.9, which read 6.1.6.1.2 of each operand.
    fn integer_binary(op: BinaryOp, left: f64, right: f64) -> Option<Value> {
        let a = number_to_i32(left);
        let b = number_to_i32(right);
        let shift = crate::value::number_uint32(right) & 0x1F;
        Some(match op {
            BinaryOp::BitAnd => Value::from_smi(a & b),
            BinaryOp::BitOr => Value::from_smi(a | b),
            BinaryOp::BitXor => Value::from_smi(a ^ b),
            BinaryOp::ShiftLeft => Value::from_smi(a.wrapping_shl(shift)),
            BinaryOp::ShiftRight => Value::from_smi(a.wrapping_shr(shift)),
            BinaryOp::UnsignedShiftRight => Value::from_f64(f64::from(
                crate::value::number_uint32(left).wrapping_shr(shift),
            )),
            _ => return None,
        })
    }

    fn primitive_string(
        &self,
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
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
        } else if value.is_symbol() {
            // 7.1.17 step 2: a Symbol has no String of its own.
            return Err(type_error(heap, realm, "cannot convert Symbol operand"));
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
        if left.is_object() || right.is_object() {
            // 7.2.14 step 12: an Object is never equal to null or undefined,
            // and nothing is converted to find that out.
            if left.is_null() || left.is_undefined() || right.is_null() || right.is_undefined() {
                return Ok(false);
            }
            // Every other Object goes through ToPrimitive, which can call a
            // `valueOf` of the Script; a comparison that reaches here has no
            // frame to run one in.
            return Err(NUMERIC_CONVERSION_GAP);
        }
        if left.is_bigint() || right.is_bigint() {
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
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        call: Call,
    ) -> Result<Option<u32>, VMError> {
        let function = self.read_reg(call.func)?;
        self.enter_call_value(function, units, active_feedback, heap, realm, call)
    }

    /// Enters a call whose callee is already a value rather than a register.
    ///
    /// An operation that converts an operand looks its method up itself, so it
    /// has no register to name it in.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a callee that is not callable, and the
    /// frame, binding and fuel limits of the caller.
    #[expect(
        clippy::too_many_lines,
        reason = "one function names every way a call is entered"
    )]
    fn enter_call_value(
        &mut self,
        function: Value,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        call: Call,
    ) -> Result<Option<u32>, VMError> {
        let mut call = call;
        // 10.4.1.1 puts the arguments a bind kept in front of the ones the
        // call site passes, which no register of the caller holds together.
        if Self::bound_with_arguments(function, heap) {
            return self.begin_bound_call(function, call, units, active_feedback, heap, realm);
        }
        let function_ref = self.resolve_callee(function, &mut call, heap, realm)?;
        let kind = heap
            .get_object(function_ref)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .kind
            .clone();
        let (unit, code_id, context) = match kind {
            ObjectKind::Function {
                unit,
                code_id,
                context,
                ..
            } => (unit, code_id, context),
            ObjectKind::NativeFunction { id, .. } => {
                let intrinsic = Intrinsic::from_id(id).ok_or(VMError::TypeError)?;
                // A native identifier and a bytecode index are separate
                // namespaces; the call site profile keeps them apart. A call an
                // operation opened has no call site and records nothing.
                if call.resume.is_none() {
                    active_feedback
                        .record_call(call.slot, NATIVE_CALL_TARGET | u64::from(id))
                        .ok_or(VMError::InvalidFeedbackVector)?;
                }
                // 22.1.3 converts the `this` value before it reads any
                // argument, so the receiver leaves first.
                if intrinsic.coerces_its_receiver() && call.receiver.is_object() {
                    return self.begin_receiver_coercion(
                        intrinsic,
                        PrimitiveHint::String,
                        call,
                        units,
                        active_feedback,
                        heap,
                        realm,
                    );
                }
                // 7.1.17 of an Object argument is a call of a method of the
                // object, and the native has no frame to make it from: it
                // leaves and runs again with the primitive in its place.
                if let Some((index, hint)) = self.next_coercion(intrinsic, &call, heap, realm)? {
                    return self.begin_coercion(
                        intrinsic,
                        index,
                        hint,
                        call,
                        units,
                        active_feedback,
                        heap,
                        realm,
                    );
                }
                return self.dispatch_native(intrinsic, call, units, active_feedback, heap, realm);
            }
            _ => return Err(type_error(heap, realm, "value is not callable")),
        };
        // The function holds the unit it was compiled with, so its index is
        // resolved there and not in the Script that is running.
        let callee = units
            .table
            .root(unit)
            .and_then(|root| root.functions.get(code_id as usize))
            .ok_or(VMError::InvalidBytecode(
                VerificationError::FunctionOutOfBounds {
                    pc: call.return_pc.saturating_sub(1),
                    index: code_id,
                },
            ))?;
        // 15.7.14 gives a class constructor a `[[Call]]` that throws, so the
        // body only ever runs under `new`.
        if callee.class_constructor && call.construct.is_none() {
            return Err(type_error(
                heap,
                realm,
                "a class constructor cannot be called without new",
            ));
        }
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
        if callee.this_register.is_some() {
            call.receiver = Self::bind_this(call.receiver, callee.strict, heap, realm)?;
        }
        let next_frame = self.open_frame(units.active, callee, call, function_ref, heap)?;
        if call.resume.is_none() {
            active_feedback
                .record_call(call.slot, bytecode_call_target(unit, code_id))
                .ok_or(VMError::InvalidFeedbackVector)?;
        }
        self.frames.push(FrameHeader {
            caller_fp: self.fp,
            return_pc: call.return_pc,
            caller_code_id: call.caller_code_id,
            caller_unit: self.unit,
            caller_binding_count: self.active_binding_count,
            caller_context: self.current_context,
            resume: call.resume,
            construct: call.construct,
            arguments: Some(FrameArguments {
                start: call.arg_start,
                count: call.arg_count,
                callee: call.func,
            }),
        });
        self.unit = unit;
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
        heap: &GenerationalHeap,
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
        // A walk of 23.1.3 has no frame, so its three arguments come from the
        // state the collector traces rather than from registers of the caller.
        // They are read here, after `bind_this` has run, because that may
        // allocate and a value taken before it would name a moved object.
        if let Some(Resume::Setter { value }) = call.resume {
            // A setter has no frame to take its argument from either, so the
            // value written comes out of the root that held it across the
            // allocation `bind_this` may have made.
            if callee.parameter_count > 0 {
                *self
                    .stack
                    .get_mut(next_frame)
                    .ok_or(VMError::StackOverflow)? =
                    heap.root_value(value).unwrap_or(VALUE_UNDEFINED);
            }
        } else if let Some(arguments) = call.resume.and_then(Resume::list) {
            // 20.2.3.1 and 28.1.1 pass a List, which the Array the root names
            // holds; the frame takes as many of them as it has parameters.
            let list = heap
                .root_value(arguments)
                .and_then(Value::as_object)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let store = heap
                .get_object(list)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .elements
                .and_then(|elements| heap.get_elements(elements));
            for index in 0..u32::from(call.arg_count.min(callee.parameter_count)) {
                let argument = store
                    .and_then(|store| store.get(index))
                    .unwrap_or(VALUE_UNDEFINED);
                *self
                    .stack
                    .get_mut(next_frame.saturating_add(index as usize))
                    .ok_or(VMError::StackOverflow)? = argument;
            }
        } else if let Some(Resume::Iteration { state }) = call.resume {
            let arguments = Self::iteration_arguments(state, heap)?;
            for (index, argument) in arguments.into_iter().enumerate() {
                if index >= usize::from(callee.parameter_count) {
                    break;
                }
                *self
                    .stack
                    .get_mut(next_frame.saturating_add(index))
                    .ok_or(VMError::StackOverflow)? = argument;
            }
        } else {
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
        // 9.4.3 answers the `[[NewTarget]]` of the call, which `new` and
        // 7.3.15 give and every other call leaves undefined.
        let new_target = core::mem::replace(&mut self.pending_new_target, VALUE_UNDEFINED);
        if let Some(register) = callee.new_target_register {
            *self
                .stack
                .get_mut(next_frame.saturating_add(register.0 as usize))
                .ok_or(VMError::StackOverflow)? = new_target;
        }
        // 13.3.7.3 reads the `[[HomeObject]]` of the running function, which
        // is a value of the closure and not of the frame, so it travels into a
        // register the collector sees.
        if let Some(home_register) = callee.home_register {
            let home = heap.function_home(function).unwrap_or(VALUE_UNDEFINED);
            *self
                .stack
                .get_mut(next_frame.saturating_add(home_register.0 as usize))
                .ok_or(VMError::StackOverflow)? = home;
        }
        Ok(next_frame)
    }

    /// Runs one native intrinsic and returns its value.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for the exceptions the intrinsic's algorithm
    /// specifies, and a heap error when a value cannot be materialized.
    #[expect(
        clippy::too_many_lines,
        reason = "one function names every intrinsic beside the one that runs it"
    )]
    fn call_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        call: Call,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        match intrinsic {
            // 27.2.3.1 step 6 calls the executor, which only a frame the
            // instruction opens can do.
            Intrinsic::PromiseConstructor => {
                if call.construct.is_some() {
                    return Err(VMError::Unsupported("a Promise of a derived class"));
                }
                Err(type_error(
                    heap,
                    realm,
                    "Promise cannot be called without new",
                ))
            }
            Intrinsic::PromiseResolveFunction | Intrinsic::PromiseRejectFunction => {
                let function = self.read_reg(call.func)?;
                let value = self.call_argument(&call, 0, heap)?;
                self.settle_through(
                    function,
                    value,
                    intrinsic == Intrinsic::PromiseRejectFunction,
                    heap,
                    realm,
                )?;
                Ok(VALUE_UNDEFINED)
            }
            Intrinsic::Print => self.print_line(&call, heap, realm),
            Intrinsic::PromiseAll | Intrinsic::PromiseRace | Intrinsic::PromiseAllSettled => {
                self.promise_combinator(intrinsic, &call, heap, realm)
            }
            Intrinsic::PromiseWithResolvers => Self::promise_with_resolvers(heap, realm),
            Intrinsic::PromiseAllElement
            | Intrinsic::PromiseAllSettledFulfilled
            | Intrinsic::PromiseAllSettledRejected => {
                let function = self.read_reg(call.func)?;
                let value = self.call_argument(&call, 0, heap)?;
                self.settle_element(intrinsic, function, value, heap, realm)?;
                Ok(VALUE_UNDEFINED)
            }
            Intrinsic::PromiseResolve => self.promise_resolve(call, heap, realm),
            Intrinsic::PromiseReject => self.promise_reject(call, heap, realm),
            Intrinsic::PromisePrototypeThen => self.promise_then(call, heap, realm),
            Intrinsic::PromisePrototypeCatch => self.promise_catch(call, heap, realm),
            Intrinsic::ObjectPrototypeIsPrototypeOf => Self::is_prototype_of(
                self.call_argument(&call, 0, heap)?,
                call.receiver,
                heap,
                realm,
            ),
            Intrinsic::ObjectPrototypeHasOwnProperty
            | Intrinsic::ObjectPrototypePropertyIsEnumerable => Self::own_property_test(
                intrinsic,
                self.call_argument(&call, 0, heap)?,
                call,
                heap,
                realm,
            ),
            Intrinsic::ObjectPrototypeToString => self.object_to_string(call.receiver, heap, realm),
            // 28.1.2 leaves to the constructor it was given, so it never
            // answers here.
            Intrinsic::ReflectConstruct => Err(VMError::InvalidFeedbackVector),
            // 20.5.3.4 joins the `name` and the `message` the Error holds.
            Intrinsic::ErrorPrototypeToString => self.error_text(call.receiver, heap, realm),
            // 20.2.3 accepts any argument and answers undefined.
            Intrinsic::FunctionPrototype => Ok(VALUE_UNDEFINED),
            // 20.1.3.7 is `ToObject(this value)` and nothing else.
            Intrinsic::ObjectPrototypeValueOf => {
                Self::coerce_object(call.receiver, heap, realm).map(Value::from_object)
            }
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
            | Intrinsic::StringPrototypeTrimStart
            | Intrinsic::StringPrototypeIsWellFormed
            | Intrinsic::StringPrototypeToWellFormed
            | Intrinsic::StringPrototypeSubstr
            | Intrinsic::StringPrototypeLocaleCompare => {
                self.call_string_intrinsic(intrinsic, call, units, heap, realm)
            }
            Intrinsic::StringPrototypeSplit => self.call_split_intrinsic(&call, units, heap, realm),
            // 19.2.2 and 19.2.3 answer about the Number their argument is.
            Intrinsic::IsNaN | Intrinsic::IsFinite => {
                let number = primitive_number(self.call_argument(&call, 0, heap)?, heap)?;
                Ok(Value::from_bool(if intrinsic == Intrinsic::IsNaN {
                    number.is_nan()
                } else {
                    number.is_finite()
                }))
            }
            // 19.2.5 and 19.2.4 read a Number out of the text of their first
            // argument. The walk charges for the text it passes over.
            Intrinsic::ParseInt | Intrinsic::ParseFloat => {
                let text = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                self.fuel = self
                    .fuel
                    .checked_sub(u64::try_from(text.len()).unwrap_or(u64::MAX))
                    .ok_or(VMError::OutOfFuel)?;
                let number = if intrinsic == Intrinsic::ParseFloat {
                    crate::number::parse_float(&text)
                } else {
                    let radix = crate::value::number_uint32(primitive_number(
                        self.call_argument(&call, 1, heap)?,
                        heap,
                    )?);
                    crate::number::parse_integer(&text, i32::from_ne_bytes(radix.to_ne_bytes()))
                };
                Ok(Value::from_f64(number))
            }
            Intrinsic::StringPrototypeMatch | Intrinsic::StringPrototypeSearch => {
                self.call_string_regexp_intrinsic(intrinsic, &call, units, heap, realm)
            }
            // 22.1.3.19 gives the search value its own say through
            // `@@replace`, which 22.2.6.11 answers for a RegExp.
            Intrinsic::StringPrototypeReplace => self.string_replace(&call, units, heap, realm),
            Intrinsic::RegExpPrototypeMatch
            | Intrinsic::RegExpPrototypeSearch
            | Intrinsic::RegExpPrototypeSplit => {
                self.call_regexp_symbol_intrinsic(intrinsic, &call, heap, realm)
            }
            Intrinsic::RegExpPrototypeReplace => {
                let text = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                let receiver = call
                    .receiver
                    .as_object()
                    .ok_or_else(|| type_error(heap, realm, "this value is not a RegExp"))?;
                let replacement = self.call_argument(&call, 1, heap)?;
                self.regexp_replace(receiver, &text, replacement, heap, realm)
            }
            // 27.1.2.1, 23.1.2.5 and 22.2.5.2 answer the value they were
            // called on.
            Intrinsic::IteratorPrototypeIterator | Intrinsic::SpeciesGetter => Ok(call.receiver),
            // 23.1.2.3 makes an Array of the arguments it was given.
            Intrinsic::ArrayOf => {
                let mut values = Vec::new();
                for index in 0..call.arg_count {
                    values.push(self.call_argument(&call, index, heap)?);
                }
                Self::array_of(values, heap, realm)
            }
            Intrinsic::ArrayPrototypeValues
            | Intrinsic::ArrayPrototypeKeys
            | Intrinsic::ArrayPrototypeEntries
            | Intrinsic::ArrayIteratorPrototypeNext => {
                Self::call_iterator_intrinsic(intrinsic, call, heap, realm)
            }
            Intrinsic::ArrayPrototypeAt
            | Intrinsic::ArrayPrototypeIncludes
            | Intrinsic::ArrayPrototypeIndexOf
            | Intrinsic::ArrayPrototypeLastIndexOf
            | Intrinsic::ArrayPrototypeJoin
            | Intrinsic::ArrayPrototypePop
            | Intrinsic::ArrayPrototypePush
            | Intrinsic::ArrayPrototypeReverse
            | Intrinsic::ArrayPrototypeSlice
            | Intrinsic::ArrayPrototypeToString => {
                self.call_array_intrinsic(intrinsic, call, None, units, heap, realm)
            }
            // Never reached: `enter_call_value` sends these to the walk of
            // 23.1.3 before an intrinsic is called at all.
            Intrinsic::ArrayPrototypeForEach
            | Intrinsic::ArrayPrototypeMap
            | Intrinsic::ArrayPrototypeFilter
            | Intrinsic::ArrayPrototypeEvery
            | Intrinsic::ArrayPrototypeSome
            | Intrinsic::ArrayPrototypeFind
            | Intrinsic::ArrayPrototypeFindIndex
            | Intrinsic::ArrayPrototypeFindLast
            | Intrinsic::ArrayPrototypeFindLastIndex
            | Intrinsic::ArrayPrototypeReduce
            | Intrinsic::ArrayPrototypeReduceRight
            | Intrinsic::ArrayFrom => Err(VMError::TypeError),
            Intrinsic::ArrayPrototypeShift
            | Intrinsic::ArrayPrototypeUnshift
            | Intrinsic::ArrayPrototypeSplice
            | Intrinsic::ArrayPrototypeFill
            | Intrinsic::ArrayPrototypeCopyWithin
            | Intrinsic::ArrayPrototypeConcat
            | Intrinsic::ArrayPrototypeWith
            | Intrinsic::ArrayPrototypeFlat
            | Intrinsic::ArrayPrototypeSort
            | Intrinsic::ArrayPrototypeToSorted
            | Intrinsic::ArrayPrototypeToSpliced
            | Intrinsic::ArrayPrototypeToReversed => {
                self.call_array_edit_intrinsic(intrinsic, call, None, heap, realm)
            }
            // 20.2.1.1 builds the source text of a function and asks the
            // embedding for a unit of it.
            Intrinsic::FunctionConstructor => {
                self.create_dynamic_function(&call, units, heap, realm)
            }
            // 19.2.1 evaluates a Script, which the embedding runs.
            Intrinsic::Eval => self.perform_eval(&call, units, heap, realm),
            Intrinsic::ArrayIsArray
            | Intrinsic::FunctionPrototypeCall
            | Intrinsic::MathPow
            | Intrinsic::MathAbs
            | Intrinsic::MathCeil
            | Intrinsic::MathFloor
            | Intrinsic::MathTrunc
            | Intrinsic::MathRound
            | Intrinsic::MathSign
            | Intrinsic::MathClz32
            | Intrinsic::MathImul
            | Intrinsic::MathFround
            | Intrinsic::MathSin => Self::call_plain_intrinsic(
                intrinsic,
                self.call_argument(&call, 0, heap)?,
                self.call_argument(&call, 1, heap)?,
                heap,
            ),
            Intrinsic::MathMax | Intrinsic::MathMin => {
                self.call_math_extremum(intrinsic, call, heap)
            }
            Intrinsic::FunctionPrototypeBind => self.bind_function(call, heap, realm),
            // 20.2.3.1 and 28.1.1 leave to call what they were given, so they
            // never answer here.
            Intrinsic::FunctionPrototypeApply | Intrinsic::ReflectApply => {
                Err(VMError::InvalidFeedbackVector)
            }
            Intrinsic::ArrayConstructor => self.construct_array(call, heap, realm),
            Intrinsic::StringConstructor => Self::call_string_constructor(
                call.construct.is_some(),
                (call.arg_count > 0)
                    .then(|| self.call_argument(&call, 0, heap))
                    .transpose()?,
                heap,
                realm,
            ),
            // 21.1.1.1: a call with no argument is +0, and every other value
            // goes through ToNumber. `new` makes the Number exotic object of
            // 21.1.3, which this engine has not built.
            Intrinsic::NumberConstructor => {
                let argument = self.call_argument(&call, 0, heap)?;
                let number = if call.arg_count == 0 {
                    0.0
                } else {
                    primitive_number(argument, heap)?
                };
                if call.construct.is_some() {
                    // 21.1.3: the wrapper holds `[[NumberData]]`.
                    return Self::wrapper(
                        ObjectKind::NumberWrapper(number),
                        realm.number_prototype(heap)?,
                        heap,
                    );
                }
                Ok(Value::from_f64(number))
            }
            // 20.3.1.1: ToBoolean of the argument, which is false for no
            // argument at all. `new` makes the Boolean exotic object of
            // 20.3.3, whose [[BooleanData]] this engine has not built.
            Intrinsic::BooleanConstructor => {
                let boolean = Self::to_boolean(self.call_argument(&call, 0, heap)?, heap)?;
                if call.construct.is_some() {
                    // 20.3.3: the wrapper holds `[[BooleanData]]`.
                    return Self::wrapper(
                        ObjectKind::BooleanWrapper(boolean),
                        realm.boolean_prototype(heap)?,
                        heap,
                    );
                }
                Ok(Value::from_bool(boolean))
            }
            // 21.1.3.7 and 20.3.3.3 answer the data the wrapper holds, and
            // 21.1.3.6 and 20.3.3.2 its text. A receiver of another kind is a
            // TypeError, which is what `thisNumberValue` and
            // `thisBooleanValue` say.
            Intrinsic::NumberPrototypeValueOf
            | Intrinsic::NumberPrototypeToString
            | Intrinsic::BooleanPrototypeValueOf
            | Intrinsic::BooleanPrototypeToString => {
                self.wrapped_value(intrinsic, &call, heap, realm)
            }
            // 22.1.3.32 and 22.1.3.28 are `thisStringValue`, which answers a
            // String and the `[[StringData]]` of a wrapper.
            // 25.5.1 and 25.5.2, neither of which takes the function the
            // other argument may be: a reviver and a replacer are calls, and
            // a native has no frame to make one from.
            Intrinsic::JsonParse | Intrinsic::JsonStringify => {
                self.call_json_intrinsic(intrinsic, &call, heap, realm)
            }
            // 22.2.3.1 makes a RegExp of a pattern and flags 22.2.4.1
            // compiles where the call stands.
            Intrinsic::RegExpConstructor => self.construct_regexp(&call, heap, realm),
            Intrinsic::RegExpPrototypeExec
            | Intrinsic::RegExpPrototypeTest
            | Intrinsic::RegExpPrototypeToString => {
                self.call_regexp_intrinsic(intrinsic, &call, heap, realm)
            }
            // 22.2.6.4 reads the eight flag accessors of the receiver rather
            // than its own `[[OriginalFlags]]`, so it takes any object.
            Intrinsic::RegExpPrototypeFlags => self.regexp_flags(call.receiver, heap, realm),
            Intrinsic::RegExpPrototypeSource => self.regexp_source(call.receiver, heap, realm),
            Intrinsic::RegExpPrototypeHasIndices
            | Intrinsic::RegExpPrototypeGlobal
            | Intrinsic::RegExpPrototypeIgnoreCase
            | Intrinsic::RegExpPrototypeMultiline
            | Intrinsic::RegExpPrototypeDotAll
            | Intrinsic::RegExpPrototypeUnicode
            | Intrinsic::RegExpPrototypeUnicodeSets
            | Intrinsic::RegExpPrototypeSticky => {
                Self::regexp_flag(intrinsic, call.receiver, heap, realm)
            }
            // 20.2.3.5 answers the source text of the grammar node a function
            // was written as, and the NativeFunction string of 20.2.3.5 step 3
            // for one this engine wrote.
            Intrinsic::FunctionPrototypeToString => {
                self.function_source(call.receiver, units, heap, realm)
            }
            // 20.4.1.1: `Symbol` is not a constructor, and its description
            // is undefined or the text of its argument.
            // 22.1.2.1: each argument is a code unit of the answer.
            Intrinsic::StringFromCharCode => {
                let mut units = Vec::with_capacity(usize::from(call.arg_count));
                for index in 0..call.arg_count {
                    let argument = self.call_argument(&call, index, heap)?;
                    // 7.1.7 keeps the low sixteen bits, which is the code unit.
                    let unit = crate::value::number_uint32(primitive_number(argument, heap)?);
                    units.push(u16::try_from(unit & 0xFFFF).unwrap_or_default());
                }
                if units.len() > self.string_units_limit {
                    return Err(VMError::StringLimit);
                }
                self.allocate_string(heap, &units)
            }
            // 22.1.2.2: each argument is a code point, which step 2.c refuses
            // where it is not an integer of the Unicode range.
            Intrinsic::StringFromCodePoint => {
                let mut units = Vec::with_capacity(usize::from(call.arg_count));
                for index in 0..call.arg_count {
                    let argument = self.call_argument(&call, index, heap)?;
                    let number = primitive_number(argument, heap)?;
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "the range is checked before the cast is used"
                    )]
                    let point = number as u32;
                    #[expect(
                        clippy::float_cmp,
                        reason = "step 2.c asks for an integral binary64 value"
                    )]
                    let integral = number.is_finite() && f64::from(point) == number;
                    if !integral || point > 0x0010_FFFF {
                        return Err(raise(
                            heap,
                            realm,
                            super::realm::NativeErrorKind::RangeError,
                            "not a valid code point",
                        ));
                    }
                    match char::from_u32(point) {
                        Some(point) => {
                            let mut buffer = [0u16; 2];
                            units.extend_from_slice(point.encode_utf16(&mut buffer));
                        }
                        // A lone surrogate is a code point of the language and
                        // no character of Rust, so it is its own code unit.
                        None => units.push(u16::try_from(point & 0xFFFF).unwrap_or_default()),
                    }
                }
                if units.len() > self.string_units_limit {
                    return Err(VMError::StringLimit);
                }
                self.allocate_string(heap, &units)
            }
            Intrinsic::StringRaw => self.string_raw(&call, heap, realm),
            Intrinsic::SymbolConstructor => {
                if call.construct.is_some() {
                    return Err(type_error(heap, realm, "Symbol is not a constructor"));
                }
                let description = self.call_argument(&call, 0, heap)?;
                let description = if description.is_undefined() {
                    None
                } else {
                    Some(property_name_units(description, heap)?)
                };
                self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                Ok(Value::from_symbol(heap.create_symbol(description)?))
            }
            // 20.4.2.2 and 20.4.2.3 are the two halves of the registry.
            Intrinsic::SymbolFor => {
                let key = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                Ok(Value::from_symbol(heap.registered_symbol(&key)?))
            }
            Intrinsic::SymbolKeyFor => {
                let Some(symbol) = self.call_argument(&call, 0, heap)?.as_symbol() else {
                    return Err(type_error(heap, realm, "value is not a Symbol"));
                };
                match heap.symbol_registry_key(symbol).map(<[u16]>::to_vec) {
                    Some(key) => self.allocate_string(heap, &key),
                    None => Ok(VALUE_UNDEFINED),
                }
            }
            // 20.4.3.3 and 20.4.3.4 take a `this` that is a Symbol or the
            // wrapper 7.1.18 makes of one.
            Intrinsic::SymbolPrototypeToString | Intrinsic::SymbolPrototypeValueOf => {
                let Some(symbol) = Self::this_symbol_value(call.receiver, heap) else {
                    return Err(type_error(heap, realm, "value is not a Symbol"));
                };
                if intrinsic == Intrinsic::SymbolPrototypeValueOf {
                    return Ok(Value::from_symbol(symbol));
                }
                let text = Self::symbol_descriptive_string(symbol, heap);
                self.allocate_string(heap, &text)
            }
            // 10.2.4.1 throws whenever it is called, however it is reached.
            Intrinsic::ThrowTypeError => Err(type_error(
                heap,
                realm,
                "the callee of a strict arguments object cannot be read",
            )),
            Intrinsic::StringPrototypeValueOf | Intrinsic::StringPrototypeToString => {
                if call.receiver.is_string() {
                    return Ok(call.receiver);
                }
                call.receiver
                    .as_object()
                    .and_then(|object| Self::string_data(object, heap))
                    .ok_or_else(|| {
                        type_error(
                            heap,
                            realm,
                            "this value is not of the type the method belongs to",
                        )
                    })
            }
            Intrinsic::NumberIsFinite
            | Intrinsic::NumberIsInteger
            | Intrinsic::NumberIsNaN
            | Intrinsic::NumberIsSafeInteger => Ok(Value::from_bool(Self::number_predicate(
                intrinsic,
                self.call_argument(&call, 0, heap)?,
            ))),
            Intrinsic::ErrorConstructor
            | Intrinsic::EvalErrorConstructor
            | Intrinsic::RangeErrorConstructor
            | Intrinsic::ReferenceErrorConstructor
            | Intrinsic::SyntaxErrorConstructor
            | Intrinsic::TypeErrorConstructor
            | Intrinsic::UriErrorConstructor
            | Intrinsic::ObjectConstructor
            | Intrinsic::ObjectDefineProperty
            | Intrinsic::ObjectGetOwnPropertyDescriptor
            | Intrinsic::ObjectCreate
            | Intrinsic::ObjectDefineProperties
            | Intrinsic::ObjectGetPrototypeOf
            | Intrinsic::ObjectSetPrototypeOf
            | Intrinsic::ReflectSetPrototypeOf
            | Intrinsic::ObjectKeys
            | Intrinsic::ObjectIs
            | Intrinsic::ObjectHasOwn
            | Intrinsic::ObjectPreventExtensions
            | Intrinsic::ObjectIsExtensible
            | Intrinsic::ObjectSeal
            | Intrinsic::ObjectIsSealed
            | Intrinsic::ObjectFreeze
            | Intrinsic::ObjectIsFrozen
            | Intrinsic::ObjectValues
            | Intrinsic::ObjectEntries
            | Intrinsic::ReflectDefineProperty
            | Intrinsic::ReflectDeleteProperty
            | Intrinsic::ReflectGet
            | Intrinsic::ReflectGetOwnPropertyDescriptor
            | Intrinsic::ReflectGetPrototypeOf
            | Intrinsic::ReflectHas
            | Intrinsic::ReflectIsExtensible
            | Intrinsic::ReflectOwnKeys
            | Intrinsic::ReflectPreventExtensions
            | Intrinsic::ObjectGetOwnPropertyNames => Self::call_object_intrinsic(
                intrinsic,
                self.call_argument(&call, 0, heap)?,
                self.call_argument(&call, 1, heap)?,
                self.call_argument(&call, 2, heap)?,
                heap,
                realm,
            ),
        }
    }

    /// The argument at this position, or undefined when the call passed
    /// fewer, which 10.2.11 binds for a parameter the caller left out.
    fn call_argument(
        &self,
        call: &Call,
        index: u16,
        heap: &GenerationalHeap,
    ) -> Result<Value, VMError> {
        if index >= call.arg_count {
            return Ok(VALUE_UNDEFINED);
        }
        // 7.3.15 and 20.2.3.1 pass a List, which no register of the caller
        // holds; the Array the root names holds it instead.
        if let Some(Resume::Spread { arguments }) = call.resume {
            let list = heap
                .root_value(arguments)
                .and_then(Value::as_object)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let store = heap
                .get_object(list)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .elements
                .and_then(|elements| heap.get_elements(elements));
            return Ok(store
                .and_then(|store| store.get(u32::from(index)))
                .unwrap_or(VALUE_UNDEFINED));
        }
        let slot = self
            .fp
            .checked_add(call.arg_start.0 as usize)
            .and_then(|start| start.checked_add(index as usize))
            .ok_or(VMError::InvalidRegister)?;
        self.stack
            .get(slot)
            .copied()
            .ok_or(VMError::InvalidRegister)
    }

    /// `Function.prototype.bind` of 20.2.3.2: the bound function exotic object
    /// of 10.4.1.
    ///
    /// `[[BoundArguments]]` is empty here. A bound argument would have to be
    /// put in front of the arguments the call passes, which lie in the
    /// registers of the caller and cannot be pushed apart, so a `bind` that
    /// carries one names that instead of dropping it.
    fn bind_function(
        &self,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let target = call.receiver;
        if !Self::is_callable(target, heap) {
            return Err(type_error(
                heap,
                realm,
                "bind called on a value that is not callable",
            ));
        }
        let receiver = if call.arg_count == 0 {
            VALUE_UNDEFINED
        } else {
            self.read_reg(call.arg_start)?
        };
        // Step 2 keeps every argument after the `this` value, which 10.4.1.1
        // puts in front of the ones the call site passes. Everything the bind
        // holds becomes a root of its own, because the allocations below move
        // what a register of the caller does not name.
        heap.enter_scope();
        let bound = (|| {
            let held_target = heap.push_root(target)?;
            let held_receiver = heap.push_root(receiver)?;
            let mut held_arguments = Vec::new();
            for index in 1..call.arg_count {
                held_arguments.push(heap.push_root(self.call_argument(&call, index, heap)?)?);
            }
            let count = held_arguments.len();
            // 20.2.3.2 steps 5 and 8 read the target before the bind exists,
            // so no allocation stands between the read and the object.
            let target_object = target.as_object().ok_or(VMError::TypeError)?;
            let name = Self::bound_name(target_object, heap)?;
            let length = Self::bound_length(target_object, count, heap)?;
            let arguments = if count == 0 {
                VALUE_UNDEFINED
            } else {
                let array = realm.array(heap, u32::try_from(count).unwrap_or(u32::MAX))?;
                for (index, held) in held_arguments.into_iter().enumerate() {
                    let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
                    let value = heap.root_value(held).unwrap_or(VALUE_UNDEFINED);
                    heap.set_array_element(array, index, value)?;
                }
                Value::from_object(array)
            };
            let held_arguments = heap.push_root(arguments)?;
            // 10.4.1.3 step 1 gives the bound function the Prototype of its
            // target.
            let target_object = heap
                .root_value(held_target)
                .and_then(Value::as_object)
                .ok_or(VMError::TypeError)?;
            let prototype = heap
                .get_object(target_object)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .prototype;
            let shape = heap.shapes.root_shape();
            let bound = heap.allocate_object(shape, prototype)?;
            heap.set_object_kind(
                bound,
                ObjectKind::BoundFunction {
                    target: heap.root_value(held_target).unwrap_or(VALUE_UNDEFINED),
                    receiver: heap.root_value(held_receiver).unwrap_or(VALUE_UNDEFINED),
                    arguments: heap.root_value(held_arguments).unwrap_or(VALUE_UNDEFINED),
                },
            )?;
            let held_bound = heap.push_root(Value::from_object(bound))?;
            Self::define_bound_length_and_name(held_bound, length, &name, heap)?;
            heap.root_value(held_bound).ok_or(VMError::TypeError)
        })();
        heap.exit_scope();
        bound
    }

    /// The `length` 20.2.3.2 step 5 gives a bound function: an own `length` of
    /// the target that is a Number, less the arguments the bind kept, and zero
    /// where the target has no such property.
    fn bound_length(
        target: ObjectRef,
        count: usize,
        heap: &mut GenerationalHeap,
    ) -> Result<f64, VMError> {
        let key = PropertyKey::String(heap.strings.intern("length")?);
        let held = heap
            .own_named_flags(target, key)?
            .filter(|flags| !flags.is_accessor)
            .and(heap.lookup_named(target, key)?)
            .map(|property| property.value)
            .and_then(Value::as_f64);
        #[expect(
            clippy::cast_precision_loss,
            reason = "an argument count is below 2^16"
        )]
        let taken = count as f64;
        Ok(held.map_or(0.0, |length| (length - taken).max(0.0)))
    }

    /// The `name` 20.2.3.2 step 8 gives a bound function: "bound " before the
    /// `name` of the target where that is a String.
    fn bound_name(target: ObjectRef, heap: &mut GenerationalHeap) -> Result<Vec<u16>, VMError> {
        let key = PropertyKey::String(heap.strings.intern("name")?);
        let held = heap
            .lookup_named(target, key)?
            .filter(|property| !property.flags.is_accessor)
            .map(|property| property.value)
            .filter(|value| value.is_string())
            .and_then(|value| heap.strings.to_utf16(value))
            .unwrap_or_default();
        let mut units: Vec<u16> = "bound ".encode_utf16().collect();
        units.extend_from_slice(&held);
        Ok(units)
    }

    /// Writes the `length` and `name` 20.2.3.2 gives a bound function.
    fn define_bound_length_and_name(
        bound: Root,
        length: f64,
        name: &[u16],
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let key = PropertyKey::String(heap.strings.intern("length")?);
        let object = heap
            .root_value(bound)
            .and_then(Value::as_object)
            .ok_or(VMError::TypeError)?;
        heap.define_own_named(
            object,
            key,
            Value::from_f64(length),
            super::realm::builtin_metadata(),
        )?;
        let key = PropertyKey::String(heap.strings.intern("name")?);
        let text = heap.strings.allocate_units(name)?;
        let object = heap
            .root_value(bound)
            .and_then(Value::as_object)
            .ok_or(VMError::TypeError)?;
        heap.define_own_named(
            object,
            key,
            Value::from_string(text),
            super::realm::builtin_metadata(),
        )?;
        Ok(())
    }

    /// Whether a bound function of the chain carries arguments of its own,
    /// which 10.4.1.1 puts in front of the ones the call site passes.
    fn bound_with_arguments(function: Value, heap: &GenerationalHeap) -> bool {
        let mut current = function;
        while let Some(object) = current.as_object() {
            let Some(ObjectKind::BoundFunction {
                target, arguments, ..
            }) = heap.get_object(object).map(|entry| &entry.kind)
            else {
                return false;
            };
            if !arguments.is_undefined() {
                return true;
            }
            current = *target;
        }
        false
    }

    /// The object a call finally reaches, and the call that reaches it.
    ///
    /// 20.2.3.3 calls the function it was reached through, with the first
    /// argument as the `this` value and the rest shifted down by one. The
    /// arguments lie in neighbouring registers, so the shift is where the
    /// list starts and how long it is, and forwarding again drops one more
    /// argument, which is why a chain of them ends.
    fn resolve_callee(
        &self,
        function: Value,
        call: &mut Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<ObjectRef, VMError> {
        let mut function = function;
        loop {
            // 13.3.6.1: a callee that is not callable is a TypeError, not a
            // failure of execution.
            let Some(function_ref) = function.as_object() else {
                return Err(type_error(heap, realm, "value is not callable"));
            };
            let forwards = matches!(
                heap.get_object(function_ref)
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?
                    .kind,
                ObjectKind::NativeFunction { id, .. }
                    if id == Intrinsic::FunctionPrototypeCall.id()
            );
            // 10.4.1.1 calls the target with the `this` value the bind gave
            // it, whatever the call site passed.
            if let ObjectKind::BoundFunction {
                target, receiver, ..
            } = heap
                .get_object(function_ref)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .kind
            {
                function = target;
                call.receiver = receiver;
                continue;
            }
            if !forwards {
                return Ok(function_ref);
            }
            function = call.receiver;
            call.receiver = if call.arg_count == 0 {
                VALUE_UNDEFINED
            } else {
                self.read_reg(call.arg_start)?
            };
            call.arg_start = Reg(call
                .arg_start
                .0
                .checked_add(1)
                .ok_or(VMError::InvalidRegister)?);
            call.arg_count = call.arg_count.saturating_sub(1);
        }
    }

    /// `IsArray` of 7.2.2, which answers for an Array exotic object and, for a
    /// Proxy, for what it wraps.
    fn is_array(value: Value, heap: &GenerationalHeap) -> bool {
        value
            .as_object()
            .and_then(|object| heap.get_object(object))
            .is_some_and(|object| matches!(object.kind, ObjectKind::Array { .. }))
    }

    /// The intrinsic of a callee that constructs without 10.1.13.
    ///
    /// `%Array%` answers an Array of its own, so the object
    /// `OrdinaryCreateFromConstructor` would make is never the value `new`
    /// takes; the same holds for an error and for `%String%`, which names the
    /// exotic object it would have to make. Every other native of this Realm
    /// has no `[[Construct]]`, and 7.3.15 refuses it where it is reached.
    fn native_constructor(&self, func: Reg, heap: &GenerationalHeap) -> Option<Intrinsic> {
        let object = self.read_reg(func).ok()?.as_object()?;
        let ObjectKind::NativeFunction { id, .. } = heap.get_object(object)?.kind else {
            return None;
        };
        Intrinsic::from_id(id).filter(|intrinsic| {
            matches!(
                intrinsic,
                Intrinsic::ArrayConstructor
                    | Intrinsic::ObjectConstructor
                    | Intrinsic::RegExpConstructor
                    | Intrinsic::ErrorConstructor
                    | Intrinsic::EvalErrorConstructor
                    | Intrinsic::RangeErrorConstructor
                    | Intrinsic::ReferenceErrorConstructor
                    | Intrinsic::SyntaxErrorConstructor
                    | Intrinsic::TypeErrorConstructor
                    | Intrinsic::UriErrorConstructor
                    | Intrinsic::StringConstructor
                    | Intrinsic::NumberConstructor
                    | Intrinsic::BooleanConstructor
                    | Intrinsic::FunctionConstructor
                    | Intrinsic::PromiseConstructor
            )
        })
    }

    /// `Object.prototype.isPrototypeOf` of 20.1.3.3.
    ///
    /// A non-Object argument is false without coercing `this`.
    fn is_prototype_of(
        value: Value,
        receiver: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let Some(mut value) = value.as_object() else {
            return Ok(VALUE_FALSE);
        };
        let object = Self::coerce_object(receiver, heap, realm)?;
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

    /// The four questions 21.1.2 asks about a value.
    ///
    /// None of them coerces: 21.1.2.2 step 1 and its siblings answer false for
    /// anything that is not a Number, where the global `isFinite` and `isNaN`
    /// of 19.2.2 and 19.2.3 take `ToNumber` first.
    fn number_predicate(intrinsic: Intrinsic, value: Value) -> bool {
        let Some(number) = value.as_f64() else {
            return false;
        };
        match intrinsic {
            Intrinsic::NumberIsNaN => number.is_nan(),
            Intrinsic::NumberIsFinite => number.is_finite(),
            // 21.1.2.3 asks whether the Number has no fractional part, which
            // is where its floor is the Number itself, and 21.1.2.5 whether it
            // is one 6.1.6.1 can tell from its neighbours.
            Intrinsic::NumberIsInteger => Self::is_integral(number),
            _ => {
                Self::is_integral(number)
                    && (-9_007_199_254_740_991.0..=9_007_199_254_740_991.0).contains(&number)
            }
        }
    }

    /// Whether a Number has no fractional part, which is where its floor is
    /// the Number itself.
    #[expect(
        clippy::float_cmp,
        reason = "the question 21.1.2.3 asks is exactly whether the two are the same Number"
    )]
    fn is_integral(number: f64) -> bool {
        number.is_finite() && Self::round_toward(number, true) == number
    }

    /// `%String%` of 22.1.1.1.
    ///
    /// A call with no argument is the empty String, and every other value
    /// goes through `ToString`. `new` makes the String exotic object of
    /// 10.4.3, whose `[[StringData]]` and index properties this engine has
    /// not built, so it names that instead of answering the primitive a call
    /// answers.
    fn call_string_constructor(
        construct: bool,
        target: Option<Value>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        // 22.1.1.1 step 1: an argument that is not there is the empty String,
        // where one that is `undefined` is the text of it.
        let units = match target {
            None => alloc::vec::Vec::new(),
            // Step 2.a answers the text 20.4.3.3.1 gives a Symbol, where every
            // other conversion of one is a `TypeError`.
            Some(target) => match target.as_symbol().filter(|_| !construct) {
                Some(symbol) => Self::symbol_descriptive_string(symbol, heap),
                None => property_name_units(target, heap)?,
            },
        };
        let text = heap.strings.allocate_units(&units)?;
        if construct {
            // 22.1.4: the wrapper holds `[[StringData]]`, and 10.4.3 answers
            // its indices and its `length` from there.
            return Self::wrapper(
                ObjectKind::StringWrapper(Value::from_string(text)),
                realm.string_prototype(heap)?,
                heap,
            );
        }
        Ok(Value::from_string(text))
    }

    /// 10.1.6.3 step 2: a name the object does not own yet is added only while
    /// the object is extensible.
    ///
    /// 10.1.9.2 step 3.d answers `false` for a refused write, which 13.15.2
    /// turns into a `TypeError` for a strict Reference and drops otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` for a strict write.
    fn refuses_a_new_property(
        object: ObjectRef,
        strict: bool,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<bool, VMError> {
        if heap.is_extensible(object).unwrap_or(true) {
            return Ok(false);
        }
        if strict {
            return Err(type_error(
                heap,
                realm,
                "cannot add a property to an object that is not extensible",
            ));
        }
        Ok(true)
    }

    /// `OrdinarySetPrototypeOf` of 10.1.2: the same value is always taken, a
    /// different one only while the object is extensible and the chain stays
    /// acyclic.
    fn set_object_prototype(
        object: ObjectRef,
        prototype: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<bool, VMError> {
        let current = heap
            .get_object(object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .prototype;
        if same_value(current, prototype, heap)? {
            return Ok(true);
        }
        if !heap.is_extensible(object).unwrap_or(false) {
            return Ok(false);
        }
        match heap.set_object_prototype(object, prototype) {
            Ok(()) => Ok(true),
            Err(HeapError::PrototypeCycle) => Ok(false),
            Err(error) => Err(VMError::Heap(error)),
        }
    }

    /// `Error.prototype.toString` of 20.5.3.4.
    ///
    /// The clause reads `name` and `message` with 7.3.2 and sends each
    /// through `ToString`. A value only a frame could convert is a gap here,
    /// because this native has none.
    fn error_text(
        &self,
        receiver: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let Some(object) = receiver.as_object() else {
            return Err(type_error(heap, realm, "this value is not an object"));
        };
        let mut parts: [Vec<u16>; 2] = [Vec::new(), Vec::new()];
        for (slot, (field, absent)) in [("name", "Error"), ("message", "")].into_iter().enumerate()
        {
            let key = PropertyKey::String(heap.strings.intern(field)?);
            let found = heap
                .lookup_named(object, key)?
                .map(Self::plain_value)
                .transpose()?
                .unwrap_or(VALUE_UNDEFINED);
            let text = if found.is_undefined() {
                absent.encode_utf16().collect()
            } else if found.is_object() {
                return Err(NUMERIC_CONVERSION_GAP);
            } else {
                let string = self.primitive_string(found, heap, realm)?;
                heap.strings
                    .to_utf16(string)
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?
            };
            *parts.get_mut(slot).ok_or(VMError::TypeError)? = text;
        }
        let [name, message] = parts;
        // Steps 6 and 7: an empty half leaves the other one alone.
        let joined = if name.is_empty() {
            message
        } else if message.is_empty() {
            name
        } else {
            let mut joined = name;
            joined.extend(": ".encode_utf16());
            joined.extend_from_slice(&message);
            joined
        };
        self.allocate_string(heap, &joined)
    }

    /// The number of code units a String exotic object of 10.4.3 wraps.
    fn string_data_length(
        object: ObjectRef,
        heap: &GenerationalHeap,
    ) -> Result<Option<u32>, VMError> {
        let Some(data) = Self::string_data(object, heap) else {
            return Ok(None);
        };
        let length = heap
            .strings
            .length_of(data)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        Ok(Some(u32::try_from(length).unwrap_or(u32::MAX)))
    }

    /// The `[[StringData]]` of a String exotic object of 10.4.3.
    fn string_data(object: ObjectRef, heap: &GenerationalHeap) -> Option<Value> {
        match heap.get_object(object)?.kind {
            ObjectKind::StringWrapper(value) => Some(value),
            _ => None,
        }
    }

    /// The functions 28.1 gives `%Reflect%` that this Realm builds.
    ///
    /// Each one is an operation of clause 20.1.2 without its coercion, and
    /// answers whether it worked where 20.1.2 throws.
    fn call_reflect_intrinsic(
        intrinsic: Intrinsic,
        object: ObjectRef,
        key: Value,
        attributes: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let target = Value::from_object(object);
        match intrinsic {
            // 28.1.7 and 28.1.9 are [[GetPrototypeOf]] and [[IsExtensible]].
            Intrinsic::ReflectGetPrototypeOf => Ok(heap
                .get_object(object)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .prototype),
            Intrinsic::ReflectIsExtensible => Ok(Value::from_bool(
                heap.is_extensible(object).unwrap_or(false),
            )),
            // 28.1.11 answers whether [[PreventExtensions]] worked, and it
            // always does here.
            Intrinsic::ReflectPreventExtensions => {
                heap.prevent_extensions(object)?;
                Ok(VALUE_TRUE)
            }
            // 28.1.10 answers every own key, which for this engine is every
            // own String key: it gives no Script a way to make a Symbol one.
            Intrinsic::ReflectOwnKeys => {
                let names: Vec<Value> = heap
                    .own_keys(object)?
                    .into_iter()
                    .filter_map(|(name, _)| name.as_string())
                    .map(Value::from_string)
                    .collect();
                Self::array_of(names, heap, realm)
            }
            // 28.1.8 is HasProperty of 7.3.11, which walks the chain.
            Intrinsic::ReflectHas => {
                let name = property_key(key, heap)?;
                Ok(Value::from_bool(heap.lookup_named(object, name)?.is_some()))
            }
            // 28.1.5 is [[Get]], which for this engine answers what a read
            // answers, and names the gap a read would name.
            Intrinsic::ReflectGet => {
                let name = property_key(key, heap)?;
                if let Some(property) = heap.lookup_named(object, name)? {
                    return Self::plain_value(property);
                }
                let units = name
                    .as_string()
                    .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
                    .unwrap_or_default();
                Self::absent_property(target, &units, heap, realm)
            }
            // 28.1.4 is [[Delete]] of 10.1.10, answering whether it worked
            // where 13.5.1.2 throws in strict code.
            Intrinsic::ReflectDeleteProperty => {
                let name = property_key(key, heap)?;
                let units = name
                    .as_string()
                    .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
                    .unwrap_or_default();
                let index = array_index_units(&units);
                Ok(Value::from_bool(delete_property(
                    object, name, index, heap,
                )?))
            }
            // 28.1.6 answers the descriptor 6.2.6.4 makes, or undefined.
            Intrinsic::ReflectGetOwnPropertyDescriptor => Self::call_object_intrinsic(
                Intrinsic::ObjectGetOwnPropertyDescriptor,
                target,
                key,
                attributes,
                heap,
                realm,
            ),
            // 28.1.3 defines the descriptor and answers whether it worked.
            _ => {
                let Some(source) = attributes.as_object() else {
                    return Err(type_error(
                        heap,
                        realm,
                        "property descriptor must be an object",
                    ));
                };
                let name = property_key(key, heap)?;
                let descriptor = Self::to_property_descriptor(source, heap, realm)?;
                Ok(Value::from_bool(Self::define_property_from(
                    object,
                    name,
                    &descriptor,
                    heap,
                    realm,
                )?))
            }
        }
    }

    /// An Array of the Realm holding these values, which the collector can
    /// see from the moment it exists.
    fn array_of(
        values: Vec<Value>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let count = u32::try_from(values.len()).map_err(|_| VMError::PropertyLimit)?;
        let array = realm.array(heap, count)?;
        for (index, value) in values.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
            heap.set_array_element(array, index, value)?;
        }
        Ok(Value::from_object(array))
    }

    /// Sets or tests the integrity level of 7.3.14 and 7.3.15.
    ///
    /// Step 1 of each clause answers a value that is not an Object: setting a
    /// level on one answers it unchanged, and testing one answers true,
    /// because there is nothing on it to configure.
    fn integrity_level(
        intrinsic: Intrinsic,
        target: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let tests = matches!(
            intrinsic,
            Intrinsic::ObjectIsSealed | Intrinsic::ObjectIsFrozen
        );
        let Some(object) = target.as_object() else {
            return Ok(if tests { VALUE_TRUE } else { target });
        };
        if !tests {
            // 7.3.15 step 3 prevents extensions and then makes every own
            // property of the object the level asks for.
            heap.prevent_extensions(object)?;
            let frozen = intrinsic == Intrinsic::ObjectFreeze;
            for (key, _) in heap.own_keys(object)? {
                let Some(current) = heap.own_named_flags(object, key)? else {
                    continue;
                };
                // 10.4.3.1 gives a String exotic object own names that are
                // neither writable nor configurable, so both levels hold of
                // them already and 10.1.6.3 would answer true for each.
                if Self::owns_string_exotic(object, key, heap)? {
                    continue;
                }
                let descriptor = PartialDescriptor {
                    configurable: Some(false),
                    // Step 3.b.ii.2: a frozen data property is not writable
                    // either; an accessor keeps the two halves it has.
                    writable: (frozen && !current.is_accessor).then_some(false),
                    ..PartialDescriptor::default()
                };
                Self::define_property_from(object, key, &descriptor, heap, realm)?;
            }
            return Ok(target);
        }
        if heap.is_extensible(object).unwrap_or(true) {
            return Ok(VALUE_FALSE);
        }
        let frozen = intrinsic == Intrinsic::ObjectIsFrozen;
        for (key, _) in heap.own_keys(object)? {
            let Some(flags) = heap.own_named_flags(object, key)? else {
                continue;
            };
            if flags.configurable || (frozen && !flags.is_accessor && flags.writable) {
                return Ok(VALUE_FALSE);
            }
        }
        Ok(VALUE_TRUE)
    }

    /// The own String keys of an object, in the order 10.1.11 gives them:
    /// every one for 20.1.2.10, and the enumerable ones for 20.1.2.19.
    ///
    /// The names are interned, and interning moves no object, so the Array
    /// may be allocated after them.
    fn own_string_keys(
        intrinsic: Intrinsic,
        target: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        // 20.1.2.10 answers every own String key; 20.1.2.19, 20.1.2.24 and
        // 20.1.2.5 answer only the enumerable ones, as the key, the value, or
        // the two of them in an Array of their own.
        let every = intrinsic == Intrinsic::ObjectGetOwnPropertyNames;
        let object = Self::coerce_object(target, heap, realm)?;
        let names: Vec<StringRef> = heap
            .own_keys(object)?
            .into_iter()
            .filter(|(_, enumerable)| *enumerable || every)
            .filter_map(|(key, _)| key.as_string())
            .collect();
        let mut answers = Vec::with_capacity(names.len());
        for name in names {
            let key = Value::from_string(name);
            let answer = match intrinsic {
                Intrinsic::ObjectValues | Intrinsic::ObjectEntries => {
                    let held = heap
                        .lookup_named(object, PropertyKey::String(name))?
                        .map(Self::plain_value)
                        .transpose()?
                        .unwrap_or(VALUE_UNDEFINED);
                    if intrinsic == Intrinsic::ObjectValues {
                        held
                    } else {
                        Self::array_of(alloc::vec![key, held], heap, realm)?
                    }
                }
                _ => key,
            };
            answers.push(answer);
        }
        Self::array_of(answers, heap, realm)
    }

    /// `Object.create` of 20.1.2.2: an ordinary object under the Prototype
    /// given, with the properties of 20.1.2.3 when a second argument names
    /// any.
    fn create_object(
        prototype: Value,
        properties: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if !prototype.is_null() && prototype.as_object().is_none() {
            return Err(type_error(
                heap,
                realm,
                "Object.create called with a value that is neither an object nor null",
            ));
        }
        let shape = heap.shapes.root_shape();
        let created = heap.allocate_object(shape, prototype)?;
        if properties.is_undefined() {
            return Ok(Value::from_object(created));
        }
        Self::define_properties(created, properties, heap, realm)
    }

    /// `ObjectDefineProperties` of 20.1.2.3.2.
    ///
    /// Step 4 reads every descriptor before step 5 defines any, so a source
    /// whose second descriptor is malformed leaves the object untouched.
    fn define_properties(
        object: ObjectRef,
        properties: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let source = Self::coerce_object(properties, heap, realm)?;
        let mut descriptors = Vec::new();
        for (key, enumerable) in heap.own_keys(source)? {
            if !enumerable {
                continue;
            }
            let value = heap
                .lookup_named(source, key)?
                .map(Self::plain_value)
                .transpose()?
                .unwrap_or(VALUE_UNDEFINED);
            let Some(descriptor) = value.as_object() else {
                return Err(type_error(
                    heap,
                    realm,
                    "property descriptor must be an object",
                ));
            };
            descriptors.push((key, Self::to_property_descriptor(descriptor, heap, realm)?));
        }
        for (key, descriptor) in descriptors {
            if !Self::define_property_from(object, key, &descriptor, heap, realm)? {
                return Err(type_error(heap, realm, "property definition rejected"));
            }
        }
        Ok(Value::from_object(object))
    }

    /// The functions 20.1.2 gives `%Object%` that this Realm builds.
    ///
    /// Each of them answers or takes a Property Descriptor, which 6.2.6.4 and
    /// 6.2.6.5 turn into and out of an ordinary object. This engine has no
    /// accessor properties, so a descriptor that names a `get` or a `set` is
    /// a gap rather than a descriptor it would silently drop.
    #[expect(
        clippy::too_many_lines,
        reason = "one function keeps each function beside the clause it implements"
    )]
    fn call_object_intrinsic(
        intrinsic: Intrinsic,
        target: Value,
        key: Value,
        attributes: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        match intrinsic {
            // 20.5.1.1 and 20.5.6.1.1 make an error whose Prototype belongs to
            // the constructor that was called.
            Intrinsic::ErrorConstructor
            | Intrinsic::EvalErrorConstructor
            | Intrinsic::RangeErrorConstructor
            | Intrinsic::ReferenceErrorConstructor
            | Intrinsic::SyntaxErrorConstructor
            | Intrinsic::TypeErrorConstructor
            | Intrinsic::UriErrorConstructor => {
                Self::construct_error(intrinsic, target, heap, realm)
            }
            // 20.1.1.1: undefined and null make an ordinary object, and every
            // other value goes through ToObject.
            Intrinsic::ObjectConstructor => Self::construct_object(target, heap, realm),
            // 28.1 does what clause 20.1.2 does, and answers whether it
            // worked instead of throwing where it did not. Step 1 of each
            // refuses a target that is not an Object, which 20.1.2 coerces.
            Intrinsic::ReflectGetPrototypeOf
            | Intrinsic::ReflectIsExtensible
            | Intrinsic::ReflectPreventExtensions
            | Intrinsic::ReflectOwnKeys
            | Intrinsic::ReflectHas
            | Intrinsic::ReflectGet
            | Intrinsic::ReflectDeleteProperty
            | Intrinsic::ReflectGetOwnPropertyDescriptor
            | Intrinsic::ReflectDefineProperty => {
                let Some(object) = target.as_object() else {
                    return Err(type_error(
                        heap,
                        realm,
                        "Reflect called on a value that is not an object",
                    ));
                };
                Self::call_reflect_intrinsic(intrinsic, object, key, attributes, heap, realm)
            }
            // 20.1.2.14 is SameValue of 7.2.11, which the engine already has
            // for the strict comparison that differs from it only in how it
            // treats zero and NaN.
            Intrinsic::ObjectIs => Ok(Value::from_bool(same_value(target, key, heap)?)),
            // 20.1.2.13 is HasOwnProperty of 7.3.13 without reaching the
            // Prototype Chain, on a `this` that goes through ToObject.
            Intrinsic::ObjectHasOwn => {
                let object = Self::coerce_object(target, heap, realm)?;
                let name = property_key(key, heap)?;
                Ok(Value::from_bool(
                    heap.own_named_flags(object, name)?.is_some(),
                ))
            }
            // 20.1.2.22 answers the object it was given, and throws where
            // 10.1.2 refuses; 28.1.14 answers whether 10.1.2 took it.
            Intrinsic::ObjectSetPrototypeOf | Intrinsic::ReflectSetPrototypeOf => {
                let reflect = intrinsic == Intrinsic::ReflectSetPrototypeOf;
                if !key.is_null() && !key.is_object() {
                    return Err(type_error(
                        heap,
                        realm,
                        "a prototype must be an object or null",
                    ));
                }
                let Some(object) = target.as_object() else {
                    if reflect {
                        return Err(type_error(
                            heap,
                            realm,
                            "Reflect called on a value that is not an object",
                        ));
                    }
                    // 20.1.2.22 step 1 refuses undefined and null and answers
                    // every other primitive as it is.
                    if target.is_undefined() || target.is_null() {
                        return Err(type_error(
                            heap,
                            realm,
                            "cannot set the prototype of null or undefined",
                        ));
                    }
                    return Ok(target);
                };
                let took = Self::set_object_prototype(object, key, heap)?;
                if reflect {
                    return Ok(Value::from_bool(took));
                }
                if !took {
                    return Err(type_error(heap, realm, "the prototype cannot be set"));
                }
                Ok(target)
            }
            // 20.1.2.12 answers the [[Prototype]] of the object ToObject made.
            Intrinsic::ObjectGetPrototypeOf => {
                let object = Self::coerce_object(target, heap, realm)?;
                Ok(heap
                    .get_object(object)
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?
                    .prototype)
            }
            Intrinsic::ObjectCreate => Self::create_object(target, key, heap, realm),
            // 20.1.2.3: every own enumerable property of the source is a
            // Property Descriptor of 6.2.6.5, defined on the object.
            Intrinsic::ObjectDefineProperties => {
                let object = target.as_object().ok_or_else(|| {
                    type_error(
                        heap,
                        realm,
                        "Object.defineProperties called on a value that is not an object",
                    )
                })?;
                Self::define_properties(object, key, heap, realm)
            }
            Intrinsic::ObjectGetOwnPropertyNames
            | Intrinsic::ObjectKeys
            | Intrinsic::ObjectValues
            | Intrinsic::ObjectEntries => Self::own_string_keys(intrinsic, target, heap, realm),
            // 20.1.2.20 and 20.1.2.16 are [[PreventExtensions]] and
            // [[IsExtensible]] of 10.1.4 and 10.1.3, on a value that is not an
            // Object unchanged and true respectively (steps 1 of each).
            Intrinsic::ObjectPreventExtensions => {
                if let Some(object) = target.as_object() {
                    heap.prevent_extensions(object)?;
                }
                Ok(target)
            }
            Intrinsic::ObjectIsExtensible => Ok(Value::from_bool(
                target
                    .as_object()
                    .and_then(|object| heap.is_extensible(object))
                    .unwrap_or(false),
            )),
            // 20.1.2.22 and 20.1.2.6 set the integrity level of 7.3.14, and
            // 20.1.2.18 and 20.1.2.17 test it with 7.3.15.
            Intrinsic::ObjectSeal
            | Intrinsic::ObjectFreeze
            | Intrinsic::ObjectIsSealed
            | Intrinsic::ObjectIsFrozen => Self::integrity_level(intrinsic, target, heap, realm),
            // 20.1.2.8: the own property, as the object 6.2.6.4 makes of it.
            Intrinsic::ObjectGetOwnPropertyDescriptor => {
                let object = Self::coerce_object(target, heap, realm)?;
                let name = property_key(key, heap)?;
                let Some(flags) = heap.own_named_flags(object, name)? else {
                    // A name this Realm owes the object has no descriptor to
                    // answer, and undefined would say the object has none.
                    let units = name
                        .as_string()
                        .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
                        .unwrap_or_default();
                    Self::absent_own_property(target, &units, heap, realm)?;
                    return Ok(VALUE_UNDEFINED);
                };
                let indexed = Self::element_index_of(object, name, heap);
                let value = Self::own_property_value(object, name, indexed, heap)?;
                Self::from_property_descriptor(value, flags, heap, realm)
            }
            // 20.1.2.4: the descriptor 6.2.6.5 reads, defined on the object.
            _ => {
                let Some(object) = target.as_object() else {
                    return Err(type_error(
                        heap,
                        realm,
                        "Object.defineProperty called on a value that is not an object",
                    ));
                };
                let name = property_key(key, heap)?;
                let Some(source) = attributes.as_object() else {
                    return Err(type_error(
                        heap,
                        realm,
                        "property descriptor must be an object",
                    ));
                };
                let descriptor = Self::to_property_descriptor(source, heap, realm)?;
                if !Self::define_property_from(object, name, &descriptor, heap, realm)? {
                    return Err(type_error(heap, realm, "property definition rejected"));
                }
                Ok(target)
            }
        }
    }

    /// `FromPropertyDescriptor` of 6.2.6.4.
    ///
    /// `value` is the slot of the property, which holds the pair of 6.1.7.1
    /// when `flags` says the property is an accessor.
    fn from_property_descriptor(
        value: Value,
        flags: PropertyFlags,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let (first, second) = if flags.is_accessor {
            Self::accessor_parts(value, heap)?
        } else {
            (value, VALUE_UNDEFINED)
        };
        // Both travel in roots, because allocating the descriptor may scavenge
        // and an Object they hold would not survive that otherwise.
        heap.enter_scope();
        let held_first = heap.push_root(first)?;
        let held_second = heap.push_root(second)?;
        let descriptor = realm.ordinary_object(heap);
        let first = heap.root_value(held_first).unwrap_or(VALUE_UNDEFINED);
        let second = heap.root_value(held_second).unwrap_or(VALUE_UNDEFINED);
        heap.exit_scope();
        let descriptor = descriptor?;
        let named: [(&str, Value); 4] = if flags.is_accessor {
            [
                ("get", first),
                ("set", second),
                ("enumerable", Value::from_bool(flags.enumerable)),
                ("configurable", Value::from_bool(flags.configurable)),
            ]
        } else {
            [
                ("value", first),
                ("writable", Value::from_bool(flags.writable)),
                ("enumerable", Value::from_bool(flags.enumerable)),
                ("configurable", Value::from_bool(flags.configurable)),
            ]
        };
        for (name, entry) in named {
            let key = PropertyKey::String(heap.strings.intern(name)?);
            heap.define_own_named(descriptor, key, entry, PropertyFlags::ordinary_data())?;
        }
        Ok(Value::from_object(descriptor))
    }

    /// A wrapper object of 20.3.3, 21.1.3 or 22.1.4, holding one primitive.
    fn wrapper(
        kind: ObjectKind,
        prototype: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<Value, VMError> {
        let shape = heap.shapes.root_shape();
        let object = heap.allocate_object(shape, prototype)?;
        heap.set_object_kind(object, kind)?;
        Ok(Value::from_object(object))
    }

    /// `thisNumberValue` of 21.1.3 and `thisBooleanValue` of 20.3.3, as the
    /// method that asked for it answers.
    fn wrapped_value(
        &self,
        intrinsic: Intrinsic,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let number = matches!(
            intrinsic,
            Intrinsic::NumberPrototypeValueOf | Intrinsic::NumberPrototypeToString
        );
        let receiver = call.receiver;
        let held = if number {
            receiver.as_f64().or_else(|| {
                match receiver
                    .as_object()
                    .and_then(|object| heap.get_object(object))
                {
                    Some(object) => match object.kind {
                        ObjectKind::NumberWrapper(number) => Some(number),
                        _ => None,
                    },
                    None => None,
                }
            })
        } else {
            receiver
                .as_boolean()
                .or_else(|| {
                    match receiver
                        .as_object()
                        .and_then(|object| heap.get_object(object))
                    {
                        Some(object) => match object.kind {
                            ObjectKind::BooleanWrapper(boolean) => Some(boolean),
                            _ => None,
                        },
                        None => None,
                    }
                })
                .map(|boolean| f64::from(u8::from(boolean)))
        };
        let Some(held) = held else {
            return Err(type_error(
                heap,
                realm,
                "this value is not of the type the method belongs to",
            ));
        };
        match intrinsic {
            Intrinsic::NumberPrototypeValueOf => Ok(Value::from_f64(held)),
            Intrinsic::BooleanPrototypeValueOf => Ok(Value::from_bool(held != 0.0)),
            Intrinsic::BooleanPrototypeToString => {
                let text = if held == 0.0 { "false" } else { "true" };
                let units: Vec<u16> = text.encode_utf16().collect();
                self.allocate_string(heap, &units)
            }
            // 21.1.3.6 takes a radix between 2 and 36; 10 is 6.1.6.1.20 and
            // every other one is `Number::toString` with that radix.
            _ => {
                let radix = self.call_argument(call, 0, heap)?;
                if !radix.is_undefined() {
                    let radix = integer_argument(radix, heap, realm)?;
                    if !(2..=36).contains(&radix) {
                        return Err(raise(
                            heap,
                            realm,
                            super::realm::NativeErrorKind::RangeError,
                            "invalid number radix",
                        ));
                    }
                    if radix != 10 {
                        let radix = u32::try_from(radix).map_err(|_| VMError::TypeError)?;
                        let units: Vec<u16> = crate::number::format_with_radix(held, radix)
                            .encode_utf16()
                            .collect();
                        if units.len() > self.string_units_limit {
                            return Err(VMError::StringLimit);
                        }
                        return self.allocate_string(heap, &units);
                    }
                }
                let units: Vec<u16> = crate::number::decimal_string(held).encode_utf16().collect();
                self.allocate_string(heap, &units)
            }
        }
    }

    /// The next argument of this call that 7.1.1 still has to convert.
    ///
    /// 22.1.3 and 23.1.3 each begin with the `this` value, so a receiver those
    /// clauses refuse is refused here, before a conversion of an argument
    /// could be observed.
    fn next_coercion(
        &self,
        intrinsic: Intrinsic,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<(u16, PrimitiveHint)>, VMError> {
        // A clause of 23.1.3 that walks reads the `length` before it converts
        // the argument it starts from, so the walk converts it where 7.1.5
        // stands and this conversion does not run at all.
        if Self::iterates_with_callback(intrinsic) {
            return Ok(None);
        }
        // A conversion writes the primitive back into the argument register,
        // and a call that passes a List has none.
        if matches!(call.resume, Some(Resume::Spread { .. })) {
            return Ok(None);
        }
        for (index, hint) in intrinsic.coerced_arguments() {
            if *index >= call.arg_count
                || self
                    .call_argument(call, *index, heap)?
                    .as_object()
                    .is_none()
            {
                continue;
            }
            if (call.receiver.is_undefined() || call.receiver.is_null())
                && matches!(
                    intrinsic.holder(),
                    super::realm::IntrinsicHolder::StringPrototype
                        | super::realm::IntrinsicHolder::ArrayPrototype
                )
            {
                return Err(match intrinsic.holder() {
                    super::realm::IntrinsicHolder::StringPrototype => {
                        type_error(heap, realm, "String method called on null or undefined")
                    }
                    _ => type_error(heap, realm, "cannot box null or undefined"),
                });
            }
            // 20.1.2.4, 20.1.2.8, 20.1.2.13 and 28.1 read the target before
            // they convert the name, so a target the clause refuses is
            // refused by the clause and the conversion never runs.
            if *index == 1
                && Self::refuses_its_target(intrinsic, self.call_argument(call, 0, heap)?)
            {
                return Ok(None);
            }
            return Ok(Some((*index, *hint)));
        }
        Ok(None)
    }

    /// Whether step 1 of the clause throws for this target, which it reads
    /// before `ToPropertyKey` converts the name that follows.
    const fn refuses_its_target(intrinsic: Intrinsic, target: Value) -> bool {
        match intrinsic {
            Intrinsic::ObjectDefineProperty
            | Intrinsic::ReflectDefineProperty
            | Intrinsic::ReflectDeleteProperty
            | Intrinsic::ReflectGet
            | Intrinsic::ReflectGetOwnPropertyDescriptor
            | Intrinsic::ReflectHas => !target.is_object(),
            // These two send the target through 7.1.18, which takes every
            // value but null and undefined.
            Intrinsic::ObjectGetOwnPropertyDescriptor | Intrinsic::ObjectHasOwn => {
                target.is_undefined() || target.is_null()
            }
            _ => false,
        }
    }

    /// Runs a native operation once its arguments need no conversion.
    ///
    /// Three of them open a frame instead of answering, so the call site takes
    /// the unit they entered rather than the accumulator.
    #[expect(
        clippy::too_many_lines,
        reason = "one function names every native that leaves before it answers"
    )]
    fn dispatch_native(
        &mut self,
        intrinsic: Intrinsic,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        // 20.2.3.1 and 28.1.1 leave to call what they were given, with the
        // List 7.3.18 makes; 28.1.2 leaves with the object 10.1.13 makes for
        // the `newTarget` it was given.
        if intrinsic == Intrinsic::ReflectConstruct {
            return self.begin_reflect_construct(call, units, active_feedback, heap, realm);
        }
        // 27.2.3.1 calls the executor before it answers, which needs a frame.
        if intrinsic == Intrinsic::PromiseConstructor && call.construct.is_some() {
            return self.begin_promise(call, units, active_feedback, heap, realm);
        }
        if matches!(
            intrinsic,
            Intrinsic::FunctionPrototypeApply | Intrinsic::ReflectApply
        ) {
            return self.begin_spread_call(intrinsic, call, units, active_feedback, heap, realm);
        }
        // 6.2.6.5 reads six fields of the descriptor, and a getter among them
        // runs a method of the Script; 7.3.25 reads one descriptor per key,
        // and the key itself may be a getter too.
        if let Some((register, many)) = Self::descriptor_argument(intrinsic, &call)
            && let Some(source) = self.read_reg(register)?.as_object()
            && if many {
                Self::properties_read_a_getter(source, heap)?
            } else {
                Self::descriptor_reads_a_getter(source, heap)?
            }
        {
            return self.begin_descriptor_copy(
                intrinsic,
                register,
                many,
                call,
                units,
                active_feedback,
                heap,
                realm,
            );
        }
        // 22.1.3 sends the `this` value through ToString before it reads its
        // arguments, which is a method of the Script for an Object.
        if intrinsic.coerces_its_receiver() && call.receiver.is_object() {
            return self.begin_receiver_coercion(
                intrinsic,
                PrimitiveHint::String,
                call,
                units,
                active_feedback,
                heap,
                realm,
            );
        }
        // A method of 23.1.3 that asks the Script about each element leaves to
        // make the first call and comes back through the frame it opens.
        if Self::iterates_with_callback(intrinsic) {
            return self.begin_array_iteration(
                intrinsic,
                call,
                units,
                active_feedback,
                heap,
                realm,
            );
        }
        // 7.3.18 reads the `length` before the clause does anything else. A
        // getter there runs a method of the Script, and so does 7.1.20 of an
        // Object the read answered.
        if Self::reads_an_array_like_length(intrinsic) {
            let object = Self::coerce_object(call.receiver, heap, realm)?;
            let getter = Self::array_like_length_getter(heap, object)?;
            let raw = if getter.is_some() {
                VALUE_UNDEFINED
            } else {
                Self::array_like_length_value(heap, object, realm)?
            };
            if getter.is_some() || raw.is_object() {
                heap.enter_scope();
                let receiver = heap.push_root(Value::from_object(object))?;
                let held = heap.push_root(raw)?;
                let resume = Resume::Length {
                    intrinsic: intrinsic.id(),
                    receiver,
                    held,
                    step: 0,
                    arg_start: call.arg_start,
                    arg_count: call.arg_count,
                    construct: call.construct,
                };
                let mut call = call;
                call.resume = Some(resume);
                call.receiver = Value::from_object(object);
                call.arg_count = 0;
                call.arg_start = Reg(0);
                call.construct = None;
                if let Some(getter) = getter {
                    return self.enter_call_value(
                        getter,
                        units,
                        active_feedback,
                        heap,
                        realm,
                        call,
                    );
                }
                return self.convert_array_like_length(
                    resume,
                    call,
                    units,
                    active_feedback,
                    heap,
                    realm,
                );
            }
        }
        self.acc = self.call_intrinsic(intrinsic, call, units, heap, realm)?;
        Ok(None)
    }

    /// Whether the clause begins with `LengthOfArrayLike` of 7.3.18, which a
    /// getter of the Script can answer.
    const fn reads_an_array_like_length(intrinsic: Intrinsic) -> bool {
        matches!(
            intrinsic,
            Intrinsic::ArrayPrototypeAt
                | Intrinsic::ArrayPrototypeJoin
                | Intrinsic::ArrayPrototypePop
                | Intrinsic::ArrayPrototypePush
                | Intrinsic::ArrayPrototypeReverse
                | Intrinsic::ArrayPrototypeSlice
                | Intrinsic::ArrayPrototypeToString
                | Intrinsic::ArrayPrototypeIncludes
                | Intrinsic::ArrayPrototypeShift
                | Intrinsic::ArrayPrototypeUnshift
                | Intrinsic::ArrayPrototypeSplice
                | Intrinsic::ArrayPrototypeFill
                | Intrinsic::ArrayPrototypeCopyWithin
                | Intrinsic::ArrayPrototypeConcat
                | Intrinsic::ArrayPrototypeWith
                | Intrinsic::ArrayPrototypeFlat
                | Intrinsic::ArrayPrototypeSort
                | Intrinsic::ArrayPrototypeToSorted
                | Intrinsic::ArrayPrototypeToSpliced
                | Intrinsic::ArrayPrototypeToReversed
        )
    }

    /// Whether the clause edits the object it was called on, which decides
    /// which of the two tables answers it.
    const fn edits_an_array(intrinsic: Intrinsic) -> bool {
        matches!(
            intrinsic,
            Intrinsic::ArrayPrototypeShift
                | Intrinsic::ArrayPrototypeUnshift
                | Intrinsic::ArrayPrototypeSplice
                | Intrinsic::ArrayPrototypeFill
                | Intrinsic::ArrayPrototypeCopyWithin
                | Intrinsic::ArrayPrototypeConcat
                | Intrinsic::ArrayPrototypeWith
                | Intrinsic::ArrayPrototypeFlat
                | Intrinsic::ArrayPrototypeSort
                | Intrinsic::ArrayPrototypeToSorted
                | Intrinsic::ArrayPrototypeToSpliced
                | Intrinsic::ArrayPrototypeToReversed
        )
    }

    /// Takes what a method answered for the `length` and either runs the
    /// clause or asks the next method of 7.1.1.
    #[expect(
        clippy::too_many_arguments,
        reason = "a clause runs again where a call does, with what a call has"
    )]
    fn finish_array_like_length(
        &mut self,
        resume: Resume,
        return_pc: usize,
        caller_code_id: Option<u32>,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let Resume::Length {
            intrinsic,
            receiver,
            held,
            step,
            arg_start,
            arg_count,
            construct,
        } = resume
        else {
            return Ok(None);
        };
        let call = Call {
            receiver: heap.root_value(receiver).unwrap_or(VALUE_UNDEFINED),
            func: arg_start,
            arg_start,
            arg_count,
            slot: 0,
            resume: Some(resume),
            construct,
            return_pc,
            caller_code_id,
        };
        let answered = self.acc;
        // 7.1.1 asks the next method when the one that answered gave an
        // Object; the getter answered the value 7.1.20 converts.
        if answered.is_object() {
            // The getter answered the value 7.1.20 converts; a method of
            // 7.1.1.1 answered something it discards, and the next method is
            // asked of the same object.
            let held = if step == 0 {
                heap.push_root(answered)?
            } else {
                held
            };
            let resume = Resume::Length {
                intrinsic,
                receiver,
                held,
                step,
                arg_start,
                arg_count,
                construct,
            };
            return self.convert_array_like_length(
                resume,
                call,
                units,
                active_feedback,
                heap,
                realm,
            );
        }
        heap.exit_scope();
        let intrinsic = Intrinsic::from_id(intrinsic).ok_or(VMError::TypeError)?;
        let length = integer_argument(answered, heap, realm)?.max(0);
        let call = Call {
            resume: None,
            ..call
        };
        self.acc = if Self::edits_an_array(intrinsic) {
            self.call_array_edit_intrinsic(intrinsic, call, Some(length), heap, realm)?
        } else {
            self.call_array_intrinsic(intrinsic, call, Some(length), units, heap, realm)?
        };
        if let Some(target) = construct {
            self.write_reg(target, self.acc)?;
        }
        Ok(None)
    }

    /// Asks the next method of 7.1.1 for the `length` the clause read, with
    /// the hint `number`.
    ///
    /// The object waits in a root, which the collector traces. A method of the
    /// Realm answers without a frame, so the loop takes that answer and goes
    /// on rather than leaving.
    fn convert_array_like_length(
        &mut self,
        resume: Resume,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let Resume::Length {
            intrinsic,
            receiver,
            mut held,
            mut step,
            arg_start,
            arg_count,
            construct,
        } = resume
        else {
            return Ok(None);
        };
        loop {
            let object = heap
                .root_value(held)
                .and_then(Value::as_object)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            // 7.1.1 step 1 would pass a hint to `@@toPrimitive`, which needs a
            // register of the caller this clause has none of.
            if step == 0
                && heap
                    .lookup_named(object, super::realm::WellKnownSymbol::ToPrimitive.key())?
                    .is_some()
            {
                heap.exit_scope();
                return Err(VMError::Unsupported(
                    "the @@toPrimitive of an array-like length",
                ));
            }
            step = step.saturating_add(1);
            let name = match step {
                1 => "valueOf",
                2 => "toString",
                _ => {
                    heap.exit_scope();
                    return Err(type_error(heap, realm, "an object has no primitive value"));
                }
            };
            let key = PropertyKey::String(heap.strings.intern(name)?);
            let object = heap
                .root_value(held)
                .and_then(Value::as_object)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let method = heap
                .lookup_named(object, key)?
                .map(Self::plain_value)
                .transpose()?
                .filter(|method| Self::is_callable(*method, heap));
            let Some(method) = method else {
                continue;
            };
            let mut next = call;
            next.receiver = Value::from_object(object);
            next.arg_count = 0;
            next.arg_start = Reg(0);
            next.construct = None;
            next.resume = Some(Resume::Length {
                intrinsic,
                receiver,
                held,
                step,
                arg_start,
                arg_count,
                construct,
            });
            if let Some(code_id) =
                self.enter_call_value(method, units, active_feedback, heap, realm, next)?
            {
                return Ok(Some(code_id));
            }
            // A method of the Realm answered without a frame of its own.
            let answered = self.acc;
            if !answered.is_object() {
                // The receiver is read while its root still stands.
                let call = Call {
                    receiver: heap.root_value(receiver).unwrap_or(VALUE_UNDEFINED),
                    resume: None,
                    arg_start,
                    arg_count,
                    construct,
                    ..call
                };
                heap.exit_scope();
                let intrinsic = Intrinsic::from_id(intrinsic).ok_or(VMError::TypeError)?;
                let length = integer_argument(answered, heap, realm)?.max(0);
                self.acc = if Self::edits_an_array(intrinsic) {
                    self.call_array_edit_intrinsic(intrinsic, call, Some(length), heap, realm)?
                } else {
                    self.call_array_intrinsic(intrinsic, call, Some(length), units, heap, realm)?
                };
                if let Some(target) = construct {
                    self.write_reg(target, self.acc)?;
                }
                return Ok(None);
            }
            held = heap.push_root(answered)?;
        }
    }

    /// The argument a clause reads a descriptor out of, and whether it reads
    /// one per key (7.3.25) rather than one (6.2.6.5).
    fn descriptor_argument(intrinsic: Intrinsic, call: &Call) -> Option<(Reg, bool)> {
        let (index, many) = match intrinsic {
            Intrinsic::ObjectDefineProperty | Intrinsic::ReflectDefineProperty => (2, false),
            Intrinsic::ObjectCreate | Intrinsic::ObjectDefineProperties => (1, true),
            _ => return None,
        };
        (call.arg_count > index).then(|| (Reg(call.arg_start.0.saturating_add(index)), many))
    }

    /// The six fields 6.2.6.5 reads, in the order it reads them.
    const DESCRIPTOR_FIELDS: [&'static str; 6] = [
        "enumerable",
        "configurable",
        "value",
        "writable",
        "get",
        "set",
    ];

    /// The walk is between the keys 7.3.25 lists.
    const DESCRIPTOR_KEY: u8 = u8::MAX;

    /// The walk is waiting for the value 7.3.25 step 4.a reads off the key.
    const DESCRIPTOR_VALUE: u8 = u8::MAX - 1;

    /// Whether any field 6.2.6.5 reads is an accessor of the chain.
    fn descriptor_reads_a_getter(
        source: ObjectRef,
        heap: &mut GenerationalHeap,
    ) -> Result<bool, VMError> {
        for name in Self::DESCRIPTOR_FIELDS {
            let key = PropertyKey::String(heap.strings.intern(name)?);
            if heap
                .lookup_named(source, key)?
                .is_some_and(|found| found.flags.is_accessor)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether 7.3.25 would read a getter of the Script off the properties
    /// object or off one of the descriptors it holds.
    fn properties_read_a_getter(
        source: ObjectRef,
        heap: &mut GenerationalHeap,
    ) -> Result<bool, VMError> {
        for (key, enumerable) in heap.own_keys(source)? {
            if !enumerable {
                continue;
            }
            let Some(found) = heap.lookup_named(source, key)? else {
                continue;
            };
            if found.flags.is_accessor {
                return Ok(true);
            }
            if let Some(descriptor) = found.value.as_object()
                && Self::descriptor_reads_a_getter(descriptor, heap)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Leaves a native operation to read a descriptor whose fields 6.2.6.5
    /// has to call getters for.
    ///
    /// Each field waits in an object of its own, and the clause runs again on
    /// that object, so every getter of the descriptor runs exactly once and in
    /// the order 6.2.6.5 reads them. 7.3.25 reads one descriptor per key, so
    /// the copy holds one copy per key there.
    #[expect(
        clippy::too_many_arguments,
        reason = "a descriptor read opens a frame, which needs what a call needs"
    )]
    fn begin_descriptor_copy(
        &mut self,
        intrinsic: Intrinsic,
        register: Reg,
        many: bool,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        // A copy has no Prototype, so a field it does not carry is absent
        // there as 6.2.6.5 found it absent here.
        let shape = heap.shapes.root_shape();
        let copy = heap.allocate_object(shape, VALUE_NULL)?;
        heap.enter_scope();
        let receiver = heap.push_root(call.receiver)?;
        let copy = heap.push_root(Value::from_object(copy))?;
        let source = self
            .read_reg(register)?
            .as_object()
            .ok_or(VMError::TypeError)?;
        let (keys, pending, target, field) = if many {
            // 7.3.25 step 2 lists the keys before it reads any of them.
            let mut names = Vec::new();
            for (key, enumerable) in heap.own_keys(source)? {
                if enumerable {
                    names.push(Self::key_value(key));
                }
            }
            let list = Self::array_of(names, heap, realm)?;
            let keys = heap.push_root(list)?;
            let pending = heap.push_root(VALUE_UNDEFINED)?;
            let target = heap.push_root(VALUE_UNDEFINED)?;
            (keys, pending, target, Self::DESCRIPTOR_KEY)
        } else {
            let keys = heap.push_root(VALUE_UNDEFINED)?;
            let pending = heap.push_root(Value::from_object(source))?;
            let target = heap.push_root(heap.root_value(copy).unwrap_or(VALUE_UNDEFINED))?;
            (keys, pending, target, 0)
        };
        let resume = Resume::Descriptor {
            intrinsic: intrinsic.id(),
            receiver,
            copy,
            keys,
            pending,
            target,
            key_index: 0,
            field,
            register,
            arg_start: call.arg_start,
            arg_count: call.arg_count,
            construct: call.construct,
        };
        self.continue_descriptor(
            resume,
            call.return_pc,
            call.caller_code_id,
            units,
            active_feedback,
            heap,
            realm,
        )
    }

    /// The value a property key carries.
    const fn key_value(key: PropertyKey) -> Value {
        match key {
            PropertyKey::String(name) => Value::from_string(name),
            PropertyKey::Symbol(symbol) => Value::from_symbol(symbol),
        }
    }

    /// Reads what 6.2.6.5 and 7.3.25 have still to read, and runs the clause
    /// on the copy once nothing is left.
    ///
    /// # Errors
    ///
    /// Returns the errors the getters and the clause raise.
    #[expect(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "one function carries the whole walk of 7.3.25 and 6.2.6.5"
    )]
    fn continue_descriptor(
        &mut self,
        resume: Resume,
        return_pc: usize,
        caller_code_id: Option<u32>,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let Resume::Descriptor {
            intrinsic,
            receiver,
            copy,
            keys,
            pending,
            target,
            mut key_index,
            mut field,
            register,
            arg_start,
            arg_count,
            construct,
        } = resume
        else {
            return Ok(None);
        };
        let many = !heap
            .root_value(keys)
            .unwrap_or(VALUE_UNDEFINED)
            .is_undefined();
        // The getter that just answered belongs to the step before this one.
        let mut answered = (field != Self::DESCRIPTOR_KEY && field > 0).then_some(self.acc);
        loop {
            if field == Self::DESCRIPTOR_VALUE {
                // 7.3.25 step 4.a answered the descriptor of the key.
                let value = answered.take().unwrap_or(VALUE_UNDEFINED);
                let Some(descriptor) = value.as_object() else {
                    heap.exit_scope();
                    return Err(type_error(
                        heap,
                        realm,
                        "property descriptor must be an object",
                    ));
                };
                heap.set_root(pending, Value::from_object(descriptor))
                    .map_err(VMError::Heap)?;
                let shape = heap.shapes.root_shape();
                let fresh = heap.allocate_object(shape, VALUE_NULL)?;
                heap.set_root(target, Value::from_object(fresh))
                    .map_err(VMError::Heap)?;
                field = 0;
                continue;
            }
            if field == Self::DESCRIPTOR_KEY {
                // 7.3.25 step 4 reads the descriptor of the next key.
                let Some(key) = Self::list_element(keys, key_index, heap)? else {
                    break;
                };
                key_index = key_index.saturating_add(1);
                let source = self
                    .read_reg(register)?
                    .as_object()
                    .ok_or(VMError::TypeError)?;
                let name = property_key(key, heap)?;
                let Some(found) = heap.lookup_named(source, name)? else {
                    continue;
                };
                if !found.flags.is_accessor {
                    answered = Some(found.value);
                    field = Self::DESCRIPTOR_VALUE;
                    continue;
                }
                let (get, _) = Self::accessor_parts(found.value, heap)?;
                if get.is_undefined() {
                    answered = Some(VALUE_UNDEFINED);
                    field = Self::DESCRIPTOR_VALUE;
                    continue;
                }
                let next = Self::descriptor_call(
                    Value::from_object(source),
                    Resume::Descriptor {
                        intrinsic,
                        receiver,
                        copy,
                        keys,
                        pending,
                        target,
                        key_index,
                        field: Self::DESCRIPTOR_VALUE,
                        register,
                        arg_start,
                        arg_count,
                        construct,
                    },
                    register,
                    return_pc,
                    caller_code_id,
                );
                if let Some(code_id) =
                    self.enter_call_value(get, units, active_feedback, heap, realm, next)?
                {
                    return Ok(Some(code_id));
                }
                answered = Some(self.acc);
                field = Self::DESCRIPTOR_VALUE;
                continue;
            }
            if let Some(value) = answered.take() {
                let name = Self::DESCRIPTOR_FIELDS
                    .get(usize::from(field).saturating_sub(1))
                    .ok_or(VMError::InvalidFeedbackVector)?;
                Self::define_descriptor_field(target, name, value, heap)?;
            }
            let Some(name) = Self::DESCRIPTOR_FIELDS.get(usize::from(field)) else {
                // Every field of this descriptor is in its copy.
                if !many {
                    break;
                }
                let key = Self::list_element(keys, key_index.saturating_sub(1), heap)?
                    .ok_or(VMError::InvalidFeedbackVector)?;
                let name = property_key(key, heap)?;
                let held = heap.root_value(target).unwrap_or(VALUE_UNDEFINED);
                let copy = heap
                    .root_value(copy)
                    .and_then(Value::as_object)
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                heap.define_own_named(copy, name, held, PropertyFlags::ordinary_data())?;
                field = Self::DESCRIPTOR_KEY;
                continue;
            };
            field = field.saturating_add(1);
            let source = heap
                .root_value(pending)
                .and_then(Value::as_object)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let key = PropertyKey::String(heap.strings.intern(name)?);
            let Some(found) = heap.lookup_named(source, key)? else {
                continue;
            };
            if !found.flags.is_accessor {
                Self::define_descriptor_field(target, name, found.value, heap)?;
                continue;
            }
            let (get, _) = Self::accessor_parts(found.value, heap)?;
            if get.is_undefined() {
                Self::define_descriptor_field(target, name, VALUE_UNDEFINED, heap)?;
                continue;
            }
            let next = Self::descriptor_call(
                Value::from_object(source),
                Resume::Descriptor {
                    intrinsic,
                    receiver,
                    copy,
                    keys,
                    pending,
                    target,
                    key_index,
                    field,
                    register,
                    arg_start,
                    arg_count,
                    construct,
                },
                register,
                return_pc,
                caller_code_id,
            );
            if let Some(code_id) =
                self.enter_call_value(get, units, active_feedback, heap, realm, next)?
            {
                return Ok(Some(code_id));
            }
            // A getter of the Realm answered without a frame of its own.
            answered = Some(self.acc);
        }
        // Everything the clause reads is in the copy, so it reads that.
        let call = Call {
            receiver: heap.root_value(receiver).unwrap_or(VALUE_UNDEFINED),
            func: arg_start,
            arg_start,
            arg_count,
            slot: 0,
            resume: None,
            construct,
            return_pc,
            caller_code_id,
        };
        let copy = heap.root_value(copy).unwrap_or(VALUE_UNDEFINED);
        heap.exit_scope();
        self.write_reg(register, copy)?;
        let intrinsic = Intrinsic::from_id(intrinsic).ok_or(VMError::TypeError)?;
        self.dispatch_native(intrinsic, call, units, active_feedback, heap, realm)
    }

    /// The call a getter of the walk is entered with.
    const fn descriptor_call(
        receiver: Value,
        resume: Resume,
        register: Reg,
        return_pc: usize,
        caller_code_id: Option<u32>,
    ) -> Call {
        Call {
            receiver,
            func: register,
            arg_start: register,
            arg_count: 0,
            slot: 0,
            resume: Some(resume),
            construct: None,
            return_pc,
            caller_code_id,
        }
    }

    /// One element of the key list 7.3.25 step 2 made.
    fn list_element(
        keys: Root,
        index: u32,
        heap: &GenerationalHeap,
    ) -> Result<Option<Value>, VMError> {
        let Some(list) = heap.root_value(keys).and_then(Value::as_object) else {
            return Ok(None);
        };
        let store = heap
            .get_object(list)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .elements
            .and_then(|elements| heap.get_elements(elements));
        Ok(store.and_then(|store| store.get(index)))
    }

    /// Puts one field 6.2.6.5 read into the copy the walk builds.
    fn define_descriptor_field(
        target: Root,
        name: &str,
        value: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let key = PropertyKey::String(heap.strings.intern(name)?);
        let target = heap
            .root_value(target)
            .and_then(Value::as_object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        heap.define_own_named(target, key, value, PropertyFlags::ordinary_data())?;
        Ok(())
    }

    /// Leaves a native operation to convert one of its arguments (7.1.17).
    ///
    /// The receiver travels in a root, because the argument registers belong
    /// to the caller and the collector sees them, but the `this` value of a
    /// native call is a value of its own.
    #[expect(
        clippy::too_many_arguments,
        reason = "a conversion runs where a call does, with what a call has"
    )]
    fn begin_coercion(
        &mut self,
        intrinsic: Intrinsic,
        index: u16,
        hint: PrimitiveHint,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let register = Reg(call.arg_start.0.saturating_add(index));
        let argument = self.read_reg(register)?;
        heap.enter_scope();
        let receiver = heap.push_root(call.receiver)?;
        let resume = Resume::Coercion {
            intrinsic: intrinsic.id(),
            receiver,
            register,
            arg_start: call.arg_start,
            arg_count: call.arg_count,
            construct: call.construct,
            step: PrimitiveStep::Exotic,
            hint,
            of_receiver: false,
        };
        let conversion = Call {
            receiver: argument,
            resume: Some(resume),
            construct: None,
            ..call
        };
        match self.convert_to_primitive(conversion, units, active_feedback, heap, realm)? {
            Conversion::Done(value) => {
                self.write_reg(register, value)?;
                self.finish_coerced(
                    resume,
                    call.return_pc,
                    call.caller_code_id,
                    units,
                    active_feedback,
                    heap,
                    realm,
                )
            }
            Conversion::Suspended(code_id) => Ok(Some(code_id)),
        }
    }

    /// Leaves a native operation to convert its `this` value (22.1.3).
    ///
    /// The receiver has no register of the caller to answer into, so the
    /// primitive replaces the value the root holds and the clause runs again
    /// with it.
    #[expect(
        clippy::too_many_arguments,
        reason = "a conversion runs where a call does, with what a call has"
    )]
    fn begin_receiver_coercion(
        &mut self,
        intrinsic: Intrinsic,
        hint: PrimitiveHint,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        heap.enter_scope();
        let receiver = heap.push_root(call.receiver)?;
        let resume = Resume::Coercion {
            intrinsic: intrinsic.id(),
            receiver,
            register: call.arg_start,
            arg_start: call.arg_start,
            arg_count: call.arg_count,
            construct: call.construct,
            step: PrimitiveStep::Exotic,
            hint,
            of_receiver: true,
        };
        let conversion = Call {
            receiver: call.receiver,
            resume: Some(resume),
            construct: None,
            ..call
        };
        match self.convert_to_primitive(conversion, units, active_feedback, heap, realm)? {
            Conversion::Done(value) => {
                heap.set_root(receiver, value).map_err(VMError::Heap)?;
                self.finish_coerced(
                    resume,
                    call.return_pc,
                    call.caller_code_id,
                    units,
                    active_feedback,
                    heap,
                    realm,
                )
            }
            Conversion::Suspended(code_id) => Ok(Some(code_id)),
        }
    }

    /// `Function.prototype.apply` of 20.2.3.1 and `Reflect.apply` of 28.1.1.
    ///
    /// `CreateListFromArrayLike` of 7.3.18 collects the arguments into an
    /// Array the call carries, because a frame of the caller holds none of
    /// them. A callee written in Rust reads its arguments out of registers of
    /// the caller, so one of those is a gap here.
    fn begin_spread_call(
        &mut self,
        intrinsic: Intrinsic,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let reflect = intrinsic == Intrinsic::ReflectApply;
        let (target, receiver, list) = if reflect {
            (
                self.call_argument(&call, 0, heap)?,
                self.call_argument(&call, 1, heap)?,
                self.call_argument(&call, 2, heap)?,
            )
        } else {
            (
                call.receiver,
                self.call_argument(&call, 0, heap)?,
                self.call_argument(&call, 1, heap)?,
            )
        };
        if !Self::is_script_function(target, heap) {
            if !Self::is_callable(target, heap) {
                return Err(type_error(heap, realm, "value is not callable"));
            }
            return Err(VMError::Unsupported("apply of a function written in Rust"));
        }
        // 7.3.18 step 2: undefined and null are an empty List for 20.2.3.1,
        // and a `TypeError` for 28.1.1, which asks for an Object.
        let arguments = if list.is_undefined() && !reflect {
            Vec::new()
        } else {
            let Some(source) = list.as_object() else {
                return Err(type_error(
                    heap,
                    realm,
                    "the arguments of apply are not an object",
                ));
            };
            let length = Self::array_like_length(heap, source, realm)?;
            let length = u16::try_from(length).map_err(|_| {
                VMError::Unsupported("a call of more arguments than a frame passes")
            })?;
            let mut collected = Vec::with_capacity(usize::from(length));
            for index in 0..u32::from(length) {
                self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                collected.push(Self::element_at(heap, source, index)?.unwrap_or(VALUE_UNDEFINED));
            }
            collected
        };
        let arg_count = u16::try_from(arguments.len()).map_err(|_| VMError::TypeError)?;
        let list = Self::array_of(arguments, heap, realm)?;
        // The List outlives every frame the call opens, so it is a root of a
        // scope of its own, which the return leaves.
        heap.enter_scope();
        let arguments = heap.push_root(list)?;
        let call = Call {
            receiver,
            arg_count,
            resume: Some(Resume::Spread { arguments }),
            construct: None,
            ..call
        };
        self.enter_call_value(target, units, active_feedback, heap, realm, call)
    }

    /// `[[Call]]` of 10.4.1.1 for a bound function that kept arguments.
    ///
    /// The List the target is called with is every bound argument of the
    /// chain, outermost bind last, followed by the arguments of the call
    /// site. No frame of the caller holds the two together, so the call
    /// carries the Array 7.3.18 would make.
    fn begin_bound_call(
        &mut self,
        function: Value,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        if call.construct.is_some() {
            return Err(VMError::Unsupported(
                "new of a bound function that kept arguments",
            ));
        }
        let mut arguments = Vec::new();
        for index in 0..call.arg_count {
            arguments.push(self.call_argument(&call, index, heap)?);
        }
        let mut current = function;
        let mut receiver = call.receiver;
        while let Some(object) = current.as_object() {
            let Some(ObjectKind::BoundFunction {
                target,
                receiver: bound,
                arguments: held,
            }) = heap.get_object(object).map(|entry| entry.kind.clone())
            else {
                break;
            };
            let mut prefix = Vec::new();
            if let Some(list) = held.as_object() {
                let length = Self::array_like_length(heap, list, realm)?;
                for index in Self::scan_range(0, length) {
                    prefix.push(Self::element_at(heap, list, index)?.unwrap_or(VALUE_UNDEFINED));
                }
            }
            prefix.append(&mut arguments);
            arguments = prefix;
            receiver = bound;
            current = target;
        }
        let arg_count = u16::try_from(arguments.len())
            .map_err(|_| VMError::Unsupported("a call of more arguments than a frame passes"))?;
        let list = Self::array_of(arguments, heap, realm)?;
        // The List outlives every frame the call opens, so it is a root of a
        // scope of its own, which the return leaves.
        heap.enter_scope();
        let arguments = heap.push_root(list)?;
        let call = Call {
            receiver,
            arg_count,
            resume: Some(Resume::Spread { arguments }),
            construct: None,
            ..call
        };
        self.enter_call_value(current, units, active_feedback, heap, realm, call)
    }

    /// `Reflect.construct` of 28.1.2.
    ///
    /// The arguments are a List 7.3.18 makes, like 28.1.1, and the object the
    /// frame starts from is the one 10.1.13 makes for `newTarget`.
    fn begin_reflect_construct(
        &mut self,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let target = self.call_argument(&call, 0, heap)?;
        let list = self.call_argument(&call, 1, heap)?;
        let new_target = if call.arg_count > 2 {
            self.call_argument(&call, 2, heap)?
        } else {
            target
        };
        // Steps 1 and 3 ask both for `[[Construct]]`.
        if !Self::constructs(target, heap) || !Self::constructs(new_target, heap) {
            return Err(type_error(heap, realm, "value is not a constructor"));
        }
        if !Self::is_script_function(target, heap) {
            return Err(VMError::Unsupported(
                "construct of a constructor written in Rust",
            ));
        }
        // Step 4: 7.3.18 asks for an Object and makes a List of its indices.
        let Some(source) = list.as_object() else {
            return Err(type_error(
                heap,
                realm,
                "the arguments of construct are not an object",
            ));
        };
        let length = Self::array_like_length(heap, source, realm)?;
        let length = u16::try_from(length)
            .map_err(|_| VMError::Unsupported("a call of more arguments than a frame passes"))?;
        let mut collected = Vec::with_capacity(usize::from(length));
        for index in 0..u32::from(length) {
            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
            collected.push(Self::element_at(heap, source, index)?.unwrap_or(VALUE_UNDEFINED));
        }
        let arg_count = u16::try_from(collected.len()).map_err(|_| VMError::TypeError)?;
        let arguments = Self::array_of(collected, heap, realm)?;
        // The List outlives every frame the call opens, so it is a root of a
        // scope of its own, which the return leaves.
        heap.enter_scope();
        let arguments = heap.push_root(arguments)?;
        // The `newTarget` and the callee are read out of registers, which the
        // collector forwards across the allocation between the two reads.
        let new_target = if call.arg_count > 2 {
            Reg(call.arg_start.0.saturating_add(2))
        } else {
            call.arg_start
        };
        let object = self.create_from_new_target(heap, realm, new_target)?;
        // The new object goes where the List came in: the caller has no use
        // for that register any more, and the collector sees it there.
        let held = Reg(call.arg_start.0.saturating_add(1));
        self.write_reg(held, Value::from_object(object))?;
        let target = self.read_reg(call.arg_start)?;
        // Step 5 gives the call the `newTarget` it was passed, and nothing
        // allocates between here and the frame.
        self.pending_new_target = self.read_reg(new_target)?;
        let call = Call {
            receiver: Value::from_object(object),
            arg_count,
            resume: Some(Resume::Spread { arguments }),
            construct: Some(held),
            ..call
        };
        self.enter_call_value(target, units, active_feedback, heap, realm, call)
    }

    /// `OrdinaryCreateFromConstructor` of 10.1.13 for a `newTarget` that need
    /// not be the function the call enters.
    fn create_from_new_target(
        &self,
        heap: &mut GenerationalHeap,
        realm: &Realm,
        new_target: Reg,
    ) -> Result<ObjectRef, VMError> {
        let shape = heap.shapes.root_shape();
        let prototype = realm.object_prototype(heap)?;
        let object = heap.allocate_object(shape, prototype)?;
        // The register is a root the collector forwards, so it names the same
        // function on the other side of the allocation above.
        let function = self
            .read_reg(new_target)?
            .as_object()
            .ok_or(VMError::TypeError)?;
        let name = PropertyKey::String(heap.strings.intern("prototype")?);
        let Some(property) = heap.lookup_named(function, name)? else {
            return Err(type_error(heap, realm, "value is not a constructor"));
        };
        let prototype = Self::plain_value(property)?;
        if prototype.as_object().is_some() {
            heap.set_object_prototype(object, prototype)?;
        }
        Ok(object)
    }

    /// Whether the value has a `[[Construct]]`, which 7.2.4 asks.
    ///
    /// A function a Script wrote carries one where 10.2.5 gave it a
    /// `prototype`; an arrow and a method have none. A native has one where
    /// the Realm builds the object it would make.
    fn constructs(value: Value, heap: &GenerationalHeap) -> bool {
        let Some(object) = value.as_object() else {
            return false;
        };
        let Some(entry) = heap.get_object(object) else {
            return false;
        };
        match entry.kind {
            ObjectKind::Function { .. } => heap
                .strings
                .lookup_interned_units(&PROTOTYPE_NAME)
                .and_then(|name| {
                    heap.own_named_flags(object, PropertyKey::String(name))
                        .ok()
                        .flatten()
                })
                .is_some(),
            ObjectKind::NativeFunction { id, .. } => matches!(
                Intrinsic::from_id(id),
                Some(
                    Intrinsic::ArrayConstructor
                        | Intrinsic::ObjectConstructor
                        | Intrinsic::ErrorConstructor
                        | Intrinsic::EvalErrorConstructor
                        | Intrinsic::RangeErrorConstructor
                        | Intrinsic::ReferenceErrorConstructor
                        | Intrinsic::SyntaxErrorConstructor
                        | Intrinsic::TypeErrorConstructor
                        | Intrinsic::UriErrorConstructor
                        | Intrinsic::StringConstructor
                        | Intrinsic::NumberConstructor
                        | Intrinsic::BooleanConstructor
                        | Intrinsic::RegExpConstructor
                        | Intrinsic::PromiseConstructor
                        | Intrinsic::FunctionConstructor
                )
            ),
            _ => false,
        }
    }

    /// Whether the value is a function a Script wrote, which answers through a
    /// frame and so can carry a [`Resume`].
    /// Whether a value is the function object of this intrinsic.
    fn is_intrinsic(value: Value, intrinsic: Intrinsic, heap: &GenerationalHeap) -> bool {
        value
            .as_object()
            .and_then(|reference| heap.get_object(reference))
            .is_some_and(|object| {
                matches!(object.kind, ObjectKind::NativeFunction { id, .. } if id == intrinsic.id())
            })
    }

    fn is_script_function(value: Value, heap: &GenerationalHeap) -> bool {
        value
            .as_object()
            .and_then(|reference| heap.get_object(reference))
            .is_some_and(|object| matches!(object.kind, ObjectKind::Function { .. }))
    }

    /// Calls the getter or the setter of an accessor property, as 10.1.8.1
    /// step 3 and 10.1.9.2 step 5 do.
    ///
    /// The `this` value of the call is the object the read or the write named,
    /// not the object the property was found on. `assigned` tells the two
    /// apart: a read passes none.
    #[expect(
        clippy::too_many_arguments,
        reason = "an accessor call runs where a call does, with what a call has"
    )]
    fn enter_accessor(
        &mut self,
        pair: Value,
        receiver: Value,
        assigned: Option<Value>,
        return_pc: usize,
        caller_code_id: Option<u32>,
        code: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let (get, set) = Self::accessor_parts(pair, heap)?;
        let call = Call {
            receiver,
            func: Reg(0),
            arg_start: Reg(0),
            arg_count: 0,
            slot: 0,
            resume: Some(Resume::Getter),
            construct: None,
            return_pc,
            caller_code_id,
        };
        let Some(assigned) = assigned else {
            // 10.1.8.1 step 3.b: a property with no getter reads undefined.
            if get.is_undefined() {
                self.acc = VALUE_UNDEFINED;
                return Ok(None);
            }
            return self.enter_call_value(get, code, active_feedback, heap, realm, call);
        };
        // 10.1.9.2 step 5.b: a property with no setter takes no value.
        if set.is_undefined() {
            self.acc = assigned;
            return Ok(None);
        }
        // A native takes its arguments from registers of the caller, and a
        // setter reaches its frame from a root instead, so one written in Rust
        // would be called with whatever those registers hold.
        if !Self::is_script_function(set, heap) {
            // 10.2.4.1 throws for every call, whatever it is called with, so
            // the restricted properties of 20.2.3 read no argument and the
            // call needs none.
            if Self::is_intrinsic(set, Intrinsic::ThrowTypeError, heap) {
                return self.enter_call_value(set, code, active_feedback, heap, realm, call);
            }
            return Err(VMError::Unsupported(
                "a setter that is not a Script function",
            ));
        }
        // The value outlives the frame the setter opens, so it is a root of a
        // scope of its own, which the return leaves.
        heap.enter_scope();
        let held = heap.push_root(assigned)?;
        let call = Call {
            arg_count: 1,
            resume: Some(Resume::Setter { value: held }),
            ..call
        };
        self.enter_call_value(set, code, active_feedback, heap, realm, call)
    }

    /// The value of a property a native operation found.
    ///
    /// Reading an accessor property is a call of its getter, and a native
    /// operation has no frame to make one from, so it names the gap rather
    /// than reading the pair as though it were the value.
    const fn plain_value(found: NamedProperty) -> Result<Value, VMError> {
        if found.flags.is_accessor {
            return Err(VMError::Unsupported("a property that is an accessor"));
        }
        Ok(found.value)
    }

    /// The `[[Get]]` and `[[Set]]` an accessor property's slot holds.
    fn accessor_parts(pair: Value, heap: &GenerationalHeap) -> Result<(Value, Value), VMError> {
        let reference = pair
            .as_object()
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        let ObjectKind::Accessor { get, set } = heap
            .get_object(reference)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .kind
        else {
            return Err(VMError::Heap(HeapError::InvalidReference));
        };
        Ok((get, set))
    }

    /// The pair of 6.1.7.1 an accessor property's slot holds.
    ///
    /// It is an object of the heap so that the collector traces both halves,
    /// and it has no Prototype because no Script can reach it.
    fn make_accessor(
        get: Value,
        set: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<Value, VMError> {
        heap.enter_scope();
        let held_get = heap.push_root(get)?;
        let held_set = heap.push_root(set)?;
        let shape = heap.shapes.root_shape();
        let pair = heap.allocate_object(shape, VALUE_NULL);
        let get = heap.root_value(held_get).unwrap_or(VALUE_UNDEFINED);
        let set = heap.root_value(held_set).unwrap_or(VALUE_UNDEFINED);
        heap.exit_scope();
        let pair = pair?;
        heap.set_object_kind(pair, ObjectKind::Accessor { get, set })?;
        Ok(Value::from_object(pair))
    }

    /// `ToPropertyDescriptor` of 6.2.6.5.
    ///
    /// A field the descriptor does not have stays absent, which 10.1.6.3
    /// leaves as it was on an existing property and 6.2.6.6 fills in with the
    /// default on a new one: false for each attribute, undefined for the value
    /// and for each half of an accessor.
    fn to_property_descriptor(
        source: ObjectRef,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<PartialDescriptor, VMError> {
        let field = |name: &str, heap: &mut GenerationalHeap| -> Result<Option<Value>, VMError> {
            let key = PropertyKey::String(heap.strings.intern(name)?);
            let Some(found) = heap.lookup_named(source, key)? else {
                return Ok(None);
            };
            if found.flags.is_accessor {
                // Reading it would call the getter, which this engine can only
                // do from an instruction of a Script.
                return Err(VMError::Unsupported(
                    "a descriptor whose own field is an accessor",
                ));
            }
            Ok(Some(found.value))
        };
        let enumerable = field("enumerable", heap)?;
        let configurable = field("configurable", heap)?;
        let value = field("value", heap)?;
        let writable = field("writable", heap)?;
        let get = field("get", heap)?;
        let set = field("set", heap)?;
        for half in [get, set] {
            // 6.2.6.5 steps 7.b and 8.b: a half that is neither callable nor
            // undefined is not a getter or a setter.
            if let Some(half) = half
                && !half.is_undefined()
                && !Self::is_callable(half, heap)
            {
                return Err(type_error(
                    heap,
                    realm,
                    "accessor must be callable or undefined",
                ));
            }
        }
        let truth = |found: Option<Value>| -> Result<Option<bool>, VMError> {
            found.map(|value| Self::to_boolean(value, heap)).transpose()
        };
        let descriptor = PartialDescriptor {
            value,
            writable: truth(writable)?,
            get,
            set,
            enumerable: truth(enumerable)?,
            configurable: truth(configurable)?,
        };
        // 6.2.6.5 step 9: a descriptor is one kind or the other, never both.
        if descriptor.is_accessor() && descriptor.is_data() {
            return Err(type_error(
                heap,
                realm,
                "descriptor mixes data and accessors",
            ));
        }
        Ok(descriptor)
    }

    /// `ValidateAndApplyPropertyDescriptor` of 10.1.6.3, which answers whether
    /// the descriptor could be applied and applies it when it could.
    ///
    /// A field the descriptor does not name keeps what the property had, and
    /// is the default of 6.2.6.6 on a property that did not exist.
    fn define_property_from(
        object: ObjectRef,
        name: PropertyKey,
        descriptor: &PartialDescriptor,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<bool, VMError> {
        if Self::names_array_length(object, name, heap) {
            return Self::define_array_length(object, descriptor, heap, realm);
        }
        let indexed = Self::element_index_of(object, name, heap);
        let Some(current) = heap.own_named_flags(object, name)? else {
            // Step 2: the property does not exist, so the descriptor makes it.
            if !heap.is_extensible(object).unwrap_or(false) {
                return Ok(false);
            }
            let flags = PropertyFlags {
                writable: descriptor.writable.unwrap_or(false),
                enumerable: descriptor.enumerable.unwrap_or(false),
                configurable: descriptor.configurable.unwrap_or(false),
                is_accessor: descriptor.is_accessor(),
            };
            let stored = Self::descriptor_slot(descriptor, VALUE_UNDEFINED, VALUE_UNDEFINED, heap)?;
            Self::place_property(object, name, indexed, stored, flags, heap)?;
            return Ok(true);
        };
        // Step 4: a descriptor with no field changes nothing.
        if descriptor.is_empty() {
            return Ok(true);
        }
        let held = Self::own_property_value(object, name, indexed, heap)?;
        let (current_get, current_set) = if current.is_accessor {
            Self::accessor_parts(held, heap)?
        } else {
            (VALUE_UNDEFINED, VALUE_UNDEFINED)
        };
        // Step 5: what a non-configurable property does not allow.
        if !current.configurable {
            if descriptor.configurable == Some(true) {
                return Ok(false);
            }
            if descriptor
                .enumerable
                .is_some_and(|wanted| wanted != current.enumerable)
            {
                return Ok(false);
            }
            if !descriptor.is_generic() && descriptor.is_accessor() != current.is_accessor {
                return Ok(false);
            }
            if current.is_accessor {
                for (wanted, existing) in
                    [(descriptor.get, current_get), (descriptor.set, current_set)]
                {
                    if let Some(wanted) = wanted
                        && !same_value(wanted, existing, heap)?
                    {
                        return Ok(false);
                    }
                }
            } else if !current.writable {
                if descriptor.writable == Some(true) {
                    return Ok(false);
                }
                // Step 5.d.ii: the property keeps what it holds, so a
                // descriptor naming the same value is applied by doing
                // nothing at all.
                if let Some(wanted) = descriptor.value {
                    return same_value(wanted, held, heap);
                }
            }
        }
        // Step 6: a property changes kind, or keeps its kind and takes the
        // fields the descriptor names.
        let becomes_accessor = if descriptor.is_generic() {
            current.is_accessor
        } else {
            descriptor.is_accessor()
        };
        let changes_kind = becomes_accessor != current.is_accessor;
        let flags = PropertyFlags {
            writable: if changes_kind {
                descriptor.writable.unwrap_or(false)
            } else {
                descriptor.writable.unwrap_or(current.writable)
            },
            enumerable: descriptor.enumerable.unwrap_or(current.enumerable),
            configurable: descriptor.configurable.unwrap_or(current.configurable),
            is_accessor: becomes_accessor,
        };
        let (get, set) = if changes_kind {
            (VALUE_UNDEFINED, VALUE_UNDEFINED)
        } else {
            (current_get, current_set)
        };
        let stored = if becomes_accessor {
            Self::make_accessor(
                descriptor.get.unwrap_or(get),
                descriptor.set.unwrap_or(set),
                heap,
            )?
        } else if changes_kind {
            descriptor.value.unwrap_or(VALUE_UNDEFINED)
        } else {
            descriptor.value.unwrap_or(held)
        };
        Self::place_property(object, name, indexed, stored, flags, heap)?;
        Ok(true)
    }

    /// The value of the own property a descriptor is about to redefine.
    fn own_property_value(
        object: ObjectRef,
        name: PropertyKey,
        indexed: Option<(ElementsRef, u32)>,
        heap: &mut GenerationalHeap,
    ) -> Result<Value, VMError> {
        if let Some(data) = Self::string_data(object, heap) {
            let units = name
                .as_string()
                .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
                .unwrap_or_default();
            let length = heap
                .strings
                .length_of(data)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            if units == LENGTH_NAME {
                let length = i32::try_from(length).map_err(|_| VMError::StringLimit)?;
                return Ok(Value::from_smi(length));
            }
            if let Some(unit) = string_index(&units)
                .and_then(|index| usize::try_from(index).ok())
                .filter(|index| *index < length)
                .and_then(|index| heap.strings.char_code_at(data, index))
            {
                return Ok(Value::from_string(heap.strings.allocate_units(&[unit])?));
            }
        }
        if let Some((elements, index)) = indexed
            && let Some(value) = heap
                .get_elements(elements)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .get(index)
        {
            return Ok(value);
        }
        // 10.4.2.1: an Array keeps its `length` beside the Shape, so the
        // descriptor of that name reads it from there.
        if let Some(length) = heap.array_length(object)
            && name
                .as_string()
                .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
                .as_deref()
                == Some(&LENGTH_NAME)
        {
            return Ok(index_value(i64::from(length)));
        }
        Ok(heap
            .lookup_named(object, name)?
            .map_or(VALUE_UNDEFINED, |property| property.value))
    }

    /// Puts a property where the object keeps one of its kind, which 10.4.2.1
    /// decides for an index.
    ///
    /// The element store holds a value and nothing else, so it carries exactly
    /// the ordinary data properties. An index that is anything else leaves the
    /// store and becomes a property of the Shape, where a read finds it once
    /// the store answers a hole.
    fn place_property(
        object: ObjectRef,
        name: PropertyKey,
        indexed: Option<(ElementsRef, u32)>,
        stored: Value,
        flags: PropertyFlags,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        // 10.4.3.1 answers an index and the `length` from the
        // `[[StringData]]`, so a Shape that took one of those names would hold
        // a second answer beside the one a read finds. Every other own name of
        // such an object is ordinary and belongs in the Shape.
        if Self::owns_string_exotic(object, name, heap)? {
            return Err(VMError::Unsupported(
                "a descriptor for an own name of a String exotic object",
            ));
        }
        let Some((elements, index)) = indexed else {
            heap.define_own_named(object, name, stored, flags)?;
            return Ok(());
        };
        if flags == PropertyFlags::ordinary_data() {
            heap.remove_own_named(object, name)?;
            if heap.array_length(object).is_some() {
                heap.set_array_element(object, index, stored)?;
            } else {
                heap.set_element(elements, index, stored)?;
            }
            return Ok(());
        }
        heap.delete_element(elements, index)?;
        // 10.4.2.1 step 3.g grows the Array to hold the index it defined.
        if let Some(length) = heap.array_length(object)
            && index >= length
        {
            heap.set_array_length(object, index.saturating_add(1))?;
        }
        heap.define_own_named(object, name, stored, flags)?;
        Ok(())
    }

    /// Whether the Shape of the object carries this name.
    ///
    /// An index is normally in the element store, and one 10.4.2.1 moved out
    /// of it is here instead, where a read and a write have to look.
    fn shape_holds(
        object: ObjectRef,
        name: PropertyKey,
        heap: &GenerationalHeap,
    ) -> Result<bool, VMError> {
        let shape = heap
            .get_object(object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .shape_id;
        Ok(heap.shapes.lookup(shape, name).is_some())
    }

    /// The element store and the index a name denotes on it, when 10.4.2.1
    /// defines the name there rather than in the Shape.
    fn element_index_of(
        object: ObjectRef,
        name: PropertyKey,
        heap: &GenerationalHeap,
    ) -> Option<(ElementsRef, u32)> {
        let elements = heap.get_object(object)?.elements?;
        let units = name
            .as_string()
            .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))?;
        Some((elements, array_index_units(&units)?))
    }

    /// The slot a descriptor gives a property that is being made.
    fn descriptor_slot(
        descriptor: &PartialDescriptor,
        get: Value,
        set: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<Value, VMError> {
        if descriptor.is_accessor() {
            return Self::make_accessor(
                descriptor.get.unwrap_or(get),
                descriptor.set.unwrap_or(set),
                heap,
            );
        }
        Ok(descriptor.value.unwrap_or(VALUE_UNDEFINED))
    }

    /// 10.4.2.4 sets an Array's `length` by deleting what is above the new
    /// one, which this engine does not do.
    ///
    /// The length is a field of the object and no property of its Shape, so a
    /// Shape that took the name would hold a second answer beside it.
    fn names_array_length(object: ObjectRef, name: PropertyKey, heap: &GenerationalHeap) -> bool {
        let units = name
            .as_string()
            .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
            .unwrap_or_default();
        units.as_slice() == LENGTH_NAME && heap.array_length(object).is_some()
    }

    /// `ArraySetLength` of 10.4.2.4 as a `[[DefineOwnProperty]]`.
    ///
    /// The property is never enumerable and never configurable, and its
    /// `[[Writable]]` only ever goes from true to false. A descriptor with no
    /// value only says what those three are; one with a value moves the length
    /// and deletes every index at or above it.
    fn define_array_length(
        object: ObjectRef,
        descriptor: &PartialDescriptor,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<bool, VMError> {
        let writable = heap
            .array_length_is_writable(object)
            .ok_or(VMError::TypeError)?;
        if descriptor.is_accessor()
            || descriptor.enumerable == Some(true)
            || descriptor.configurable == Some(true)
        {
            return Ok(false);
        }
        // Step 2: a descriptor that asks for a writable the property no longer
        // has is refused, and one that asks for the writable it has is not.
        if !writable && (descriptor.writable == Some(true) || descriptor.value.is_some()) {
            return Ok(false);
        }
        let Some(value) = descriptor.value else {
            if descriptor.writable == Some(false) {
                heap.freeze_array_length(object)?;
            }
            return Ok(true);
        };
        let wanted = Self::array_length_of(value, heap, realm)?;
        let current = heap.array_length(object).ok_or(VMError::TypeError)?;
        if wanted < current {
            Self::shorten_array(object, wanted, heap)?;
        }
        heap.set_array_length(object, wanted)?;
        if descriptor.writable == Some(false) {
            heap.freeze_array_length(object)?;
        }
        Ok(true)
    }

    /// The `ToUint32` of 10.4.2.4 step 3, which has to be the Number
    /// `ToNumber` gives.
    fn array_length_of(
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<u32, VMError> {
        let number = primitive_number(value, heap)?;
        let wanted = crate::value::number_uint32(number);
        #[expect(
            clippy::float_cmp,
            reason = "10.4.2.4 asks whether the two are the same Number"
        )]
        let differs = f64::from(wanted) != number;
        if differs {
            return Err(raise(
                heap,
                realm,
                super::realm::NativeErrorKind::RangeError,
                "invalid array length",
            ));
        }
        Ok(wanted)
    }

    /// Deletes every index at or above the new length, which 10.4.2.4 does in
    /// descending order.
    fn shorten_array(
        object: ObjectRef,
        wanted: u32,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        // An index the Shape took over (10.4.2.1) would have to be deleted
        // here too, and 10.4.2.4 stops at one that is not configurable.
        let shape = heap
            .get_object(object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .shape_id;
        for (name, _, _) in heap.shapes.own_properties(shape) {
            let units = name
                .as_string()
                .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
                .unwrap_or_default();
            if array_index_units(&units).is_some_and(|index| index >= wanted) {
                return Err(VMError::Unsupported(
                    "an Array length that deletes a property of the Shape",
                ));
            }
        }
        if let Some(elements) = heap
            .get_object(object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .elements
        {
            let indices = heap
                .get_elements(elements)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .indices();
            for index in indices.into_iter().filter(|index| *index >= wanted) {
                heap.delete_element(elements, index)?;
            }
        }
        Ok(())
    }

    /// The intrinsics that need nothing of the call but its arguments.
    ///
    /// 20.2.1.1 compiles its arguments into a function body, which needs the
    /// parser at run time. 20.2.3.3 never reaches here at all, because
    /// [`Self::enter_call_value`] answers it by entering the call it forwards
    /// to rather than by answering a value.
    fn call_plain_intrinsic(
        intrinsic: Intrinsic,
        first: Value,
        second: Value,
        heap: &GenerationalHeap,
    ) -> Result<Value, VMError> {
        match intrinsic {
            // 20.2.1.1 does not answer here: it stops the run for a unit.
            Intrinsic::FunctionConstructor => Err(VMError::InvalidFeedbackVector),
            // 21.3.2.26 is Number::exponentiate of 6.1.6.1.3 on the two
            // arguments, after 7.1.4 has made numbers of them.
            Intrinsic::MathPow => Ok(Value::from_f64(audhsos_math::pow(
                primitive_number(first, heap)?,
                primitive_number(second, heap)?,
            ))),
            // 23.1.2.3 answers IsArray, which 7.2.2 answers for an Array
            // exotic object and, for a Proxy, for what it wraps.
            Intrinsic::ArrayIsArray => Ok(Value::from_bool(Self::is_array(first, heap))),
            // 21.3.2.19 multiplies the two as 32-bit integers and answers the
            // low half, which 6.1.6.1.4 wraps.
            Intrinsic::MathImul => {
                let left = number_to_i32(primitive_number(first, heap)?);
                let right = number_to_i32(primitive_number(second, heap)?);
                Ok(Value::from_smi(left.wrapping_mul(right)))
            }
            // 21.3.2.30 is the only transcendental this Realm has built.
            Intrinsic::MathSin => Ok(Value::from_f64(audhsos_math::sin(primitive_number(
                first, heap,
            )?))),
            _ => Ok(Value::from_f64(Self::math_of_one(
                intrinsic,
                primitive_number(first, heap)?,
            )?)),
        }
    }

    /// The functions of 21.3.2 that take one Number and need no library of
    /// their own.
    ///
    /// Each one answers NaN for NaN, because 6.1.6.1 propagates it, and each
    /// keeps the sign of a zero where the clause says so.
    fn math_of_one(intrinsic: Intrinsic, value: f64) -> Result<f64, VMError> {
        if value.is_nan() {
            return Ok(f64::NAN);
        }
        Ok(match intrinsic {
            // 21.3.2.1: the magnitude, so -0 becomes +0.
            Intrinsic::MathAbs => {
                if value < 0.0 || (value == 0.0 && value.is_sign_negative()) {
                    -value
                } else {
                    value
                }
            }
            // 21.3.2.16 and 21.3.2.10 are each other's negation, and 21.3.2.10
            // answers -0 for an argument in (-1, 0).
            Intrinsic::MathFloor => Self::round_toward(value, true),
            Intrinsic::MathCeil => -Self::round_toward(-value, true),
            // 21.3.2.35 drops the fraction and keeps the sign.
            Intrinsic::MathTrunc => {
                if value < 0.0 {
                    -Self::round_toward(-value, true)
                } else {
                    Self::round_toward(value, true)
                }
            }
            // 21.3.2.28 is floor(x + 0.5), except that it keeps the sign of a
            // zero and of every argument in [-0.5, 0).
            Intrinsic::MathRound => {
                let rounded = Self::round_toward(value + 0.5, true);
                if rounded == 0.0 && (value < 0.0 || value.is_sign_negative()) {
                    -0.0
                } else {
                    rounded
                }
            }
            // 21.3.2.29 answers the argument itself for either zero.
            Intrinsic::MathSign => {
                if value == 0.0 {
                    value
                } else if value < 0.0 {
                    -1.0
                } else {
                    1.0
                }
            }
            // 21.3.2.11 counts the leading zeroes of the 32-bit integer
            // 7.1.6 makes of the argument.
            Intrinsic::MathClz32 => f64::from(number_to_i32(value).leading_zeros()),
            // 21.3.2.17 is the binary32 nearest the argument, back as a
            // binary64.
            Intrinsic::MathFround => {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "21.3.2.17 is that rounding, not an accident of it"
                )]
                let narrowed = value as f32;
                f64::from(narrowed)
            }
            _ => return Err(VMError::TypeError),
        })
    }

    /// The integer part of a finite, non-negative-rounded value.
    ///
    /// `floor` of an f64 without a library: a magnitude at or above 2^52 is
    /// already an integer, and below it the round trip through i64 is exact.
    fn round_toward(value: f64, down: bool) -> f64 {
        if !value.is_finite() || value.abs() >= 4_503_599_627_370_496.0 {
            return value;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the magnitude is below 2^52, which i64 holds exactly"
        )]
        let truncated = value as i64;
        #[expect(
            clippy::cast_precision_loss,
            reason = "the magnitude is below 2^52, which binary64 holds exactly"
        )]
        let mut integral = truncated as f64;
        if down && integral > value {
            integral -= 1.0;
        }
        // 21.3.2.16 answers -0 for an argument in (-1, 0), which the round
        // trip through i64 loses.
        if integral == 0.0 && value.is_sign_negative() {
            return -0.0;
        }
        integral
    }

    /// `Math.max` of 21.3.2.24 and `Math.min` of 21.3.2.25.
    ///
    /// Every argument goes through `ToNumber` before any comparison, a NaN
    /// among them makes the answer NaN, and +0 is larger than -0.
    fn call_math_extremum(
        &self,
        intrinsic: Intrinsic,
        call: Call,
        heap: &GenerationalHeap,
    ) -> Result<Value, VMError> {
        let highest = intrinsic == Intrinsic::MathMax;
        let mut answer = if highest {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
        let mut saw_nan = false;
        for index in 0..call.arg_count {
            let value = primitive_number(self.call_argument(&call, index, heap)?, heap)?;
            if value.is_nan() {
                saw_nan = true;
                continue;
            }
            let wins = if highest {
                value > answer || (value == 0.0 && answer == 0.0 && value.is_sign_positive())
            } else {
                value < answer || (value == 0.0 && answer == 0.0 && value.is_sign_negative())
            };
            if wins {
                answer = value;
            }
        }
        Ok(Value::from_f64(if saw_nan { f64::NAN } else { answer }))
    }

    /// What 20.1.3.2 and 20.1.3.4 answer about an own property.
    ///
    /// Both apply `ToPropertyKey` to the argument and `ToObject` to the
    /// receiver before they look, and both answer false for a property that is
    /// not there; 20.1.3.4 answers false for one that is there and is not
    /// enumerable.
    fn own_property_test(
        intrinsic: Intrinsic,
        key: Value,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let key = property_key(key, heap)?;
        let object = Self::coerce_object(call.receiver, heap, realm)?;
        let own = heap.own_named_flags(object, key)?;
        Ok(Value::from_bool(match intrinsic {
            Intrinsic::ObjectPrototypePropertyIsEnumerable => {
                own.is_some_and(|flags| flags.enumerable)
            }
            _ => own.is_some(),
        }))
    }

    /// `Error ( message )` of 20.5.1.1 and `NativeError ( message )` of
    /// 20.5.6.1.1.
    ///
    /// Both make an ordinary object with an `[[ErrorData]]` slot whose
    /// Prototype belongs to the constructor that was called, and give it an
    /// own `message` when one was passed. `new` reaches the same function,
    /// because a native constructor answers an object of its own.
    fn construct_error(
        intrinsic: Intrinsic,
        message: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        // 20.5.1.1 step 3 leaves an absent message out rather than writing
        // "undefined", which the Prototype's empty String then answers. Every
        // other value goes through ToString, which 7.1.17 gives it.
        let text = if message.is_undefined() {
            None
        } else {
            Some(property_name_units(message, heap)?)
        };
        let error = match super::realm::NativeErrorKind::of(intrinsic) {
            Some(kind) => realm.create_native_error_units(heap, kind, text.as_deref())?,
            None => realm.create_error_units(heap, text.as_deref())?,
        };
        Ok(Value::from_object(error))
    }

    /// `Object ( value )` of 20.1.1.1.
    ///
    /// undefined and null make an ordinary object, and every other value goes
    /// through `ToObject`, which answers an Object unchanged and wraps a
    /// primitive.
    fn construct_object(
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if value.is_undefined() || value.is_null() {
            let shape = heap.shapes.root_shape();
            let prototype = realm.object_prototype(heap)?;
            let object = heap.allocate_object(shape, prototype)?;
            return Ok(Value::from_object(object));
        }
        if value.as_object().is_some() {
            return Ok(value);
        }
        Ok(Value::from_object(Self::coerce_object(value, heap, realm)?))
    }

    /// `Array ( ...values )` of 23.1.1.1.
    ///
    /// The prototype is `%Array.prototype%`, because this Realm builds no
    /// constructor that could be a `NewTarget` of its own.
    fn construct_array(
        &self,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if call.arg_count == 1 {
            let length = self.call_argument(&call, 0, heap)?;
            let Some(number) = length.as_f64() else {
                let array = realm.array(heap, 1)?;
                heap.set_array_element(array, 0, length)?;
                return Ok(Value::from_object(array));
            };
            // ToUint32 and SameValueZero together: a Number that is not a
            // length names no Array of that length.
            let Some(length) = exact_array_length(number) else {
                return Err(raise_message(
                    heap,
                    realm,
                    super::realm::NativeErrorKind::RangeError,
                    "invalid array length",
                ));
            };
            let array = realm.array(heap, 0)?;
            heap.set_array_length(array, length)?;
            return Ok(Value::from_object(array));
        }
        let count = u32::from(call.arg_count);
        let array = realm.array(heap, count)?;
        for index in 0..call.arg_count {
            let value = self.call_argument(&call, index, heap)?;
            heap.set_array_element(array, u32::from(index), value)?;
        }
        Ok(Value::from_object(array))
    }

    /// Runs one of the `%Array.prototype%` search methods of 23.1.3.
    ///
    /// Each one reads `length` first and then the indices below it, so a scan
    /// bounded by the index space the Elements store addresses answers the
    /// same: every index above it is absent, which `indexOf` and `lastIndexOf`
    /// skip and `includes` reads as undefined, as it reads the absent index
    /// the bounded scan already visits.
    ///
    /// Starts one of the methods of 23.1.3 that ask the Script about each
    /// element (23.1.3.6, .8, .9, .10, .15, .21, .24, .25, .29).
    ///
    /// The walk cannot finish here: every element is a call, and the engine
    /// leaves this function to make it. What it has reached goes into an
    /// object of the heap, which the collector traces and which
    /// [`Self::step_array_iteration`] reads again each time a callback
    /// answers.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` for a callback that is
    /// not callable, which is step 3 of each clause, and for the empty Array
    /// of 23.1.3.24 step 6 that was given no initial value.
    fn begin_array_iteration(
        &mut self,
        intrinsic: Intrinsic,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        // 23.1.2.1 takes the array-like as its first argument and the mapper
        // as its second; every clause of 23.1.3 takes the receiver and the
        // callback.
        let from = intrinsic == Intrinsic::ArrayFrom;
        let source = if from {
            let items = self.call_argument(&call, 0, heap)?;
            if items.is_undefined() || items.is_null() {
                return Err(type_error(heap, realm, "cannot box null or undefined"));
            }
            items
        } else {
            call.receiver
        };
        let target = Self::coerce_object(source, heap, realm)?;
        if from {
            let mapper = self.call_argument(&call, 1, heap)?;
            if !mapper.is_undefined() && !Self::is_callable(mapper, heap) {
                return Err(type_error(heap, realm, "the mapper is not callable"));
            }
            // Step 2: an `@@iterator` decides the whole clause, and this Realm
            // reaches one only through a frame. 22.1.3.36 gives a String one
            // that yields code points, which this Realm has not built and
            // which no index walk stands in for.
            if heap
                .lookup_named(target, super::realm::WellKnownSymbol::Iterator.key())?
                .is_some()
                || Self::string_data(target, heap).is_some()
            {
                return Err(VMError::Unsupported("an @@iterator in Array.from"));
            }
        }
        let callback = self.call_argument(&call, u16::from(from), heap)?;
        // 23.1.3.24 takes its second argument as the accumulator; every other
        // clause takes it as the `this` value of the callback. Both are read
        // before the `length`, because reading a register has no effect a
        // Script can see and its getter may open a frame of its own.
        let second = self.call_argument(&call, if from { 2 } else { 1 }, heap)?;
        let reduces = matches!(
            intrinsic,
            Intrinsic::ArrayPrototypeReduce | Intrinsic::ArrayPrototypeReduceRight
        );
        // A scan keeps its search element where a callback would go and the
        // index it was given where the accumulator would, because it has
        // neither; `started` says whether it was given one, as it says for
        // 23.1.3.24 whether it was given an initial value.
        let (receiver, output, started) = if reduces || Self::scans_for_an_element(intrinsic) {
            (VALUE_UNDEFINED, second, call.arg_count > 1)
        } else {
            (second, VALUE_UNDEFINED, true)
        };
        let backwards = matches!(
            intrinsic,
            Intrinsic::ArrayPrototypeReduceRight
                | Intrinsic::ArrayPrototypeFindLast
                | Intrinsic::ArrayPrototypeFindLastIndex
        );
        let pending = Self::array_like_length_getter(heap, target)?;
        // A `length` that is a data property answers a value 7.1.20 converts;
        // an Object there runs a method of the Script, which the walk enters
        // the way it enters the getter.
        let unconverted = if pending.is_some() {
            VALUE_UNDEFINED
        } else {
            Self::array_like_length_value(heap, target, realm)?
        };
        let mut length = 0;
        if pending.is_none() && !unconverted.is_object() {
            let read = integer_argument(unconverted, heap, realm)?.max(0);
            Self::refuse_long_array(intrinsic, read, heap, realm)?;
            length = u32::try_from(read).unwrap_or(u32::MAX);
        }
        let converts = pending.is_none() && unconverted.is_object();
        let prototype = realm.object_prototype(heap)?;
        let shape = heap.shapes.root_shape();
        let state = heap.allocate_object(shape, prototype)?;
        heap.set_object_kind(
            state,
            ObjectKind::ArrayIteration {
                intrinsic: intrinsic.id(),
                target: Value::from_object(target),
                callback,
                receiver,
                output,
                element: unconverted,
                element_index: 0,
                index: if backwards { length } else { 0 },
                length,
                started,
                getter: false,
                converting: if pending.is_some() || converts {
                    CONVERTING_LENGTH
                } else {
                    CONVERTING_NOTHING
                },
                convert_step: 0,
            },
        )?;
        // The state outlives every frame the walk opens, so it is a root of a
        // scope of its own, which the walk leaves when it answers.
        heap.enter_scope();
        let state = heap.push_root(Value::from_object(state))?;
        // 7.1.20 reads a `length` that is an accessor by calling its getter,
        // which the walk enters the way it enters the callback.
        if let Some(getter) = pending {
            let mut call = call;
            call.arg_count = 0;
            call.arg_start = Reg(0);
            call.receiver = Value::from_object(target);
            call.resume = Some(Resume::Iteration { state });
            call.construct = None;
            let mut walk = Self::read_iteration(state, heap)?;
            walk.convert_step = 1;
            Self::write_iteration(state, &walk, heap)?;
            return self.enter_call_value(getter, units, active_feedback, heap, realm, call);
        }
        if converts {
            return self.convert_pending_value(state, call, units, active_feedback, heap, realm);
        }
        self.begin_array_walk(state, call, units, active_feedback, heap, realm)
    }

    /// Asks the next method of 7.1.1 for the value the walk is converting,
    /// with the hint `number`.
    ///
    /// The object waits in `element` of the walk state, which the collector
    /// traces; `convert_step` says which method has already answered, and
    /// `converting` which value the answer is for. A
    /// method of the Realm answers without a frame, so the loop takes that
    /// answer and goes on rather than leaving.
    fn convert_pending_value(
        &mut self,
        state: Root,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        loop {
            let mut walk = Self::read_iteration(state, heap)?;
            let object = walk
                .element
                .as_object()
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            // 7.1.1 step 1 would ask @@toPrimitive first, which takes the hint
            // as an argument and so a register of the caller. A walk has none
            // to give, so an object that carries one is a gap.
            if walk.convert_step < 2
                && heap
                    .lookup_named(object, super::realm::WellKnownSymbol::ToPrimitive.key())?
                    .is_some()
            {
                heap.exit_scope();
                return Err(VMError::Unsupported(
                    "the @@toPrimitive of an argument a walk of 23.1.3 converts",
                ));
            }
            let step = walk.convert_step.max(1).saturating_add(1);
            let name = match step {
                2 => "valueOf",
                3 => "toString",
                _ => {
                    heap.exit_scope();
                    return Err(type_error(heap, realm, "an object has no primitive value"));
                }
            };
            walk.convert_step = step;
            Self::write_iteration(state, &walk, heap)?;
            let key = PropertyKey::String(heap.strings.intern(name)?);
            let object = Self::read_iteration(state, heap)?
                .element
                .as_object()
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let method = heap
                .lookup_named(object, key)?
                .map(Self::plain_value)
                .transpose()?
                .filter(|method| Self::is_callable(*method, heap));
            let Some(method) = method else {
                continue;
            };
            let mut call = call;
            call.arg_count = 0;
            call.arg_start = Reg(0);
            call.receiver = Value::from_object(object);
            call.resume = Some(Resume::Iteration { state });
            call.construct = None;
            if let Some(code_id) =
                self.enter_call_value(method, units, active_feedback, heap, realm, call)?
            {
                return Ok(Some(code_id));
            }
            // A method of the Realm answered without a frame of its own.
            let answer = self.acc;
            if !answer.is_object() {
                Self::take_converted(state, answer, heap, realm)?;
                return self.begin_array_walk(state, call, units, active_feedback, heap, realm);
            }
        }
    }

    /// Puts the primitive 7.1.1 answered where the walk was waiting for it.
    fn take_converted(
        state: Root,
        answer: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let mut walk = Self::read_iteration(state, heap)?;
        walk.element = VALUE_UNDEFINED;
        walk.convert_step = 0;
        if walk.converting == CONVERTING_FROM {
            walk.converting = CONVERTING_NOTHING;
            walk.output = answer;
            Self::write_iteration(state, &walk, heap)?;
            return Ok(());
        }
        walk.converting = CONVERTING_NOTHING;
        let length = integer_argument(answer, heap, realm)?.max(0);
        if let Err(refused) = Self::refuse_long_array(walk.intrinsic, length, heap, realm) {
            heap.exit_scope();
            return Err(refused);
        }
        walk.length = u32::try_from(length).unwrap_or(u32::MAX);
        if walk.backwards() {
            walk.index = walk.length;
        }
        Self::write_iteration(state, &walk, heap)?;
        Ok(())
    }

    /// The value the `length` of an array-like holds, before 7.1.20 converts
    /// it.
    fn array_like_length_value(
        heap: &GenerationalHeap,
        object: ObjectRef,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if let Some(length) = heap.array_length(object) {
            return Ok(index_value(i64::from(length)));
        }
        if let Some(count) = Self::string_data_length(object, heap)? {
            return Ok(index_value(i64::from(count)));
        }
        let Some(name) = heap.strings.lookup_interned_units(&LENGTH_NAME) else {
            return Ok(VALUE_UNDEFINED);
        };
        let Some(property) = heap.lookup_named(object, PropertyKey::String(name))? else {
            Self::absent_property(Value::from_object(object), &LENGTH_NAME, heap, realm)?;
            return Ok(VALUE_UNDEFINED);
        };
        if property.flags.is_accessor {
            let (get, _) = Self::accessor_parts(property.value, heap)?;
            if !get.is_undefined() {
                return Err(VMError::Unsupported("a property that is an accessor"));
            }
            return Ok(VALUE_UNDEFINED);
        }
        Ok(property.value)
    }

    /// 10.4.2.2 step 1 refuses a length past 2^32-1, which 7.1.20 allows and
    /// an Array cannot hold. Only 23.1.3.21 makes an Array of that length.
    fn refuse_long_array(
        intrinsic: Intrinsic,
        length: i64,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        if intrinsic == Intrinsic::ArrayPrototypeMap && u32::try_from(length).is_err() {
            return Err(raise(
                heap,
                realm,
                super::realm::NativeErrorKind::RangeError,
                "invalid array length",
            ));
        }
        Ok(())
    }

    /// The getter of a `length` that is an accessor, which 7.1.20 calls.
    ///
    /// A getter written in Rust takes its arguments from registers of the
    /// caller, so one of those is a gap rather than a call.
    fn array_like_length_getter(
        heap: &GenerationalHeap,
        object: ObjectRef,
    ) -> Result<Option<Value>, VMError> {
        if heap.array_length(object).is_some() {
            return Ok(None);
        }
        let Some(name) = heap.strings.lookup_interned_units(&LENGTH_NAME) else {
            return Ok(None);
        };
        let Some(property) = heap.lookup_named(object, PropertyKey::String(name))? else {
            return Ok(None);
        };
        if !property.flags.is_accessor {
            return Ok(None);
        }
        let (get, _) = Self::accessor_parts(property.value, heap)?;
        if get.is_undefined() {
            // 10.1.8.1 step 3.b: a property with no getter reads undefined,
            // which 7.1.20 clamps to zero.
            return Ok(None);
        }
        if !Self::is_script_function(get, heap) {
            return Err(VMError::Unsupported(
                "a getter that is not a Script function",
            ));
        }
        Ok(Some(get))
    }

    /// The checks 23.1.3 makes once it knows the length, and the Array the two
    /// clauses that build one start from.
    fn begin_array_walk(
        &mut self,
        state: Root,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let mut walk = Self::read_iteration(state, heap)?;
        if Self::scans_for_an_element(walk.intrinsic) {
            // 7.1.5 converts that argument only after the `length` is read, so
            // an Object there runs its methods here and not before the walk.
            if walk.started && walk.output.is_object() {
                walk.converting = CONVERTING_FROM;
                walk.convert_step = 1;
                walk.element = walk.output;
                Self::write_iteration(state, &walk, heap)?;
                return self.convert_pending_value(
                    state,
                    call,
                    units,
                    active_feedback,
                    heap,
                    realm,
                );
            }
            // 23.1.3.16, 23.1.3.17 and 23.1.3.20 take the index they start
            // from out of the argument they were given, against the length
            // they have just read.
            let length = i64::from(walk.length);
            let start = if walk.intrinsic == Intrinsic::ArrayPrototypeLastIndexOf {
                let from = if walk.started {
                    integer_argument(walk.output, heap, realm)?
                } else {
                    length.saturating_sub(1)
                };
                if from >= 0 {
                    from.min(length.saturating_sub(1)).saturating_add(1)
                } else {
                    length.saturating_add(from).saturating_add(1).max(0)
                }
            } else {
                let from = integer_argument(walk.output, heap, realm)?;
                absolute_index(from, length).clamp(0, length)
            };
            walk.index = u32::try_from(start).unwrap_or(u32::MAX);
            walk.output = VALUE_UNDEFINED;
            Self::write_iteration(state, &walk, heap)?;
            return self.step_array_iteration(
                state,
                None,
                call,
                units,
                active_feedback,
                heap,
                realm,
            );
        }
        // 23.1.2.1 takes a mapper or none at all; every other clause here
        // asks for a callback.
        if !Self::is_callable(walk.callback, heap)
            && (walk.intrinsic != Intrinsic::ArrayFrom || !walk.callback.is_undefined())
        {
            heap.exit_scope();
            return Err(type_error(heap, realm, "callback is not callable"));
        }
        let reduces = matches!(
            walk.intrinsic,
            Intrinsic::ArrayPrototypeReduce | Intrinsic::ArrayPrototypeReduceRight
        );
        if reduces && !walk.started && walk.length == 0 {
            heap.exit_scope();
            return Err(type_error(
                heap,
                realm,
                "reduce of an empty Array with no initial value",
            ));
        }
        // 23.1.3.21 and 23.1.3.8 answer an Array of their own, made before the
        // walk so the collector sees it from its first element on. 10.4.2.2
        // step 1 refuses a length past 2^32-1, which 7.1.20 allows and an
        // Array cannot hold.
        if matches!(
            walk.intrinsic,
            Intrinsic::ArrayPrototypeMap | Intrinsic::ArrayPrototypeFilter | Intrinsic::ArrayFrom
        ) {
            let wanted = if walk.intrinsic == Intrinsic::ArrayPrototypeFilter {
                0
            } else {
                walk.length
            };
            // 23.1.3.21 and 23.1.3.8 make their answer with 23.1.3.4;
            // 23.1.2.1 makes an ordinary Array of the Realm.
            let created = if walk.intrinsic == Intrinsic::ArrayFrom {
                realm.array(heap, wanted)?
            } else {
                let object = walk
                    .target
                    .as_object()
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                match Self::array_species_create(object, wanted, heap, realm) {
                    Ok(created) => created,
                    Err(refused) => {
                        heap.exit_scope();
                        return Err(refused);
                    }
                }
            };
            walk.output = Value::from_object(created);
            Self::write_iteration(state, &walk, heap)?;
        }
        self.step_array_iteration(state, None, call, units, active_feedback, heap, realm)
    }

    /// Takes the answer of one callback, then asks for the next element or
    /// answers what the clause answers.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Heap`] when the root no longer names the state the
    /// walk started with.
    #[expect(
        clippy::too_many_arguments,
        reason = "a step of the walk runs where a call does, with what a call has"
    )]
    #[expect(
        clippy::too_many_lines,
        reason = "one function carries every phase of the walk of 23.1.3"
    )]
    fn step_array_iteration(
        &mut self,
        state: Root,
        mut answered: Option<Value>,
        mut call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        // An index the object does not have opens no frame, so nothing would
        // charge for looking at it. A clause of 23.1.3 walks to the `length`
        // 7.1.20 gave it, which a Script can make 2^32-1 while holding one
        // element, so the walk charges for the indices it passes over, at the
        // rate a batch of them costs.
        // 7.1.20 answered the `length` its getter gave, and the walk starts
        // from the checks 23.1.3 makes once it knows it.
        if let Some(answer) = answered {
            let mut walk = Self::read_iteration(state, heap)?;
            if walk.converting != CONVERTING_NOTHING {
                // 7.1.1 asks the next method when the one that answered gave
                // an Object. The getter answered the value to convert; a
                // method of 7.1.1.1 answered something 7.1.1.1 discards, and
                // the next method is asked of the same object.
                if answer.is_object() {
                    if walk.convert_step <= 1 {
                        walk.element = answer;
                        Self::write_iteration(state, &walk, heap)?;
                    }
                    return self.convert_pending_value(
                        state,
                        call,
                        units,
                        active_feedback,
                        heap,
                        realm,
                    );
                }
                Self::take_converted(state, answer, heap, realm)?;
                return self.begin_array_walk(state, call, units, active_feedback, heap, realm);
            }
        }
        let mut skipped: u32 = 0;
        loop {
            let mut walk = Self::read_iteration(state, heap)?;
            // A call the walk made answered: the getter of the element it is
            // at, or the callback of the clause.
            let mut got = None;
            if let Some(answer) = answered.take() {
                if walk.getter {
                    walk.getter = false;
                    got = Some(answer);
                } else if let Some(done) = Self::take_callback_answer(&mut walk, answer, heap)? {
                    heap.exit_scope();
                    self.acc = done;
                    return Ok(None);
                }
            }
            let object = walk
                .target
                .as_object()
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let element = if let Some(element) = got {
                element
            } else {
                let Some(element_index) = walk.next_index() else {
                    // 23.1.3.24 step 6: a walk that found no element and was
                    // given no initial value has no accumulator to answer. A
                    // scan keeps its own meaning for `started`.
                    if !walk.started
                        && matches!(
                            walk.intrinsic,
                            Intrinsic::ArrayPrototypeReduce | Intrinsic::ArrayPrototypeReduceRight
                        )
                    {
                        heap.exit_scope();
                        return Err(type_error(
                            heap,
                            realm,
                            "reduce of an empty Array with no initial value",
                        ));
                    }
                    heap.exit_scope();
                    self.acc = walk.answer();
                    return Ok(None);
                };
                walk.advance(element_index);
                // 23.1.3 walks the indices the object has, so a hole never
                // reaches the callback.
                let found = Self::element_slot_at(heap, object, element_index)?;
                let Some((slot, accessor)) = found.or_else(|| {
                    // 23.1.3.9, 23.1.3.10, 23.1.3.12 and 23.1.3.13 read every
                    // index with 7.3.2, so a hole reaches the callback as
                    // undefined instead of being passed over.
                    Self::visits_holes(walk.intrinsic).then_some((VALUE_UNDEFINED, false))
                }) else {
                    Self::write_iteration(state, &walk, heap)?;
                    skipped = skipped.saturating_add(1);
                    if skipped.is_multiple_of(HOLES_PER_FUEL_UNIT) {
                        self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                    }
                    continue;
                };
                walk.element_index = element_index;
                if accessor {
                    let (get, _) = Self::accessor_parts(slot, heap)?;
                    // 10.1.8.1 step 3.b: a property with no getter reads
                    // undefined, which needs no frame.
                    if get.is_undefined() {
                        VALUE_UNDEFINED
                    } else {
                        if !Self::is_script_function(get, heap) {
                            return Err(VMError::Unsupported(
                                "a getter that is not a Script function",
                            ));
                        }
                        walk.getter = true;
                        Self::write_iteration(state, &walk, heap)?;
                        call.arg_count = 0;
                        call.arg_start = Reg(0);
                        call.receiver = walk.target;
                        call.resume = Some(Resume::Iteration { state });
                        call.construct = None;
                        return self.enter_call_value(
                            get,
                            units,
                            active_feedback,
                            heap,
                            realm,
                            call,
                        );
                    }
                } else {
                    slot
                }
            };
            // 23.1.3.16, 23.1.3.17 and 23.1.3.20 compare the element
            // themselves and never open a frame for one.
            if Self::scans_for_an_element(walk.intrinsic) {
                self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                let found = if walk.intrinsic == Intrinsic::ArrayPrototypeIncludes {
                    same_value_zero(walk.callback, element, heap)?
                } else {
                    Self::strictly_equals(walk.callback, element, heap)?
                };
                if found {
                    heap.exit_scope();
                    self.acc = if walk.intrinsic == Intrinsic::ArrayPrototypeIncludes {
                        VALUE_TRUE
                    } else {
                        index_value(i64::from(walk.element_index))
                    };
                    return Ok(None);
                }
                Self::write_iteration(state, &walk, heap)?;
                continue;
            }
            // 23.1.2.1 without a mapper writes the element it read, with no
            // call to make for it.
            if walk.intrinsic == Intrinsic::ArrayFrom && walk.callback.is_undefined() {
                let array = walk
                    .output
                    .as_object()
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                heap.set_array_element(array, walk.element_index, element)?;
                Self::write_iteration(state, &walk, heap)?;
                continue;
            }
            // 23.1.3.24 takes the first element it finds as the accumulator and
            // calls the callback only from the next one on.
            if !walk.started {
                walk.started = true;
                walk.output = element;
                Self::write_iteration(state, &walk, heap)?;
                continue;
            }
            walk.element = element;
            Self::write_iteration(state, &walk, heap)?;
            let callback = walk.callback;
            call.arg_count = walk.callback_arity();
            call.arg_start = Reg(0);
            call.receiver = walk.receiver;
            call.resume = Some(Resume::Iteration { state });
            call.construct = None;
            return self.enter_call_value(callback, units, active_feedback, heap, realm, call);
        }
    }

    /// Reads the state of a walk out of the object the root names.
    fn read_iteration(state: Root, heap: &GenerationalHeap) -> Result<ArrayWalk, VMError> {
        let reference = heap
            .root_value(state)
            .and_then(Value::as_object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        let ObjectKind::ArrayIteration {
            intrinsic,
            target,
            callback,
            receiver,
            output,
            element,
            element_index,
            index,
            length,
            started,
            getter,
            converting,
            convert_step,
        } = heap
            .get_object(reference)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .kind
        else {
            return Err(VMError::Heap(HeapError::InvalidReference));
        };
        Ok(ArrayWalk {
            intrinsic: Intrinsic::from_id(intrinsic).ok_or(VMError::TypeError)?,
            target,
            callback,
            receiver,
            output,
            element,
            element_index,
            index,
            length,
            started,
            getter,
            converting,
            convert_step,
        })
    }

    /// Writes the state of a walk back into the object the root names.
    fn write_iteration(
        state: Root,
        walk: &ArrayWalk,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let reference = heap
            .root_value(state)
            .and_then(Value::as_object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        heap.set_object_kind(
            reference,
            ObjectKind::ArrayIteration {
                intrinsic: walk.intrinsic.id(),
                target: walk.target,
                callback: walk.callback,
                receiver: walk.receiver,
                output: walk.output,
                element: walk.element,
                element_index: walk.element_index,
                index: walk.index,
                length: walk.length,
                started: walk.started,
                getter: walk.getter,
                converting: walk.converting,
                convert_step: walk.convert_step,
            },
        )?;
        Ok(())
    }

    /// What one callback answered, by the clause that asked.
    ///
    /// Answers `Some` when the clause stops there, which 23.1.3.6, 23.1.3.29,
    /// 23.1.3.9 and 23.1.3.10 each do at the first element that decides them.
    fn take_callback_answer(
        walk: &mut ArrayWalk,
        answer: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<Option<Value>, VMError> {
        let truthy = Self::to_boolean(answer, heap)?;
        match walk.intrinsic {
            // 23.1.3.15 uses no answer at all.
            Intrinsic::ArrayPrototypeForEach => {}
            // 23.1.3.21 and 23.1.2.1 write the answer at the same index.
            Intrinsic::ArrayPrototypeMap | Intrinsic::ArrayFrom => {
                let array = walk
                    .output
                    .as_object()
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                heap.set_array_element(array, walk.element_index, answer)?;
            }
            // 23.1.3.8 appends the element the answer kept.
            Intrinsic::ArrayPrototypeFilter => {
                if truthy {
                    let array = walk
                        .output
                        .as_object()
                        .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                    let next = heap.array_length(array).unwrap_or(0);
                    heap.set_array_element(array, next, walk.element)?;
                }
            }
            // 23.1.3.6 stops at the first false, 23.1.3.29 at the first true.
            Intrinsic::ArrayPrototypeEvery => {
                if !truthy {
                    return Ok(Some(VALUE_FALSE));
                }
            }
            Intrinsic::ArrayPrototypeSome => {
                if truthy {
                    return Ok(Some(VALUE_TRUE));
                }
            }
            // 23.1.3.9 answers the element and 23.1.3.10 its index.
            Intrinsic::ArrayPrototypeFind | Intrinsic::ArrayPrototypeFindLast => {
                if truthy {
                    return Ok(Some(walk.element));
                }
            }
            Intrinsic::ArrayPrototypeFindIndex | Intrinsic::ArrayPrototypeFindLastIndex => {
                if truthy {
                    return Ok(Some(index_value(i64::from(walk.element_index))));
                }
            }
            // 23.1.3.24 and 23.1.3.25 carry the answer to the next element.
            _ => walk.output = answer,
        }
        Ok(None)
    }

    /// `CreateMethodProperty` of 7.3.5, which 15.7.14 relies on: the method
    /// in the accumulator becomes a writable and configurable property that is
    /// not enumerable.
    fn define_method(
        &self,
        obj: Reg,
        name: PropertyKey,
        enumerable: bool,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let method = self.acc;
        let target = self.read_reg(obj)?.as_object().ok_or(VMError::TypeError)?;
        Self::make_method(method, target, heap)?;
        heap.define_own_named(
            target,
            name,
            method,
            PropertyFlags {
                enumerable,
                ..PropertyFlags::constructor_data()
            },
        )?;
        Ok(())
    }

    /// The `[[HomeObject]]` the running function carries (10.2).
    fn home_of(&self, code: &BytecodeFunction) -> Result<Value, VMError> {
        let register = code
            .home_register
            .ok_or(VMError::Unsupported("super in a function with no home"))?;
        self.read_reg(register)
    }

    /// `super.name` of 13.3.7: the property of the base, read through `this`.
    ///
    /// The receiver of the read is the `this` value of the running call, so a
    /// getter of the chain is called with it and not with the base.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a base that is not an object.
    #[expect(
        clippy::too_many_arguments,
        reason = "a super read enters a getter, which needs what a call needs"
    )]
    fn read_super(
        &mut self,
        base: Reg,
        name: PropertyKey,
        return_pc: usize,
        caller_code_id: Option<u32>,
        code: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let value = self.read_reg(base)?;
        let Some(object) = value.as_object() else {
            return Err(property_base_error(value, heap, realm));
        };
        let receiver = self.this_of(code.active)?;
        let Some(found) = heap.lookup_named(object, name)? else {
            let units = name
                .as_string()
                .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
                .unwrap_or_default();
            self.acc = Self::absent_property(value, &units, heap, realm)?;
            return Ok(None);
        };
        if !found.flags.is_accessor {
            self.acc = found.value;
            return Ok(None);
        }
        self.enter_accessor(
            found.value,
            receiver,
            None,
            return_pc,
            caller_code_id,
            code,
            active_feedback,
            heap,
            realm,
        )
    }

    /// The `this` value of the running call (9.4.5).
    fn this_of(&self, code: &BytecodeFunction) -> Result<Value, VMError> {
        let register = code
            .this_register
            .ok_or(VMError::Unsupported("super in a function with no this"))?;
        self.read_reg(register)
    }

    /// 15.7.14 steps 6 through 8 and 14: ties the class in the accumulator to
    /// the value its `extends` clause produced.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` for a heritage that is
    /// neither `null` nor a constructor, and for one whose `prototype` is
    /// neither `null` nor an Object.
    fn derive_class(
        &self,
        heritage: Reg,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let class = self.acc.as_object().ok_or(VMError::TypeError)?;
        let name = PropertyKey::String(heap.strings.intern("prototype")?);
        let prototype = heap
            .lookup_named(class, name)?
            .map_or(VALUE_UNDEFINED, |property| property.value)
            .as_object()
            .ok_or(VMError::TypeError)?;
        let value = self.read_reg(heritage)?;
        // Step 6.a: a class of `extends null` inherits nothing and is
        // constructed like any other ordinary function.
        let (proto_parent, class_parent) = if value.is_null() {
            (VALUE_NULL, realm.function_prototype(heap)?)
        } else {
            if !Self::constructs(value, heap) {
                return Err(type_error(heap, realm, "class extends a non-constructor"));
            }
            let parent = value.as_object().ok_or(VMError::TypeError)?;
            let found = heap
                .lookup_named(parent, name)?
                .map(Self::plain_value)
                .transpose()?
                .unwrap_or(VALUE_UNDEFINED);
            if !found.is_null() && found.as_object().is_none() {
                return Err(type_error(
                    heap,
                    realm,
                    "the prototype of a superclass is not an object",
                ));
            }
            (found, value)
        };
        heap.set_object_prototype(class, class_parent)
            .map_err(VMError::Heap)?;
        heap.set_object_prototype(prototype, proto_parent)
            .map_err(VMError::Heap)?;
        Ok(())
    }

    /// `SuperCall` of 13.3.7.1.
    ///
    /// 9.4.4 answers the Prototype of the running function, which 7.3.15
    /// constructs with the `[[NewTarget]]` of this call. The instruction that
    /// follows writes the answer into the register of the `this` binding.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `ReferenceError` for a binding
    /// 13.3.7.1 already made, and with a `TypeError` for a Prototype that is
    /// no constructor.
    #[expect(
        clippy::too_many_arguments,
        reason = "a super call opens a frame, which needs what a call needs"
    )]
    fn super_call(
        &mut self,
        arg_start: Reg,
        arg_count: u16,
        forwarded: bool,
        slot: u16,
        return_pc: usize,
        caller_code_id: Option<u32>,
        code: &BytecodeFunction,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let this_register = code
            .this_register
            .ok_or(VMError::Unsupported("a super call with no this binding"))?;
        // Step 8: the binding is made once.
        if self.read_reg(this_register)? != VALUE_UNINITIALIZED {
            return Err(raise(
                heap,
                realm,
                super::realm::NativeErrorKind::ReferenceError,
                "this is already initialized",
            ));
        }
        let self_register = code
            .self_register
            .ok_or(VMError::Unsupported("a super call with no callee"))?;
        let new_target_register = code
            .new_target_register
            .ok_or(VMError::Unsupported("a super call with no new target"))?;
        let active = self
            .read_reg(self_register)?
            .as_object()
            .ok_or(VMError::TypeError)?;
        // 9.4.4 answers the Prototype of the running function.
        let parent = heap
            .get_object(active)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .prototype;
        if !Self::constructs(parent, heap) {
            return Err(type_error(heap, realm, "super is not a constructor"));
        }
        // 15.7.14 step 10: the default constructor passes the arguments its
        // own call was given, which are registers of the caller and not of
        // this frame, so they travel as the List of 7.3.15.
        let (arg_start, arg_count, spread) = if forwarded {
            let passed = self.passed_arguments()?;
            let count = u16::try_from(passed.len()).map_err(|_| VMError::TypeError)?;
            let list = Self::array_of(passed, heap, realm)?;
            heap.enter_scope();
            let list = heap.push_root(list)?;
            (Reg(0), count, Some(Resume::Spread { arguments: list }))
        } else {
            (arg_start, arg_count, None)
        };
        // 10.2.2 creates the object from the `[[NewTarget]]`, and a derived
        // parent creates none of its own until its own super call.
        let receiver = if Self::derives(parent, units, heap) {
            VALUE_UNINITIALIZED
        } else {
            let object = self.create_from_new_target(heap, realm, new_target_register)?;
            Value::from_object(object)
        };
        self.write_reg(this_register, receiver)?;
        self.pending_new_target = self.read_reg(new_target_register)?;
        let parent = self.read_reg(self_register)?;
        let parent = heap
            .get_object(parent.as_object().ok_or(VMError::TypeError)?)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .prototype;
        let call = Call {
            receiver,
            func: self_register,
            arg_start,
            arg_count,
            slot,
            resume: spread,
            construct: Some(this_register),
            return_pc,
            caller_code_id,
        };
        // 10.1.13: every constructor of this Realm written in Rust makes its
        // object with `OrdinaryCreateFromConstructor`, so the object it
        // answers takes the Prototype the `newTarget` names.
        if let Some(intrinsic) = Self::native_constructor_of(parent, heap) {
            self.acc = self.call_intrinsic(intrinsic, call, units, heap, realm)?;
            if spread.is_some() {
                heap.exit_scope();
            }
            let prototype = self.prototype_of_new_target(new_target_register, heap)?;
            if let Some(object) = self.acc.as_object()
                && let Some(prototype) = prototype
            {
                heap.set_object_prototype(object, prototype)
                    .map_err(VMError::Heap)?;
            }
            self.write_reg(this_register, self.acc)?;
            return Ok(None);
        }
        if !Self::is_script_function(parent, heap) {
            if spread.is_some() {
                heap.exit_scope();
            }
            return Err(VMError::Unsupported(
                "a super call of a constructor written in Rust",
            ));
        }
        self.enter_call_value(parent, units, active_feedback, heap, realm, call)
    }

    /// The arguments the running call was given, which 10.4.4 and 15.7.14 read
    /// out of the registers of the caller.
    fn passed_arguments(&self) -> Result<Vec<Value>, VMError> {
        let frame = *self.frames.last().ok_or(VMError::InvalidRegister)?;
        let arguments = frame.arguments.ok_or(VMError::Unsupported(
            "the arguments of a call the engine did not open",
        ))?;
        let mut passed = Vec::with_capacity(usize::from(arguments.count));
        for index in 0..arguments.count {
            let slot = frame
                .caller_fp
                .checked_add(arguments.start.0 as usize)
                .and_then(|start| start.checked_add(index as usize))
                .ok_or(VMError::InvalidRegister)?;
            passed.push(
                self.stack
                    .get(slot)
                    .copied()
                    .ok_or(VMError::InvalidRegister)?,
            );
        }
        Ok(passed)
    }

    /// 10.2.2 step 13 for a derived constructor: the value the body answered
    /// when it is an Object, and the `this` binding of 13.3.7.1 otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` for a value that is
    /// neither an Object nor undefined, and with a `ReferenceError` for a
    /// binding no super call made.
    fn derived_result(
        &self,
        code: &BytecodeFunction,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if self.acc.as_object().is_some() {
            return Ok(self.acc);
        }
        if !self.acc.is_undefined() {
            return Err(type_error(
                heap,
                realm,
                "a derived constructor answered a value that is not an object",
            ));
        }
        let register = code
            .this_register
            .ok_or(VMError::Unsupported("a derived constructor with no this"))?;
        let value = self.read_reg(register)?;
        if value == VALUE_UNINITIALIZED {
            return Err(raise(
                heap,
                realm,
                super::realm::NativeErrorKind::ReferenceError,
                "this is not initialized",
            ));
        }
        Ok(value)
    }

    /// The intrinsic of a constructor of this Realm written in Rust.
    fn native_constructor_of(value: Value, heap: &GenerationalHeap) -> Option<Intrinsic> {
        let object = value.as_object()?;
        let ObjectKind::NativeFunction { id, .. } = heap.get_object(object)?.kind else {
            return None;
        };
        Intrinsic::from_id(id).filter(|intrinsic| intrinsic.constructs_an_object())
    }

    /// The `prototype` 10.1.13 reads off the `newTarget` of the call.
    fn prototype_of_new_target(
        &self,
        new_target: Reg,
        heap: &mut GenerationalHeap,
    ) -> Result<Option<Value>, VMError> {
        let Some(function) = self.read_reg(new_target)?.as_object() else {
            return Ok(None);
        };
        let name = PropertyKey::String(heap.strings.intern("prototype")?);
        let Some(found) = heap.lookup_named(function, name)? else {
            return Ok(None);
        };
        let prototype = Self::plain_value(found)?;
        Ok(prototype.as_object().map(|_| prototype))
    }

    /// Whether the function is the constructor of a class with a heritage.
    fn derives(value: Value, units: CodeUnits<'_>, heap: &GenerationalHeap) -> bool {
        let Some(object) = value.as_object() else {
            return false;
        };
        let Some(ObjectKind::Function { unit, code_id, .. }) =
            heap.get_object(object).map(|object| object.kind.clone())
        else {
            return false;
        };
        units
            .table
            .root(unit)
            .and_then(|root| code_unit(root, Some(code_id)))
            .is_some_and(|code| code.derived)
    }

    /// `MakeMethod` of 10.2.11: 13.2.5.5 and 15.7.14 give the function the
    /// object it is defined on as its `[[HomeObject]]`.
    fn make_method(
        method: Value,
        target: ObjectRef,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let Some(function) = method.as_object() else {
            return Ok(());
        };
        if !matches!(
            heap.get_object(function).map(|object| &object.kind),
            Some(ObjectKind::Function { .. })
        ) {
            return Ok(());
        }
        heap.set_function_home(function, Value::from_object(target))?;
        Ok(())
    }

    /// One half of an accessor property of 13.2.5.1.
    ///
    /// The other half is whatever the property already holds, so the two
    /// clauses of one name meet on the object.
    fn define_accessor(
        &self,
        obj: Reg,
        name: PropertyKey,
        setter: bool,
        enumerable: bool,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let oref = self.read_reg(obj)?.as_object().ok_or(VMError::TypeError)?;
        Self::make_method(self.acc, oref, heap)?;
        let (get, set) = match heap.own_named_flags(oref, name)? {
            Some(flags) if flags.is_accessor => {
                let held = heap
                    .lookup_named(oref, name)?
                    .map_or(VALUE_UNDEFINED, |property| property.value);
                Self::accessor_parts(held, heap)?
            }
            _ => (VALUE_UNDEFINED, VALUE_UNDEFINED),
        };
        let function = self.acc;
        let (get, set) = if setter {
            (get, function)
        } else {
            (function, set)
        };
        let pair = Self::make_accessor(get, set, heap)?;
        // The pair was allocated, which may have moved the object.
        let oref = self.read_reg(obj)?.as_object().ok_or(VMError::TypeError)?;
        heap.define_own_named(
            oref,
            name,
            pair,
            PropertyFlags {
                writable: false,
                enumerable,
                configurable: true,
                is_accessor: true,
            },
        )?;
        Ok(())
    }

    /// `SetFunctionName` of 10.2.10 for a function the lowering could not name,
    /// because its key is only known at run time.
    ///
    /// 10.2.10 step 2 writes `[description]` for a Symbol key and nothing at
    /// all for one whose description is undefined, and step 4 prefixes `get `
    /// or `set ` for the two halves of an accessor.
    fn name_from_key(
        &self,
        name: PropertyKey,
        accessor: Option<bool>,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let mut units: Vec<u16> = match accessor {
            Some(true) => "set ".encode_utf16().collect(),
            Some(false) => "get ".encode_utf16().collect(),
            None => Vec::new(),
        };
        match name {
            PropertyKey::String(text) => {
                units.extend(
                    heap.strings
                        .to_utf16(Value::from_string(text))
                        .ok_or(VMError::Heap(HeapError::InvalidReference))?,
                );
            }
            PropertyKey::Symbol(symbol) => {
                if let Some(description) = Self::symbol_description(symbol, heap) {
                    units.push(0x5B);
                    units.extend_from_slice(&description);
                    units.push(0x5D);
                }
            }
        }
        // The text is made first, because the accumulator holds the function
        // and the collector only follows what it can see.
        let text = self.allocate_string(heap, &units)?;
        let function = self.acc.as_object().ok_or(VMError::TypeError)?;
        let key = PropertyKey::String(heap.strings.intern("name")?);
        heap.define_own_named(
            function,
            key,
            text,
            PropertyFlags {
                writable: false,
                enumerable: false,
                configurable: true,
                is_accessor: false,
            },
        )?;
        Ok(())
    }

    /// `SetFunctionLength` of 10.2.9: not writable, not enumerable and
    /// configurable.
    /// `SetFunctionName` of 10.2.10, which 8.5.2 gives a name a function or a
    /// class does not carry itself.
    ///
    /// The accumulator holds the function, because defining the name may
    /// allocate and the collector only follows what it can see.
    fn set_function_name(
        &self,
        code: &BytecodeFunction,
        code_id: u32,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let target = code
            .functions
            .get(code_id as usize)
            .ok_or(VMError::InvalidRegister)?;
        // 10.2.5 step 8 gives every function object a `name`; one the lowering
        // could not name is the empty String of 15.2.5 and 15.3.4, not a
        // missing property.
        let units = match target.name {
            Some(index) => target
                .string_constants
                .get(index as usize)
                .ok_or(VMError::InvalidRegister)?
                .clone(),
            None => Vec::new(),
        };
        let text = self.allocate_string(heap, &units)?;
        let function = self.acc.as_object().ok_or(VMError::TypeError)?;
        let key = PropertyKey::String(heap.strings.intern("name")?);
        heap.define_own_named(
            function,
            key,
            text,
            PropertyFlags {
                writable: false,
                enumerable: false,
                configurable: true,
                is_accessor: false,
            },
        )?;
        Ok(())
    }

    fn set_function_length(
        function: ObjectRef,
        parameters: u16,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let key = PropertyKey::String(heap.strings.intern_units(&LENGTH_NAME)?);
        heap.define_own_named(
            function,
            key,
            Value::from_smi(i32::from(parameters)),
            PropertyFlags {
                writable: false,
                enumerable: false,
                configurable: true,
                is_accessor: false,
            },
        )?;
        Ok(())
    }

    /// Whether this method of 23.1.3 asks the Script about each element, and
    /// so cannot answer without leaving the engine first.
    /// Whether the clause compares each element itself rather than asking the
    /// Script about it (23.1.3.16, 23.1.3.17 and 23.1.3.20).
    const fn scans_for_an_element(intrinsic: Intrinsic) -> bool {
        matches!(
            intrinsic,
            Intrinsic::ArrayPrototypeIncludes
                | Intrinsic::ArrayPrototypeIndexOf
                | Intrinsic::ArrayPrototypeLastIndexOf
        )
    }

    const fn iterates_with_callback(intrinsic: Intrinsic) -> bool {
        Self::scans_for_an_element(intrinsic)
            || matches!(
                intrinsic,
                Intrinsic::ArrayPrototypeForEach
                    | Intrinsic::ArrayPrototypeMap
                    | Intrinsic::ArrayPrototypeFilter
                    | Intrinsic::ArrayPrototypeEvery
                    | Intrinsic::ArrayPrototypeSome
                    | Intrinsic::ArrayPrototypeFind
                    | Intrinsic::ArrayPrototypeFindIndex
                    | Intrinsic::ArrayPrototypeFindLast
                    | Intrinsic::ArrayPrototypeFindLastIndex
                    | Intrinsic::ArrayPrototypeReduce
                    | Intrinsic::ArrayPrototypeReduceRight
                    | Intrinsic::ArrayFrom
            )
    }

    /// Whether the clause reads every index from zero to the length, which
    /// 23.1.3.9, 23.1.3.10, 23.1.3.12 and 23.1.3.13 do with 7.3.2 and every
    /// other clause of 23.1.3 does only where the object has the index.
    const fn visits_holes(intrinsic: Intrinsic) -> bool {
        matches!(
            intrinsic,
            Intrinsic::ArrayPrototypeFind
                | Intrinsic::ArrayPrototypeFindIndex
                | Intrinsic::ArrayPrototypeFindLast
                | Intrinsic::ArrayPrototypeFindLastIndex
                // 23.1.2.1 step 5.e reads every index with 7.3.2 too.
                | Intrinsic::ArrayFrom
                // 23.1.3.16 reads a missing index as undefined, which
                // 23.1.3.17 and 23.1.3.20 pass over.
                | Intrinsic::ArrayPrototypeIncludes
        )
    }

    /// The three arguments 23.1.3 gives a callback: the element, its index and
    /// the object being walked.
    fn iteration_arguments(state: Root, heap: &GenerationalHeap) -> Result<[Value; 4], VMError> {
        let reference = heap
            .root_value(state)
            .and_then(Value::as_object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        let ObjectKind::ArrayIteration {
            target,
            element,
            element_index,
            output,
            intrinsic,
            ..
        } = heap
            .get_object(reference)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .kind
        else {
            return Err(VMError::Heap(HeapError::InvalidReference));
        };
        let index = index_value(i64::from(element_index));
        // Note 1 of 23.1.3.24 gives its callback four: the value the last call
        // answered, the element, its index and the object. Every other clause
        // gives three and leaves the fourth unread.
        if matches!(
            Intrinsic::from_id(intrinsic),
            Some(Intrinsic::ArrayPrototypeReduce | Intrinsic::ArrayPrototypeReduceRight)
        ) {
            return Ok([output, element, index, target]);
        }
        // 23.1.2.1 step 5.f.iii passes the element and its index alone.
        if Intrinsic::from_id(intrinsic) == Some(Intrinsic::ArrayFrom) {
            return Ok([element, index, VALUE_UNDEFINED, VALUE_UNDEFINED]);
        }
        Ok([element, index, target, VALUE_UNDEFINED])
    }

    /// Charges the fuel a walk of this many indices costs.
    ///
    /// An index costs a lookup and no more, so a batch of them costs one unit;
    /// what this bounds is the walk of a `length` no object could fill.
    fn charge_for_scan(&mut self, indices: i64) -> Result<(), VMError> {
        let indices = u64::try_from(indices.max(0)).unwrap_or(u64::MAX);
        let units = indices
            .checked_div(u64::from(HOLES_PER_FUEL_UNIT))
            .unwrap_or(u64::MAX);
        self.fuel = self.fuel.checked_sub(units).ok_or(VMError::OutOfFuel)?;
        Ok(())
    }

    /// Charges for one index of a scan that stops where it finds what it
    /// looks for, so that only what it looked at is paid for.
    fn charge_for_one_of_a_scan(&mut self, index: u32) -> Result<(), VMError> {
        if index.is_multiple_of(HOLES_PER_FUEL_UNIT) {
            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
        }
        Ok(())
    }

    /// The methods of 23.1.3 that move elements of the receiver, or copy it
    /// into an Array of their own.
    ///
    /// None of them calls back into the Script, so each one runs to its end
    /// here. Every one reads through `element_at`, which answers an index the
    /// Elements store holds and the Prototype Chain for the rest, so a sparse
    /// Array keeps its holes.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a receiver that is not an Object and
    /// for the `RangeError` 23.1.3.39 raises, and a heap error when an index
    /// name cannot be interned.
    #[expect(
        clippy::too_many_lines,
        reason = "one function keeps each method beside the clause it implements"
    )]
    fn call_array_edit_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        call: Call,
        known: Option<i64>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let object = Self::coerce_object(call.receiver, heap, realm)?;
        // 7.3.18 read it once already where a getter answered it.
        let length = match known {
            Some(length) => length,
            None => Self::array_like_length(heap, object, realm)?,
        };
        // Each of these walks the whole `length`, which a Script can make
        // 2^32-1 without holding one element. The walk is charged before it
        // starts, so a budget that cannot pay for it ends here rather than
        // after four billion lookups.
        self.charge_for_scan(length)?;
        // The range 7.1.25 makes of a relative index, clamped into the Array.
        let bounded = |value: Value, fallback: i64, heap: &mut GenerationalHeap| {
            if value.is_undefined() {
                return Ok(fallback);
            }
            Ok::<i64, VMError>(
                absolute_index(integer_argument(value, heap, realm)?, length).clamp(0, length),
            )
        };
        match intrinsic {
            // 23.1.3.27: the first element leaves, the rest move down by one,
            // and the Array is one shorter.
            Intrinsic::ArrayPrototypeShift => {
                if length <= 0 {
                    Self::set_array_like_length(object, 0, heap, realm)?;
                    return Ok(VALUE_UNDEFINED);
                }
                let first = Self::element_at(heap, object, 0)?.unwrap_or(VALUE_UNDEFINED);
                let elements = Self::array_elements(heap, object)?;
                for index in 1..Self::scan_range(0, length).end {
                    let moved = Self::element_at(heap, object, index)?;
                    Self::place_element(
                        heap,
                        object,
                        elements,
                        index.saturating_sub(1),
                        moved,
                        realm,
                    )?;
                }
                let last = u32::try_from(length.saturating_sub(1)).unwrap_or(u32::MAX);
                heap.delete_element(elements, last)?;
                Self::set_array_like_length(object, last, heap, realm)?;
                Ok(first)
            }
            // 23.1.3.37: the arguments go in front, so every element moves up
            // by as many as there are.
            Intrinsic::ArrayPrototypeUnshift => {
                let count = i64::from(call.arg_count);
                if count > 0 {
                    let elements = Self::array_elements(heap, object)?;
                    for index in Self::scan_range(0, length).rev() {
                        let moved = Self::element_at(heap, object, index)?;
                        let target = u32::try_from(i64::from(index).saturating_add(count))
                            .map_err(|_| VMError::PropertyLimit)?;
                        Self::place_element(heap, object, elements, target, moved, realm)?;
                    }
                    for offset in 0..call.arg_count {
                        Self::set_element(
                            object,
                            u32::from(offset),
                            self.call_argument(&call, offset, heap)?,
                            heap,
                            realm,
                        )?;
                    }
                }
                // Step 4.e sets the length with 7.3.4, which throws where
                // 10.4.2.4 refuses it.
                let written = u32::try_from(length.saturating_add(count))
                    .map_err(|_| VMError::PropertyLimit)?;
                Self::set_array_like_length(object, written, heap, realm)?;
                Ok(index_value(length.saturating_add(count)))
            }
            // 23.1.3.31: the removed elements answer as an Array of their own,
            // and the tail closes the distance the arguments leave.
            Intrinsic::ArrayPrototypeSplice => {
                let start = bounded(self.call_argument(&call, 0, heap)?, 0, heap)?;
                let removed = if call.arg_count == 0 {
                    0
                } else if call.arg_count == 1 {
                    length.saturating_sub(start)
                } else {
                    integer_argument(self.call_argument(&call, 1, heap)?, heap, realm)?
                        .clamp(0, length.saturating_sub(start))
                };
                let inserted = i64::from(call.arg_count.saturating_sub(2));
                // 23.1.3.31 step 8 makes the answer with 23.1.3.4.
                let result = Self::array_species_create(
                    object,
                    u32::try_from(removed).map_err(|_| VMError::PropertyLimit)?,
                    heap,
                    realm,
                )?;
                let mut target = 0u32;
                for index in Self::scan_range(start, start.saturating_add(removed)) {
                    if let Some(value) = Self::element_at(heap, object, index)? {
                        heap.set_array_element(result, target, value)?;
                    }
                    target = target.saturating_add(1);
                }
                Self::splice_tail(object, start, removed, inserted, length, heap, realm)?;
                for offset in 2..call.arg_count {
                    let index = start.saturating_add(i64::from(offset.saturating_sub(2)));
                    let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
                    Self::set_element(
                        object,
                        index,
                        self.call_argument(&call, offset, heap)?,
                        heap,
                        realm,
                    )?;
                }
                let final_length = length.saturating_sub(removed).saturating_add(inserted);
                Self::set_array_like_length(
                    object,
                    u32::try_from(final_length).map_err(|_| VMError::PropertyLimit)?,
                    heap,
                    realm,
                )?;
                Ok(Value::from_object(result))
            }
            // 23.1.3.7: one value fills the range, and answers the receiver.
            Intrinsic::ArrayPrototypeFill => {
                let value = self.call_argument(&call, 0, heap)?;
                let start = bounded(self.call_argument(&call, 1, heap)?, 0, heap)?;
                let end = bounded(self.call_argument(&call, 2, heap)?, length, heap)?;
                for index in Self::scan_range(start, end) {
                    Self::set_element(object, index, value, heap, realm)?;
                }
                Ok(call.receiver)
            }
            // 23.1.3.4: a range of the Array is copied over another range of
            // it, and the length does not change.
            Intrinsic::ArrayPrototypeCopyWithin => {
                let target = bounded(self.call_argument(&call, 0, heap)?, 0, heap)?;
                let start = bounded(self.call_argument(&call, 1, heap)?, 0, heap)?;
                let end = bounded(self.call_argument(&call, 2, heap)?, length, heap)?;
                let count = end
                    .saturating_sub(start)
                    .min(length.saturating_sub(target))
                    .max(0);
                let elements = Self::array_elements(heap, object)?;
                // The ranges may overlap, so the copy reads every element
                // before it writes any of them.
                let mut taken = Vec::new();
                for index in Self::scan_range(start, start.saturating_add(count)) {
                    taken.push(Self::element_at(heap, object, index)?);
                }
                for (offset, value) in taken.into_iter().enumerate() {
                    let index = target.saturating_add(i64::try_from(offset).unwrap_or(i64::MAX));
                    let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
                    Self::place_element(heap, object, elements, index, value, realm)?;
                }
                Ok(call.receiver)
            }
            // 23.1.3.2: the receiver and then each argument, an Array of them
            // one level flat. 23.1.3.2.1 asks @@isConcatSpreadable first, and
            // this Realm builds no Symbol, so IsArray decides alone.
            Intrinsic::ArrayPrototypeConcat => {
                let mut values = Vec::new();
                // Step 3 makes the receiver the first item, which 23.1.3.2.1
                // spreads only when it is an Array; every other object is one
                // element of the answer.
                if Self::is_array(call.receiver, heap) {
                    Self::spread_into(&mut values, object, length, heap)?;
                } else {
                    values.push(Some(Value::from_object(object)));
                }
                for offset in 0..call.arg_count {
                    let item = self.call_argument(&call, offset, heap)?;
                    match item.as_object().filter(|_| Self::is_array(item, heap)) {
                        Some(part) => {
                            let part_length = Self::array_like_length(heap, part, realm)?;
                            Self::spread_into(&mut values, part, part_length, heap)?;
                        }
                        None => values.push(Some(item)),
                    }
                }
                // 23.1.3.1 step 2 makes the answer with 23.1.3.4.
                Self::array_species_of_holes(object, values, heap, realm)
            }
            // 23.1.3.14: every index of an Array element becomes an index of
            // the answer, as deep as the depth allows.
            Intrinsic::ArrayPrototypeFlat => {
                let depth = if call.arg_count == 0 {
                    1
                } else {
                    integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?
                };
                let values = self.flatten(object, length, depth, heap, realm)?;
                // 23.1.3.14 step 4 makes the answer with 23.1.3.4.
                Self::array_species_of_holes(object, values, heap, realm)
            }
            // 23.1.3.35: a copy with one range replaced, which reads every
            // index it passes and so answers no hole.
            Intrinsic::ArrayPrototypeToSpliced => {
                let start = bounded(self.call_argument(&call, 0, heap)?, 0, heap)?;
                let skipped = match call.arg_count {
                    0 => 0,
                    1 => length.saturating_sub(start),
                    _ => integer_argument(self.call_argument(&call, 1, heap)?, heap, realm)?
                        .clamp(0, length.saturating_sub(start)),
                };
                let inserted = i64::from(call.arg_count).saturating_sub(2).max(0);
                let mut values = Vec::new();
                for position in Self::scan_range(0, start) {
                    values.push(Some(
                        Self::element_at(heap, object, position)?.unwrap_or(VALUE_UNDEFINED),
                    ));
                }
                for offset in 0..inserted {
                    let index = u16::try_from(offset.saturating_add(2)).unwrap_or(u16::MAX);
                    values.push(Some(self.call_argument(&call, index, heap)?));
                }
                for position in Self::scan_range(start.saturating_add(skipped), length) {
                    values.push(Some(
                        Self::element_at(heap, object, position)?.unwrap_or(VALUE_UNDEFINED),
                    ));
                }
                Self::array_from_holes(values, heap, realm)
            }
            // 23.1.3.30 and 23.1.3.34: the elements in the order 23.1.3.30.1
            // compares them, written back over the receiver or into a copy.
            Intrinsic::ArrayPrototypeSort | Intrinsic::ArrayPrototypeToSorted => {
                let comparator = self.call_argument(&call, 0, heap)?;
                if !comparator.is_undefined() {
                    if !Self::is_callable(comparator, heap) {
                        return Err(type_error(heap, realm, "comparator is not callable"));
                    }
                    return Err(VMError::Unsupported("a sort with a comparator"));
                }
                let sorts_in_place = intrinsic == Intrinsic::ArrayPrototypeSort;
                // 23.1.3.30 step 5 passes over a hole; 23.1.3.34 step 5 reads
                // it as undefined and keeps the length.
                let mut sorted = Vec::new();
                for position in Self::scan_range(0, length) {
                    self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                    let found = Self::element_at(heap, object, position)?;
                    match found {
                        Some(value) => sorted.push(value),
                        None if !sorts_in_place => sorted.push(VALUE_UNDEFINED),
                        None => {}
                    }
                }
                // 23.1.3.30.1 puts undefined last and orders the rest by the
                // code units of their `ToString`, which a key holds so the
                // comparison itself allocates nothing.
                let mut keyed = Vec::with_capacity(sorted.len());
                for value in sorted {
                    keyed.push((Self::sort_key(value, heap, realm)?, value));
                }
                self.charge_for_scan(length)?;
                keyed.sort_by(|left, right| left.0.cmp(&right.0));
                let values: Vec<Option<Value>> =
                    keyed.into_iter().map(|(_, value)| Some(value)).collect();
                if !sorts_in_place {
                    return Self::array_from_holes(values, heap, realm);
                }
                let elements = Self::array_elements(heap, object)?;
                let count = i64::try_from(values.len()).unwrap_or(i64::MAX);
                for (offset, value) in values.into_iter().enumerate() {
                    let index = u32::try_from(offset).map_err(|_| VMError::PropertyLimit)?;
                    Self::place_element(heap, object, elements, index, value, realm)?;
                }
                // Step 8 deletes the indices the holes left behind.
                for position in Self::scan_range(count, length) {
                    heap.delete_element(elements, position)?;
                }
                Ok(call.receiver)
            }
            // 23.1.3.39: a copy with one index replaced, which 23.1.3.39 step
            // 5 refuses for an index outside the Array.
            Intrinsic::ArrayPrototypeWith => {
                let relative = integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?;
                let index = absolute_index(relative, length);
                if index < 0 || index >= length {
                    return Err(raise(
                        heap,
                        realm,
                        super::realm::NativeErrorKind::RangeError,
                        "index is outside the Array",
                    ));
                }
                let replacement = self.call_argument(&call, 1, heap)?;
                let mut values = Vec::new();
                for position in Self::scan_range(0, length) {
                    let value = if i64::from(position) == index {
                        Some(replacement)
                    } else {
                        // 23.1.3.39 reads every index, so a hole of the source
                        // is undefined in the copy rather than a hole.
                        Some(Self::element_at(heap, object, position)?.unwrap_or(VALUE_UNDEFINED))
                    };
                    values.push(value);
                }
                Self::array_from_holes(values, heap, realm)
            }
            // 23.1.3.33: a copy in the other order, which reads every index.
            _ => {
                let mut values = Vec::new();
                for position in Self::scan_range(0, length).rev() {
                    values.push(Some(
                        Self::element_at(heap, object, position)?.unwrap_or(VALUE_UNDEFINED),
                    ));
                }
                Self::array_from_holes(values, heap, realm)
            }
        }
    }

    /// `Set(O, ! ToString(𝔽(index)), value, true)` of 7.3.4 for an index.
    ///
    /// This engine keeps indexed elements in the store 10.4.2 gives an Array,
    /// so an array-like without one has nowhere to put them, which it says
    /// rather than reporting a broken frame.
    fn set_element(
        object: ObjectRef,
        index: u32,
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        if heap.array_length(object).is_none() {
            return Err(VMError::Unsupported(
                "an indexed write to a receiver that is not an Array",
            ));
        }
        // 10.4.2.1 step 3.g: an index at or above the length needs the length
        // to grow, which 10.4.2.4 refuses once 7.3.15 made it unwritable.
        if !heap.array_length_is_writable(object).unwrap_or(true)
            && index >= heap.array_length(object).unwrap_or(0)
        {
            return Err(type_error(
                heap,
                realm,
                "cannot write a property that is not writable",
            ));
        }
        // 7.3.4 writes with `Throw` true, so a write 10.1.9.2 refuses raises a
        // TypeError. An Array whose Shape carries no name of its own and that
        // is extensible refuses none, and 7.3.15 is what puts an index there.
        let shape = heap
            .get_object(object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .shape_id;
        if shape != heap.shapes.root_shape() || !heap.is_extensible(object).unwrap_or(true) {
            let name = alloc::format!("{index}");
            let key = PropertyKey::String(
                heap.strings
                    .intern_units(&name.encode_utf16().collect::<Vec<u16>>())?,
            );
            match heap.own_named_flags(object, key)? {
                Some(flags) if flags.is_accessor || !flags.writable => {
                    return Err(type_error(
                        heap,
                        realm,
                        "cannot write a property that is not writable",
                    ));
                }
                // The index is a property of the Shape, which 7.3.15 moved it
                // to, so the write belongs there and not in the store.
                Some(_) => {
                    heap.define_own_named(object, key, value, PropertyFlags::ordinary_data())?;
                    return Ok(());
                }
                None => {
                    if !heap.is_extensible(object).unwrap_or(true) {
                        return Err(type_error(
                            heap,
                            realm,
                            "cannot add a property to an object that is not extensible",
                        ));
                    }
                }
            }
        }
        heap.set_array_element(object, index, value)?;
        Ok(())
    }

    /// `Set(O, "length", 𝔽(length), true)` of 7.3.4.
    ///
    /// An Array keeps its length where 10.4.2 puts it; any other array-like
    /// keeps it as an ordinary property, which is where 23.1.3 writes it.
    /// `ArraySetLength` of 10.4.2.4: the new length is `ToUint32` of the
    /// value, which has to be the number `ToNumber` gives, and every index at
    /// or above it is deleted.
    fn set_array_length(
        object: ObjectRef,
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let wanted = Self::array_length_of(value, heap, realm)?;
        // 10.4.2.4 step 2: a `length` that is no longer writable takes no
        // value at all, which 10.1.9.1 answers false for.
        if heap.array_length_is_writable(object) != Some(true) {
            return Ok(());
        }
        let current = heap.array_length(object).ok_or(VMError::TypeError)?;
        if wanted < current {
            Self::shorten_array(object, wanted, heap)?;
        }
        heap.set_array_length(object, wanted)?;
        Ok(())
    }

    fn set_array_like_length(
        object: ObjectRef,
        length: u32,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        if heap.array_length(object).is_some() {
            // 7.3.4 writes with `Throw` true, and 10.4.2.4 refuses a `length`
            // 7.3.15 made unwritable.
            if !heap.array_length_is_writable(object).unwrap_or(true) {
                return Err(type_error(
                    heap,
                    realm,
                    "cannot write a property that is not writable",
                ));
            }
            heap.set_array_length(object, length)?;
            return Ok(());
        }
        let name = PropertyKey::String(heap.strings.intern_units(&LENGTH_NAME)?);
        let length = i32::try_from(length).map_err(|_| VMError::PropertyLimit)?;
        heap.define_own_named(
            object,
            name,
            Value::from_smi(length),
            PropertyFlags::ordinary_data(),
        )?;
        Ok(())
    }

    /// Moves the tail of 23.1.3.31 over the elements it removed, in the
    /// direction that does not overwrite what it has still to read.
    fn splice_tail(
        object: ObjectRef,
        start: i64,
        removed: i64,
        inserted: i64,
        length: i64,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        if inserted == removed {
            return Ok(());
        }
        let elements = Self::array_elements(heap, object)?;
        let shift = |index: u32, heap: &mut GenerationalHeap| -> Result<(), VMError> {
            let from = i64::from(index);
            let moved = Self::element_at(heap, object, index)?;
            let to = from.saturating_sub(removed).saturating_add(inserted);
            let to = u32::try_from(to).map_err(|_| VMError::PropertyLimit)?;
            Self::place_element(heap, object, elements, to, moved, realm)
        };
        let tail = Self::scan_range(start.saturating_add(removed), length);
        if inserted < removed {
            for index in tail {
                shift(index, heap)?;
            }
            // The Array is shorter, so the indices past its new end are gone.
            for index in Self::scan_range(
                length.saturating_sub(removed).saturating_add(inserted),
                length,
            ) {
                heap.delete_element(elements, index)?;
            }
        } else {
            for index in tail.rev() {
                shift(index, heap)?;
            }
        }
        Ok(())
    }

    /// `FlattenIntoArray` of 23.1.3.14.1 without a mapper.
    ///
    /// The nesting is walked with a stack of its own rather than by recursion,
    /// so a deeply nested Array reaches a named limit instead of the Rust
    /// stack.
    fn flatten(
        &mut self,
        object: ObjectRef,
        length: i64,
        depth: i64,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Vec<Option<Value>>, VMError> {
        /// How deep 23.1.3.14 follows an Array element before it names a gap.
        const NESTING_LIMIT: usize = 64;
        let mut values = Vec::new();
        let mut open = alloc::vec![(object, 0i64, length, depth)];
        while let Some((source, index, end, left)) = open.pop() {
            if index >= end {
                continue;
            }
            open.push((source, index.saturating_add(1), end, left));
            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
            let position = u32::try_from(index).unwrap_or(u32::MAX);
            let Some(element) = Self::element_at(heap, source, position)? else {
                continue;
            };
            // Step 5.c.iv: only an Array is flattened, and only while the
            // depth allows it.
            if left > 0 && Self::is_array(element, heap) {
                let part = element.as_object().ok_or(VMError::TypeError)?;
                let part_length = Self::array_like_length(heap, part, realm)?;
                self.charge_for_scan(part_length)?;
                if open.len() >= NESTING_LIMIT {
                    return Err(VMError::Unsupported(
                        "a flat of more nesting than the engine walks",
                    ));
                }
                open.push((part, 0, part_length, left.saturating_sub(1)));
                continue;
            }
            values.push(Some(element));
        }
        Ok(values)
    }

    /// The key 23.1.3.30.1 compares two elements by when it was given no
    /// comparator: undefined after everything, and every other value by the
    /// code units of its `ToString`.
    fn sort_key(
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(bool, Vec<u16>), VMError> {
        if value.is_undefined() {
            return Ok((true, Vec::new()));
        }
        if value.is_string() {
            let units = heap
                .strings
                .to_utf16(value)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            return Ok((false, units));
        }
        if value.is_object() {
            return Err(VMError::Unsupported("ToString of an Object"));
        }
        if value.is_symbol() {
            return Err(type_error(heap, realm, "cannot convert Symbol operand"));
        }
        let text = if let Some(number) = value.as_f64() {
            crate::number::decimal_string(number)
        } else if let Some(boolean) = value.as_boolean() {
            alloc::string::String::from(if boolean { "true" } else { "false" })
        } else if value.is_null() {
            alloc::string::String::from("null")
        } else {
            return Err(VMError::TypeError);
        };
        Ok((false, text.encode_utf16().collect()))
    }

    /// Appends every index of an array-like, a hole as a hole.
    fn spread_into(
        values: &mut Vec<Option<Value>>,
        object: ObjectRef,
        length: i64,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        for index in Self::scan_range(0, length) {
            values.push(Self::element_at(heap, object, index)?);
        }
        Ok(())
    }

    /// The Array 23.1.3.4 makes for this receiver, holding these values.
    fn array_species_of_holes(
        original: ObjectRef,
        values: Vec<Option<Value>>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let count = u32::try_from(values.len()).map_err(|_| VMError::PropertyLimit)?;
        let array = Self::array_species_create(original, count, heap, realm)?;
        for (index, value) in values.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
            if let Some(value) = value {
                heap.set_array_element(array, index, value)?;
            }
        }
        Ok(Value::from_object(array))
    }

    /// `ArraySpeciesCreate` of 23.1.3.4.
    ///
    /// The Realm has one `%Array%`, so step 3 never sets `C` aside; a
    /// `constructor` or an `@@species` of the Script decides the answer and
    /// needs a frame, which the clause names rather than making an Array of
    /// its own.
    fn array_species_create(
        original: ObjectRef,
        length: u32,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<ObjectRef, VMError> {
        // Step 1: only an Array asks its constructor at all.
        if !Self::is_array(Value::from_object(original), heap) {
            return Ok(realm.array(heap, length)?);
        }
        let key = PropertyKey::String(heap.strings.intern("constructor")?);
        let found = heap.lookup_named(original, key)?;
        let constructor = match found {
            None => VALUE_UNDEFINED,
            Some(property) if property.flags.is_accessor => {
                return Err(VMError::Unsupported("a constructor that is an accessor"));
            }
            Some(property) => property.value,
        };
        if constructor.is_undefined() {
            return Ok(realm.array(heap, length)?);
        }
        // Step 4 asks an Object for its `@@species`; anything else reaches
        // step 6 as a value that is no constructor.
        let Some(object) = constructor.as_object() else {
            return Err(type_error(
                heap,
                realm,
                "the constructor is not a constructor",
            ));
        };
        let species = heap.lookup_named(object, super::realm::WellKnownSymbol::Species.key())?;
        let species = match species {
            None => VALUE_UNDEFINED,
            Some(property) if property.flags.is_accessor => {
                let (get, _) = Self::accessor_parts(property.value, heap)?;
                let native = get
                    .as_object()
                    .and_then(|get| heap.get_object(get))
                    .and_then(|get| match get.kind {
                        ObjectKind::NativeFunction { id, .. } => Intrinsic::from_id(id),
                        _ => None,
                    });
                if native != Some(Intrinsic::SpeciesGetter) {
                    return Err(VMError::Unsupported("an @@species of the Script"));
                }
                // 23.1.2.5 answers the constructor it was read off.
                constructor
            }
            Some(property) => property.value,
        };
        if species.is_undefined() || species.is_null() {
            return Ok(realm.array(heap, length)?);
        }
        if !Self::constructs(species, heap) {
            return Err(type_error(heap, realm, "the species is not a constructor"));
        }
        // Step 7 constructs it, which for `%Array%` is the Array 23.1.1.1
        // makes and for anything else a frame of the Script.
        if species.as_object()
            == realm
                .intrinsic(heap, Intrinsic::ArrayConstructor)?
                .as_object()
        {
            return Ok(realm.array(heap, length)?);
        }
        Err(VMError::Unsupported("a species constructor of the Script"))
    }

    /// An Array of the Realm holding these values, where a `None` stays the
    /// hole 10.4.2 lets an Array have.
    fn array_from_holes(
        values: Vec<Option<Value>>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let count = u32::try_from(values.len()).map_err(|_| VMError::PropertyLimit)?;
        let array = realm.array(heap, count)?;
        for (index, value) in values.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
            if let Some(value) = value {
                heap.set_array_element(array, index, value)?;
            }
        }
        Ok(Value::from_object(array))
    }

    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] for a receiver that is not an Object, and a
    /// heap error when an index name cannot be interned.
    #[expect(
        clippy::too_many_lines,
        reason = "one function keeps each method beside the clause it implements"
    )]
    fn call_array_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        call: Call,
        known: Option<i64>,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let object = Self::coerce_object(call.receiver, heap, realm)?;
        // 7.3.18 read it once already where a getter answered it.
        let length = match known {
            Some(length) => length,
            None => Self::array_like_length(heap, object, realm)?,
        };
        // A clause that walks the whole `length` is charged for it before it
        // starts, because a Script can make one 2^32-1 without holding an
        // element. 23.1.3.1 reads a single index, and the three that search
        // stop at what they find, so those are charged where they look.
        if !matches!(
            intrinsic,
            Intrinsic::ArrayPrototypeAt
                | Intrinsic::ArrayPrototypeIncludes
                | Intrinsic::ArrayPrototypeIndexOf
                | Intrinsic::ArrayPrototypeLastIndexOf
        ) {
            self.charge_for_scan(length)?;
        }
        let search = self.call_argument(&call, 0, heap)?;
        match intrinsic {
            // 23.1.3.1: an index outside the Array is undefined.
            Intrinsic::ArrayPrototypeAt => {
                let index = absolute_index(integer_argument(search, heap, realm)?, length);
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
                let from = integer_argument(self.call_argument(&call, 1, heap)?, heap, realm)?;
                let start = absolute_index(from, length).clamp(0, length);
                for index in Self::scan_range(start, length) {
                    self.charge_for_one_of_a_scan(index)?;
                    let element = Self::element_at(heap, object, index)?.unwrap_or(VALUE_UNDEFINED);
                    if same_value_zero(search, element, heap)? {
                        return Ok(VALUE_TRUE);
                    }
                }
                Ok(VALUE_FALSE)
            }
            // 23.1.3.17: IsStrictlyEqual, and a missing index is skipped.
            Intrinsic::ArrayPrototypeIndexOf => {
                let from = integer_argument(self.call_argument(&call, 1, heap)?, heap, realm)?;
                let start = absolute_index(from, length).clamp(0, length);
                for index in Self::scan_range(start, length) {
                    self.charge_for_one_of_a_scan(index)?;
                    let Some(element) = Self::element_at(heap, object, index)? else {
                        continue;
                    };
                    if Self::strictly_equals(search, element, heap)? {
                        return Ok(index_value(i64::from(index)));
                    }
                }
                Ok(Value::from_smi(-1))
            }
            // 23.1.3.37: the `join` of the receiver, or 20.1.3.6 when it is
            // not callable.
            Intrinsic::ArrayPrototypeToString => {
                let key = PropertyKey::String(heap.strings.intern("join")?);
                let join = heap
                    .lookup_named(object, key)?
                    .map(Self::plain_value)
                    .transpose()?;
                let native = join.and_then(Value::as_object).and_then(|reference| {
                    match heap.get_object(reference)?.kind {
                        ObjectKind::NativeFunction { id, .. } => Intrinsic::from_id(id),
                        _ => None,
                    }
                });
                match native {
                    Some(Intrinsic::ArrayPrototypeJoin) => self.call_array_intrinsic(
                        Intrinsic::ArrayPrototypeJoin,
                        Call {
                            arg_count: 0,
                            ..call
                        },
                        None,
                        units,
                        heap,
                        realm,
                    ),
                    _ if join.is_some_and(|join| Self::is_callable(join, heap)) => {
                        // Calling it needs a frame, which an intrinsic has no
                        // way to open yet.
                        Err(VMError::Unsupported(
                            "a user join in Array.prototype.toString",
                        ))
                    }
                    _ => self.object_to_string(call.receiver, heap, realm),
                }
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
                let mut out: Vec<u16> = Vec::new();
                for index in Self::scan_range(0, scanned) {
                    if index > 0 {
                        out.extend_from_slice(&separator);
                    }
                    let element = Self::element_at(heap, object, index)?.unwrap_or(VALUE_UNDEFINED);
                    if element.is_object() {
                        // 7.1.17 of an Object is 7.1.1 with the hint `string`.
                        let text = self.text_of(element, units, heap, realm)?;
                        let text = heap
                            .strings
                            .to_utf16(text)
                            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                        out.extend_from_slice(&text);
                    } else if !element.is_undefined() && !element.is_null() {
                        out.extend(property_name_units(element, heap)?);
                    }
                    if out.len() > self.string_units_limit {
                        return Err(VMError::StringLimit);
                    }
                }
                // Every index above that space is absent, so what remains is
                // separators alone, which no string limit admits.
                if length > scanned && !separator.is_empty() {
                    return Err(VMError::StringLimit);
                }
                self.allocate_string(heap, &out)
            }
            // 23.1.3.23: each argument is written at the length reached so
            // far, and the new length is the answer.
            Intrinsic::ArrayPrototypePush => {
                let mut next = length;
                for offset in 0..call.arg_count {
                    let index = u32::try_from(next).map_err(|_| VMError::PropertyLimit)?;
                    Self::set_element(
                        object,
                        index,
                        self.call_argument(&call, offset, heap)?,
                        heap,
                        realm,
                    )?;
                    next = next.saturating_add(1);
                }
                // Step 5 sets the length with 7.3.4, which throws where
                // 10.4.2.4 refuses it.
                let written = u32::try_from(next).map_err(|_| VMError::PropertyLimit)?;
                Self::set_array_like_length(object, written, heap, realm)?;
                Ok(index_value(next))
            }
            // 23.1.3.22: the last element leaves the Array, which is then one
            // shorter; an empty Array only has its length set again.
            Intrinsic::ArrayPrototypePop => {
                let elements = Self::array_elements(heap, object)?;
                let Ok(last) = u32::try_from(length.saturating_sub(1)) else {
                    Self::set_array_like_length(object, 0, heap, realm)?;
                    return Ok(VALUE_UNDEFINED);
                };
                let element = Self::element_at(heap, object, last)?.unwrap_or(VALUE_UNDEFINED);
                // Step 4.b deletes the index with 7.3.9 and sets the length
                // with 7.3.4, and both throw where 10.1 refuses.
                Self::delete_element_or_throw(object, elements, last, heap, realm)?;
                Self::set_array_like_length(object, last, heap, realm)?;
                Ok(element)
            }
            // 23.1.3.26: the two ends swap until they meet, and an index that
            // is absent stays absent at the position it moves to.
            Intrinsic::ArrayPrototypeReverse => {
                let elements = Self::array_elements(heap, object)?;
                let mut lower = 0u32;
                let mut upper = Self::scan_range(0, length).end.saturating_sub(1);
                while lower < upper {
                    let lower_value = Self::element_at(heap, object, lower)?;
                    let upper_value = Self::element_at(heap, object, upper)?;
                    Self::place_element(heap, object, elements, lower, upper_value, realm)?;
                    Self::place_element(heap, object, elements, upper, lower_value, realm)?;
                    lower = lower.saturating_add(1);
                    upper = upper.saturating_sub(1);
                }
                Ok(call.receiver)
            }
            // 23.1.3.28: the elements of a range, in the Array 23.1.3.4
            // makes.
            Intrinsic::ArrayPrototypeSlice => {
                let start =
                    absolute_index(integer_argument(search, heap, realm)?, length).clamp(0, length);
                let last = self.call_argument(&call, 1, heap)?;
                let end = if last.is_undefined() {
                    length
                } else {
                    absolute_index(integer_argument(last, heap, realm)?, length).clamp(0, length)
                };
                let count = end.saturating_sub(start).max(0);
                let result = Self::array_species_create(
                    object,
                    u32::try_from(count).map_err(|_| VMError::PropertyLimit)?,
                    heap,
                    realm,
                )?;
                let mut target = 0u32;
                for index in Self::scan_range(start, end) {
                    if let Some(value) = Self::element_at(heap, object, index)? {
                        heap.set_array_element(result, target, value)?;
                    }
                    target = target.saturating_add(1);
                }
                Ok(Value::from_object(result))
            }
            // 23.1.3.20: the same in descending order, from the last index
            // when no second argument is present.
            _ => {
                let from = if call.arg_count > 1 {
                    absolute_index(
                        integer_argument(self.call_argument(&call, 1, heap)?, heap, realm)?,
                        length,
                    )
                    .min(length.saturating_sub(1))
                } else {
                    length.saturating_sub(1)
                };
                for index in Self::scan_range(0, from.saturating_add(1)).rev() {
                    self.charge_for_one_of_a_scan(index)?;
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

    /// `ToPrimitive` of 7.1.1 for the Object `call.receiver`, starting at the
    /// method `call.resume` names.
    ///
    /// Answers the primitive when the conversion finishes without running
    /// bytecode. Answers the bytecode unit of a method that has to run first;
    /// its answer arrives through [`Resume`], and the operation runs again.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` when no method of the
    /// object answers a primitive.
    fn convert_to_primitive(
        &mut self,
        mut call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Conversion, VMError> {
        let object = call.receiver.as_object().ok_or(VMError::TypeError)?;
        let Some(resume) = call.resume else {
            return Err(VMError::TypeError);
        };
        let Some((register, first, hint)) = resume.conversion() else {
            return Err(VMError::TypeError);
        };
        let kind = heap
            .get_object(object)
            .ok_or(VMError::TypeError)?
            .kind
            .clone();
        if let Some(feature) = Self::unimplemented_conversion(&kind) {
            return Err(VMError::Unsupported(feature));
        }
        let mut step = first;
        loop {
            let key = match step {
                PrimitiveStep::Exotic => super::realm::WellKnownSymbol::ToPrimitive.key(),
                PrimitiveStep::ValueOf => PropertyKey::String(heap.strings.intern("valueOf")?),
                PrimitiveStep::ToString => PropertyKey::String(heap.strings.intern("toString")?),
            };
            let method = heap
                .lookup_named(object, key)?
                .map(Self::plain_value)
                .transpose()?
                .filter(|method| Self::is_callable(*method, heap));
            if let Some(method) = method {
                // 7.1.1 step 2 passes the hint; 7.1.1.1 passes nothing. The
                // hint goes in the register the answer comes back to, which is
                // where the collector can see it.
                call.arg_count = u16::from(step == PrimitiveStep::Exotic);
                if step == PrimitiveStep::Exotic {
                    let text = self.allocate_string(heap, hint.units())?;
                    self.write_reg(register, text)?;
                    call.arg_start = register;
                }
                call.resume = Some(resume.with_step(step));
                if let Some(code_id) =
                    self.enter_call_value(method, units, active_feedback, heap, realm, call)?
                {
                    return Ok(Conversion::Suspended(code_id));
                }
                // An intrinsic answered without a frame of its own.
                if let Some(value) = Self::primitive_answer(self.acc, step, heap, realm)? {
                    return Ok(Conversion::Done(value));
                }
            }
            let Some(next) = hint.after(step) else {
                return Err(type_error(heap, realm, "an object has no primitive value"));
            };
            step = next;
        }
    }

    /// Continues the conversion of 7.1.1 with the answer a method gave.
    ///
    /// Answers the bytecode unit of the next method when one has to run, and
    /// nothing when the operation can start again.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` when no method of the
    /// object answers a primitive.
    fn finish_conversion(
        &mut self,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let Some(resume) = call.resume else {
            return Err(VMError::TypeError);
        };
        let Some((register, step, hint)) = resume.conversion() else {
            return Err(VMError::TypeError);
        };
        if let Some(value) = Self::primitive_answer(self.acc, step, heap, realm)? {
            if let Resume::Coercion {
                receiver,
                of_receiver: true,
                ..
            } = resume
            {
                heap.set_root(receiver, value).map_err(VMError::Heap)?;
            } else {
                self.write_reg(register, value)?;
            }
            // 13.15.2 has the value in the accumulator, which the frame of
            // the conversion took; the root it travelled in gives it back.
            if let Resume::Primitive {
                held: Some(root), ..
            } = resume
            {
                self.acc = heap.root_value(root).unwrap_or(VALUE_UNDEFINED);
                heap.exit_scope();
            }
            return self.finish_coerced(
                resume,
                call.return_pc,
                call.caller_code_id,
                units,
                active_feedback,
                heap,
                realm,
            );
        }
        // The method answered an Object, so 7.1.1.1 asks the next one. The
        // operand is still in its register, where the collector kept it.
        let Some(next) = hint.after(step) else {
            return Err(type_error(heap, realm, "an object has no primitive value"));
        };
        let held = match resume {
            Resume::Coercion {
                receiver,
                of_receiver: true,
                ..
            } => heap.root_value(receiver).unwrap_or(VALUE_UNDEFINED),
            _ => self.read_reg(register)?,
        };
        let object = held.as_object().ok_or(VMError::TypeError)?;
        let call = Call {
            receiver: Value::from_object(object),
            resume: Some(resume.with_step(next)),
            ..call
        };
        match self.convert_to_primitive(call, units, active_feedback, heap, realm)? {
            Conversion::Done(value) => {
                if let Resume::Coercion {
                    receiver,
                    of_receiver: true,
                    ..
                } = resume
                {
                    heap.set_root(receiver, value).map_err(VMError::Heap)?;
                } else {
                    self.write_reg(register, value)?;
                }
                self.finish_coerced(
                    resume,
                    call.return_pc,
                    call.caller_code_id,
                    units,
                    active_feedback,
                    heap,
                    realm,
                )
            }
            Conversion::Suspended(code_id) => Ok(Some(code_id)),
        }
    }

    /// Runs the native operation again once its argument is a primitive.
    ///
    /// A conversion an instruction asked for answers into the register the
    /// instruction reads, and there is nothing left to do.
    #[expect(
        clippy::too_many_arguments,
        reason = "a conversion runs where a call does, with what a call has"
    )]
    fn finish_coerced(
        &mut self,
        resume: Resume,
        return_pc: usize,
        caller_code_id: Option<u32>,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let Resume::Coercion {
            intrinsic,
            receiver,
            arg_start,
            arg_count,
            construct,
            ..
        } = resume
        else {
            return Ok(None);
        };
        let intrinsic = Intrinsic::from_id(intrinsic).ok_or(VMError::TypeError)?;
        let call = Call {
            receiver: heap.root_value(receiver).unwrap_or(VALUE_UNDEFINED),
            func: arg_start,
            arg_start,
            arg_count,
            slot: 0,
            resume: None,
            construct,
            return_pc,
            caller_code_id,
        };
        heap.exit_scope();
        // A clause that converts more than one argument converts the next one
        // the same way, from the beginning of the native.
        if let Some((index, hint)) = self.next_coercion(intrinsic, &call, heap, realm)? {
            return self.begin_coercion(
                intrinsic,
                index,
                hint,
                call,
                units,
                active_feedback,
                heap,
                realm,
            );
        }
        // The operation takes the path a call of it takes, so one that opens
        // a frame of its own still does after a conversion.
        let entered = self.dispatch_native(intrinsic, call, units, active_feedback, heap, realm)?;
        // 23.1.1.1 and 20.5.1.1 answer an object of their own, which the
        // instruction keeps where the collector sees it.
        if entered.is_none()
            && let Some(target) = construct
        {
            self.write_reg(target, self.acc)?;
        }
        Ok(entered)
    }

    /// The method of 7.1.1.1 this Realm still owes an object of this kind.
    ///
    /// A prototype that should own `toString` but does not lets the lookup walk
    /// to %Object.prototype% and answer `[object …]`, which is a wrong answer
    /// rather than a missing feature. Naming the gap keeps it a gap.
    const fn unimplemented_conversion(kind: &ObjectKind) -> Option<&'static str> {
        match kind {
            // 20.1.3.6 is the right answer for these, 23.1.3.37 and 20.5.3.4
            // are implemented.
            // 20.2.3.5 answers the source text of the function, which
            // %Function.prototype% carries.
            ObjectKind::Error
            | ObjectKind::Function { .. }
            | ObjectKind::NativeFunction { .. }
            | ObjectKind::BoundFunction { .. }
            | ObjectKind::Ordinary
            | ObjectKind::NumberWrapper(_)
            | ObjectKind::BooleanWrapper(_)
            | ObjectKind::StringWrapper(_)
            | ObjectKind::SymbolWrapper(_)
            | ObjectKind::Arguments
            | ObjectKind::Math
            | ObjectKind::Array { .. }
            | ObjectKind::ArrayIterator { .. }
            // The state of a walk of 23.1.3 and the pair of 6.1.7.1 are
            // reachable from no Script, so no conversion of them is owed.
            | ObjectKind::ArrayIteration { .. }
            | ObjectKind::Accessor { .. }
            | ObjectKind::RegExp { .. }
            // 27.2.5.5 tags a Promise through @@toStringTag.
            | ObjectKind::Promise { .. }
            | ObjectKind::Json
            | ObjectKind::Reflect => None,
        }
    }

    /// The intrinsic that owns this name and has not been built, when the
    /// object is that intrinsic itself rather than something under it.
    fn prototype_owes(
        target: Value,
        name: &[u16],
        heap: &GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<&'static str>, VMError> {
        let Some(object) = target.as_object() else {
            return Ok(None);
        };
        // 19.1 and 19.2 give the global object properties this Realm reaches
        // through the Global Environment Record rather than through it.
        if realm.global_environment().global_object(heap)? == object
            && super::realm::global_properties_own(name)
        {
            return Ok(Some("a property of the global object"));
        }
        for (prototype, owns, owner) in [
            (
                realm.string_prototype(heap)?,
                super::realm::string_prototype_owns as fn(&[u16]) -> bool,
                "a property of %String.prototype%",
            ),
            (
                realm.array_prototype(heap)?,
                super::realm::array_prototype_owns,
                "a property of %Array.prototype%",
            ),
            (
                realm.function_prototype(heap)?,
                super::realm::function_prototype_owns,
                "a property of %Function.prototype%",
            ),
            (
                realm.number_prototype(heap)?,
                super::realm::number_prototype_owns,
                "a property of %Number.prototype%",
            ),
            (
                realm.boolean_prototype(heap)?,
                super::realm::boolean_prototype_owns,
                "a property of %Boolean.prototype%",
            ),
            (
                realm.promise_prototype(heap)?,
                super::realm::promise_prototype_owns,
                "a property of %Promise.prototype%",
            ),
        ] {
            if prototype.as_object() == Some(object) && owns(name) {
                return Ok(Some(owner));
            }
        }
        Ok(None)
    }

    /// What a well-known Symbol answers when no object of the chain has it.
    ///
    /// Every Prototype of clause 22 and 23 that this Realm has not finished
    /// building owns `@@iterator`, so a miss on one of them is a gap. An
    /// ordinary object owns none, and its miss is the undefined that 7.4.2
    /// turns into a `TypeError`.
    fn absent_well_known(
        target: Value,
        object: ObjectRef,
        symbol: Option<super::realm::WellKnownSymbol>,
        heap: &GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        const GAP: VMError = VMError::Unsupported("a well-known Symbol of an unbuilt Prototype");
        if target.is_string() {
            return Err(GAP);
        }
        // 22.2.6 gives `%RegExp.prototype%` five Symbol-keyed methods, and
        // this Realm has not built `@@matchAll`.
        if symbol == Some(super::realm::WellKnownSymbol::MatchAll)
            && (realm.regexp_prototype(heap)?.as_object() == Some(object)
                || matches!(
                    heap.get_object(object).map(|entry| &entry.kind),
                    Some(&ObjectKind::RegExp { .. })
                ))
        {
            return Err(VMError::Unsupported("a property of %RegExp.prototype%"));
        }
        match heap.get_object(object).map(|object| &object.kind) {
            Some(
                ObjectKind::StringWrapper(_)
                | ObjectKind::ArrayIterator { .. }
                | ObjectKind::NumberWrapper(_)
                | ObjectKind::BooleanWrapper(_)
                | ObjectKind::Error,
            ) => Err(GAP),
            _ => Ok(VALUE_UNDEFINED),
        }
    }

    /// The answer 10.1.8.1 gives when no object of the Prototype Chain has the
    /// name.
    ///
    /// That is `undefined` only when the chain is complete. A prototype this
    /// Realm has not finished building would have owned the name, so the miss
    /// is a gap and never an answer.
    fn absent_property(
        target: Value,
        name: &[u16],
        heap: &GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        Self::missing_property(target, name, Reach::Chain, heap, realm)
    }

    /// The same question for an own property: only a name the object itself
    /// owes is a gap, because 10.1.5 never reaches a Prototype.
    fn absent_own_property(
        target: Value,
        name: &[u16],
        heap: &GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        Self::missing_property(target, name, Reach::Own, heap, realm)
    }

    /// Whether the miss is an answer or a gap, for either reach.
    #[expect(
        clippy::too_many_lines,
        reason = "one function names every Prototype that owes a name"
    )]
    fn missing_property(
        target: Value,
        name: &[u16],
        reach: Reach,
        heap: &GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let chain = matches!(reach, Reach::Chain);
        if target.is_string() {
            if chain && super::realm::string_prototype_owns(name) {
                return Err(VMError::Unsupported("a property of %String.prototype%"));
            }
            return Ok(VALUE_UNDEFINED);
        }
        // The Prototype itself owns the names its instances resolve on it, so
        // a miss there is the same gap read one object earlier.
        if let Some(owner) = Self::prototype_owes(target, name, heap, realm)? {
            return Err(VMError::Unsupported(owner));
        }
        let kind = target
            .as_object()
            .and_then(|reference| heap.get_object(reference))
            .map(|object| object.kind.clone());
        match kind {
            Some(ObjectKind::Array { .. }) if chain && super::realm::array_prototype_owns(name) => {
                Err(VMError::Unsupported("a property of %Array.prototype%"))
            }
            // 23.1.2 gives `%Array%` more than 17 gives a built-in function,
            // and this Realm builds only some of them.
            Some(ObjectKind::NativeFunction { id, .. })
                if id == Intrinsic::ArrayConstructor.id()
                    && super::realm::array_constructor_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %Array%"))
            }
            // 22.1.2 gives `%String%` more than 17 gives a built-in function.
            Some(ObjectKind::NativeFunction { id, .. })
                if id == Intrinsic::StringConstructor.id()
                    && super::realm::string_constructor_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %String%"))
            }
            // 27.2.4 gives `%Promise%` five combinators and the pair of
            // 27.2.4.9, none of which this Realm builds.
            Some(ObjectKind::NativeFunction { id, .. })
                if Intrinsic::from_id(id) == Some(Intrinsic::PromiseConstructor)
                    && super::realm::promise_constructor_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %Promise%"))
            }
            // 27.2.5 gives `%Promise.prototype%` the `finally` of 27.2.5.3.
            Some(ObjectKind::Promise { .. })
                if chain && super::realm::promise_prototype_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %Promise.prototype%"))
            }
            // 20.4.2 gives `%Symbol%` more than the thirteen of table 1.
            Some(ObjectKind::NativeFunction { id, .. })
                if Intrinsic::from_id(id) == Some(Intrinsic::SymbolConstructor)
                    && super::realm::symbol_constructor_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %Symbol%"))
            }
            // 22.2.6 gives `%RegExp.prototype%` more than this Realm builds.
            Some(ObjectKind::RegExp { .. })
                if chain && super::realm::regexp_prototype_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %RegExp.prototype%"))
            }
            // 28.1 gives `%Reflect%` more than this Realm builds.
            Some(ObjectKind::Reflect) if super::realm::reflect_owns(name) => {
                Err(VMError::Unsupported("a property of %Reflect%"))
            }
            // 25.5 gives `%JSON%` more than this Realm builds.
            Some(ObjectKind::Json) if super::realm::json_owns(name) => {
                Err(VMError::Unsupported("a property of %JSON%"))
            }
            // 21.3 gives `%Math%` more than this Realm builds.
            Some(ObjectKind::Math) if super::realm::math_owns(name) => {
                Err(VMError::Unsupported("a property of %Math%"))
            }
            // 20.1.2 gives `%Object%` more than 17 gives a built-in function,
            // and this Realm builds only some of them.
            Some(ObjectKind::NativeFunction { id, .. })
                if id == Intrinsic::ObjectConstructor.id()
                    && super::realm::object_constructor_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %Object%"))
            }
            Some(ObjectKind::Function { .. } | ObjectKind::NativeFunction { .. })
                if chain && super::realm::function_prototype_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %Function.prototype%"))
            }
            Some(ObjectKind::Function { .. } | ObjectKind::NativeFunction { .. }) => {
                Ok(VALUE_UNDEFINED)
            }
            // These reach a Prototype the Realm has not built at all, and it
            // owns names no list here carries, so every miss is a gap.
            // These reach a Prototype this Realm has not built, so a name it
            // owns is a gap. A name it does not own is absent there as it is
            // anywhere: 20.3.3 gives %Boolean.prototype% no `length`, so an
            // Array method called on a boolean reads none and walks nothing.
            Some(ObjectKind::Error)
                if chain
                    && super::realm::wrapper_prototype_owns(
                        &super::realm::ERROR_PROTOTYPE_PROPERTIES,
                        name,
                    ) =>
            {
                Err(VMError::Unsupported("a property of %Error.prototype%"))
            }
            Some(ObjectKind::StringWrapper(_))
                if chain && super::realm::string_prototype_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %String.prototype%"))
            }
            Some(ObjectKind::NumberWrapper(_))
                if chain
                    && super::realm::wrapper_prototype_owns(
                        &super::realm::NUMBER_PROTOTYPE_PROPERTIES,
                        name,
                    ) =>
            {
                Err(VMError::Unsupported("a property of %Number.prototype%"))
            }
            Some(ObjectKind::BooleanWrapper(_))
                if chain
                    && super::realm::wrapper_prototype_owns(
                        &super::realm::BOOLEAN_PROTOTYPE_PROPERTIES,
                        name,
                    ) =>
            {
                Err(VMError::Unsupported("a property of %Boolean.prototype%"))
            }
            Some(ObjectKind::ArrayIterator { .. })
                if chain
                    && super::realm::wrapper_prototype_owns(
                        &super::realm::ARRAY_ITERATOR_PROTOTYPE_PROPERTIES,
                        name,
                    ) =>
            {
                Err(VMError::Unsupported(
                    "a property of %ArrayIteratorPrototype%",
                ))
            }
            // 20.4.3 gives `%Symbol.prototype%` a `description` accessor this
            // Realm resolves for a Symbol and has not built as a property.
            Some(ObjectKind::SymbolWrapper(_))
                if chain && super::realm::symbol_prototype_owns(name) =>
            {
                Err(VMError::Unsupported("a property of %Symbol.prototype%"))
            }
            _ if chain && super::realm::object_prototype_owns(name) => {
                Err(VMError::Unsupported("a property of %Object.prototype%"))
            }
            _ => Ok(VALUE_UNDEFINED),
        }
    }

    /// Whether a value is one of the callables this engine knows (7.2.3).
    fn is_callable(value: Value, heap: &GenerationalHeap) -> bool {
        value.as_object().is_some_and(|reference| {
            heap.get_object(reference).is_some_and(|object| {
                matches!(
                    object.kind,
                    ObjectKind::Function { .. }
                        | ObjectKind::NativeFunction { .. }
                        | ObjectKind::BoundFunction { .. }
                )
            })
        })
    }

    /// The answer one method of 7.1.1 gave, or none when the conversion goes
    /// on with the next method.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` when `@@toPrimitive`
    /// answered an Object, which 7.1.1 does not continue past.
    fn primitive_answer(
        value: Value,
        step: PrimitiveStep,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<Value>, VMError> {
        if !value.is_object() {
            return Ok(Some(value));
        }
        if step == PrimitiveStep::Exotic {
            return Err(type_error(
                heap,
                realm,
                "Symbol.toPrimitive answered an object",
            ));
        }
        Ok(None)
    }

    /// The error a refused binding operation of 9.1.1 raises.
    fn binding_error(
        heap: &mut GenerationalHeap,
        realm: &Realm,
        outcome: super::realm::BindingOutcome,
        name: &[u16],
    ) -> VMError {
        use super::realm::{BindingOutcome, NativeErrorKind};
        let mut message = alloc::string::String::from_utf16_lossy(name);
        let kind = match outcome {
            // 9.1.1.1.6 and 9.1.1.2.7 both name the binding that is missing.
            BindingOutcome::Uninitialized | BindingOutcome::Unresolvable => {
                message.push_str(UNRESOLVABLE_SUFFIX);
                NativeErrorKind::ReferenceError
            }
            BindingOutcome::Missing(feature) => return VMError::Unsupported(feature),
            BindingOutcome::Accessor => {
                return VMError::Unsupported("a binding the global object holds as an accessor");
            }
            BindingOutcome::Immutable => {
                message.push_str(" is not writable");
                NativeErrorKind::TypeError
            }
        };
        raise_message(heap, realm, kind, &message)
    }

    /// The Elements store of an Array receiver.
    ///
    /// A receiver `Function.prototype.call` brought here need not be one, so
    /// the absence is named where it is found.
    fn array_elements(
        heap: &GenerationalHeap,
        object: ObjectRef,
    ) -> Result<super::elements::ElementsRef, VMError> {
        heap.get_object(object)
            .and_then(|object| object.elements)
            // 23.1.3 is generic over an array-like, which it reads and writes
            // through 7.3.2 and 7.3.4. This engine moves elements in the store
            // 10.4.2 gives an Array, and a receiver without one has none to
            // move, which it names rather than reporting a broken frame.
            .ok_or(VMError::Unsupported(
                "an Array method that moves elements, on a receiver that is not an Array",
            ))
    }

    /// `DeletePropertyOrThrow` of 7.3.9 for an index of an Array.
    ///
    /// An index 7.3.15 moved into the Shape is not configurable, which 10.1.10
    /// refuses to delete.
    fn delete_element_or_throw(
        object: ObjectRef,
        elements: super::elements::ElementsRef,
        index: u32,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let shape = heap
            .get_object(object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .shape_id;
        if shape != heap.shapes.root_shape() {
            let name = alloc::format!("{index}");
            let key = PropertyKey::String(
                heap.strings
                    .intern_units(&name.encode_utf16().collect::<Vec<u16>>())?,
            );
            if let Some(flags) = heap.own_named_flags(object, key)? {
                if !flags.configurable {
                    return Err(type_error(
                        heap,
                        realm,
                        "cannot delete a property that is not configurable",
                    ));
                }
                delete_property(object, key, None, heap)?;
                return Ok(());
            }
        }
        heap.delete_element(elements, index)?;
        Ok(())
    }

    /// `Set(O, ! ToString(𝔽(index)), value, true)` of 7.3.4, or
    /// `DeletePropertyOrThrow` of 7.3.9 when the index it came from was absent.
    fn place_element(
        heap: &mut GenerationalHeap,
        object: ObjectRef,
        elements: super::elements::ElementsRef,
        index: u32,
        value: Option<Value>,
        realm: &Realm,
    ) -> Result<(), VMError> {
        match value {
            Some(value) => Self::set_element(object, index, value, heap, realm)?,
            None => drop(heap.delete_element(elements, index)?),
        }
        Ok(())
    }

    /// The indices of `start..end` an Elements store can address.
    fn scan_range(start: i64, end: i64) -> core::ops::Range<u32> {
        let bound = |value: i64| u32::try_from(value.max(0)).unwrap_or(u32::MAX);
        bound(start)..bound(end)
    }

    /// `LengthOfArrayLike` of 7.3.18.
    fn array_like_length(
        heap: &mut GenerationalHeap,
        object: ObjectRef,
        realm: &Realm,
    ) -> Result<i64, VMError> {
        if let Some(length) = heap.array_length(object) {
            return Ok(i64::from(length));
        }
        // 10.4.3.1 gives a String exotic object a `length` of its
        // `[[StringData]]`, which no Shape of the object holds.
        if let Some(count) = Self::string_data_length(object, heap)? {
            return Ok(i64::from(count));
        }
        let Some(name) = heap.strings.lookup_interned_units(&LENGTH_NAME) else {
            return Ok(0);
        };
        let found = heap.lookup_named(object, PropertyKey::String(name))?;
        // A `length` this Realm has not built is a gap, and reading it as zero
        // would say the array-like is empty.
        let Some(property) = found else {
            Self::absent_property(Value::from_object(object), &LENGTH_NAME, heap, realm)?;
            return Ok(0);
        };
        // 10.1.8.1 step 3.b: an accessor with no getter reads undefined,
        // which 7.1.20 clamps to zero.
        let value = if property.flags.is_accessor {
            let (get, _) = Self::accessor_parts(property.value, heap)?;
            if !get.is_undefined() {
                return Err(VMError::Unsupported("a property that is an accessor"));
            }
            VALUE_UNDEFINED
        } else {
            property.value
        };
        // 7.1.20 ToLength clamps into 0..2^53-1; the scan is bounded again by
        // the index space, so the clamp loses no reachable index.
        Ok(integer_argument(value, heap, realm)?.max(0))
    }

    /// `Get(O, ! ToString(𝔽(index)))` of 7.3.2, absent when `HasProperty` is
    /// false: the Elements store answers an index it holds, and a name on the
    /// Prototype Chain answers the rest.
    fn element_at(
        heap: &mut GenerationalHeap,
        object: ObjectRef,
        index: u32,
    ) -> Result<Option<Value>, VMError> {
        match Self::element_slot_at(heap, object, index)? {
            Some((value, false)) => Ok(Some(value)),
            Some((_, true)) => Err(VMError::Unsupported("a property that is an accessor")),
            None => Ok(None),
        }
    }

    /// The same index, with the pair of 6.1.7.1 where the property is an
    /// accessor rather than the gap a native names for one.
    fn element_slot_at(
        heap: &mut GenerationalHeap,
        object: ObjectRef,
        index: u32,
    ) -> Result<Option<(Value, bool)>, VMError> {
        // A named property is found with its depth, so the chain walk below
        // stops where a nearer one would already have answered. The lookup
        // also rejects a cycle, which is why the walk needs no guard of its
        // own.
        let key = PropertyKey::String(heap.intern_index(index)?);
        let named = heap.lookup_named(object, key)?;
        let limit = named.as_ref().map_or(u16::MAX, |found| found.holder_depth);
        let mut current = object;
        let mut depth = 0u16;
        while depth <= limit {
            // 10.4.3.1 owns every index below the length of its
            // `[[StringData]]`, each one the String of that code unit.
            if let Some(count) = Self::string_data_length(current, heap)?
                && index < count
            {
                let data = Self::string_data(current, heap).ok_or(VMError::TypeError)?;
                let unit = heap
                    .strings
                    .char_code_at(data, index as usize)
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                let text = heap.strings.allocate_units(&[unit])?;
                return Ok(Some((Value::from_string(text), false)));
            }
            // 10.4.2 keeps an index of an Array in the Elements store, which
            // a Prototype of the chain has as much as the receiver does.
            let elements = heap.get_object(current).ok_or(VMError::TypeError)?.elements;
            if let Some(elements) = elements
                && let Some(value) = heap
                    .get_elements(elements)
                    .ok_or(VMError::TypeError)?
                    .get(index)
            {
                return Ok(Some((value, false)));
            }
            let prototype = heap
                .get_object(current)
                .ok_or(VMError::TypeError)?
                .prototype;
            let Some(next) = prototype.as_object() else {
                break;
            };
            current = next;
            depth = depth.saturating_add(1);
        }
        Ok(named.map(|found| (found.value, found.flags.is_accessor)))
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
            // 23.1.3.38, 23.1.3.17 and 23.1.3.4: the iterator holds the
            // Array, the next index and which of the three it yields.
            Intrinsic::ArrayPrototypeValues
            | Intrinsic::ArrayPrototypeKeys
            | Intrinsic::ArrayPrototypeEntries => {
                let kind = match intrinsic {
                    Intrinsic::ArrayPrototypeKeys => ArrayIterationKind::Key,
                    Intrinsic::ArrayPrototypeEntries => ArrayIterationKind::KeyAndValue,
                    _ => ArrayIterationKind::Value,
                };
                let target = Self::coerce_object(call.receiver, heap, realm)?;
                let prototype = realm.array_iterator_prototype(heap)?;
                let shape = heap.shapes.root_shape();
                let iterator = heap.allocate_object(shape, prototype)?;
                heap.set_object_kind(
                    iterator,
                    ObjectKind::ArrayIterator {
                        target: Value::from_object(target),
                        index: 0,
                        kind,
                    },
                )?;
                Ok(Value::from_object(iterator))
            }
            // 23.1.5.2.1: the length is read again at every step, and the
            // iterator is exhausted once the index reaches it.
            Intrinsic::ArrayIteratorPrototypeNext => {
                let iterator = call.receiver.as_object().ok_or(VMError::TypeError)?;
                let ObjectKind::ArrayIterator {
                    target,
                    index,
                    kind,
                } = heap.get_object(iterator).ok_or(VMError::TypeError)?.kind
                else {
                    return Err(type_error(
                        heap,
                        realm,
                        "next called on a value that is not an Array Iterator",
                    ));
                };
                // 23.1.5.2.1 reads the length of the array-like again at
                // every step and takes the index out of it with 7.3.2, so an
                // object that is no Array is walked the same way.
                let value = match target.as_object() {
                    Some(object)
                        if i64::from(index) < Self::array_like_length(heap, object, realm)? =>
                    {
                        let key = index_value(i64::from(index));
                        match kind {
                            ArrayIterationKind::Key => Some(key),
                            ArrayIterationKind::Value => Some(
                                Self::element_at(heap, object, index)?.unwrap_or(VALUE_UNDEFINED),
                            ),
                            ArrayIterationKind::KeyAndValue => {
                                let element = Self::element_at(heap, object, index)?
                                    .unwrap_or(VALUE_UNDEFINED);
                                Some(Self::array_of(alloc::vec![key, element], heap, realm)?)
                            }
                        }
                    }
                    _ => None,
                };
                let next = match value {
                    Some(_) => ObjectKind::ArrayIterator {
                        target,
                        index: index.saturating_add(1),
                        kind,
                    },
                    None => ObjectKind::ArrayIterator {
                        target: VALUE_UNDEFINED,
                        index,
                        kind,
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

    /// `String.raw` of 22.1.2.4.
    ///
    /// The literals come out of the `raw` of the template and the
    /// substitutions out of the arguments that follow it, one between each
    /// pair of literals.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] where the template or its `raw` is not an
    /// object, and a gap where a value only a frame could convert stands in
    /// either list.
    fn string_raw(
        &self,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let template = self.call_argument(call, 0, heap)?;
        let cooked = Self::coerce_object(template, heap, realm)?;
        let key = PropertyKey::String(heap.strings.intern("raw")?);
        let raw = heap
            .lookup_named(cooked, key)?
            .map(Self::plain_value)
            .transpose()?
            .unwrap_or(VALUE_UNDEFINED);
        let raw = Self::coerce_object(raw, heap, realm)?;
        let length = Self::array_like_length(heap, raw, realm)?;
        let mut units: Vec<u16> = Vec::new();
        for index in Self::scan_range(0, length) {
            let literal = Self::element_at(heap, raw, index)?.unwrap_or(VALUE_UNDEFINED);
            if literal.is_object() {
                return Err(VMError::Unsupported("ToString of an Object"));
            }
            units.extend(property_name_units(literal, heap)?);
            // Step 4.e: the last literal has no substitution after it.
            if i64::from(index).saturating_add(1) >= length {
                break;
            }
            let Ok(offset) = u16::try_from(index.saturating_add(1)) else {
                break;
            };
            if offset >= call.arg_count {
                continue;
            }
            let substitution = self.call_argument(call, offset, heap)?;
            if substitution.is_object() {
                return Err(VMError::Unsupported("ToString of an Object"));
            }
            units.extend(property_name_units(substitution, heap)?);
            if units.len() > self.string_units_limit {
                return Err(VMError::StringLimit);
            }
        }
        if units.len() > self.string_units_limit {
            return Err(VMError::StringLimit);
        }
        self.allocate_string(heap, &units)
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
        let mut boxed = None;
        let tag = if receiver.is_undefined() {
            "Undefined"
        } else if receiver.is_null() {
            "Null"
        } else {
            // Step 3 boxes the receiver, and step 14 reads @@toStringTag out
            // of that object rather than out of the primitive.
            let object = Self::coerce_object(receiver, heap, realm)?;
            boxed = Some(object);
            match heap.get_object(object).ok_or(VMError::TypeError)?.kind {
                ObjectKind::Array { .. } => "Array",
                ObjectKind::Function { .. }
                | ObjectKind::NativeFunction { .. }
                | ObjectKind::BoundFunction { .. } => "Function",
                ObjectKind::Error => "Error",
                ObjectKind::BooleanWrapper(_) => "Boolean",
                ObjectKind::NumberWrapper(_) => "Number",
                ObjectKind::StringWrapper(_) => "String",
                // 10.4.4 gives the object [[ParameterMap]], which step 8 reads.
                ObjectKind::Arguments => "Arguments",
                // 20.4.3.5 tags a Symbol wrapper through @@toStringTag, so its
                // builtin tag is the ordinary one, as it is for the namespaces
                // of 21.3, 25.5 and 28.1 and the iterator of 23.1.5.2.2.
                ObjectKind::Ordinary
                | ObjectKind::SymbolWrapper(_)
                | ObjectKind::Promise { .. }
                | ObjectKind::ArrayIterator { .. }
                | ObjectKind::ArrayIteration { .. }
                | ObjectKind::Accessor { .. }
                | ObjectKind::Reflect
                | ObjectKind::Json
                | ObjectKind::Math => "Object",
                // 22.2.6.17 tags a RegExp through its own `toString`, and
                // 20.1.3.6 gives it the builtin tag of 22.2.
                ObjectKind::RegExp { .. } => "RegExp",
            }
        };
        let mut units: Vec<u16> = "[object ".encode_utf16().collect();
        match boxed
            .map(|object| {
                heap.lookup_named(object, super::realm::WellKnownSymbol::ToStringTag.key())
            })
            .transpose()?
            .flatten()
            .map(Self::plain_value)
            .transpose()?
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
        code: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let units = self.receiver_units(call.receiver, code, heap, realm)?;
        match intrinsic {
            // 22.1.3.1: an index outside the String is the empty String.
            Intrinsic::StringPrototypeCharAt => {
                let position = integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?;
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
                let position = integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?;
                Ok(usize::try_from(position)
                    .ok()
                    .and_then(|position| units.get(position).copied())
                    .map_or(VALUE_NAN, |unit| Value::from_smi(i32::from(unit))))
            }
            // 22.1.3.9: the search starts at the clamped position and -1 says
            // the String does not occur.
            Intrinsic::StringPrototypeIndexOf => {
                let search = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                let start = integer_argument(self.call_argument(&call, 1, heap)?, heap, realm)?
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
                let relative = integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?;
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
                    result.extend(property_name_units(
                        self.call_argument(&call, index, heap)?,
                        heap,
                    )?);
                }
                self.allocate_string(heap, &result)
            }
            // 22.1.3.7: the search ends at the clamped position.
            Intrinsic::StringPrototypeEndsWith => {
                let search = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                let end = match self.call_argument(&call, 1, heap)? {
                    value if value.is_undefined() => units.len(),
                    value => clamped_index(integer_argument(value, heap, realm)?, units.len()),
                };
                let start = end.checked_sub(search.len());
                Ok(Value::from_bool(start.is_some_and(|start| {
                    units.get(start..end) == Some(search.as_slice())
                })))
            }
            // 22.1.3.8: the search starts at the clamped position.
            Intrinsic::StringPrototypeIncludes => {
                let search = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                let start = clamped_index(
                    integer_argument(self.call_argument(&call, 1, heap)?, heap, realm)?,
                    units.len(),
                );
                Ok(Value::from_bool(
                    find_units(&units, &search, start).is_some(),
                ))
            }
            // 22.1.3.10: the last occurrence at or before the clamped position.
            Intrinsic::StringPrototypeLastIndexOf => {
                let search = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                let position = self.call_argument(&call, 1, heap)?;
                let last = units.len().saturating_sub(search.len());
                let end = if position.is_undefined() || primitive_number(position, heap)?.is_nan() {
                    last
                } else {
                    clamped_index(integer_argument(position, heap, realm)?, last)
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
                if primitive_number(self.call_argument(&call, 0, heap)?, heap)? == f64::INFINITY {
                    return Err(range_error(heap, realm, "repeat count is out of range"));
                }
                // ToIntegerOrInfinity truncates toward zero, so -0.5 is 0 and
                // only a count that is negative after that is out of range.
                let count = integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?;
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
                let start = relative_index(
                    integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?,
                    units.len(),
                );
                let end = match self.call_argument(&call, 1, heap)? {
                    value if value.is_undefined() => units.len(),
                    value => relative_index(integer_argument(value, heap, realm)?, units.len()),
                };
                let slice = units.get(start..end.max(start)).unwrap_or_default();
                self.allocate_string(heap, slice)
            }
            // 22.1.3.24: the search starts at the clamped position.
            Intrinsic::StringPrototypeStartsWith => {
                let search = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                let start = clamped_index(
                    integer_argument(self.call_argument(&call, 1, heap)?, heap, realm)?,
                    units.len(),
                );
                let end = start.saturating_add(search.len());
                Ok(Value::from_bool(
                    units.get(start..end) == Some(search.as_slice()),
                ))
            }
            // 22.1.3.25: both ends are clamped and then ordered.
            Intrinsic::StringPrototypeSubstring => {
                let first = clamped_index(
                    integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?,
                    units.len(),
                );
                let second = match self.call_argument(&call, 1, heap)? {
                    value if value.is_undefined() => units.len(),
                    value => clamped_index(integer_argument(value, heap, realm)?, units.len()),
                };
                let slice = units
                    .get(first.min(second)..first.max(second))
                    .unwrap_or_default();
                self.allocate_string(heap, slice)
            }
            // 22.1.3.4: the code point at an index, which pairs a surrogate
            // with the one after it.
            Intrinsic::StringPrototypeCodePointAt => {
                let position = integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?;
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
                let width = integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?;
                let width = usize::try_from(width).unwrap_or(0);
                let filler = match self.call_argument(&call, 1, heap)? {
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
            // 22.1.3.9 answers whether the text has a lone surrogate, and
            // 22.1.3.29 replaces each of them with U+FFFD.
            Intrinsic::StringPrototypeIsWellFormed | Intrinsic::StringPrototypeToWellFormed => {
                let tests = intrinsic == Intrinsic::StringPrototypeIsWellFormed;
                let mut out: Option<Vec<u16>> = None;
                let mut index = 0usize;
                while let Some((_, width, lone)) = audhsos_utf16::code_point_at(&units, index) {
                    if lone {
                        if tests {
                            return Ok(VALUE_FALSE);
                        }
                        let buffer = out.get_or_insert_with(|| units.clone());
                        *buffer.get_mut(index).ok_or(VMError::TypeError)? = 0xFFFD;
                    }
                    index = index.saturating_add(width);
                }
                if tests {
                    return Ok(VALUE_TRUE);
                }
                self.allocate_string(heap, &out.unwrap_or(units))
            }
            // B.2.2.1: the second argument is a length and not an end, and a
            // negative start counts from the end of the text.
            Intrinsic::StringPrototypeSubstr => {
                let size = i64::try_from(units.len()).map_err(|_| VMError::PropertyLimit)?;
                let start = integer_argument(self.call_argument(&call, 0, heap)?, heap, realm)?;
                let start = if start < 0 {
                    size.saturating_add(start).max(0)
                } else {
                    start.min(size)
                };
                let length = self.call_argument(&call, 1, heap)?;
                let length = if length.is_undefined() {
                    size.saturating_sub(start)
                } else {
                    integer_argument(length, heap, realm)?.clamp(0, size.saturating_sub(start))
                };
                let end = start.saturating_add(length);
                let (Ok(start), Ok(end)) = (usize::try_from(start), usize::try_from(end)) else {
                    return Err(VMError::PropertyLimit);
                };
                let part = units.get(start..end).unwrap_or_default().to_vec();
                self.allocate_string(heap, &part)
            }
            // 22.1.3.12 orders by the code units, which is the order this
            // Realm has no locale data to refine.
            Intrinsic::StringPrototypeLocaleCompare => {
                let other = property_name_units(self.call_argument(&call, 0, heap)?, heap)?;
                let order = match units.cmp(&other) {
                    core::cmp::Ordering::Less => -1,
                    core::cmp::Ordering::Equal => 0,
                    core::cmp::Ordering::Greater => 1,
                };
                Ok(Value::from_smi(order))
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

    /// `String.prototype.split` of 22.1.3.23.
    ///
    /// A separator that is a `RegExp` carries the `@@split` of 22.2.6.14;
    /// every other Object reaches `ToString`, which names its own gap.
    fn call_split_intrinsic(
        &mut self,
        call: &Call,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let text = self.receiver_units(call.receiver, units, heap, realm)?;
        let separator = self.call_argument(call, 0, heap)?;
        if let Some(reference) = separator.as_object()
            && let Some(pattern) = Self::regexp_pattern(reference, heap)
        {
            return self.regexp_split(&text, &pattern, call, heap, realm);
        }
        self.string_split(&text, call, heap, realm)
    }

    /// `RegExp.prototype[@@match]`, `[@@search]` and `[@@split]` of 22.2.6.
    ///
    /// Each one reads the text with 22.2.7.1, which calls the `exec` of the
    /// object: one of the Script is a call the clause has no frame to make, so
    /// only the `exec` of this Realm answers here. 22.2.6.14 constructs the
    /// splitter with `SpeciesConstructor`, which is this Realm's `%RegExp%`
    /// alone.
    fn call_regexp_symbol_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let Some(receiver) = call.receiver.as_object() else {
            return Err(type_error(
                heap,
                realm,
                "a method of 22.2.6 called on a value that is no Object",
            ));
        };
        let Some(pattern) = Self::regexp_pattern(receiver, heap) else {
            return Err(VMError::Unsupported(
                "a method of 22.2.6 on a receiver that is no RegExp",
            ));
        };
        let key = PropertyKey::String(heap.strings.intern("exec")?);
        let exec = match heap.lookup_named(receiver, key)? {
            Some(found) => Self::plain_value(found)?,
            None => VALUE_UNDEFINED,
        };
        if !Self::is_intrinsic(exec, Intrinsic::RegExpPrototypeExec, heap) {
            return Err(VMError::Unsupported("an exec of the Script"));
        }
        let text = property_name_units(self.call_argument(call, 0, heap)?, heap)?;
        if intrinsic == Intrinsic::RegExpPrototypeSearch {
            // Step 3 keeps `lastIndex` as it found it, and step 4 searches
            // from the start.
            let key = PropertyKey::String(heap.strings.intern("lastIndex")?);
            let held = heap
                .lookup_named(receiver, key)?
                .map_or(VALUE_UNDEFINED, |property| property.value);
            Self::set_last_index(receiver, Value::from_smi(0), heap, realm)?;
            let matched = self.regexp_exec(receiver, &pattern, &text, heap, realm)?;
            Self::set_last_index(receiver, held, heap, realm)?;
            let Some(matched) = matched else {
                return Ok(Value::from_smi(-1));
            };
            let start = i32::try_from(matched.range.start).map_err(|_| VMError::StringLimit)?;
            return Ok(Value::from_smi(start));
        }
        if intrinsic == Intrinsic::RegExpPrototypeSplit {
            let name = PropertyKey::String(heap.strings.intern("constructor")?);
            let constructor = match heap.lookup_named(receiver, name)? {
                Some(found) => Self::plain_value(found)?,
                None => VALUE_UNDEFINED,
            };
            if !Self::is_intrinsic(constructor, Intrinsic::RegExpConstructor, heap) {
                return Err(VMError::Unsupported(
                    "a splitter of a constructor that is not %RegExp%",
                ));
            }
            return self.regexp_split(&text, &pattern, call, heap, realm);
        }
        // Step 6: a pattern without `g` answers what 22.2.7.2 answers.
        if !pattern.global {
            let matched = self.regexp_exec(receiver, &pattern, &text, heap, realm)?;
            let Some(matched) = matched else {
                return Ok(VALUE_NULL);
            };
            return self.match_array(&text, &matched, heap, realm);
        }
        self.global_match(receiver, &pattern, &text, heap, realm)
    }

    /// `RegExp.prototype[@@split]` of 22.2.6.14.
    ///
    /// The splitter 22.2.6.14 constructs differs from the receiver only in
    /// carrying `y`, so the walk matches stickily at each position instead of
    /// building a second `RegExp`. `SpeciesConstructor` is the default one:
    /// this Realm has no `@@species`.
    ///
    /// The parts are cut out of the text before anything is allocated, so no
    /// String of a part is held unrooted while the next one is made.
    fn regexp_split(
        &mut self,
        text: &[u16],
        pattern: &crate::regexp::RegExp,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let limit = self.call_argument(call, 1, heap)?;
        let limit = if limit.is_undefined() {
            u32::MAX
        } else {
            crate::value::number_uint32(primitive_number(limit, heap)?)
        };
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        if limit == 0 {
            return self.split_result(&[], heap, realm);
        }
        // Step 10: an empty text answers itself unless the pattern matches it.
        if text.is_empty() {
            let matched = self.sticky_match(pattern, text, 0)?.is_some();
            let parts: &[Option<&[u16]>] = if matched { &[] } else { &[Some(text)] };
            return self.split_result(parts, heap, realm);
        }
        let mut parts: Vec<Option<&[u16]>> = Vec::new();
        let mut start = 0usize;
        let mut at = 0usize;
        while at < text.len() {
            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
            let Some(found) = self.sticky_match(pattern, text, at)? else {
                at = at.saturating_add(1);
                continue;
            };
            let end = found.range.end.min(text.len());
            // A match that ends where the last part began makes no progress.
            if end == start {
                at = at.saturating_add(1);
                continue;
            }
            parts.push(Some(text.get(start..at).unwrap_or_default()));
            if parts.len() == limit {
                return self.split_result(&parts, heap, realm);
            }
            start = end;
            for capture in &found.captures {
                parts.push(
                    capture
                        .clone()
                        .map(|range| text.get(range).unwrap_or_default()),
                );
                if parts.len() == limit {
                    return self.split_result(&parts, heap, realm);
                }
            }
            at = start;
        }
        parts.push(Some(text.get(start..).unwrap_or_default()));
        self.split_result(&parts, heap, realm)
    }

    /// One match of the pattern that has to begin exactly at `from`, which is
    /// what the `y` of the splitter 22.2.6.14 builds asks for.
    fn sticky_match(
        &mut self,
        pattern: &crate::regexp::RegExp,
        text: &[u16],
        from: usize,
    ) -> Result<Option<audhsos_regex::Match>, VMError> {
        let limits = audhsos_regex::Limits {
            input_units: self.string_units_limit,
            work: self.fuel,
            ..audhsos_regex::Limits::default()
        };
        let report = pattern
            .regex
            .find(text, from, true, limits)
            .map_err(|_| VMError::Unsupported("a RegExp beyond the limits of the automaton"))?;
        self.fuel = self.fuel.saturating_sub(report.work);
        Ok(report.matched)
    }

    /// `String.prototype.split` of 22.1.3.23 for a separator that is not a
    /// `RegExp`.
    ///
    /// The parts are cut out of the text before anything is allocated, so no
    /// String of a part is held unrooted while the next one is made.
    fn string_split(
        &self,
        units: &[u16],
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let separator = self.call_argument(call, 0, heap)?;
        let limit = self.call_argument(call, 1, heap)?;
        let limit = if limit.is_undefined() {
            u32::MAX
        } else {
            crate::value::number_uint32(primitive_number(limit, heap)?)
        };
        // 22.1.3.23 answers an empty Array for a limit of zero before it reads
        // the separator's text at all.
        if limit == 0 {
            return self.split_result(&[], heap, realm);
        }
        if separator.is_undefined() {
            return self.split_result(&[Some(units)], heap, realm);
        }
        let pattern = property_name_units(separator, heap)?;
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        // An empty separator matches no empty substring, so it answers the code
        // units themselves, at most `limit` of them.
        if pattern.is_empty() {
            let parts: Vec<Option<&[u16]>> = units.chunks(1).take(limit).map(Some).collect();
            return self.split_result(&parts, heap, realm);
        }
        if units.is_empty() {
            return self.split_result(&[Some(units)], heap, realm);
        }
        let mut parts: Vec<Option<&[u16]>> = Vec::new();
        let mut start = 0usize;
        let mut at = 0usize;
        while at.saturating_add(pattern.len()) <= units.len() {
            if units.get(at..at.saturating_add(pattern.len())) != Some(pattern.as_slice()) {
                at = at.saturating_add(1);
                continue;
            }
            parts.push(Some(units.get(start..at).unwrap_or_default()));
            if parts.len() == limit {
                return self.split_result(&parts, heap, realm);
            }
            at = at.saturating_add(pattern.len());
            start = at;
        }
        parts.push(Some(units.get(start..).unwrap_or_default()));
        self.split_result(&parts, heap, realm)
    }

    /// `CreateArrayFromList` of 7.3.18 for the parts of a split.
    ///
    /// A part that is `None` is a capture of 22.2.6.14 that did not
    /// participate, which is undefined.
    fn split_result(
        &self,
        parts: &[Option<&[u16]>],
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let length = u32::try_from(parts.len()).map_err(|_| VMError::PropertyLimit)?;
        let array = realm.array(heap, length)?;
        for (index, part) in parts.iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
            let value = match part {
                Some(part) => self.allocate_string(heap, part)?,
                None => VALUE_UNDEFINED,
            };
            heap.set_array_element(array, index, value)?;
        }
        Ok(Value::from_object(array))
    }

    /// The code units of the `this` value of a `%String.prototype%` method.
    ///
    /// Every one of them begins with `RequireObjectCoercible` and `ToString`
    /// (22.1.3), so undefined and null are a `TypeError` and every other
    /// primitive answers its text. An Object would need the `ToPrimitive` of
    /// 7.1.1, which names itself as a gap.
    fn receiver_units(
        &self,
        receiver: Value,
        code: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Vec<u16>, VMError> {
        if receiver.is_undefined() || receiver.is_null() {
            return Err(type_error(
                heap,
                realm,
                "String method called on null or undefined",
            ));
        }
        if let Some(object) = receiver.as_object() {
            return self.unframed_text_of(object, code, heap, realm);
        }
        property_name_units(receiver, heap)
    }

    /// The text 7.1.17 gives an Object whose conversion needs no frame.
    ///
    /// 7.1.1 with the hint `string` asks `@@toPrimitive`, then `toString`. An
    /// object that carries neither of the Script's own answers here; one that
    /// carries either of them names the gap, because a native has no frame to
    /// run a method in.
    fn unframed_text_of(
        &self,
        object: ObjectRef,
        code: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Vec<u16>, VMError> {
        let exotic = super::realm::WellKnownSymbol::ToPrimitive.key();
        if heap.lookup_named(object, exotic)?.is_some() {
            return Err(VMError::Unsupported("ToString of an Object"));
        }
        let key = PropertyKey::String(heap.strings.intern("toString")?);
        let found = heap
            .lookup_named(object, key)?
            .map(Self::plain_value)
            .transpose()?
            .and_then(Value::as_object)
            .and_then(|method| heap.get_object(method))
            .map(|method| method.kind.clone());
        let Some(ObjectKind::NativeFunction { id, .. }) = found else {
            return Err(VMError::Unsupported("ToString of an Object"));
        };
        let intrinsic = Intrinsic::from_id(id).ok_or(VMError::TypeError)?;
        let receiver = Value::from_object(object);
        let text = match intrinsic {
            Intrinsic::ObjectPrototypeToString => self.object_to_string(receiver, heap, realm)?,
            // 20.2.3.5 answers the text the unit kept beside the code.
            Intrinsic::FunctionPrototypeToString => {
                self.function_source(receiver, code, heap, realm)?
            }
            // 22.1.4 keeps the text in `[[StringData]]`, which 22.1.3.29
            // answers as it is.
            Intrinsic::StringPrototypeToString => Self::string_data(object, heap)
                .ok_or(VMError::Unsupported("ToString of an Object"))?,
            Intrinsic::NumberPrototypeToString | Intrinsic::BooleanPrototypeToString => {
                let call = Call {
                    receiver,
                    func: Reg(0),
                    arg_start: Reg(0),
                    arg_count: 0,
                    slot: 0,
                    resume: None,
                    construct: None,
                    return_pc: 0,
                    caller_code_id: None,
                };
                self.wrapped_value(intrinsic, &call, heap, realm)?
            }
            _ => return Err(VMError::Unsupported("ToString of an Object")),
        };
        property_name_units(text, heap)
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
            return match heap.lookup_named(prototype, name)? {
                Some(property) => Self::plain_value(property),
                None => Self::absent_property(value, &units, heap, realm),
            };
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

    /// The value a Number or a Boolean answers for one property name.
    ///
    /// 7.1.18 would produce a wrapper whose own properties are none at all, so
    /// every name is resolved on the Prototype 21.1.3 or 20.3.3 names.
    fn primitive_member(
        value: Value,
        name: PropertyKey,
        heap: &GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<Value>, VMError> {
        let prototype = if value.as_boolean().is_some() {
            realm.boolean_prototype(heap)?
        } else if value.as_f64().is_some() {
            realm.number_prototype(heap)?
        } else {
            return Ok(None);
        };
        let prototype = prototype
            .as_object()
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        let units = name
            .as_string()
            .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
            .unwrap_or_default();
        Ok(Some(match heap.lookup_named(prototype, name)? {
            Some(property) => Self::plain_value(property)?,
            None => Self::absent_property(value, &units, heap, realm)?,
        }))
    }

    /// `SymbolDescriptiveString` of 20.4.3.3.1: `Symbol(` and the description.
    fn symbol_descriptive_string(symbol: SymbolRef, heap: &GenerationalHeap) -> Vec<u16> {
        let mut text: Vec<u16> = "Symbol(".encode_utf16().collect();
        if let Some(description) = Self::symbol_description(symbol, heap) {
            text.extend_from_slice(&description);
        }
        text.push(0x29);
        text
    }

    /// The `[[Description]]` of a Symbol, which table 1 of 20.4.2 gives the
    /// well-known ones and the heap gives every other.
    /// `thisSymbolValue` of 20.4.3: the Symbol itself or the one a wrapper
    /// holds in `[[SymbolData]]`.
    fn this_symbol_value(receiver: Value, heap: &GenerationalHeap) -> Option<SymbolRef> {
        if let Some(symbol) = receiver.as_symbol() {
            return Some(symbol);
        }
        match heap.get_object(receiver.as_object()?)?.kind {
            ObjectKind::SymbolWrapper(symbol) => Some(symbol),
            _ => None,
        }
    }

    fn symbol_description(symbol: SymbolRef, heap: &GenerationalHeap) -> Option<Vec<u16>> {
        if let Some(well_known) = super::realm::WellKnownSymbol::from_reference(symbol) {
            return Some(well_known.description().encode_utf16().collect());
        }
        heap.symbol_description(symbol)?.map(<[u16]>::to_vec)
    }

    /// The value a Symbol answers for one property name.
    ///
    /// 7.1.18 would produce a wrapper whose own properties are none at all, so
    /// every name but the `description` accessor of 20.4.3.2 is resolved on
    /// `%Symbol.prototype%`.
    fn symbol_member(
        &self,
        value: Value,
        name: PropertyKey,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<Value>, VMError> {
        let Some(symbol) = value.as_symbol() else {
            return Ok(None);
        };
        let units = name
            .as_string()
            .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
            .unwrap_or_default();
        if units == DESCRIPTION_NAME {
            return match Self::symbol_description(symbol, heap) {
                Some(description) => self.allocate_string(heap, &description).map(Some),
                None => Ok(Some(VALUE_UNDEFINED)),
            };
        }
        let prototype = realm
            .symbol_prototype(heap)?
            .as_object()
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        Ok(Some(match heap.lookup_named(prototype, name)? {
            Some(property) => Self::plain_value(property)?,
            None => Self::absent_property(value, &units, heap, realm)?,
        }))
    }

    /// Whether 10.4.3.1 gives this object this name out of its
    /// `[[StringData]]` rather than out of its Shape.
    fn owns_string_exotic(
        object: ObjectRef,
        name: PropertyKey,
        heap: &GenerationalHeap,
    ) -> Result<bool, VMError> {
        if Self::string_data(object, heap).is_none() {
            return Ok(false);
        }
        // A name the Shape carries is an ordinary own property of the object,
        // which 10.4.3.1 leaves to 10.1.
        if Self::shape_holds(object, name, heap)? {
            return Ok(false);
        }
        Ok(heap.own_named_flags(object, name)?.is_some())
    }

    /// The own property 10.4.3.1 gives a String exotic object: its `length`
    /// and every index the `[[StringData]]` has.
    ///
    /// Every other name is ordinary, and answers nothing here.
    fn string_exotic_member(
        &self,
        object: ObjectRef,
        units: &[u16],
        heap: &mut GenerationalHeap,
    ) -> Result<Option<Value>, VMError> {
        let Some(data) = Self::string_data(object, heap) else {
            return Ok(None);
        };
        let length = heap
            .strings
            .length_of(data)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        if units == LENGTH_NAME {
            let length = i32::try_from(length).map_err(|_| VMError::StringLimit)?;
            return Ok(Some(Value::from_smi(length)));
        }
        let Some(unit) = string_index(units)
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < length)
            .and_then(|index| heap.strings.char_code_at(data, index))
        else {
            return Ok(None);
        };
        self.allocate_string(heap, &[unit]).map(Some)
    }

    /// The two functions 25.5 gives `%JSON%`.
    fn call_json_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let first = self.call_argument(call, 0, heap)?;
        let second = self.call_argument(call, 1, heap)?;
        if intrinsic == Intrinsic::JsonParse {
            // 25.5.1 step 7 calls the reviver for every name it parsed.
            if Self::is_callable(second, heap) {
                return Err(VMError::Unsupported("a reviver of 25.5.1"));
            }
            let text = property_name_units(first, heap)?;
            return self.json_parse(&text, heap, realm);
        }
        // 25.5.2 step 2 takes a replacer that is a function or an Array of
        // names; step 4 takes a space; neither is built.
        if !second.is_undefined() {
            return Err(VMError::Unsupported("a replacer of 25.5.2"));
        }
        let third = self.call_argument(call, 2, heap)?;
        if !third.is_undefined() {
            return Err(VMError::Unsupported("a space of 25.5.2"));
        }
        let mut out = Vec::new();
        if !self.json_quote_value(first, &mut out, 0, heap, realm)? {
            return Ok(VALUE_UNDEFINED);
        }
        self.allocate_string(heap, &out)
    }

    /// `JSON.parse` of 25.5.1 without its reviver: the text is parsed once
    /// and the tree is built from the leaves up.
    fn json_parse(
        &mut self,
        text: &[u16],
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let limits = audhsos_json::Limits {
            input: self.string_units_limit,
            ..audhsos_json::Limits::default()
        };
        let mut work = self.fuel;
        let document = audhsos_json::parse(text, limits, &mut work).map_err(|_| {
            raise(
                heap,
                realm,
                super::realm::NativeErrorKind::SyntaxError,
                "invalid JSON text",
            )
        })?;
        self.fuel = work;
        // The arena is in postorder, so a node is built after everything it
        // holds, and each one is rooted while the next is allocated.
        heap.enter_scope();
        let mut built: Vec<Root> = Vec::with_capacity(document.nodes.len());
        for node in &document.nodes {
            let value = match &node.kind {
                audhsos_json::Kind::Null => VALUE_NULL,
                audhsos_json::Kind::Boolean(boolean) => Value::from_bool(*boolean),
                audhsos_json::Kind::Number(number) => Value::from_f64(*number),
                audhsos_json::Kind::String(units) => self.allocate_string(heap, units)?,
                audhsos_json::Kind::Array(elements) => {
                    let count =
                        u32::try_from(elements.len()).map_err(|_| VMError::PropertyLimit)?;
                    let array = realm.array(heap, count)?;
                    for (index, element) in elements.iter().enumerate() {
                        let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
                        let held = *built.get(*element).ok_or(VMError::InvalidRegister)?;
                        let value = heap.root_value(held).unwrap_or(VALUE_UNDEFINED);
                        heap.set_array_element(array, index, value)?;
                    }
                    Value::from_object(array)
                }
                audhsos_json::Kind::Object(properties) => {
                    let object = realm.ordinary_object(heap)?;
                    for (name, child) in properties {
                        let key = PropertyKey::String(heap.strings.intern_units(name)?);
                        let held = *built.get(*child).ok_or(VMError::InvalidRegister)?;
                        let value = heap.root_value(held).unwrap_or(VALUE_UNDEFINED);
                        heap.define_own_named(object, key, value, PropertyFlags::ordinary_data())?;
                    }
                    Value::from_object(object)
                }
            };
            built.push(heap.push_root(value)?);
        }
        let root = *built.get(document.root).ok_or(VMError::InvalidRegister)?;
        let value = heap.root_value(root).unwrap_or(VALUE_UNDEFINED);
        heap.exit_scope();
        Ok(value)
    }

    /// `SerializeJSONProperty` of 25.5.2.4, answering whether the value has a
    /// text at all.
    fn json_quote_value(
        &mut self,
        value: Value,
        out: &mut Vec<u16>,
        depth: u16,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<bool, VMError> {
        if depth > 64 {
            return Err(VMError::Unsupported(
                "a JSON text deeper than this engine walks",
            ));
        }
        let mut work = self.fuel;
        let limit = self.string_units_limit;
        let appended = |out: &mut Vec<u16>, units: &[u16], work: &mut u64| {
            audhsos_json::append(out, units, limit, work).map_err(|_| VMError::StringLimit)
        };
        if value.is_null() {
            appended(out, &NULL_UNITS, &mut work)?;
        } else if let Some(boolean) = value.as_boolean() {
            let text: Vec<u16> = if boolean { "true" } else { "false" }
                .encode_utf16()
                .collect();
            appended(out, &text, &mut work)?;
        } else if let Some(number) = value.as_f64() {
            // 25.5.2.4 step 10: a Number that is not finite has no text.
            let text: Vec<u16> = if number.is_finite() {
                crate::number::decimal_string(number)
                    .encode_utf16()
                    .collect()
            } else {
                NULL_UNITS.to_vec()
            };
            appended(out, &text, &mut work)?;
        } else if value.is_string() {
            let units = heap
                .strings
                .to_utf16(value)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            audhsos_json::quote(out, &units, limit, &mut work).map_err(|_| VMError::StringLimit)?;
        } else if let Some(object) = value.as_object() {
            // 25.5.2.4 step 5 calls `toJSON`, and step 11 a callable has no
            // text at all.
            let key = PropertyKey::String(heap.strings.intern("toJSON")?);
            if heap.lookup_named(object, key)?.is_some() {
                return Err(VMError::Unsupported("a toJSON of 25.5.2.4"));
            }
            if Self::is_callable(value, heap) {
                return Ok(false);
            }
            self.fuel = work;
            return self.json_quote_object(object, out, depth, heap, realm);
        } else {
            // A Symbol and undefined have no text (25.5.2.4 steps 11 and 12).
            return Ok(false);
        }
        self.fuel = work;
        Ok(true)
    }

    /// `SerializeJSONArray` of 25.5.2.5 and `SerializeJSONObject` of 25.5.2.6.
    fn json_quote_object(
        &mut self,
        object: ObjectRef,
        out: &mut Vec<u16>,
        depth: u16,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<bool, VMError> {
        let depth = depth.saturating_add(1);
        if Self::is_array(Value::from_object(object), heap) {
            let length = Self::array_like_length(heap, object, realm)?;
            out.push(0x5B);
            for index in 0..length {
                if index > 0 {
                    out.push(0x2C);
                }
                let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
                let element = Self::element_at(heap, object, index)?.unwrap_or(VALUE_UNDEFINED);
                if !self.json_quote_value(element, out, depth, heap, realm)? {
                    out.extend_from_slice(&NULL_UNITS);
                }
            }
            out.push(0x5D);
            return Ok(true);
        }
        out.push(0x7B);
        let mut written = 0usize;
        for (key, enumerable) in heap.own_keys(object)? {
            if !enumerable {
                continue;
            }
            let Some(name) = key.as_string() else {
                continue;
            };
            let units = heap
                .strings
                .to_utf16(Value::from_string(name))
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let held = Self::plain_value(
                heap.lookup_named(object, key)?
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?,
            )?;
            let mut text = Vec::new();
            if !self.json_quote_value(held, &mut text, depth, heap, realm)? {
                continue;
            }
            if written > 0 {
                out.push(0x2C);
            }
            let mut work = self.fuel;
            audhsos_json::quote(out, &units, self.string_units_limit, &mut work)
                .map_err(|_| VMError::StringLimit)?;
            self.fuel = work;
            out.push(0x3A);
            out.extend_from_slice(&text);
            written = written.saturating_add(1);
        }
        out.push(0x7D);
        Ok(true)
    }

    /// `eval` of 19.2.1, through `PerformEval` of 19.2.1.1.
    ///
    /// The text is a Script of the same Realm, which the embedding evaluates
    /// and answers; the call instruction runs again with what it said. A
    /// direct eval inside a function shares the variable environment of that
    /// function, which this engine keeps in registers no Script can name, so
    /// only the top level of a Script of a Realm takes this path.
    fn perform_eval(
        &mut self,
        call: &Call,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if let Some(compiled) = self.compiled_unit.take() {
            return match compiled {
                Compiled::Evaluated(outcome) => outcome,
                Compiled::Refused => Err(raise(
                    heap,
                    realm,
                    super::realm::NativeErrorKind::SyntaxError,
                    "invalid eval source",
                )),
                Compiled::Unit(_) | Compiled::Printed => Err(VMError::InvalidFeedbackVector),
            };
        }
        // Step 2: anything but a String is the answer itself.
        let source = self.call_argument(call, 0, heap)?;
        if !source.is_string() {
            return Ok(source);
        }
        if call.caller_code_id.is_some() || !units.active.realm_script {
            return Err(VMError::Unsupported(
                "an eval whose variable environment is not the global one",
            ));
        }
        let text = heap
            .strings
            .to_utf16(source)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        self.pending_script = true;
        self.pending_source = Some(alloc::rc::Rc::from(text));
        self.resume_pc = call.return_pc.saturating_sub(1);
        self.resume_code_id = call.caller_code_id;
        Ok(VALUE_UNDEFINED)
    }

    /// `CreateDynamicFunction` of 20.2.1.1.
    ///
    /// The source text is built here and compiled by the embedding, which is
    /// the only side that holds the units of the Realm: the run stops, the
    /// instruction runs again, and the second pass makes the function object
    /// of the unit it was given.
    fn create_dynamic_function(
        &mut self,
        call: &Call,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if let Some(compiled) = self.compiled_unit.take() {
            // Step 12: a text no Script accepts is a `SyntaxError`.
            let Compiled::Unit(unit) = compiled else {
                return Err(raise(
                    heap,
                    realm,
                    super::realm::NativeErrorKind::SyntaxError,
                    "invalid function body",
                ));
            };
            let expected = units
                .table
                .root(unit)
                .and_then(|root| root.functions.first())
                .ok_or(VMError::InvalidBytecode(
                    VerificationError::FunctionOutOfBounds { pc: 0, index: unit },
                ))?
                .expected_arguments;
            let prototype = realm.function_prototype(heap)?;
            let function = heap.allocate_function(unit, 0, None)?;
            heap.set_object_prototype(function, prototype)?;
            self.acc = Value::from_object(function);
            Self::set_function_length(function, expected, heap)?;
            let name = heap.strings.allocate_str("anonymous")?;
            let key = PropertyKey::String(heap.strings.intern("name")?);
            heap.define_own_named(
                function,
                key,
                Value::from_string(name),
                super::realm::builtin_metadata(),
            )?;
            // 10.2.5 gives it the `prototype` every ordinary function has.
            self.make_constructor(units.active, heap, realm)?;
            return Ok(self.acc);
        }
        // Steps 3 to 11 join every argument but the last with commas and take
        // the last as the body.
        let mut text: Vec<u16> = "(function anonymous(".encode_utf16().collect();
        let parameters = call.arg_count.saturating_sub(1);
        for index in 0..parameters {
            if index > 0 {
                text.push(0x2C);
            }
            let part = self.call_argument(call, index, heap)?;
            let part = self.text_of(part, units, heap, realm)?;
            text.extend(
                heap.strings
                    .to_utf16(part)
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?,
            );
        }
        text.extend("\n) {\n".encode_utf16());
        if call.arg_count > 0 {
            let body = self.call_argument(call, call.arg_count.saturating_sub(1), heap)?;
            let body = self.text_of(body, units, heap, realm)?;
            text.extend(
                heap.strings
                    .to_utf16(body)
                    .ok_or(VMError::Heap(HeapError::InvalidReference))?,
            );
        }
        text.extend("\n})".encode_utf16());
        if text.len() > self.string_units_limit {
            return Err(VMError::StringLimit);
        }
        self.pending_source = Some(alloc::rc::Rc::from(text));
        // The call instruction runs again once the unit exists; its callee and
        // its arguments are still in the registers it read them from.
        self.resume_pc = call.return_pc.saturating_sub(1);
        self.resume_code_id = call.caller_code_id;
        Ok(VALUE_UNDEFINED)
    }

    /// 7.1.19 of the key of a computed property access.
    ///
    /// An Object key reaches 7.1.1 with the hint `string`, which runs a method
    /// of the Script: the primitive comes back into the register and the
    /// instruction runs again. `held` is the value the instruction still has
    /// to write, which the accumulator holds and the frame would take; it
    /// travels in a root of a scope of its own.
    ///
    /// Answers whether the instruction has to run again.
    #[expect(
        clippy::too_many_arguments,
        reason = "a conversion runs where the instruction does, with what it has"
    )]
    fn convert_key(
        &mut self,
        key: Reg,
        held: Option<Value>,
        pc: &mut usize,
        current_code_id: &mut Option<u32>,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<bool, VMError> {
        let value = self.read_reg(key)?;
        let Some(object) = value.as_object() else {
            return Ok(false);
        };
        let root = match held {
            Some(held) => {
                heap.enter_scope();
                Some(heap.push_root(held)?)
            }
            None => None,
        };
        let call = Call {
            receiver: Value::from_object(object),
            func: key,
            arg_start: key,
            arg_count: 0,
            slot: 0,
            resume: Some(Resume::Primitive {
                register: key,
                step: PrimitiveStep::Exotic,
                hint: PrimitiveHint::String,
                held: root,
            }),
            return_pc: pc.saturating_sub(1),
            caller_code_id: *current_code_id,
            construct: None,
        };
        match self.convert_to_primitive(call, units, active_feedback, heap, realm)? {
            Conversion::Done(value) => {
                self.write_reg(key, value)?;
                if let Some(root) = root {
                    self.acc = heap.root_value(root).unwrap_or(VALUE_UNDEFINED);
                    heap.exit_scope();
                }
                *pc = pc.saturating_sub(1);
            }
            Conversion::Suspended(code_id) => {
                *current_code_id = Some(code_id);
                *pc = 0;
            }
        }
        Ok(true)
    }

    /// `ToString` of 7.1.17, where an Object goes through 7.1.1 with the hint
    /// `string`.
    ///
    /// Only a method this Realm built can answer here: one of the Script needs
    /// a frame this native has none of, which it names. The depth bound is
    /// what a cyclic Array would otherwise spend the Rust stack on.
    fn text_of(
        &mut self,
        value: Value,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        /// How deep one `ToString` follows the methods of this Realm.
        const DEPTH_LIMIT: usize = 16;
        let Some(object) = value.as_object() else {
            return self.primitive_string(value, heap, realm);
        };
        if self.conversion_depth >= DEPTH_LIMIT {
            return Err(VMError::Unsupported("a ToString that nests too deeply"));
        }
        // 7.1.1 step 1 asks `@@toPrimitive` before either method.
        if heap
            .lookup_named(object, super::realm::WellKnownSymbol::ToPrimitive.key())?
            .is_some()
        {
            return Err(VMError::Unsupported("the @@toPrimitive of an Object"));
        }
        for name in ["toString", "valueOf"] {
            let key = PropertyKey::String(heap.strings.intern(name)?);
            let method = heap
                .lookup_named(object, key)?
                .map(Self::plain_value)
                .transpose()?
                .filter(|method| Self::is_callable(*method, heap));
            let Some(method) = method else {
                continue;
            };
            let intrinsic = method
                .as_object()
                .and_then(|method| heap.get_object(method))
                .and_then(|entry| match entry.kind {
                    ObjectKind::NativeFunction { id, .. } => Intrinsic::from_id(id),
                    _ => None,
                });
            let Some(intrinsic) = intrinsic else {
                return Err(VMError::Unsupported("a ToString of the Script"));
            };
            let call = Call {
                receiver: value,
                func: Reg(0),
                arg_start: Reg(0),
                arg_count: 0,
                slot: 0,
                return_pc: 0,
                resume: None,
                construct: None,
                caller_code_id: None,
            };
            self.conversion_depth = self.conversion_depth.saturating_add(1);
            let answered = self.call_intrinsic(intrinsic, call, units, heap, realm);
            self.conversion_depth = self.conversion_depth.saturating_sub(1);
            let answered = answered?;
            if !answered.is_object() {
                return self.primitive_string(answered, heap, realm);
            }
        }
        Err(type_error(heap, realm, "an object has no primitive value"))
    }

    /// `String.prototype.replace` of 22.1.3.19.
    ///
    /// Step 2 gives the search value its own say through `@@replace`; a
    /// method of the Script there needs a frame this native has none of, as
    /// does a replace value that is callable (step 8).
    fn string_replace(
        &mut self,
        call: &Call,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let search = self.call_argument(call, 0, heap)?;
        let replacement = self.call_argument(call, 1, heap)?;
        if Self::is_callable(replacement, heap) {
            return Err(VMError::Unsupported("a replace value that is callable"));
        }
        // Step 2: `@@replace` of the search value answers in place of this
        // clause, and 22.2.6.11 is the only one this Realm builds.
        if let Some(object) = search.as_object() {
            let method = heap
                .lookup_named(object, super::realm::WellKnownSymbol::Replace.key())?
                .map(Self::plain_value)
                .transpose()?
                .unwrap_or(VALUE_UNDEFINED);
            if !method.is_undefined() {
                let native = method
                    .as_object()
                    .and_then(|method| heap.get_object(method))
                    .and_then(|method| match method.kind {
                        ObjectKind::NativeFunction { id, .. } => Intrinsic::from_id(id),
                        _ => None,
                    });
                if native != Some(Intrinsic::RegExpPrototypeReplace) {
                    return Err(VMError::Unsupported("a @@replace of the Script"));
                }
                let text = self.receiver_units(call.receiver, units, heap, realm)?;
                return self.regexp_replace(object, &text, replacement, heap, realm);
            }
            return Err(VMError::Unsupported("ToString of an Object"));
        }
        let text = self.receiver_units(call.receiver, units, heap, realm)?;
        let search = property_name_units(search, heap)?;
        let replacement = property_name_units(replacement, heap)?;
        // Steps 6 and 7: the first occurrence alone, and the String itself
        // where there is none.
        let Some(position) = text
            .windows(search.len().max(1))
            .position(|window| search.is_empty() || window == search.as_slice())
            .filter(|_| search.len() <= text.len())
            .or_else(|| search.is_empty().then_some(0))
        else {
            return self.allocate_string(heap, &text);
        };
        let mut out: Vec<u16> = text.get(..position).unwrap_or_default().to_vec();
        Self::append_substitution(&mut out, &search, &text, position, &[], &replacement);
        out.extend_from_slice(
            text.get(position.saturating_add(search.len())..)
                .unwrap_or_default(),
        );
        self.allocate_string(heap, &out)
    }

    /// `%RegExp.prototype%[@@replace]` of 22.2.6.11.
    ///
    /// The matches are taken before anything is built, which is the order the
    /// clause reads and writes `lastIndex` in.
    fn regexp_replace(
        &mut self,
        receiver: ObjectRef,
        text: &[u16],
        replacement: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if Self::is_callable(replacement, heap) {
            return Err(VMError::Unsupported("a replace value that is callable"));
        }
        let pattern = Self::regexp_pattern(receiver, heap)
            .ok_or_else(|| type_error(heap, realm, "this value is not a RegExp"))?;
        // 22.2.7.1 uses an `exec` the object carries where that is callable,
        // which runs a method of the Script.
        let key = PropertyKey::String(heap.strings.intern("exec")?);
        if heap.own_named_flags(receiver, key)?.is_some() {
            return Err(VMError::Unsupported("an exec of the Script"));
        }
        let replacement = property_name_units(replacement, heap)?;
        if pattern.global {
            Self::set_last_index(receiver, Value::from_smi(0), heap, realm)?;
        }
        let mut out: Vec<u16> = Vec::new();
        let mut taken = 0usize;
        while let Some(found) = self.regexp_exec(receiver, &pattern, text, heap, realm)? {
            let start = found.range.start.min(text.len());
            let end = found.range.end.min(text.len());
            out.extend_from_slice(text.get(taken..start).unwrap_or_default());
            let matched = text.get(start..end).unwrap_or_default().to_vec();
            let captures: Vec<Option<Vec<u16>>> = found
                .captures
                .iter()
                .map(|range| {
                    range
                        .as_ref()
                        .map(|range| text.get(range.clone()).unwrap_or_default().to_vec())
                })
                .collect();
            Self::append_substitution(&mut out, &matched, text, start, &captures, &replacement);
            taken = end.max(start);
            if !pattern.global {
                break;
            }
            // 22.2.6.11 step 11.c advances over an empty match, which
            // otherwise matches at the same index for ever.
            if start == end {
                let next = end.saturating_add(1);
                if next > text.len() {
                    break;
                }
                Self::set_last_index(
                    receiver,
                    Value::from_smi(i32::try_from(next).map_err(|_| VMError::StringLimit)?),
                    heap,
                    realm,
                )?;
            }
            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
        }
        out.extend_from_slice(text.get(taken..).unwrap_or_default());
        self.allocate_string(heap, &out)
    }

    /// `GetSubstitution` of 22.1.3.19.1 for a replacement that is a String.
    fn append_substitution(
        out: &mut Vec<u16>,
        matched: &[u16],
        text: &[u16],
        position: usize,
        captures: &[Option<Vec<u16>>],
        replacement: &[u16],
    ) {
        let tail = position.saturating_add(matched.len());
        let mut index = 0;
        while let Some(&unit) = replacement.get(index) {
            index = index.saturating_add(1);
            if unit != 0x24 {
                out.push(unit);
                continue;
            }
            let Some(&next) = replacement.get(index) else {
                out.push(0x24);
                continue;
            };
            index = index.saturating_add(1);
            match next {
                // `$$`
                0x24 => out.push(0x24),
                // `$&`
                0x26 => out.extend_from_slice(matched),
                // `` $` ``
                0x60 => out.extend_from_slice(text.get(..position).unwrap_or_default()),
                // `$'`
                0x27 => out.extend_from_slice(text.get(tail..).unwrap_or_default()),
                // `$n` and `$nn`, which name a capture in opening order.
                0x30..=0x39 => {
                    let single = usize::from(next.saturating_sub(0x30));
                    let double = replacement
                        .get(index)
                        .filter(|unit| (0x30..=0x39).contains(*unit))
                        .map(|unit| {
                            single
                                .saturating_mul(10)
                                .saturating_add(usize::from(unit.saturating_sub(0x30)))
                        })
                        .filter(|number| *number >= 1 && *number <= captures.len());
                    let group =
                        double
                            .or(Some(single)
                                .filter(|number| *number >= 1 && *number <= captures.len()));
                    let Some(group) = group else {
                        out.push(0x24);
                        out.push(next);
                        continue;
                    };
                    if double.is_some() {
                        index = index.saturating_add(1);
                    }
                    if let Some(Some(text)) = captures.get(group.saturating_sub(1)) {
                        out.extend_from_slice(text);
                    }
                }
                _ => {
                    out.push(0x24);
                    out.push(next);
                }
            }
        }
    }

    /// The methods 22.2.6 gives `%RegExp.prototype%` that this Realm builds.
    fn call_regexp_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let receiver = call
            .receiver
            .as_object()
            .ok_or_else(|| type_error(heap, realm, "this value is not a RegExp"))?;
        let Some(pattern) = Self::regexp_pattern(receiver, heap) else {
            return Err(type_error(heap, realm, "this value is not a RegExp"));
        };
        // 22.2.6.17 answers the text of the literal, which needs no match.
        if intrinsic == Intrinsic::RegExpPrototypeToString {
            let mut units: Vec<u16> = alloc::vec![0x2F];
            // 22.2.6.10 answers "(?:)" for an empty pattern, which is what
            // 22.2.6.17 reads and what keeps the text a literal again.
            if pattern.source.is_empty() {
                units.extend("(?:)".encode_utf16());
            } else {
                units.extend_from_slice(&pattern.source);
            }
            units.push(0x2F);
            units.extend(pattern.flags.encode_utf16());
            if units.len() > self.string_units_limit {
                return Err(VMError::StringLimit);
            }
            return self.allocate_string(heap, &units);
        }
        let text = property_name_units(self.call_argument(call, 0, heap)?, heap)?;
        let matched = self.regexp_exec(receiver, &pattern, &text, heap, realm)?;
        if intrinsic == Intrinsic::RegExpPrototypeTest {
            return Ok(Value::from_bool(matched.is_some()));
        }
        // 22.2.7.2 step 18 answers null where nothing matched, and otherwise
        // the Array 22.2.7.2 builds.
        let Some(matched) = matched else {
            return Ok(VALUE_NULL);
        };
        self.match_array(&text, &matched, heap, realm)
    }

    /// `String.prototype.match` of 22.1.3.14 and `String.prototype.search` of
    /// 22.1.3.20, both through the method 22.2.6 gives a `RegExp`.
    ///
    /// The argument has to be a `RegExp` already: 22.2.3.1 would compile a
    /// pattern at run time, which this engine has not built.
    fn call_string_regexp_intrinsic(
        &mut self,
        intrinsic: Intrinsic,
        call: &Call,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let text = self.receiver_units(call.receiver, units, heap, realm)?;
        let argument = self.call_argument(call, 0, heap)?;
        // 22.1.3.13 step 5 and 22.1.3.15 step 4 make a RegExp of an argument
        // that is not one, with no flags.
        let held = argument
            .as_object()
            .and_then(|object| Self::regexp_pattern(object, heap));
        let argument = if held.is_some() {
            argument
        } else {
            if argument.is_object() {
                return Err(VMError::Unsupported("ToString of an Object"));
            }
            let source: alloc::rc::Rc<[u16]> = if argument.is_undefined() {
                alloc::rc::Rc::from(&[][..])
            } else {
                alloc::rc::Rc::from(property_name_units(argument, heap)?)
            };
            let compiled = crate::regexp::RegExp::compile(source, "").map_err(|_| {
                raise(
                    heap,
                    realm,
                    super::realm::NativeErrorKind::SyntaxError,
                    "invalid regular expression",
                )
            })?;
            let pattern = super::object::PatternRef(alloc::rc::Rc::new(compiled));
            Self::allocate_regexp(pattern, heap, realm)?
        };
        let receiver = argument.as_object().ok_or(VMError::TypeError)?;
        let pattern = Self::regexp_pattern(receiver, heap).ok_or(VMError::TypeError)?;
        if intrinsic == Intrinsic::StringPrototypeSearch {
            // 22.2.6.12 searches from the start and leaves `lastIndex` as it
            // found it.
            let key = PropertyKey::String(heap.strings.intern("lastIndex")?);
            let held = heap
                .lookup_named(receiver, key)?
                .map_or(VALUE_UNDEFINED, |property| property.value);
            Self::set_last_index(receiver, Value::from_smi(0), heap, realm)?;
            let matched = self.regexp_exec(receiver, &pattern, &text, heap, realm)?;
            Self::set_last_index(receiver, held, heap, realm)?;
            let Some(matched) = matched else {
                return Ok(Value::from_smi(-1));
            };
            let start = i32::try_from(matched.range.start).map_err(|_| VMError::StringLimit)?;
            return Ok(Value::from_smi(start));
        }
        // 22.2.6.8: a pattern without `g` answers what 22.2.7.2 answers.
        if !pattern.global {
            let matched = self.regexp_exec(receiver, &pattern, &text, heap, realm)?;
            let Some(matched) = matched else {
                return Ok(VALUE_NULL);
            };
            return self.match_array(&text, &matched, heap, realm);
        }
        self.global_match(receiver, &pattern, &text, heap, realm)
    }

    /// Step 8 of 22.2.6.8: a global pattern walks the whole text from index
    /// zero and answers the matched substrings alone.
    ///
    /// The ranges are collected before anything is allocated, so no String of
    /// a match is held unrooted while the next one is made.
    fn global_match(
        &mut self,
        receiver: ObjectRef,
        pattern: &crate::regexp::RegExp,
        text: &[u16],
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        Self::set_last_index(receiver, Value::from_smi(0), heap, realm)?;
        let mut parts: Vec<Option<&[u16]>> = Vec::new();
        loop {
            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
            let Some(matched) = self.regexp_exec(receiver, pattern, text, heap, realm)? else {
                break;
            };
            let part = text
                .get(matched.range.clone())
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            parts.push(Some(part));
            if parts.len() > self.string_units_limit {
                return Err(VMError::PropertyLimit);
            }
            // Step 8.f.iii: an empty match advances by one code unit, which
            // `AdvanceStringIndex` of 22.2.7.3 does.
            if matched.range.is_empty() {
                let next = i32::try_from(matched.range.end.saturating_add(1))
                    .map_err(|_| VMError::StringLimit)?;
                Self::set_last_index(receiver, Value::from_smi(next), heap, realm)?;
            }
        }
        if parts.is_empty() {
            return Ok(VALUE_NULL);
        }
        self.split_result(&parts, heap, realm)
    }

    /// Writes `lastIndex`, which 22.2.6.1 keeps writable and neither
    /// enumerable nor configurable.
    fn set_last_index(
        receiver: ObjectRef,
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let key = PropertyKey::String(heap.strings.intern("lastIndex")?);
        // 7.3.4 asks 10.1.9.1 to write it and throws where that refuses,
        // which an own `lastIndex` that is not writable does.
        let writable = heap
            .own_named_flags(receiver, key)?
            .is_none_or(|flags| flags.writable && !flags.is_accessor);
        if !writable {
            return Err(type_error(heap, realm, "lastIndex is not writable"));
        }
        heap.define_own_named(
            receiver,
            key,
            value,
            PropertyFlags {
                writable: true,
                enumerable: false,
                configurable: false,
                is_accessor: false,
            },
        )?;
        Ok(())
    }

    /// `Function.prototype.toString` of 20.2.3.5.
    ///
    /// A function a Script wrote answers the text of the grammar node it was
    /// written as, which the unit kept beside its code. Every other callable
    /// answers the `NativeFunction` string of step 3.
    fn function_source(
        &self,
        receiver: Value,
        units: CodeUnits<'_>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let Some(object) = receiver.as_object() else {
            return Err(type_error(heap, realm, "value is not callable"));
        };
        let kind = heap
            .get_object(object)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .kind
            .clone();
        if let ObjectKind::Function { unit, code_id, .. } = kind
            && let Some(source) = units
                .table
                .root(unit)
                .and_then(|root| root.functions.get(code_id as usize))
                .and_then(|code| {
                    code.source
                        .and_then(|index| code.string_constants.get(index as usize))
                })
        {
            let text = source.clone();
            return self.allocate_string(heap, &text);
        }
        if !Self::is_callable(receiver, heap) {
            return Err(type_error(heap, realm, "value is not callable"));
        }
        // 10.4.1 is a `NativeFunction` string without a name: the "bound f"
        // of 20.2.3.2 is not the name of a grammar node.
        if matches!(kind, ObjectKind::BoundFunction { .. }) {
            let text: Vec<u16> = "function () { [native code] }".encode_utf16().collect();
            return self.allocate_string(heap, &text);
        }
        // Step 3: a function no grammar node of a Script produced answers a
        // `NativeFunction` string, which carries its name where it has one.
        let key = PropertyKey::String(heap.strings.intern("name")?);
        let name = heap
            .own_named_flags(object, key)?
            .filter(|flags| !flags.is_accessor)
            .and(heap.lookup_named(object, key)?)
            .map(|property| property.value)
            .filter(|value| value.is_string())
            .and_then(|value| heap.strings.to_utf16(value))
            .unwrap_or_default();
        let mut text: Vec<u16> = "function ".encode_utf16().collect();
        text.extend_from_slice(&name);
        text.extend("() { [native code] }".encode_utf16());
        self.allocate_string(heap, &text)
    }

    /// `RegExpBuiltinExec` of 22.2.7.2, without the result it builds.
    ///
    /// A global or sticky pattern reads where to start from `lastIndex` and
    /// writes where it stopped back to it.
    fn regexp_exec(
        &mut self,
        receiver: ObjectRef,
        pattern: &crate::regexp::RegExp,
        text: &[u16],
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<audhsos_regex::Match>, VMError> {
        let stateful = pattern.global || pattern.sticky;
        let key = PropertyKey::String(heap.strings.intern("lastIndex")?);
        let held = heap
            .lookup_named(receiver, key)?
            .map_or(VALUE_UNDEFINED, |property| property.value);
        let index = integer_argument(held, heap, realm)?;
        let from = if !stateful || index <= 0 {
            0
        } else {
            usize::try_from(index)
                .unwrap_or(usize::MAX)
                .min(text.len().saturating_add(1))
        };
        let limits = audhsos_regex::Limits {
            input_units: self.string_units_limit,
            work: self.fuel,
            ..audhsos_regex::Limits::default()
        };
        let report = pattern
            .regex
            .find(text, from, pattern.sticky, limits)
            .map_err(|_| VMError::Unsupported("a RegExp beyond the limits of the automaton"))?;
        self.fuel = self.fuel.saturating_sub(report.work);
        if stateful {
            let end = report.matched.as_ref().map_or(0, |found| found.range.end);
            let end = i32::try_from(end).map_err(|_| VMError::StringLimit)?;
            Self::set_last_index(receiver, Value::from_smi(end), heap, realm)?;
        }
        Ok(report.matched)
    }

    /// The Array 22.2.7.2 answers: the whole match, then each capture, with an
    /// own `index` and `input`.
    fn match_array(
        &self,
        text: &[u16],
        found: &audhsos_regex::Match,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let count = u32::try_from(found.captures.len().saturating_add(1))
            .map_err(|_| VMError::PropertyLimit)?;
        let array = realm.array(heap, count)?;
        let whole = text
            .get(found.range.clone())
            .ok_or(VMError::Heap(HeapError::InvalidReference))?
            .to_vec();
        let value = self.allocate_string(heap, &whole)?;
        heap.set_array_element(array, 0, value)?;
        for (position, capture) in found.captures.iter().enumerate() {
            let index =
                u32::try_from(position.saturating_add(1)).map_err(|_| VMError::PropertyLimit)?;
            let value = match capture {
                Some(range) => {
                    let units = text
                        .get(range.clone())
                        .ok_or(VMError::Heap(HeapError::InvalidReference))?
                        .to_vec();
                    self.allocate_string(heap, &units)?
                }
                None => VALUE_UNDEFINED,
            };
            heap.set_array_element(array, index, value)?;
        }
        let start = i32::try_from(found.range.start).map_err(|_| VMError::StringLimit)?;
        let key = PropertyKey::String(heap.strings.intern("index")?);
        heap.define_own_named(
            array,
            key,
            Value::from_smi(start),
            PropertyFlags::ordinary_data(),
        )?;
        let input = self.allocate_string(heap, text)?;
        let key = PropertyKey::String(heap.strings.intern("input")?);
        heap.define_own_named(array, key, input, PropertyFlags::ordinary_data())?;
        Ok(Value::from_object(array))
    }

    /// `RegExpCreate` of 22.2.4.1 for a pattern this unit compiled.
    ///
    /// 22.2.7 gives the instance a `lastIndex` of its own, which is where
    /// 22.2.7.2 reads and writes the position a stateful match starts from.
    fn create_regexp(
        code: &BytecodeFunction,
        index: u16,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let pattern = code
            .regex_constants
            .get(usize::from(index))
            .ok_or(VMError::InvalidRegister)?;
        let pattern = super::object::PatternRef(alloc::rc::Rc::clone(pattern));
        Self::allocate_regexp(pattern, heap, realm)
    }

    /// `RegExp` of 22.2.3.1, with the `RegExpInitialize` of 22.2.3.3.
    ///
    /// A pattern that is an `Object` and not a `RegExp` would go through
    /// `ToString`, which runs a method of the Script; this native has no frame
    /// for one and names the gap.
    fn construct_regexp(
        &self,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let pattern = self.call_argument(call, 0, heap)?;
        let flags = self.call_argument(call, 1, heap)?;
        // Step 3: a RegExp pattern gives its own source, and its flags where
        // the call passed none.
        let held = pattern
            .as_object()
            .and_then(|object| Self::regexp_pattern(object, heap));
        let (source, text) = if let Some(held) = held {
            // Step 1.c: `RegExp(re)` without `new` and without flags answers
            // the same object.
            if call.construct.is_none() && flags.is_undefined() {
                return Ok(pattern);
            }
            let flags = if flags.is_undefined() {
                held.flags.clone()
            } else {
                Self::flag_units(flags, heap, realm)?
            };
            (alloc::rc::Rc::clone(&held.source), flags)
        } else {
            if pattern.is_object() {
                return Err(VMError::Unsupported("ToString of an Object"));
            }
            let source: alloc::rc::Rc<[u16]> = if pattern.is_undefined() {
                alloc::rc::Rc::from(&[][..])
            } else {
                alloc::rc::Rc::from(property_name_units(pattern, heap)?)
            };
            (source, Self::flag_units(flags, heap, realm)?)
        };
        let compiled = crate::regexp::RegExp::compile(source, &text).map_err(|_| {
            raise(
                heap,
                realm,
                super::realm::NativeErrorKind::SyntaxError,
                "invalid regular expression",
            )
        })?;
        let pattern = super::object::PatternRef(alloc::rc::Rc::new(compiled));
        Self::allocate_regexp(pattern, heap, realm)
    }

    /// The flags 22.2.3.1 was given, as the text `RegExp::compile` reads.
    fn flag_units(
        flags: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<alloc::string::String, VMError> {
        if flags.is_undefined() {
            return Ok(alloc::string::String::new());
        }
        if flags.is_object() {
            return Err(VMError::Unsupported("ToString of an Object"));
        }
        let units = property_name_units(flags, heap)?;
        alloc::string::String::from_utf16(&units).map_err(|_| {
            raise(
                heap,
                realm,
                super::realm::NativeErrorKind::SyntaxError,
                "invalid flags",
            )
        })
    }

    /// The flag character 22.2.6 gives one accessor of `%RegExp.prototype%`.
    const fn flag_character(intrinsic: Intrinsic) -> Option<u16> {
        Some(match intrinsic {
            Intrinsic::RegExpPrototypeHasIndices => 100,
            Intrinsic::RegExpPrototypeGlobal => 103,
            Intrinsic::RegExpPrototypeIgnoreCase => 105,
            Intrinsic::RegExpPrototypeMultiline => 109,
            Intrinsic::RegExpPrototypeDotAll => 115,
            Intrinsic::RegExpPrototypeUnicode => 117,
            Intrinsic::RegExpPrototypeUnicodeSets => 118,
            Intrinsic::RegExpPrototypeSticky => 121,
            _ => return None,
        })
    }

    /// What an accessor of 22.2.6 answers for a receiver with no
    /// `[[OriginalFlags]]`.
    ///
    /// Step 3 of each answers undefined for `%RegExp.prototype%` itself, which
    /// is an ordinary object, and a `TypeError` for every other one.
    fn regexp_slot_absent(
        receiver: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        if receiver.as_object() == realm.regexp_prototype(heap)?.as_object() {
            return Ok(());
        }
        Err(type_error(heap, realm, "this value is not a RegExp"))
    }

    /// One flag accessor of 22.2.6.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` for a receiver that is
    /// neither a `RegExp` nor `%RegExp.prototype%`.
    fn regexp_flag(
        intrinsic: Intrinsic,
        receiver: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let unit = Self::flag_character(intrinsic).ok_or(VMError::InvalidFeedbackVector)?;
        let pattern = receiver
            .as_object()
            .and_then(|object| Self::regexp_pattern(object, heap));
        let Some(pattern) = pattern else {
            Self::regexp_slot_absent(receiver, heap, realm)?;
            return Ok(VALUE_UNDEFINED);
        };
        Ok(Value::from_bool(
            pattern.flags.encode_utf16().any(|flag| flag == unit),
        ))
    }

    /// `get source` of 22.2.6.13, with the escape of 22.2.6.13.1.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` for a receiver that is
    /// neither a `RegExp` nor `%RegExp.prototype%`.
    fn regexp_source(
        &self,
        receiver: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let pattern = receiver
            .as_object()
            .and_then(|object| Self::regexp_pattern(object, heap));
        let Some(pattern) = pattern else {
            Self::regexp_slot_absent(receiver, heap, realm)?;
            // Step 3.a answers the pattern 22.2.6.13.1 gives an empty source.
            return self.allocate_string(heap, &"(?:)".encode_utf16().collect::<Vec<u16>>());
        };
        let units: Vec<u16> = if pattern.source.is_empty() {
            "(?:)".encode_utf16().collect()
        } else {
            pattern.source.to_vec()
        };
        self.allocate_string(heap, &units)
    }

    /// `get flags` of 22.2.6.4.
    ///
    /// The clause reads the eight flag accessors off the receiver with 7.3.2,
    /// so an object that is no `RegExp` answers the flags its own properties
    /// carry. A getter of the Script needs a frame this native has none of.
    ///
    /// # Errors
    ///
    /// Returns [`VMError::Thrown`] with a `TypeError` for a receiver that is
    /// not an Object.
    fn regexp_flags(
        &self,
        receiver: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let Some(object) = receiver.as_object() else {
            return Err(type_error(heap, realm, "this value is not an object"));
        };
        let mut units: Vec<u16> = Vec::new();
        for intrinsic in super::realm::REGEXP_FLAG_ACCESSORS {
            let unit = Self::flag_character(intrinsic).ok_or(VMError::InvalidFeedbackVector)?;
            let name = intrinsic
                .name()
                .strip_prefix("get ")
                .ok_or(VMError::InvalidFeedbackVector)?;
            let key = PropertyKey::String(heap.strings.intern(name)?);
            let found = heap.lookup_named(object, key)?;
            let value = match found {
                None => VALUE_UNDEFINED,
                Some(property) if !property.flags.is_accessor => property.value,
                Some(property) => {
                    let (get, _) = Self::accessor_parts(property.value, heap)?;
                    if !Self::is_intrinsic(get, intrinsic, heap) {
                        return Err(VMError::Unsupported("a property that is an accessor"));
                    }
                    Self::regexp_flag(intrinsic, receiver, heap, realm)?
                }
            };
            if Self::to_boolean(value, heap)? {
                units.push(unit);
            }
        }
        self.allocate_string(heap, &units)
    }

    /// `RegExpAlloc` of 22.2.3.2 with the `lastIndex` 22.2.3.3 initializes.
    fn allocate_regexp(
        pattern: super::object::PatternRef,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let prototype = realm.regexp_prototype(heap)?;
        let shape = heap.shapes.root_shape();
        let object = heap.allocate_object(shape, prototype)?;
        heap.set_object_kind(object, ObjectKind::RegExp { pattern })?;
        let key = PropertyKey::String(heap.strings.intern("lastIndex")?);
        heap.define_own_named(
            object,
            key,
            Value::from_smi(0),
            PropertyFlags {
                writable: true,
                enumerable: false,
                configurable: false,
                is_accessor: false,
            },
        )?;
        Ok(Value::from_object(object))
    }

    /// The pattern a `RegExp` instance was made from.
    fn regexp_pattern(
        object: ObjectRef,
        heap: &GenerationalHeap,
    ) -> Option<alloc::rc::Rc<crate::regexp::RegExp>> {
        let ObjectKind::RegExp { pattern } = &heap.get_object(object)?.kind else {
            return None;
        };
        Some(alloc::rc::Rc::clone(&pattern.0))
    }

    /// `HasProperty` of 7.3.11: the own property, then the Prototype Chain.
    fn has_property(
        object: ObjectRef,
        name: PropertyKey,
        heap: &GenerationalHeap,
        realm: &Realm,
    ) -> Result<bool, VMError> {
        let mut current = object;
        let mut depth = 0u16;
        loop {
            if heap.own_named_flags(current, name)?.is_some() {
                return Ok(true);
            }
            let prototype = heap
                .get_object(current)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .prototype;
            let Some(next) = prototype.as_object() else {
                // A name a Prototype this Realm has not built would own is a
                // gap, and answering false would say the object has not got
                // what it has.
                let units = name
                    .as_string()
                    .and_then(|name| heap.strings.to_utf16(Value::from_string(name)))
                    .unwrap_or_default();
                Self::absent_property(Value::from_object(object), &units, heap, realm)?;
                return Ok(false);
            };
            depth = depth
                .checked_add(1)
                .ok_or(VMError::Heap(HeapError::ReferenceSpaceExhausted))?;
            current = next;
        }
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
        // 7.1.18 gives each wrapper the Prototype of its own constructor.
        let (kind, prototype) = if let Some(boolean) = value.as_boolean() {
            (
                ObjectKind::BooleanWrapper(boolean),
                realm.boolean_prototype(heap)?,
            )
        } else if let Some(number) = value.as_f64() {
            (
                ObjectKind::NumberWrapper(number),
                realm.number_prototype(heap)?,
            )
        } else if value.is_string() {
            (
                ObjectKind::StringWrapper(value),
                realm.string_prototype(heap)?,
            )
        } else if let Some(symbol) = value.as_symbol() {
            (
                ObjectKind::SymbolWrapper(symbol),
                realm.symbol_prototype(heap)?,
            )
        } else {
            return Err(type_error(heap, realm, "cannot box null or undefined"));
        };
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
        units: CodeUnits<'_>,
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
            .map(Self::plain_value)
            .transpose()?
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
                resume: None,
                construct: None,
                caller_code_id: None,
            },
            units,
            heap,
            realm,
        )?;
        let result = result
            .as_object()
            .ok_or_else(|| type_error(heap, realm, "iterator result is not an object"))?;
        let done_key = PropertyKey::String(heap.strings.intern("done")?);
        let done = heap
            .lookup_named(result, done_key)?
            .map(Self::plain_value)
            .transpose()?
            .is_some_and(Value::to_boolean);
        let value_key = PropertyKey::String(heap.strings.intern("value")?);
        let value = heap
            .lookup_named(result, value_key)?
            .map(Self::plain_value)
            .transpose()?
            .unwrap_or(VALUE_UNDEFINED);
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
    /// Builds the arguments object of the running call (10.4.4).
    ///
    /// The frame keeps the caller registers the call passed, so the object
    /// holds every argument and not only the declared parameters. It is
    /// written to `target` before its properties are, because every
    /// allocation after that is a Safe Point and the register is what the
    /// collector sees.
    ///
    /// The mapping of 10.4.4.7 is not built. The lowering only takes a body
    /// where the mapping cannot be observed: one that assigns no parameter and
    /// uses `arguments` for nothing but reading a property of it.
    fn create_arguments(
        &mut self,
        code: &BytecodeFunction,
        target: Reg,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let frame = *self.frames.last().ok_or(VMError::InvalidRegister)?;
        let arguments = frame.arguments.ok_or(VMError::Unsupported(
            "the arguments object of a call the engine did not open",
        ))?;
        let count = usize::from(arguments.count);
        if count.saturating_add(2) > self.property_limit {
            return Err(VMError::PropertyLimit);
        }
        // The values are taken before anything is allocated: 20.2.3.1 and
        // 28.1.1 passed a List no register of the caller holds, and every
        // other call passed registers the allocation below does not move.
        let mut passed: Vec<Value> = Vec::with_capacity(count);
        // A walk of 23.1.3 has no frame of a caller either: the arguments of
        // its callback come from the state the collector traces.
        if let Some(Resume::Iteration { state }) = frame.resume {
            let passed_in = Self::iteration_arguments(state, heap)?;
            for index in 0..count.min(passed_in.len()) {
                passed.push(*passed_in.get(index).unwrap_or(&VALUE_UNDEFINED));
            }
        } else if let Some(list) = frame.resume.and_then(Resume::list) {
            let list = heap
                .root_value(list)
                .and_then(Value::as_object)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
            let store = heap
                .get_object(list)
                .ok_or(VMError::Heap(HeapError::InvalidReference))?
                .elements
                .and_then(|elements| heap.get_elements(elements));
            for index in 0..arguments.count {
                passed.push(
                    store
                        .and_then(|store| store.get(u32::from(index)))
                        .unwrap_or(VALUE_UNDEFINED),
                );
            }
        } else {
            for index in 0..arguments.count {
                let slot = frame
                    .caller_fp
                    .checked_add(arguments.start.0 as usize)
                    .and_then(|start| start.checked_add(index as usize))
                    .ok_or(VMError::InvalidRegister)?;
                passed.push(
                    self.stack
                        .get(slot)
                        .copied()
                        .ok_or(VMError::InvalidRegister)?,
                );
            }
        }
        let root_shape = heap.shapes.root_shape();
        let object = self.allocate_object(code, heap, realm, root_shape)?;
        heap.set_object_kind(object, ObjectKind::Arguments)?;
        self.write_reg(target, Value::from_object(object))?;
        for (index, argument) in passed.into_iter().enumerate() {
            let name = alloc::format!("{index}");
            let key = PropertyKey::String(
                heap.strings
                    .intern_units(&name.encode_utf16().collect::<alloc::vec::Vec<u16>>())?,
            );
            self.define_own(target, key, PropertyFlags::ordinary_data(), argument, heap)?;
        }
        let length = Value::from_f64(f64::from(arguments.count));
        let key = PropertyKey::String(heap.strings.intern_units(&LENGTH_NAME)?);
        self.define_own(target, key, PropertyFlags::constructor_data(), length, heap)?;
        // 10.4.4 gives the object the iterator of 23.1.3.33.
        let values = realm.intrinsic(heap, Intrinsic::ArrayPrototypeValues)?;
        self.define_own(
            target,
            super::realm::WellKnownSymbol::Iterator.key(),
            PropertyFlags::constructor_data(),
            values,
            heap,
        )?;
        // 10.4.4 step 7: a strict function's `callee` is the accessor of
        // 10.2.4.1 on both halves, and nothing else can be read out of it.
        if code.strict {
            let throws = realm.intrinsic(heap, Intrinsic::ThrowTypeError)?;
            let pair = Self::make_accessor(throws, throws, heap)?;
            let key = PropertyKey::String(heap.strings.intern_units(&CALLEE_NAME)?);
            self.define_own(
                target,
                key,
                PropertyFlags {
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    is_accessor: true,
                },
                pair,
                heap,
            )?;
            return Ok(());
        }
        // 10.4.4 step 8 gives a sloppy function's object the callee itself,
        // which the caller kept in a register of its own.
        let callee = self
            .stack
            .get(
                frame
                    .caller_fp
                    .checked_add(arguments.callee.0 as usize)
                    .ok_or(VMError::InvalidRegister)?,
            )
            .copied()
            .ok_or(VMError::InvalidRegister)?;
        let key = PropertyKey::String(heap.strings.intern_units(&CALLEE_NAME)?);
        self.define_own(target, key, PropertyFlags::constructor_data(), callee, heap)?;
        Ok(())
    }

    /// Adds one own property to an object this frame holds in `holder`.
    ///
    /// The object is read from the register again, because interning the name
    /// is a Safe Point that can forward it.
    fn define_own(
        &self,
        holder: Reg,
        name: PropertyKey,
        flags: PropertyFlags,
        value: Value,
        heap: &mut GenerationalHeap,
    ) -> Result<(), VMError> {
        let object = self
            .read_reg(holder)?
            .as_object()
            .ok_or(VMError::TypeError)?;
        let shape = heap.get_object(object).ok_or(VMError::TypeError)?.shape_id;
        let (next, slot) = heap.shapes.transition(shape, name, flags);
        heap.set_object_shape(object, next)?;
        heap.set_object_slot(object, slot, value)?;
        Ok(())
    }

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
            let enumerated = self.read_reg(object_slot)?;
            let Some(object) = enumerated.as_object() else {
                // 14.7.5.6 step 2: undefined and null enumerate nothing. Every
                // other primitive needs the ToObject of 7.1.18, which this
                // engine has no wrapper Object for.
                if enumerated.is_undefined() || enumerated.is_null() {
                    return Ok(VALUE_UNDEFINED);
                }
                return Err(VMError::Unsupported(
                    "ToObject of a primitive in a for-in head",
                ));
            };
            let Some(keys) = self.read_reg(keys_slot)?.as_object() else {
                // 14.7.5.9 enumerates String keys only, so the Symbol keys of
                // 10.1.11.1 are dropped before the level's Array is sized.
                let length = u32::try_from(
                    heap.own_keys(object)?
                        .into_iter()
                        .filter(|(name, _)| name.as_string().is_some())
                        .count(),
                )
                .map_err(|_| VMError::PropertyLimit)?;
                // Allocating the Array is a Safe Point, so the object and its
                // keys are read again after it and this level starts over.
                let keys = self.allocate_array(code, heap, realm, length)?;
                self.write_reg(keys_slot, Value::from_object(keys))?;
                self.write_reg(index_slot, Value::from_smi(0))?;
                let object = self
                    .read_reg(object_slot)?
                    .as_object()
                    .ok_or(VMError::TypeError)?;
                for (index, name) in heap
                    .own_keys(object)?
                    .into_iter()
                    .filter_map(|(name, _)| name.as_string())
                    .enumerate()
                {
                    let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
                    heap.set_array_element(keys, index, Value::from_string(name))?;
                }
                continue;
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

    /// 27.2.4.1, 27.2.4.2 and 27.2.4.5, which differ only in what each element
    /// is given as its pair of handlers.
    ///
    /// Steps 3 to 6 stand inside one guard: a value thrown while the iterable
    /// is read or an element is started rejects the capability rather than
    /// reaching the caller.
    fn promise_combinator(
        &self,
        intrinsic: Intrinsic,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if !Self::is_intrinsic(call.receiver, Intrinsic::PromiseConstructor, heap) {
            if call.receiver.as_object().is_none() {
                return Err(type_error(
                    heap,
                    realm,
                    "a combinator of 27.2.4 called on a value that is no Object",
                ));
            }
            return Err(VMError::Unsupported(
                "a combinator of 27.2.4 on a constructor that is not %Promise%",
            ));
        }
        let capability = promise::capability(heap, realm)?;
        let argument = self.call_argument(call, 0, heap)?;
        match self.start_elements(intrinsic, argument, capability, heap, realm) {
            Ok(()) => {}
            Err(VMError::Thrown(value, _)) => {
                let reject = promise::slot(heap, capability, promise::CAPABILITY_REJECT);
                self.settle_through(reject, value, true, heap, realm)?;
            }
            Err(error) => return Err(error),
        }
        Ok(promise::slot(heap, capability, promise::CAPABILITY_PROMISE))
    }

    /// Steps 3 to 6 of the three combinators.
    fn start_elements(
        &self,
        intrinsic: Intrinsic,
        argument: Value,
        capability: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        // Step 3 reads `resolve` once, before the iterable is read.
        let constructor = realm
            .intrinsic(heap, Intrinsic::PromiseConstructor)?
            .as_object()
            .ok_or(VMError::TypeError)?;
        let key = PropertyKey::String(heap.strings.intern("resolve")?);
        let resolve = match heap.lookup_named(constructor, key)? {
            Some(found) => Self::plain_value(found)?,
            None => VALUE_UNDEFINED,
        };
        if !Self::is_intrinsic(resolve, Intrinsic::PromiseResolve, heap) {
            return Err(VMError::Unsupported(
                "a `resolve` that is not %Promise.resolve%",
            ));
        }
        let elements = Self::iterable_elements(argument, heap, realm)?;
        let values = Value::from_object(realm.array(heap, 0)?);
        let group = promise::record(heap, realm, &[values, capability, Value::from_smi(1)])?;
        for (index, element) in elements.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| VMError::PropertyLimit)?;
            if intrinsic != Intrinsic::PromiseRace {
                promise::set_slot(heap, values, index, VALUE_UNDEFINED)?;
            }
            promise::set_slot(
                heap,
                group,
                promise::GROUP_REMAINING,
                Value::from_smi(Self::remaining(heap, group).saturating_add(1)),
            )?;
            let next = self.resolved_promise(element, heap, realm)?;
            let (on_fulfilled, on_rejected) = match intrinsic {
                Intrinsic::PromiseRace => (
                    promise::slot(heap, capability, promise::CAPABILITY_RESOLVE),
                    promise::slot(heap, capability, promise::CAPABILITY_REJECT),
                ),
                Intrinsic::PromiseAllSettled => (
                    promise::element_function(
                        heap,
                        realm,
                        Intrinsic::PromiseAllSettledFulfilled,
                        group,
                        index,
                    )?,
                    promise::element_function(
                        heap,
                        realm,
                        Intrinsic::PromiseAllSettledRejected,
                        group,
                        index,
                    )?,
                ),
                _ => (
                    promise::element_function(
                        heap,
                        realm,
                        Intrinsic::PromiseAllElement,
                        group,
                        index,
                    )?,
                    promise::slot(heap, capability, promise::CAPABILITY_REJECT),
                ),
            };
            // Step 6.q calls `then` of the promise the element resolved to,
            // which is observable and so must be the one of this Realm.
            let object = next.as_object().ok_or(VMError::TypeError)?;
            let key = PropertyKey::String(heap.strings.intern("then")?);
            let then = match heap.lookup_named(object, key)? {
                Some(found) => Self::plain_value(found)?,
                None => VALUE_UNDEFINED,
            };
            if !Self::is_intrinsic(then, Intrinsic::PromisePrototypeThen, heap) {
                return Err(VMError::Unsupported(
                    "a `then` that is not %Promise.prototype.then%",
                ));
            }
            self.perform_then(
                next,
                on_fulfilled,
                on_rejected,
                VALUE_UNDEFINED,
                heap,
                realm,
            )?;
        }
        if intrinsic == Intrinsic::PromiseRace {
            return Ok(());
        }
        let remaining = Self::remaining(heap, group).saturating_sub(1);
        promise::set_slot(
            heap,
            group,
            promise::GROUP_REMAINING,
            Value::from_smi(remaining),
        )?;
        if remaining == 0 {
            let resolve = promise::slot(heap, capability, promise::CAPABILITY_RESOLVE);
            self.settle_through(resolve, values, false, heap, realm)?;
        }
        Ok(())
    }

    /// `[[RemainingElements]]` of the shared record.
    fn remaining(heap: &GenerationalHeap, group: Value) -> i32 {
        promise::slot(heap, group, promise::GROUP_REMAINING)
            .as_smi()
            .unwrap_or(0)
    }

    /// 27.2.4.7.1 for `%Promise%`: the value itself where it is already a
    /// promise of this Realm, and a new promise resolved with it otherwise.
    fn resolved_promise(
        &self,
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if promise::state_of(value, heap).is_some() {
            let object = value.as_object().ok_or(VMError::TypeError)?;
            let key = PropertyKey::String(heap.strings.intern("constructor")?);
            let constructor = match heap.lookup_named(object, key)? {
                Some(found) => Self::plain_value(found)?,
                None => VALUE_UNDEFINED,
            };
            if Self::is_intrinsic(constructor, Intrinsic::PromiseConstructor, heap) {
                return Ok(value);
            }
        }
        let capability = promise::capability(heap, realm)?;
        let resolve = promise::slot(heap, capability, promise::CAPABILITY_RESOLVE);
        self.settle_through(resolve, value, false, heap, realm)?;
        Ok(promise::slot(heap, capability, promise::CAPABILITY_PROMISE))
    }

    /// Every element of the iterable a combinator of 27.2.4 was given.
    ///
    /// 7.4.2 calls `@@iterator` and 7.4.4 calls `next` for each element; both
    /// are methods of the Script that a native has no frame to call. An Array
    /// whose two methods are the ones of this Realm answers its elements
    /// without either call, and every other iterable is a named gap.
    fn iterable_elements(
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Vec<Value>, VMError> {
        // 22.1.3.34 makes a String iterable, and this Realm has not built it.
        if value.is_string() {
            return Err(VMError::Unsupported("a property of %String.prototype%"));
        }
        let Some(object) = value.as_object() else {
            return Err(type_error(heap, realm, "the argument is not iterable"));
        };
        let method =
            match heap.lookup_named(object, super::realm::WellKnownSymbol::Iterator.key())? {
                Some(found) => Self::plain_value(found)?,
                None => VALUE_UNDEFINED,
            };
        if method.is_undefined() || method.is_null() {
            return Err(type_error(heap, realm, "the argument is not iterable"));
        }
        let iterates = Self::is_intrinsic(method, Intrinsic::ArrayPrototypeValues, heap)
            && matches!(
                heap.get_object(object).map(|entry| &entry.kind),
                Some(&ObjectKind::Array { .. })
            );
        if !iterates {
            return Err(VMError::Unsupported(
                "an iterable that is no Array of this Realm",
            ));
        }
        // 23.1.5.2.1 answers each element through `next`, so the method that
        // reads it must be the one of this Realm as well.
        let iterator_prototype = realm
            .array_iterator_prototype(heap)?
            .as_object()
            .ok_or(VMError::TypeError)?;
        let key = PropertyKey::String(heap.strings.intern("next")?);
        let next = match heap.lookup_named(iterator_prototype, key)? {
            Some(found) => Self::plain_value(found)?,
            None => VALUE_UNDEFINED,
        };
        if !Self::is_intrinsic(next, Intrinsic::ArrayIteratorPrototypeNext, heap) {
            return Err(VMError::Unsupported(
                "a `next` that is not %ArrayIteratorPrototype%.next",
            ));
        }
        let Some(&ObjectKind::Array { length, .. }) = heap.get_object(object).map(|e| &e.kind)
        else {
            return Err(VMError::TypeError);
        };
        let mut elements = Vec::new();
        for index in 0..length {
            elements.push(Self::element_at(heap, object, index)?.unwrap_or(VALUE_UNDEFINED));
        }
        Ok(elements)
    }

    /// 27.2.4.1.3, 27.2.4.2.2 and 27.2.4.2.3: one element of a combinator has
    /// settled.
    fn settle_element(
        &self,
        intrinsic: Intrinsic,
        function: Value,
        value: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let state = Self::native_state(function, heap).ok_or(VMError::TypeError)?;
        if promise::slot(heap, state, promise::ELEMENT_CALLED) == VALUE_TRUE {
            return Ok(());
        }
        promise::set_slot(heap, state, promise::ELEMENT_CALLED, VALUE_TRUE)?;
        let group = promise::slot(heap, state, promise::ELEMENT_GROUP);
        let index = promise::slot(heap, state, promise::ELEMENT_INDEX)
            .as_smi()
            .and_then(|index| u32::try_from(index).ok())
            .ok_or(VMError::TypeError)?;
        let values = promise::slot(heap, group, promise::GROUP_VALUES);
        let held = match intrinsic {
            Intrinsic::PromiseAllSettledFulfilled => {
                Self::settled_record(heap, realm, "fulfilled", "value", value)?
            }
            Intrinsic::PromiseAllSettledRejected => {
                Self::settled_record(heap, realm, "rejected", "reason", value)?
            }
            _ => value,
        };
        promise::set_slot(heap, values, index, held)?;
        let remaining = Self::remaining(heap, group).saturating_sub(1);
        promise::set_slot(
            heap,
            group,
            promise::GROUP_REMAINING,
            Value::from_smi(remaining),
        )?;
        if remaining == 0 {
            let capability = promise::slot(heap, group, promise::GROUP_CAPABILITY);
            let resolve = promise::slot(heap, capability, promise::CAPABILITY_RESOLVE);
            self.settle_through(resolve, values, false, heap, realm)?;
        }
        Ok(())
    }

    /// The object 27.2.4.2.2 step 9 and 27.2.4.2.3 step 9 build for one
    /// element.
    fn settled_record(
        heap: &mut GenerationalHeap,
        realm: &Realm,
        status: &str,
        field: &str,
        value: Value,
    ) -> Result<Value, VMError> {
        let object = realm.ordinary_object(heap)?;
        let flags = PropertyFlags {
            writable: true,
            enumerable: true,
            configurable: true,
            is_accessor: false,
        };
        let text = heap.strings.allocate_str(status)?;
        let key = PropertyKey::String(heap.strings.intern("status")?);
        heap.define_own_named(object, key, Value::from_string(text), flags)?;
        let key = PropertyKey::String(heap.strings.intern(field)?);
        heap.define_own_named(object, key, value, flags)?;
        Ok(Value::from_object(object))
    }

    /// 27.2.4.9: the promise and the pair that settles it, in one object.
    fn promise_with_resolvers(
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let capability = promise::capability(heap, realm)?;
        let object = realm.ordinary_object(heap)?;
        let flags = PropertyFlags {
            writable: true,
            enumerable: true,
            configurable: true,
            is_accessor: false,
        };
        for (name, index) in [
            ("promise", promise::CAPABILITY_PROMISE),
            ("resolve", promise::CAPABILITY_RESOLVE),
            ("reject", promise::CAPABILITY_REJECT),
        ] {
            let value = promise::slot(heap, capability, index);
            let key = PropertyKey::String(heap.strings.intern(name)?);
            heap.define_own_named(object, key, value, flags)?;
        }
        Ok(Value::from_object(object))
    }

    /// `print`: the run stops, the embedding writes the line, and the call
    /// instruction runs again with nothing to write.
    fn print_line(
        &mut self,
        call: &Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if self.compiled_unit.take().is_some() {
            return Ok(VALUE_UNDEFINED);
        }
        let argument = self.call_argument(call, 0, heap)?;
        let text = self.primitive_string(argument, heap, realm)?;
        let units = heap
            .strings
            .to_utf16(text)
            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
        self.pending_print = true;
        self.pending_source = Some(alloc::rc::Rc::from(units));
        self.resume_pc = call.return_pc.saturating_sub(1);
        self.resume_code_id = call.caller_code_id;
        Ok(VALUE_UNDEFINED)
    }

    /// The job queue of 9.5 this run enqueues into.
    fn job_queue(&self, heap: &GenerationalHeap) -> Value {
        self.jobs
            .and_then(|root| heap.root_value(root))
            .unwrap_or(VALUE_UNDEFINED)
    }

    /// What a closure of the specification written in Rust carries besides its
    /// code (27.2.1.3.1).
    fn native_state(value: Value, heap: &GenerationalHeap) -> Option<Value> {
        let object = value.as_object()?;
        match heap.get_object(object)?.kind {
            ObjectKind::NativeFunction { state, .. } => Some(state),
            _ => None,
        }
    }

    /// 27.2.3.1: `new Promise(executor)`, which calls the executor at once
    /// with the pair of 27.2.1.3 and answers the promise whatever it does.
    fn begin_promise(
        &mut self,
        call: Call,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let target = call
            .construct
            .ok_or_else(|| type_error(heap, realm, "Promise cannot be called without new"))?;
        let executor = self.call_argument(&call, 0, heap)?;
        if !Self::is_callable(executor, heap) {
            return Err(type_error(heap, realm, "Promise executor is not callable"));
        }
        // The executor takes its two arguments from a List, which only a
        // frame of a Script function reads.
        if !Self::is_script_function(executor, heap) {
            return Err(VMError::Unsupported(
                "a Promise executor that is not a Script function",
            ));
        }
        let prototype = realm.promise_prototype(heap)?;
        let promise = Value::from_object(promise::create(heap, realm, prototype)?);
        let (resolve, reject) = promise::resolving_functions(heap, realm, promise)?;
        let state = Self::native_state(resolve, heap).ok_or(VMError::TypeError)?;
        let list = promise::record(heap, realm, &[resolve, reject])?;
        self.write_reg(target, promise)?;
        // The three outlive the frame the executor opens, so they are roots of
        // a scope of their own, which the return and the catch both leave.
        heap.enter_scope();
        let resume = Resume::Executor {
            promise: heap.push_root(promise)?,
            state: heap.push_root(state)?,
            arguments: heap.push_root(list)?,
        };
        let call = Call {
            receiver: VALUE_UNDEFINED,
            func: Reg(0),
            arg_start: Reg(0),
            arg_count: 2,
            slot: 0,
            resume: Some(resume),
            construct: None,
            return_pc: call.return_pc,
            caller_code_id: call.caller_code_id,
        };
        self.enter_call_value(executor, units, active_feedback, heap, realm, call)
    }

    /// 27.2.1.3.2 and 27.2.1.3.1: settles the promise the pair of resolving
    /// functions was made for, once.
    fn settle_through(
        &self,
        function: Value,
        value: Value,
        reject: bool,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let state = Self::native_state(function, heap).ok_or(VMError::TypeError)?;
        if promise::take_resolution(heap, state)? {
            return Ok(());
        }
        let promise = promise::slot(heap, state, promise::STATE_PROMISE);
        let queue = self.job_queue(heap);
        if reject {
            promise::settle(heap, realm, queue, promise, promise::REJECTED, value)?;
            return Ok(());
        }
        // 27.2.1.3.2 step 6: a promise resolved with itself rejects with a
        // TypeError rather than waiting for a settlement that cannot come.
        if value == promise {
            let error = realm.create_native_error(
                heap,
                super::realm::NativeErrorKind::TypeError,
                "a promise cannot be resolved with itself",
            )?;
            let error = Value::from_object(error);
            promise::settle(heap, realm, queue, promise, promise::REJECTED, error)?;
            return Ok(());
        }
        let then = Self::thenable_then(value, heap)?;
        promise::resolve(heap, realm, queue, promise, value, then)?;
        Ok(())
    }

    /// The callable `then` of 27.2.1.3.2 step 8, and nothing for a value that
    /// is no thenable.
    ///
    /// A throw of step 8 is the rejection of step 9, which the caller makes;
    /// an accessor `then` is a call of the Script that the resolution has no
    /// frame to make.
    fn thenable_then(value: Value, heap: &mut GenerationalHeap) -> Result<Option<Value>, VMError> {
        let Some(object) = value.as_object() else {
            return Ok(None);
        };
        let key = PropertyKey::String(heap.strings.intern("then")?);
        let Some(found) = heap.lookup_named(object, key)? else {
            return Ok(None);
        };
        let then = Self::plain_value(found)?;
        Ok(Self::is_callable(then, heap).then_some(then))
    }

    /// The constructor 27.2.5.4 step 3 reads, refused unless it is `%Promise%`
    /// itself: every other one makes a promise of a subclass.
    fn promise_species(promise: Value, heap: &mut GenerationalHeap) -> Result<(), VMError> {
        let object = promise.as_object().ok_or(VMError::TypeError)?;
        let key = PropertyKey::String(heap.strings.intern("constructor")?);
        let constructor = match heap.lookup_named(object, key)? {
            Some(found) => Self::plain_value(found)?,
            None => VALUE_UNDEFINED,
        };
        if !Self::is_intrinsic(constructor, Intrinsic::PromiseConstructor, heap) {
            return Err(VMError::Unsupported(
                "a Promise whose constructor is not %Promise%",
            ));
        }
        let species = match heap.lookup_named(
            constructor.as_object().ok_or(VMError::TypeError)?,
            super::realm::WellKnownSymbol::Species.key(),
        )? {
            Some(found) if !found.flags.is_accessor => Self::plain_value(found)?,
            // 27.2.4.8 is the species getter of this Realm, which answers the
            // constructor itself.
            Some(_) => constructor,
            None => VALUE_UNDEFINED,
        };
        if species != constructor {
            return Err(VMError::Unsupported(
                "a Promise whose @@species is not %Promise%",
            ));
        }
        Ok(())
    }

    /// 27.2.5.4: `Promise.prototype.then`.
    fn promise_then(
        &self,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let promise = call.receiver;
        if promise::state_of(promise, heap).is_none() {
            return Err(type_error(
                heap,
                realm,
                "Promise.prototype.then called on a value that is no Promise",
            ));
        }
        let on_fulfilled = self.call_argument(&call, 0, heap)?;
        let on_rejected = self.call_argument(&call, 1, heap)?;
        self.then_of(promise, on_fulfilled, on_rejected, heap, realm)
    }

    /// 27.2.5.4 steps 3 to 5 with the two handlers as values, which the job of
    /// 27.2.2.2 has and the registers of no caller hold.
    fn then_of(
        &self,
        promise: Value,
        on_fulfilled: Value,
        on_rejected: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if promise::state_of(promise, heap).is_none() {
            return Err(type_error(
                heap,
                realm,
                "Promise.prototype.then called on a value that is no Promise",
            ));
        }
        Self::promise_species(promise, heap)?;
        let capability = promise::capability(heap, realm)?;
        self.perform_then(promise, on_fulfilled, on_rejected, capability, heap, realm)?;
        Ok(promise::slot(heap, capability, promise::CAPABILITY_PROMISE))
    }

    /// The function of a job that is written in Rust rather than in the
    /// Script, which has no frame to take its arguments from.
    ///
    /// 27.2.2.2 reaches `%Promise.prototype.then%` whenever a promise is
    /// resolved with a promise, which is the one call this answers besides the
    /// pair of 27.2.1.3.
    fn call_native_job(
        &self,
        function: Value,
        receiver: Value,
        first: Value,
        second: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        let gap = VMError::Unsupported("a job whose handler is not a Script function");
        let Some(intrinsic) = Self::native_state(function, heap)
            .and(function.as_object())
            .and_then(|object| match heap.get_object(object)?.kind {
                ObjectKind::NativeFunction { id, .. } => Intrinsic::from_id(id),
                _ => None,
            })
        else {
            return Err(gap);
        };
        match intrinsic {
            Intrinsic::PromisePrototypeThen => self.then_of(receiver, first, second, heap, realm),
            Intrinsic::PromisePrototypeCatch => {
                self.then_of(receiver, VALUE_UNDEFINED, first, heap, realm)
            }
            Intrinsic::PromiseResolveFunction | Intrinsic::PromiseRejectFunction => {
                self.settle_through(
                    function,
                    first,
                    intrinsic == Intrinsic::PromiseRejectFunction,
                    heap,
                    realm,
                )?;
                Ok(VALUE_UNDEFINED)
            }
            Intrinsic::PromiseAllElement
            | Intrinsic::PromiseAllSettledFulfilled
            | Intrinsic::PromiseAllSettledRejected => {
                self.settle_element(intrinsic, function, first, heap, realm)?;
                Ok(VALUE_UNDEFINED)
            }
            _ => Err(gap),
        }
    }

    /// 27.2.5.1: `Promise.prototype.catch`, which 27.2.5.1 step 2 reaches
    /// through the `then` of the object it was called on.
    fn promise_catch(
        &self,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if call.receiver.is_undefined() || call.receiver.is_null() {
            return Err(type_error(
                heap,
                realm,
                "Promise.prototype.catch called on undefined or null",
            ));
        }
        // 27.2.5.1 step 2 reads `then` of the boxed receiver, which for a
        // primitive is a method of a Prototype and never this intrinsic.
        let Some(object) = call.receiver.as_object() else {
            return Err(VMError::Unsupported(
                "a `then` that is not %Promise.prototype.then%",
            ));
        };
        let key = PropertyKey::String(heap.strings.intern("then")?);
        let then = match heap.lookup_named(object, key)? {
            Some(found) => Self::plain_value(found)?,
            None => VALUE_UNDEFINED,
        };
        if !Self::is_intrinsic(then, Intrinsic::PromisePrototypeThen, heap) {
            return Err(VMError::Unsupported(
                "a `then` that is not %Promise.prototype.then%",
            ));
        }
        let on_rejected = self.call_argument(&call, 0, heap)?;
        self.then_of(call.receiver, VALUE_UNDEFINED, on_rejected, heap, realm)
    }

    /// 27.2.5.4.1: a handler that is not callable is empty, and a settled
    /// promise owes its job at once.
    fn perform_then(
        &self,
        promise: Value,
        on_fulfilled: Value,
        on_rejected: Value,
        capability: Value,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let fulfilled = if Self::is_callable(on_fulfilled, heap) {
            on_fulfilled
        } else {
            VALUE_UNDEFINED
        };
        let rejected = if Self::is_callable(on_rejected, heap) {
            on_rejected
        } else {
            VALUE_UNDEFINED
        };
        let queue = self.job_queue(heap);
        promise::react(heap, realm, queue, promise, fulfilled, rejected, capability)?;
        Ok(())
    }

    /// 27.2.4.7: `Promise.resolve`.
    fn promise_resolve(
        &self,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if !Self::is_intrinsic(call.receiver, Intrinsic::PromiseConstructor, heap) {
            if call.receiver.as_object().is_none() {
                return Err(type_error(
                    heap,
                    realm,
                    "Promise.resolve called on a value that is no Object",
                ));
            }
            return Err(VMError::Unsupported(
                "Promise.resolve of a constructor that is not %Promise%",
            ));
        }
        let value = self.call_argument(&call, 0, heap)?;
        // Step 2: a promise of this constructor is answered unchanged.
        if promise::state_of(value, heap).is_some() {
            let object = value.as_object().ok_or(VMError::TypeError)?;
            let key = PropertyKey::String(heap.strings.intern("constructor")?);
            let constructor = match heap.lookup_named(object, key)? {
                Some(found) => Self::plain_value(found)?,
                None => VALUE_UNDEFINED,
            };
            if constructor == call.receiver {
                return Ok(value);
            }
        }
        let capability = promise::capability(heap, realm)?;
        let resolve = promise::slot(heap, capability, promise::CAPABILITY_RESOLVE);
        self.settle_through(resolve, value, false, heap, realm)?;
        Ok(promise::slot(heap, capability, promise::CAPABILITY_PROMISE))
    }

    /// 27.2.4.6: `Promise.reject`.
    fn promise_reject(
        &self,
        call: Call,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Value, VMError> {
        if !Self::is_intrinsic(call.receiver, Intrinsic::PromiseConstructor, heap) {
            if call.receiver.as_object().is_none() {
                return Err(type_error(
                    heap,
                    realm,
                    "Promise.reject called on a value that is no Object",
                ));
            }
            return Err(VMError::Unsupported(
                "Promise.reject of a constructor that is not %Promise%",
            ));
        }
        let value = self.call_argument(&call, 0, heap)?;
        let capability = promise::capability(heap, realm)?;
        let reject = promise::slot(heap, capability, promise::CAPABILITY_REJECT);
        self.settle_through(reject, value, true, heap, realm)?;
        Ok(promise::slot(heap, capability, promise::CAPABILITY_PROMISE))
    }

    /// The next job of 9.5, taken from the queue and held for the frame it
    /// opens.
    fn take_job(&mut self, heap: &mut GenerationalHeap) -> Result<Option<Value>, VMError> {
        let queue = self.job_queue(heap);
        let length = promise::length_of(heap, queue);
        if self.job_head >= length {
            // Every job the queue held has run, so the slots holding them are
            // released before the next one is enqueued.
            if length > 0 {
                let object = queue.as_object().ok_or(VMError::TypeError)?;
                heap.set_array_length(object, 0)?;
            }
            self.job_head = 0;
            self.clear_job(heap)?;
            return Ok(None);
        }
        let job = promise::slot(heap, queue, self.job_head);
        self.job_head = self.job_head.saturating_add(1);
        if let Some(root) = self.running_job {
            heap.set_root(root, job)?;
        }
        Ok(Some(job))
    }

    /// Settles the capability of the job whose handler has answered.
    fn settle_job(
        &self,
        job: Value,
        value: Value,
        threw: bool,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(), VMError> {
        let kind = promise::slot(heap, job, promise::JOB_KIND)
            .as_smi()
            .unwrap_or(0);
        if kind == promise::JOB_THENABLE {
            // 27.2.2.2 step 2: only a throw of `then` is answered, through the
            // pair the job was given.
            if threw {
                let reject = promise::slot(heap, job, promise::JOB_SECOND);
                self.settle_through(reject, value, true, heap, realm)?;
            }
            return Ok(());
        }
        let capability = promise::slot(heap, job, promise::JOB_CAPABILITY);
        if capability.is_undefined() {
            return Ok(());
        }
        let index = if threw {
            promise::CAPABILITY_REJECT
        } else {
            promise::CAPABILITY_RESOLVE
        };
        let function = promise::slot(heap, capability, index);
        self.settle_through(function, value, threw, heap, realm)
    }

    /// Runs the jobs of 9.5 until one opens a frame or the queue is empty.
    ///
    /// A job whose handler is empty settles here and the next one follows; one
    /// with a handler of the Script takes a frame that stands on no caller, so
    /// its return reaches this drain again.
    fn drain_jobs(
        &mut self,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        loop {
            let Some(job) = self.take_job(heap)? else {
                return Ok(None);
            };
            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
            let kind = promise::slot(heap, job, promise::JOB_KIND)
                .as_smi()
                .unwrap_or(0);
            let function = promise::slot(heap, job, promise::JOB_FUNCTION);
            if !Self::is_callable(function, heap) {
                // 27.2.2.1 step 4: an empty handler passes the argument on
                // with the type the reaction list it came from names.
                let argument = promise::slot(heap, job, promise::JOB_FIRST);
                self.settle_job(job, argument, kind == promise::JOB_REJECT, heap, realm)?;
                self.clear_job(heap)?;
                continue;
            }
            let first = promise::slot(heap, job, promise::JOB_FIRST);
            if !Self::is_script_function(function, heap) {
                let receiver = promise::slot(heap, job, promise::JOB_RECEIVER);
                let second = promise::slot(heap, job, promise::JOB_SECOND);
                match self.call_native_job(function, receiver, first, second, heap, realm) {
                    Ok(answer) => self.settle_job(job, answer, false, heap, realm)?,
                    Err(VMError::Thrown(value, _)) => {
                        self.settle_job(job, value, true, heap, realm)?;
                    }
                    Err(error) => return Err(error),
                }
                self.clear_job(heap)?;
                continue;
            }
            let arguments = if kind == promise::JOB_THENABLE {
                let second = promise::slot(heap, job, promise::JOB_SECOND);
                promise::record(heap, realm, &[first, second])?
            } else {
                promise::record(heap, realm, &[first])?
            };
            let Some(list) = self.job_arguments else {
                return Err(VMError::Heap(HeapError::InvalidReference));
            };
            heap.set_root(list, arguments)?;
            let call = Call {
                receiver: promise::slot(heap, job, promise::JOB_RECEIVER),
                func: Reg(0),
                arg_start: Reg(0),
                arg_count: if kind == promise::JOB_THENABLE { 2 } else { 1 },
                slot: 0,
                resume: Some(Resume::Job { arguments: list }),
                construct: None,
                return_pc: 0,
                caller_code_id: None,
            };
            let entered =
                self.enter_call_value(function, units, active_feedback, heap, realm, call)?;
            if entered.is_some() {
                return Ok(entered);
            }
            self.settle_job(job, self.acc, false, heap, realm)?;
            self.clear_job(heap)?;
        }
    }

    /// Forgets the job that has settled, so that the next return settles no
    /// job twice.
    fn clear_job(&self, heap: &mut GenerationalHeap) -> Result<(), VMError> {
        if let Some(root) = self.running_job {
            heap.set_root(root, VALUE_UNDEFINED)?;
        }
        Ok(())
    }

    /// Settles the job whose frame has just ended and starts the next one.
    ///
    /// Answers the code the run continues in, and nothing once the queue is
    /// empty.
    fn continue_jobs(
        &mut self,
        threw: Option<Value>,
        units: CodeUnits<'_>,
        active_feedback: &mut FeedbackVector,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Option<u32>, VMError> {
        let running = self
            .running_job
            .and_then(|root| heap.root_value(root))
            .unwrap_or(VALUE_UNDEFINED);
        if !running.is_undefined() {
            let value = threw.unwrap_or(self.acc);
            self.settle_job(running, value, threw.is_some(), heap, realm)?;
            self.clear_job(heap)?;
        }
        self.fp = 0;
        self.active_binding_count = 0;
        self.current_context = None;
        self.drain_jobs(units, active_feedback, heap, realm)
    }

    /// Transfers control to the innermost handler protecting the throwing
    /// instruction, unwinding call frames until one is found (14.15).
    ///
    /// `pc` is the offset of the instruction that threw, not the next one.
    #[expect(
        clippy::too_many_arguments,
        reason = "a frame that catches settles a promise, which needs the heap"
    )]
    fn unwind(
        &mut self,
        table: CodeTable<'_>,
        mut pc: usize,
        mut current_code_id: Option<u32>,
        value: Value,
        native: Option<(super::realm::NativeErrorKind, &'static str)>,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<(usize, Option<u32>), VMError> {
        loop {
            let code = table.root(self.unit).ok_or(VMError::InvalidBytecode(
                VerificationError::FunctionOutOfBounds {
                    pc,
                    index: self.unit,
                },
            ))?;
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
            self.unit = frame.caller_unit;
            // 27.2.3.1 step 7: the executor of a new Promise rejects it with
            // what it throws, and the constructor answers as it does for an
            // executor that returned.
            // A job of 9.5 stands on no caller, so a value it throws looks
            // for no handler beyond its own frame: the drain rejects the
            // capability of the job with it.
            if matches!(frame.resume, Some(Resume::Job { .. })) {
                self.frames.clear();
                return Err(VMError::Thrown(value, native));
            }
            if let Some(Resume::Executor { promise, state, .. }) =
                frame.resume.filter(|resume| resume.catches())
            {
                let promise = heap.root_value(promise).unwrap_or(VALUE_UNDEFINED);
                let state = heap.root_value(state).unwrap_or(VALUE_UNDEFINED);
                heap.exit_scope();
                if !promise::take_resolution(heap, state)? {
                    let queue = self.job_queue(heap);
                    promise::settle(heap, realm, queue, promise, promise::REJECTED, value)?;
                }
                self.acc = promise;
                return Ok((frame.return_pc, frame.caller_code_id));
            }
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
        // A single unit has no embedding to compile a body for it, so
        // 20.2.1.1 is a gap on this entry point.
        match self.run_unit(
            CodeTable::new(&[code]),
            &mut [feedback],
            0,
            arguments,
            heap,
            realm,
        )? {
            Outcome::Done(value) => Ok(value),
            Outcome::Compile(_) | Outcome::Evaluate(_) | Outcome::Print(_) => Err(
                VMError::Unsupported("a call only the embedding of a Realm can answer"),
            ),
        }
    }

    /// Executes one unit of a Realm whose other units it may call into.
    ///
    /// `unit` names the entry, and every function object the run creates
    /// carries the unit it was compiled with, so a later Script of the same
    /// Realm resolves a call to it in the table it came from.
    ///
    /// # Errors
    ///
    /// Returns [`VMError`] on invalid bytecode, resource exhaustion, invalid
    /// heap references, or an operation unsupported by the current bytecode.
    pub fn run_unit(
        &mut self,
        units: CodeTable<'_>,
        feedback: &mut [&mut FeedbackVector],
        unit: u32,
        arguments: &[Value],
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Outcome, VMError> {
        self.fp = 0;
        self.frames.clear();
        self.current_context = None;
        self.unit = unit;
        // 9.5 drains the queue after the unit answers, so the queue and what
        // the drain holds are rooted before the first instruction and stay
        // rooted for the whole run.
        let queue = Value::from_object(realm.array(heap, 0)?);
        self.jobs = Some(heap.push_root(queue)?);
        self.running_job = Some(heap.push_root(VALUE_UNDEFINED)?);
        self.job_arguments = Some(heap.push_root(VALUE_UNDEFINED)?);
        self.completion = Some(heap.push_root(VALUE_UNINITIALIZED)?);
        self.job_head = 0;
        let code = units.root(unit).ok_or(VMError::InvalidBytecode(
            VerificationError::FunctionOutOfBounds { pc: 0, index: unit },
        ))?;
        let entry_feedback = feedback
            .get(unit as usize)
            .ok_or(VMError::InvalidFeedbackVector)?;
        code.verify().map_err(VMError::InvalidBytecode)?;
        if !entry_feedback.matches_code(code) {
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
        let pc: usize = 0;
        let current_code_id = None;
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
        self.run_loop(units, feedback, heap, realm, pc, current_code_id)
    }

    /// Continues a run that stopped for [`Outcome::Compile`], with the unit
    /// the embedding made of the text, or `None` where it refused it.
    ///
    /// # Errors
    ///
    /// The errors of [`RegisterVM::run_unit`], from where the run stopped.
    pub fn resume_unit(
        &mut self,
        units: CodeTable<'_>,
        feedback: &mut [&mut FeedbackVector],
        compiled: Compiled,
        heap: &mut GenerationalHeap,
        realm: &Realm,
    ) -> Result<Outcome, VMError> {
        self.compiled_unit = Some(compiled);
        let (pc, code_id) = (self.resume_pc, self.resume_code_id);
        self.run_loop(units, feedback, heap, realm, pc, code_id)
    }

    /// Runs instructions until the unit answers or asks for a compilation.
    fn run_loop(
        &mut self,
        units: CodeTable<'_>,
        feedback: &mut [&mut FeedbackVector],
        heap: &mut GenerationalHeap,
        realm: &Realm,
        mut pc: usize,
        mut current_code_id: Option<u32>,
    ) -> Result<Outcome, VMError> {
        loop {
            match self.step(units, feedback, heap, realm, &mut pc, &mut current_code_id) {
                Ok(None) => {
                    // 20.2.1.1 stopped the run where the call stands, and the
                    // instruction runs again once the unit exists.
                    if let Some(source) = self.pending_source.take() {
                        if core::mem::take(&mut self.pending_print) {
                            return Ok(Outcome::Print(source));
                        }
                        if core::mem::take(&mut self.pending_script) {
                            return Ok(Outcome::Evaluate(source));
                        }
                        return Ok(Outcome::Compile(source));
                    }
                }
                Ok(Some(value)) => return Ok(Outcome::Done(value)),
                // 14.15: a thrown value looks for a handler from the throwing
                // instruction outwards before it leaves the outermost frame.
                Err(VMError::Thrown(value, native)) => {
                    match self.unwind(
                        units,
                        pc.saturating_sub(1),
                        current_code_id,
                        value,
                        native,
                        heap,
                        realm,
                    ) {
                        Ok((next_pc, next_code_id)) => {
                            pc = next_pc;
                            current_code_id = next_code_id;
                        }
                        // 27.2.2.1 step 5: a handler that throws rejects the
                        // capability of its job, and the drain goes on. Every
                        // other value that reaches an empty frame stack leaves
                        // the run.
                        Err(VMError::Thrown(value, native)) => {
                            if self.running_job.is_none_or(|root| {
                                heap.root_value(root)
                                    .unwrap_or(VALUE_UNDEFINED)
                                    .is_undefined()
                            }) {
                                return Err(VMError::Thrown(value, native));
                            }
                            let code = units.root(self.unit).ok_or(VMError::InvalidBytecode(
                                VerificationError::FunctionOutOfBounds {
                                    pc,
                                    index: self.unit,
                                },
                            ))?;
                            let unit_feedback: &mut FeedbackVector = feedback
                                .get_mut(self.unit as usize)
                                .ok_or(VMError::InvalidFeedbackVector)?;
                            let next = self.continue_jobs(
                                Some(value),
                                CodeUnits {
                                    table: units,
                                    active: code,
                                },
                                unit_feedback,
                                heap,
                                realm,
                            )?;
                            let Some(code_id) = next else {
                                self.fp = 0;
                                self.active_binding_count = 0;
                                self.current_context = None;
                                return Ok(Outcome::Done(
                                    self.completion
                                        .and_then(|root| heap.root_value(root))
                                        .unwrap_or(VALUE_UNDEFINED),
                                ));
                            };
                            current_code_id = Some(code_id);
                            pc = 0;
                        }
                        Err(error) => return Err(error),
                    }
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
        table: CodeTable<'_>,
        feedback: &mut [&mut FeedbackVector],
        heap: &mut GenerationalHeap,
        realm: &Realm,
        pc_out: &mut usize,
        code_id_out: &mut Option<u32>,
    ) -> Result<Option<Value>, VMError> {
        let mut pc = *pc_out;
        let mut current_code_id = *code_id_out;
        let outcome = (|| -> Result<Option<Value>, VMError> {
            let code = table.root(self.unit).ok_or(VMError::InvalidBytecode(
                VerificationError::FunctionOutOfBounds {
                    pc,
                    index: self.unit,
                },
            ))?;
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
            // An instruction begins at a Safe Point: every live value is in a
            // register, the accumulator or a context. Collection is never
            // hidden inside allocation, so leaving room here is what lets an
            // operation without a retry loop of its own allocate at all.
            if heap.nursery_is_full() {
                self.collect_young(active_code, heap)?;
            }
            let units = CodeUnits {
                table,
                active: active_code,
            };
            let unit_feedback = feedback
                .get_mut(self.unit as usize)
                .ok_or(VMError::InvalidFeedbackVector)?;
            let unit_feedback: &mut FeedbackVector = unit_feedback;
            let active_feedback = feedback_unit_mut(unit_feedback, current_code_id)
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
                Instruction::LdaGlobalForTypeOf(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    // 13.5.3 answers undefined for an unresolvable Reference
                    // rather than reaching GetValue. Every other refusal of
                    // 9.1.1.4.6 still throws, a binding in its temporal dead
                    // zone included.
                    self.acc = match realm.global_environment().get_binding_value(heap, name)? {
                        Ok(value) => value,
                        Err(super::realm::BindingOutcome::Unresolvable) => VALUE_UNDEFINED,
                        Err(outcome) => {
                            return Err(Self::binding_error(heap, realm, outcome, units));
                        }
                    };
                }
                Instruction::LdaGlobalThis => {
                    // 9.1.1.4.11 answers [[GlobalThisValue]], which 9.4.2 gives
                    // `this` wherever no function bound one.
                    self.acc = Value::from_object(realm.global_environment().global_object(heap)?);
                }
                Instruction::LdaGlobal(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    // 9.1.1.4.6 reaches 9.1.1.2.7, which throws for a name the
                    // binding object does not have.
                    self.acc = realm
                        .global_environment()
                        .get_binding_value(heap, name)?
                        .map_err(|outcome| Self::binding_error(heap, realm, outcome, units))?;
                }
                Instruction::StaGlobal {
                    name: index,
                    strict,
                } => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    let value = self.acc;
                    let units = units.clone();
                    realm
                        .global_environment()
                        .set_mutable_binding(heap, name, value, strict)?
                        .map_err(|outcome| Self::binding_error(heap, realm, outcome, &units))?;
                }
                Instruction::VerifyGlobalVar(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    // 16.1.7 step 4: a lexical declaration of this Realm that
                    // the name would shadow is a SyntaxError.
                    if realm
                        .global_environment()
                        .has_lexical_declaration(heap, name)?
                    {
                        return Err(raise_message(
                            heap,
                            realm,
                            super::realm::NativeErrorKind::SyntaxError,
                            "a lexical declaration of this name already exists",
                        ));
                    }
                }
                // 16.1.7 step 3: a lexical name this Realm already binds, or
                // one an existing global property would shadow, is a
                // SyntaxError before anything of the Script runs.
                Instruction::VerifyGlobalLexical(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    let environment = realm.global_environment();
                    if environment.has_lexical_declaration(heap, name)?
                        || environment.has_restricted_global_property(heap, name)?
                    {
                        return Err(raise_message(
                            heap,
                            realm,
                            super::realm::NativeErrorKind::SyntaxError,
                            "a declaration of this name already exists",
                        ));
                    }
                }
                Instruction::DeclareGlobalLexical {
                    name: index,
                    mutable,
                } => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    realm
                        .global_environment()
                        .create_lexical_binding(heap, name, mutable)?;
                }
                Instruction::InitializeGlobalLexical(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    let value = self.acc;
                    realm
                        .global_environment()
                        .initialize_lexical_binding(heap, name, value)?;
                }
                Instruction::DeclareGlobalVar(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    realm
                        .global_environment()
                        .create_global_var_binding(heap, name)?;
                }
                Instruction::VerifyGlobalFunction(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    // 16.1.7 step 9: a global property that cannot take the
                    // function is a TypeError.
                    if !realm
                        .global_environment()
                        .can_declare_global_function(heap, name)?
                    {
                        return Err(type_error(
                            heap,
                            realm,
                            "a global property of this name cannot take a function",
                        ));
                    }
                }
                Instruction::DeclareGlobalFunction(index) => {
                    let units = active_code
                        .string_constants
                        .get(index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    let value = self.acc;
                    realm
                        .global_environment()
                        .create_global_function_binding(heap, name, value)?;
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
                    // 6.1.6.1.1 negates a Number. Anything else reached this
                    // instruction without the conversion 7.1.4 asks for.
                    let number = numeric_value(self.acc).ok_or(if self.acc.is_object() {
                        NUMERIC_CONVERSION_GAP
                    } else {
                        VMError::TypeError
                    })?;
                    self.acc = Value::from_f64(-number);
                }
                Instruction::LogicalNot => {
                    self.acc = Value::from_bool(!Self::to_boolean(self.acc, heap)?);
                }
                Instruction::ToNumber => {
                    self.acc = Value::from_f64(primitive_number(self.acc, heap)?);
                }
                Instruction::ToNumeric(register) => {
                    let value = self.read_reg(register)?;
                    // 7.1.4 of an Object is 7.1.1 with the hint `number`,
                    // which runs a method of the Script; the instruction runs
                    // again once the register holds the primitive.
                    if let Some(object) = value.as_object() {
                        let call = Call {
                            receiver: Value::from_object(object),
                            func: register,
                            arg_start: register,
                            arg_count: 0,
                            slot: 0,
                            resume: Some(Resume::Primitive {
                                register,
                                step: PrimitiveStep::Exotic,
                                hint: PrimitiveHint::Number,
                                held: None,
                            }),
                            return_pc: pc.saturating_sub(1),
                            caller_code_id: current_code_id,
                            construct: None,
                        };
                        match self.convert_to_primitive(
                            call,
                            units,
                            active_feedback,
                            heap,
                            realm,
                        )? {
                            Conversion::Done(value) => {
                                self.write_reg(register, value)?;
                                pc = pc.saturating_sub(1);
                            }
                            Conversion::Suspended(code_id) => {
                                current_code_id = Some(code_id);
                                pc = 0;
                            }
                        }
                        return Ok(None);
                    }
                    if value.is_symbol() {
                        // 7.1.4 step 2: a Symbol has no Number of its own.
                        return Err(type_error(heap, realm, "cannot convert Symbol to Number"));
                    }
                    self.acc = Value::from_f64(primitive_number(value, heap)?);
                }
                Instruction::ToText(register) => {
                    let value = self.read_reg(register)?;
                    // 7.1.17 of an Object is 7.1.1 with the hint `string`,
                    // which runs a method of the Script; the instruction runs
                    // again once the register holds the primitive.
                    if let Some(object) = value.as_object() {
                        let call = Call {
                            receiver: Value::from_object(object),
                            func: register,
                            arg_start: register,
                            arg_count: 0,
                            slot: 0,
                            resume: Some(Resume::Primitive {
                                register,
                                step: PrimitiveStep::Exotic,
                                hint: PrimitiveHint::String,
                                held: None,
                            }),
                            return_pc: pc.saturating_sub(1),
                            caller_code_id: current_code_id,
                            construct: None,
                        };
                        match self.convert_to_primitive(
                            call,
                            units,
                            active_feedback,
                            heap,
                            realm,
                        )? {
                            Conversion::Done(value) => {
                                self.write_reg(register, value)?;
                                pc = pc.saturating_sub(1);
                            }
                            Conversion::Suspended(code_id) => {
                                current_code_id = Some(code_id);
                                pc = 0;
                            }
                        }
                        return Ok(None);
                    }
                    if value.is_symbol() {
                        // 7.1.17 step 2: a Symbol has no String of its own.
                        return Err(type_error(heap, realm, "cannot convert Symbol to String"));
                    }
                    let text = self.primitive_string(value, heap, realm)?;
                    self.acc = text;
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
                Instruction::Binary { op, lhs, rhs, slot } => {
                    // 7.1.1 converts an Object operand first, which may run a
                    // method of the object. The instruction then starts again
                    // with the converted operand in its register.
                    let left = self.read_reg(lhs)?;
                    let right = self.read_reg(rhs)?;
                    // 7.2.14 compares two values of one type without
                    // converting either, and answers false for an Object
                    // against null or undefined. Every other operator of this
                    // instruction converts an Object operand.
                    let converts = if op == BinaryOp::Equals {
                        let one_object = left.as_object().is_some() != right.as_object().is_some();
                        let nullish = left.is_null()
                            || left.is_undefined()
                            || right.is_null()
                            || right.is_undefined();
                        one_object && !nullish
                    } else {
                        true
                    };
                    let pending = converts
                        .then(|| {
                            left.as_object()
                                .map(|object| (object, lhs))
                                .or_else(|| right.as_object().map(|object| (object, rhs)))
                        })
                        .flatten();
                    if let Some((object, register)) = pending {
                        let call = Call {
                            receiver: Value::from_object(object),
                            func: register,
                            arg_start: register,
                            arg_count: 0,
                            slot: 0,
                            resume: Some(Resume::Primitive {
                                register,
                                step: PrimitiveStep::Exotic,
                                // 13.15.3 and 7.1.3 take no hint.
                                hint: PrimitiveHint::Default,
                                held: None,
                            }),
                            return_pc: pc.saturating_sub(1),
                            caller_code_id: current_code_id,
                            construct: None,
                        };
                        match self.convert_to_primitive(
                            call,
                            units,
                            active_feedback,
                            heap,
                            realm,
                        )? {
                            Conversion::Done(value) => {
                                self.write_reg(register, value)?;
                                pc = pc.saturating_sub(1);
                            }
                            Conversion::Suspended(code_id) => {
                                current_code_id = Some(code_id);
                                pc = 0;
                            }
                        }
                        return Ok(None);
                    }
                    self.acc = left;
                    let observed = self.primitive_binary(op, right, heap, realm)?;
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
                Instruction::TestIn(reg) => {
                    let target = self.read_reg(reg)?;
                    let Some(object) = target.as_object() else {
                        return Err(type_error(
                            heap,
                            realm,
                            "the right-hand side of in is not an object",
                        ));
                    };
                    let key = property_key(self.acc, heap)?;
                    self.acc = Value::from_bool(Self::has_property(object, key, heap, realm)?);
                }
                Instruction::TestInstanceOf(reg) => {
                    let constructor = self.read_reg(reg)?;
                    let value = self.acc;
                    self.acc = Value::from_bool(Self::ordinary_has_instance(
                        constructor,
                        value,
                        heap,
                        realm,
                    )?);
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
                Instruction::GetWellKnown {
                    obj,
                    symbol,
                    slot: _,
                } => {
                    let code_units = units;
                    // 7.4.2 reads @@iterator, which is a Symbol key and so
                    // reaches no Elements store and no String exotic object.
                    let symbol = super::realm::WellKnownSymbol::ALL
                        .get(symbol as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let target = self.read_reg(obj)?;
                    let object = Self::coerce_object(target, heap, realm)?;
                    let found = heap.lookup_named(object, symbol.key())?;
                    if let Some(property) = found
                        && property.flags.is_accessor
                    {
                        if let Some(code_id) = self.enter_accessor(
                            property.value,
                            target,
                            None,
                            pc,
                            current_code_id,
                            code_units,
                            active_feedback,
                            heap,
                            realm,
                        )? {
                            current_code_id = Some(code_id);
                            pc = 0;
                        }
                        return Ok(None);
                    }
                    self.acc = match found {
                        Some(property) => property.value,
                        // A Prototype this Realm has not finished building
                        // owns the Symbol; answering undefined would say the
                        // value is not iterable, which is a different thing.
                        None => {
                            Self::absent_well_known(target, object, Some(*symbol), heap, realm)?
                        }
                    };
                }
                Instruction::GetNamed {
                    obj,
                    name: name_index,
                    slot,
                } => {
                    let code_units = units;
                    let name = active_code
                        .string_constants
                        .get(name_index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(name)?);
                    let target = self.read_reg(obj)?;
                    // 10.4.3: a String answers "length" and its indices from the
                    // exotic object ToObject would produce, without producing it.
                    if target.is_string() {
                        self.acc = self.string_member(target, name, heap, realm)?;
                        return Ok(None);
                    }
                    if let Some(member) = Self::primitive_member(target, name, heap, realm)? {
                        self.acc = member;
                        return Ok(None);
                    }
                    if let Some(member) = self.symbol_member(target, name, heap, realm)? {
                        self.acc = member;
                        return Ok(None);
                    }
                    let Some(oref) = target.as_object() else {
                        return Err(property_base_error(target, heap, realm));
                    };
                    // 10.4.2: an Array keeps its indices in an element store
                    // and its length in a field of its own, so a name that
                    // asks for one of them is answered there whichever
                    // instruction asks.
                    let units = active_code
                        .string_constants
                        .get(name_index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let elements = heap.get_object(oref).ok_or(VMError::TypeError)?.elements;
                    // A hole is not an answer: 10.1.8.1 reads on, over the
                    // Shape and then the Prototype Chain, where 10.4.2.1 may
                    // have put the index.
                    if let Some(eref) = elements
                        && let Some(index) = array_index_units(units)
                        && let Some(element) = heap
                            .get_elements(eref)
                            .ok_or(VMError::TypeError)?
                            .get(index)
                    {
                        self.acc = element;
                        return Ok(None);
                    }
                    if units.as_slice() == LENGTH_NAME
                        && let Some(length) = heap.array_length(oref)
                    {
                        self.acc = i32::try_from(length)
                            .map_or_else(|_| Value::from_f64(f64::from(length)), Value::from_smi);
                        return Ok(None);
                    }
                    // 10.4.3.1 answers a String exotic object's own indices
                    // and `length` from its `[[StringData]]`.
                    let units = units.clone();
                    if let Some(member) = self.string_exotic_member(oref, &units, heap)? {
                        self.acc = member;
                        return Ok(None);
                    }
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
                        // 10.1.8.1 step 3: an accessor answers what its getter
                        // answers, so the read is a call and the cache, which
                        // holds a slot and not a call, records nothing.
                        if property.flags.is_accessor {
                            if let Some(code_id) = self.enter_accessor(
                                property.value,
                                target,
                                None,
                                pc,
                                current_code_id,
                                code_units,
                                active_feedback,
                                heap,
                                realm,
                            )? {
                                current_code_id = Some(code_id);
                                pc = 0;
                            }
                            return Ok(None);
                        }
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
                        let units = active_code
                            .string_constants
                            .get(name_index as usize)
                            .ok_or(VMError::InvalidRegister)?;
                        self.acc = Self::absent_property(target, units, heap, realm)?;
                    }
                }
                Instruction::SetNamed {
                    obj,
                    name: name_index,
                    slot,
                    strict,
                    define,
                } => {
                    let code_units = units;
                    let units = active_code
                        .string_constants
                        .get(name_index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let target = self.read_reg(obj)?;
                    let Some(oref) = target.as_object() else {
                        return Err(property_store_error(target, heap, realm));
                    };
                    // 10.4.2: an Array keeps its indices in an element store,
                    // so a name that is one is written there and not as a
                    // property of its own.
                    // 10.4.2.4 sets an Array's `length` and deletes every
                    // index at or above the new one.
                    if units.as_slice() == LENGTH_NAME && heap.array_length(oref).is_some() {
                        let wanted = self.acc;
                        Self::set_array_length(oref, wanted, heap, realm)?;
                        return Ok(None);
                    }
                    let elements = heap.get_object(oref).ok_or(VMError::TypeError)?.elements;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    if let Some(eref) = elements
                        && let Some(index) = array_index_units(units)
                        && !Self::shape_holds(oref, name, heap)?
                    {
                        let stored = heap.get_elements(eref).ok_or(VMError::TypeError)?;
                        if stored.get(index).is_none() {
                            if heap.own_property_count(oref).unwrap_or(usize::MAX)
                                >= self.property_limit
                            {
                                return Err(VMError::PropertyLimit);
                            }
                            if !define && Self::refuses_a_new_property(oref, strict, heap, realm)? {
                                return Ok(None);
                            }
                        }
                        let value = self.acc;
                        heap.set_array_element(oref, index, value)?;
                        return Ok(None);
                    }
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

                    // 10.4.3.1 gives a String exotic object own names that
                    // are neither writable nor in a Shape, so a write to one
                    // has nowhere to go.
                    if Self::owns_string_exotic(oref, name, heap)? {
                        return Ok(None);
                    }
                    // 10.1.9.1: a property of the object or of a Prototype of
                    // it that is not writable refuses the write, which a
                    // strict Reference turns into a TypeError.
                    if !define
                        && let Some(found) = heap.lookup_named(oref, name)?
                        && !found.flags.is_accessor
                        && !found.flags.writable
                    {
                        if strict {
                            return Err(type_error(
                                heap,
                                realm,
                                "cannot write a property that is not writable",
                            ));
                        }
                        return Ok(None);
                    }
                    // 10.1.9.2 step 5: a property of the object or of a
                    // Prototype of it that is an accessor is written by
                    // calling its setter.
                    if !define
                        && let Some(found) = heap.lookup_named(oref, name)?
                        && found.flags.is_accessor
                    {
                        let assigned = self.acc;
                        if let Some(code_id) = self.enter_accessor(
                            found.value,
                            target,
                            Some(assigned),
                            pc,
                            current_code_id,
                            code_units,
                            active_feedback,
                            heap,
                            realm,
                        )? {
                            current_code_id = Some(code_id);
                            pc = 0;
                        }
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
                        if !define && Self::refuses_a_new_property(oref, strict, heap, realm)? {
                            return Ok(None);
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
                    let code_units = units;
                    // 7.1.19 step 2 sends an Object key through 7.1.1 with the
                    // hint `string`, which runs before the base is read again.
                    if self.convert_key(
                        key,
                        None,
                        &mut pc,
                        &mut current_code_id,
                        code_units,
                        active_feedback,
                        heap,
                        realm,
                    )? {
                        return Ok(None);
                    }
                    let key_val = self.read_reg(key)?;
                    let target = self.read_reg(obj)?;
                    if target.is_string() {
                        let name = property_key(key_val, heap)?;
                        self.acc = self.string_member(target, name, heap, realm)?;
                        return Ok(None);
                    }
                    let Some(oref) = target.as_object() else {
                        return Err(property_base_error(target, heap, realm));
                    };
                    let js_obj = heap.get_object(oref).ok_or(VMError::TypeError)?;

                    let element = match (array_index(key_val, heap)?, js_obj.elements) {
                        (Some(index), Some(eref)) => heap
                            .get_elements(eref)
                            .ok_or(VMError::TypeError)?
                            .get(index),
                        _ => None,
                    };
                    // 7.1.19 keeps a Symbol as the key it is.
                    if let Some(symbol) = key_val.as_symbol() {
                        let name = PropertyKey::Symbol(symbol);
                        self.acc = match heap.lookup_named(oref, name)? {
                            Some(property) if property.flags.is_accessor => {
                                if let Some(code_id) = self.enter_accessor(
                                    property.value,
                                    target,
                                    None,
                                    pc,
                                    current_code_id,
                                    code_units,
                                    active_feedback,
                                    heap,
                                    realm,
                                )? {
                                    current_code_id = Some(code_id);
                                    pc = 0;
                                }
                                return Ok(None);
                            }
                            Some(property) => property.value,
                            // A Prototype this Realm has not finished building
                            // may own the Symbol.
                            None => Self::absent_well_known(
                                target,
                                oref,
                                super::realm::WellKnownSymbol::from_reference(symbol),
                                heap,
                                realm,
                            )?,
                        };
                        return Ok(None);
                    }
                    // A hole is not an answer, so the read goes on.
                    if let Some(element) = element {
                        self.acc = element;
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
                        let units = name;
                        if let Some(member) = self.string_exotic_member(oref, &units, heap)? {
                            self.acc = member;
                            return Ok(None);
                        }
                        let Some(name) = heap
                            .strings
                            .lookup_interned_units(&units)
                            .map(PropertyKey::String)
                        else {
                            // A name no String of the Agent carries is owned by
                            // no object, but a Prototype the Realm has not built
                            // would have carried it.
                            self.acc = Self::absent_property(target, &units, heap, realm)?;
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
                            if property.flags.is_accessor {
                                if let Some(code_id) = self.enter_accessor(
                                    property.value,
                                    target,
                                    None,
                                    pc,
                                    current_code_id,
                                    code_units,
                                    active_feedback,
                                    heap,
                                    realm,
                                )? {
                                    current_code_id = Some(code_id);
                                    pc = 0;
                                }
                                return Ok(None);
                            }
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
                            self.acc = Self::absent_property(target, &units, heap, realm)?;
                        }
                    }
                }
                Instruction::SetByValue {
                    obj,
                    key,
                    slot,
                    define,
                    strict,
                } => {
                    let code_units = units;
                    // 7.1.19 step 2 sends an Object key through 7.1.1 with the
                    // hint `string`. 6.2.5.5 reaches it after the value, which
                    // waits in a root while the method runs.
                    if self.convert_key(
                        key,
                        Some(self.acc),
                        &mut pc,
                        &mut current_code_id,
                        code_units,
                        active_feedback,
                        heap,
                        realm,
                    )? {
                        return Ok(None);
                    }
                    let key_val = self.read_reg(key)?;
                    let target = self.read_reg(obj)?;
                    let Some(oref) = target.as_object() else {
                        return Err(property_store_error(target, heap, realm));
                    };
                    let js_obj = heap.get_object(oref).ok_or(VMError::TypeError)?;
                    let val = self.acc;

                    // 7.1.19 keeps a Symbol as the key it is, and no Symbol
                    // is an index or a name of the element store.
                    if let Some(symbol) = key_val.as_symbol() {
                        let name = PropertyKey::Symbol(symbol);
                        // 10.1.9.2 reads the property of the chain before it
                        // writes: an accessor takes the value through its
                        // setter, and a data property that is not writable
                        // takes none.
                        let found = if define {
                            None
                        } else {
                            heap.lookup_named(oref, name)?
                        };
                        if let Some(found) = found {
                            if found.flags.is_accessor {
                                if let Some(code_id) = self.enter_accessor(
                                    found.value,
                                    target,
                                    Some(val),
                                    pc,
                                    current_code_id,
                                    code_units,
                                    active_feedback,
                                    heap,
                                    realm,
                                )? {
                                    current_code_id = Some(code_id);
                                    pc = 0;
                                }
                                return Ok(None);
                            }
                            if !found.flags.writable {
                                if strict {
                                    return Err(type_error(
                                        heap,
                                        realm,
                                        "cannot write a property that is not writable",
                                    ));
                                }
                                return Ok(None);
                            }
                        }
                        // An own property keeps the attributes it was given,
                        // so the write goes to its slot and not through
                        // 10.1.6.3, which would make it an ordinary one.
                        let shape = heap.get_object(oref).ok_or(VMError::TypeError)?.shape_id;
                        if let Some(location) = heap.shapes.lookup(shape, name) {
                            heap.set_object_slot(oref, location.slot_offset, val)?;
                            return Ok(None);
                        }
                        if heap.own_property_count(oref).unwrap_or(usize::MAX)
                            >= self.property_limit
                        {
                            return Err(VMError::PropertyLimit);
                        }
                        if !define && Self::refuses_a_new_property(oref, strict, heap, realm)? {
                            return Ok(None);
                        }
                        heap.define_own_named(oref, name, val, PropertyFlags::ordinary_data())?;
                        return Ok(None);
                    }
                    let indexed = match array_index(key_val, heap)? {
                        Some(index) => {
                            let units = property_name_units(key_val, heap)?;
                            let name = heap
                                .strings
                                .lookup_interned_units(&units)
                                .map(PropertyKey::String);
                            match name {
                                // An index 10.4.2.1 moved out of the store is
                                // a property of the Shape, and the write
                                // belongs there.
                                Some(name) if Self::shape_holds(oref, name, heap)? => None,
                                _ => Some(index),
                            }
                        }
                        None => None,
                    };
                    if js_obj.elements.is_some()
                        && let Some(index) = indexed
                    {
                        let elements_reference = js_obj.elements.ok_or(VMError::TypeError)?;
                        let elements = heap
                            .get_elements(elements_reference)
                            .ok_or(VMError::TypeError)?;
                        if elements.get(index).is_none() {
                            if heap.own_property_count(oref).unwrap_or(usize::MAX)
                                >= self.property_limit
                            {
                                return Err(VMError::PropertyLimit);
                            }
                            if !define && Self::refuses_a_new_property(oref, strict, heap, realm)? {
                                return Ok(None);
                            }
                        }
                        heap.set_array_element(oref, index, val)?;
                    } else {
                        let name_units = property_name_units(key_val, heap)?;
                        if !define && store_reaches_unbuilt_prototype(oref, &name_units, heap) {
                            return Err(VMError::Unsupported(
                                "a property write under a name an unbuilt Prototype owns",
                            ));
                        }
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
                        if Self::owns_string_exotic(oref, name, heap)? {
                            return Ok(None);
                        }
                        if !define
                            && let Some(found) = heap.lookup_named(oref, name)?
                            && !found.flags.is_accessor
                            && !found.flags.writable
                        {
                            if strict {
                                return Err(type_error(
                                    heap,
                                    realm,
                                    "cannot write a property that is not writable",
                                ));
                            }
                            return Ok(None);
                        }
                        // 13.2.5.5 defines an own property of a literal and
                        // reaches no setter; 13.15.2 assigns and does.
                        if !define
                            && let Some(found) = heap.lookup_named(oref, name)?
                            && found.flags.is_accessor
                        {
                            if let Some(code_id) = self.enter_accessor(
                                found.value,
                                target,
                                Some(val),
                                pc,
                                current_code_id,
                                code_units,
                                active_feedback,
                                heap,
                                realm,
                            )? {
                                current_code_id = Some(code_id);
                                pc = 0;
                            }
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
                            if !define && Self::refuses_a_new_property(oref, strict, heap, realm)? {
                                return Ok(None);
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
                Instruction::DeleteNamed {
                    obj,
                    name: name_index,
                    strict,
                } => {
                    let units = active_code
                        .string_constants
                        .get(name_index as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let index = array_index_units(units);
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    let target = self.read_reg(obj)?;
                    self.acc = delete_reference(target, name, index, strict, heap, realm)?;
                }
                Instruction::DeleteByValue {
                    obj,
                    key: key_register,
                    strict,
                } => {
                    if self.convert_key(
                        key_register,
                        None,
                        &mut pc,
                        &mut current_code_id,
                        units,
                        active_feedback,
                        heap,
                        realm,
                    )? {
                        return Ok(None);
                    }
                    let key = self.read_reg(key_register)?;
                    let target = self.read_reg(obj)?;
                    let index = array_index(key, heap)?;
                    // 7.1.19 keeps a Symbol as the key it is.
                    let name = property_key(key, heap)?;
                    self.acc = delete_reference(target, name, index, strict, heap, realm)?;
                }
                Instruction::Require(kind) => {
                    if self.acc.is_undefined() || self.acc.is_null() {
                        let message = match kind {
                            super::bytecode::RequireKind::ObjectCoercible => {
                                "cannot destructure null or undefined"
                            }
                            super::bytecode::RequireKind::Iterable => "value is not iterable",
                        };
                        return Err(type_error(heap, realm, message));
                    }
                }
                Instruction::ThisBinding { register } => {
                    let value = self.read_reg(register)?;
                    if value == VALUE_UNINITIALIZED {
                        return Err(raise(
                            heap,
                            realm,
                            super::realm::NativeErrorKind::ReferenceError,
                            "this is not initialized",
                        ));
                    }
                    self.acc = value;
                }
                Instruction::DeriveClass { heritage } => {
                    self.derive_class(heritage, heap, realm)?;
                }
                Instruction::SuperCall {
                    arg_start,
                    arg_count,
                    forwarded,
                    slot,
                } => {
                    if let Some(code_id) = self.super_call(
                        arg_start,
                        arg_count,
                        forwarded,
                        slot,
                        pc,
                        current_code_id,
                        active_code,
                        units,
                        active_feedback,
                        heap,
                        realm,
                    )? {
                        current_code_id = Some(code_id);
                        pc = 0;
                    }
                }
                Instruction::MakeMethod { home } => {
                    let target = self.read_reg(home)?.as_object().ok_or(VMError::TypeError)?;
                    Self::make_method(self.acc, target, heap)?;
                }
                Instruction::SuperBase { target } => {
                    let home = self
                        .home_of(active_code)?
                        .as_object()
                        .ok_or(VMError::Unsupported("super in a function with no home"))?;
                    let base = heap
                        .get_object(home)
                        .ok_or(VMError::Heap(HeapError::InvalidReference))?
                        .prototype;
                    self.write_reg(target, base)?;
                }
                Instruction::GetSuper { base, name } => {
                    let code_units = units;
                    let text = active_code
                        .string_constants
                        .get(name as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(text)?);
                    if let Some(code_id) = self.read_super(
                        base,
                        name,
                        pc,
                        current_code_id,
                        code_units,
                        active_feedback,
                        heap,
                        realm,
                    )? {
                        current_code_id = Some(code_id);
                        pc = 0;
                    }
                }
                Instruction::GetSuperByValue { base, key } => {
                    let code_units = units;
                    // 13.3.7.2 makes the key before the base is read again.
                    if self.convert_key(
                        key,
                        None,
                        &mut pc,
                        &mut current_code_id,
                        code_units,
                        active_feedback,
                        heap,
                        realm,
                    )? {
                        return Ok(None);
                    }
                    let key_value = self.read_reg(key)?;
                    let name = property_key(key_value, heap)?;
                    if let Some(code_id) = self.read_super(
                        base,
                        name,
                        pc,
                        current_code_id,
                        code_units,
                        active_feedback,
                        heap,
                        realm,
                    )? {
                        current_code_id = Some(code_id);
                        pc = 0;
                    }
                }
                Instruction::DefineMethod { obj, name } => {
                    let units = active_code
                        .string_constants
                        .get(name as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    self.define_method(obj, name, false, heap)?;
                }
                Instruction::DefineMethodByValue {
                    obj,
                    key,
                    enumerable,
                } => {
                    let name = property_key(self.read_reg(key)?, heap)?;
                    // 15.7.14 and 13.2.5.5 name a method after the key only
                    // the run time knows, which 10.2.10 does here.
                    self.name_from_key(name, None, heap)?;
                    self.define_method(obj, name, enumerable, heap)?;
                }
                Instruction::DefineAccessorByValue {
                    obj,
                    key,
                    setter,
                    enumerable,
                } => {
                    let name = property_key(self.read_reg(key)?, heap)?;
                    self.name_from_key(name, Some(setter), heap)?;
                    self.define_accessor(obj, name, setter, enumerable, heap)?;
                }
                Instruction::DefineAccessor {
                    obj,
                    name,
                    setter,
                    enumerable,
                } => {
                    let units = active_code
                        .string_constants
                        .get(name as usize)
                        .ok_or(VMError::InvalidRegister)?;
                    let name = PropertyKey::String(heap.strings.intern_units(units)?);
                    self.define_accessor(obj, name, setter, enumerable, heap)?;
                }
                Instruction::GetArrayLength { obj } => {
                    let target = self.read_reg(obj)?;
                    let object = target.as_object().ok_or(VMError::TypeError)?;
                    let length = heap.array_length(object).ok_or(VMError::TypeError)?;
                    self.acc = i32::try_from(length)
                        .map_or_else(|_| Value::from_f64(f64::from(length)), Value::from_smi);
                }
                Instruction::CreateRegExp(index) => {
                    self.acc = Self::create_regexp(active_code, index, heap, realm)?;
                }
                Instruction::CreateObject => {
                    let root_shape = heap.shapes.root_shape();
                    let oref = self.allocate_object(active_code, heap, realm, root_shape)?;
                    self.acc = Value::from_object(oref);
                }
                Instruction::CopyDataProperties {
                    source,
                    excluded,
                    count,
                } => {
                    let target = self.read_reg(source)?;
                    let root_shape = heap.shapes.root_shape();
                    let rest = self.allocate_object(active_code, heap, realm, root_shape)?;
                    self.acc = Value::from_object(rest);
                    let mut names = Vec::new();
                    for offset in 0..count {
                        let register = Reg(excluded.0.saturating_add(offset));
                        names.push(property_key(self.read_reg(register)?, heap)?);
                    }
                    // 10.4.3 gives the String exotic object 7.1.18 would make
                    // one own property per code unit, all of them enumerable.
                    if target.is_string() {
                        let length = heap
                            .strings
                            .length_of(target)
                            .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                        for index in 0..u32::try_from(length).unwrap_or(u32::MAX) {
                            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                            let key = PropertyKey::String(heap.intern_index(index)?);
                            if names.contains(&key) {
                                continue;
                            }
                            let unit = usize::try_from(index)
                                .ok()
                                .and_then(|index| heap.strings.char_code_at(target, index))
                                .ok_or(VMError::Heap(HeapError::InvalidReference))?;
                            let value = self.allocate_string(heap, &[unit])?;
                            let rest = self.acc.as_object().ok_or(VMError::TypeError)?;
                            heap.define_own_named(
                                rest,
                                key,
                                value,
                                PropertyFlags::ordinary_data(),
                            )?;
                        }
                    }
                    // 7.3.25 step 3: undefined and null copy nothing at all.
                    if let Some(object) = target.as_object() {
                        for (key, enumerable) in heap.own_keys(object)? {
                            if !enumerable || names.contains(&key) {
                                continue;
                            }
                            self.fuel = self.fuel.checked_sub(1).ok_or(VMError::OutOfFuel)?;
                            if heap.own_property_count(rest).unwrap_or(usize::MAX)
                                >= self.property_limit
                            {
                                return Err(VMError::PropertyLimit);
                            }
                            if heap
                                .own_named_flags(object, key)?
                                .is_some_and(|flags| flags.is_accessor)
                            {
                                return Err(VMError::Unsupported("a property that is an accessor"));
                            }
                            let indexed = Self::element_index_of(object, key, heap);
                            let value = Self::own_property_value(object, key, indexed, heap)?;
                            heap.define_own_named(
                                rest,
                                key,
                                value,
                                PropertyFlags::ordinary_data(),
                            )?;
                        }
                    }
                }
                Instruction::CreateArguments(target) => {
                    self.create_arguments(active_code, target, heap, realm)?;
                }
                Instruction::CreateArray(length) => {
                    if self.property_limit == 0 {
                        return Err(VMError::PropertyLimit);
                    }
                    let oref = self.allocate_array(active_code, heap, realm, length)?;
                    self.acc = Value::from_object(oref);
                }
                Instruction::CreateClass(code_id) => {
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
                    let expected_arguments = target.expected_arguments;
                    let function = self.allocate_function(
                        active_code,
                        heap,
                        realm,
                        code_id,
                        captures_context,
                    )?;
                    self.acc = Value::from_object(function);
                    Self::set_function_length(function, expected_arguments, heap)?;
                    self.set_function_name(code, code_id, heap)?;
                    // 15.7.14 step 12 gives the `prototype` attributes no
                    // ordinary function's has. The accumulator carries the
                    // constructor through the allocation.
                    self.make_constructor(active_code, heap, realm)?;
                    let function = self.acc.as_object().ok_or(VMError::TypeError)?;
                    let name = PropertyKey::String(heap.strings.intern("prototype")?);
                    let prototype = heap
                        .lookup_named(function, name)?
                        .map_or(VALUE_UNDEFINED, |property| property.value);
                    heap.define_own_named(
                        function,
                        name,
                        prototype,
                        PropertyFlags {
                            writable: false,
                            enumerable: false,
                            configurable: false,
                            is_accessor: false,
                        },
                    )?;
                    // 15.7.14 step 16 makes the constructor a method of the
                    // prototype, so `super` in its body reads that chain.
                    if let Some(prototype) = prototype.as_object() {
                        Self::make_method(self.acc, prototype, heap)?;
                    }
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
                    let constructible = target.constructible;
                    let expected_arguments = target.expected_arguments;
                    let function = self.allocate_function(
                        active_code,
                        heap,
                        realm,
                        code_id,
                        captures_context,
                    )?;
                    self.acc = Value::from_object(function);
                    // 10.2.9 gives the function the `ExpectedArgumentCount` of
                    // 15.1.5 as its `length`.
                    Self::set_function_length(function, expected_arguments, heap)?;
                    self.set_function_name(code, code_id, heap)?;
                    // 10.2.5 gives an ordinary function its `prototype`; a
                    // method and an arrow have none and no `[[Construct]]`.
                    // The accumulator carries the function through it, because
                    // the object that becomes its `prototype` is allocated and
                    // the collector only follows what it can see.
                    if constructible {
                        self.make_constructor(active_code, heap, realm)?;
                    }
                }
                Instruction::Construct {
                    func,
                    target,
                    arg_start,
                    arg_count,
                    slot,
                } => {
                    // 23.1.1.1 answers an Array of its own whichever way it
                    // was reached, so a native constructor takes the arguments
                    // and none of 10.1.13, whose object it would not use.
                    if let Some(intrinsic) = self.native_constructor(func, heap) {
                        let call = Call {
                            receiver: VALUE_UNDEFINED,
                            func,
                            arg_start,
                            arg_count,
                            slot,
                            return_pc: pc,
                            resume: None,
                            caller_code_id: current_code_id,
                            construct: Some(target),
                        };
                        // The same conversion a call of the native asks for.
                        if let Some((index, hint)) =
                            self.next_coercion(intrinsic, &call, heap, realm)?
                        {
                            if let Some(code_id) = self.begin_coercion(
                                intrinsic,
                                index,
                                hint,
                                call,
                                units,
                                active_feedback,
                                heap,
                                realm,
                            )? {
                                current_code_id = Some(code_id);
                                pc = 0;
                            }
                            return Ok(None);
                        }
                        if intrinsic == Intrinsic::PromiseConstructor {
                            if let Some(code_id) =
                                self.begin_promise(call, units, active_feedback, heap, realm)?
                            {
                                current_code_id = Some(code_id);
                                pc = 0;
                            }
                            return Ok(None);
                        }
                        self.acc = self.call_intrinsic(intrinsic, call, units, heap, realm)?;
                        self.write_reg(target, self.acc)?;
                        return Ok(None);
                    }
                    // 7.3.15 refuses a callee without `[[Construct]]`, which here
                    // is a callee without the `prototype` 10.2.5 installs.
                    // 10.2.2 step 5 creates the object for a base constructor
                    // and leaves a derived one to make its own with 13.3.7.1.
                    let receiver = if Self::derives(self.read_reg(func)?, units, heap) {
                        self.write_reg(target, VALUE_UNDEFINED)?;
                        VALUE_UNINITIALIZED
                    } else {
                        let object =
                            self.ordinary_create_from_constructor(active_code, heap, realm, func)?;
                        self.write_reg(target, Value::from_object(object))?;
                        Value::from_object(object)
                    };
                    // 13.3.5.1 gives the call the constructor it named, and
                    // nothing allocates between here and the frame.
                    self.pending_new_target = self.read_reg(func)?;
                    if let Some(code_id) = self.enter_call(
                        units,
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
                            resume: None,
                            caller_code_id: current_code_id,
                            construct: Some(target),
                        },
                    )? {
                        current_code_id = Some(code_id);
                        pc = 0;
                    }
                }
                Instruction::Call {
                    func,
                    arg_start,
                    arg_count,
                    slot,
                } => {
                    if let Some(code_id) = self.enter_call(
                        units,
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
                            resume: None,
                            construct: None,
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
                        units,
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
                            resume: None,
                            construct: None,
                            caller_code_id: current_code_id,
                        },
                    )? {
                        current_code_id = Some(code_id);
                        pc = 0;
                    }
                }
                Instruction::IteratorNext { state } => {
                    self.acc = self.iterator_next(state, units, heap, realm)?;
                }
                Instruction::ForInNext { state } => {
                    self.acc = self.for_in_next(active_code, state, heap, realm)?;
                }
                Instruction::Throw => return Err(VMError::Thrown(self.acc, None)),
                Instruction::Return => {
                    // 10.2.2 step 13: a derived constructor answers the object
                    // its own `this` binding holds, and refuses every other
                    // value but undefined.
                    if active_code.derived {
                        self.acc = self.derived_result(active_code, heap, realm)?;
                    }
                    if let Some(frame) = self.frames.pop() {
                        self.fp = frame.caller_fp;
                        pc = frame.return_pc;
                        current_code_id = frame.caller_code_id;
                        self.unit = frame.caller_unit;
                        self.active_binding_count = frame.caller_binding_count;
                        self.current_context = frame.caller_context;
                        // 10.2.2 step 13: a constructor that answers no Object
                        // answers the one its call started from.
                        if let Some(target) = frame.construct
                            && self.acc.as_object().is_none()
                        {
                            self.acc = self.read_reg(target)?;
                        }
                        if matches!(frame.resume, Some(Resume::Job { .. })) {
                            // 9.5: the job has answered, so its capability
                            // settles and the next job follows; the run ends
                            // with what the unit answered once none is left.
                            let unit_feedback: &mut FeedbackVector = feedback
                                .get_mut(self.unit as usize)
                                .ok_or(VMError::InvalidFeedbackVector)?;
                            let next =
                                self.continue_jobs(None, units, unit_feedback, heap, realm)?;
                            let Some(code_id) = next else {
                                self.fp = 0;
                                self.active_binding_count = 0;
                                self.current_context = None;
                                return Ok(Some(
                                    self.completion
                                        .and_then(|root| heap.root_value(root))
                                        .unwrap_or(VALUE_UNDEFINED),
                                ));
                            };
                            current_code_id = Some(code_id);
                            pc = 0;
                        } else if let Some(resume) = frame.resume {
                            // An operation of the caller is waiting for this
                            // answer, and runs again once it has one. It belongs
                            // to the caller's unit, which the frame restored.
                            let caller_root = table.root(self.unit).ok_or(
                                VMError::InvalidBytecode(VerificationError::FunctionOutOfBounds {
                                    pc,
                                    index: self.unit,
                                }),
                            )?;
                            let caller = code_unit(caller_root, current_code_id).ok_or(
                                VMError::InvalidBytecode(VerificationError::FunctionOutOfBounds {
                                    pc,
                                    index: current_code_id.unwrap_or(u32::MAX),
                                }),
                            )?;
                            let unit_feedback: &mut FeedbackVector = feedback
                                .get_mut(self.unit as usize)
                                .ok_or(VMError::InvalidFeedbackVector)?;
                            let feedback = feedback_unit_mut(unit_feedback, current_code_id)
                                .ok_or(VMError::InvalidFeedbackVector)?;
                            // A conversion names a register of the caller; a
                            // walk of 23.1.3 names a root instead, so it has
                            // no register of the caller to name here.
                            let register = match resume {
                                Resume::Primitive { register, .. }
                                | Resume::Coercion { register, .. } => register,
                                Resume::Iteration { .. }
                                | Resume::Length { .. }
                                | Resume::Descriptor { .. }
                                | Resume::Getter
                                | Resume::Spread { .. }
                                | Resume::Executor { .. }
                                | Resume::Job { .. }
                                | Resume::Setter { .. } => Reg(0),
                            };
                            let call = Call {
                                receiver: VALUE_UNDEFINED,
                                func: register,
                                arg_start: register,
                                arg_count: 0,
                                slot: 0,
                                resume: Some(resume),
                                construct: None,
                                return_pc: pc,
                                caller_code_id: current_code_id,
                            };
                            let units = CodeUnits {
                                table,
                                active: caller,
                            };
                            let resumed = match resume {
                                Resume::Primitive { .. } | Resume::Coercion { .. } => {
                                    self.finish_conversion(call, units, feedback, heap, realm)?
                                }
                                Resume::Iteration { state } => self.step_array_iteration(
                                    state,
                                    Some(self.acc),
                                    call,
                                    units,
                                    feedback,
                                    heap,
                                    realm,
                                )?,
                                Resume::Length { .. } => self.finish_array_like_length(
                                    resume,
                                    pc,
                                    current_code_id,
                                    units,
                                    feedback,
                                    heap,
                                    realm,
                                )?,
                                Resume::Descriptor { .. } => self.continue_descriptor(
                                    resume,
                                    pc,
                                    current_code_id,
                                    units,
                                    feedback,
                                    heap,
                                    realm,
                                )?,
                                // The getter answered the value of the
                                // property, and the accumulator holds it. The
                                // job branch above answers every job, so the
                                // dispatcher reaches none of those.
                                Resume::Getter | Resume::Job { .. } => None,
                                // The call answered, and the List its
                                // arguments were is no longer reachable.
                                Resume::Spread { .. } => {
                                    heap.exit_scope();
                                    None
                                }
                                // 27.2.3.1 step 8 answers the promise, not
                                // what the executor answered.
                                Resume::Executor { promise, .. } => {
                                    self.acc = heap.root_value(promise).unwrap_or(VALUE_UNDEFINED);
                                    heap.exit_scope();
                                    None
                                }
                                // 13.15.2 answers the value assigned, not what
                                // the setter answered.
                                Resume::Setter { value } => {
                                    self.acc = heap.root_value(value).unwrap_or(VALUE_UNDEFINED);
                                    heap.exit_scope();
                                    None
                                }
                            };
                            if let Some(code_id) = resumed {
                                current_code_id = Some(code_id);
                                pc = 0;
                            }
                        }
                    } else {
                        // 9.5: the unit has answered, and the run ends only
                        // once every job it enqueued has run too.
                        if let Some(root) = self.completion
                            && heap.root_value(root) == Some(VALUE_UNINITIALIZED)
                        {
                            heap.set_root(root, self.acc)?;
                        }
                        if let Some(code_id) =
                            self.continue_jobs(None, units, active_feedback, heap, realm)?
                        {
                            current_code_id = Some(code_id);
                            pc = 0;
                            return Ok(None);
                        }
                        self.fp = 0;
                        self.active_binding_count = 0;
                        self.current_context = None;
                        return Ok(Some(
                            self.completion
                                .and_then(|root| heap.root_value(root))
                                .unwrap_or(self.acc),
                        ));
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

/// An Object reached a conversion that cannot open a frame for `ToPrimitive`.
const NUMERIC_CONVERSION_GAP: VMError =
    VMError::Unsupported("ToPrimitive of an Object outside a call");

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
    // 7.1.4 sends an Object through ToPrimitive, which can call a `valueOf`
    // of the Script. These conversions have no frame to run one in.
    if value.is_object() {
        return Err(NUMERIC_CONVERSION_GAP);
    }
    if value.is_symbol() {
        // 7.1.4 step 2 is a `TypeError`, which this function has no Realm to
        // raise.
        return Err(VMError::Unsupported("ToNumber of a Symbol"));
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

/// What a property access on a base that is no Object answers.
///
/// 7.3.2 sends the base through `ToObject`, which 7.1.18 refuses for undefined
/// and null alone. Every other primitive gets a wrapper Object this engine has
/// not built, so the access names that instead of throwing the `TypeError` only
/// the first two deserve.
fn property_base_error(base: Value, heap: &mut GenerationalHeap, realm: &Realm) -> VMError {
    if base.is_undefined() || base.is_null() {
        return type_error(heap, realm, "property access on null or undefined");
    }
    VMError::Unsupported("ToObject of a primitive for a property access")
}

/// Whether a `[[Set]]` of `name` would reach something this Realm has not
/// built (10.1.9.2).
///
/// A store walks the Prototype Chain when the receiver owns no property of
/// that name. `__proto__` is an accessor of %Object.prototype% on every
/// object; `length` and `name` are own properties an Array or a function
/// carries under rules of its own. None of the three is built, so a store
/// that reaches one names the gap instead of shadowing it. A definition of a
/// literal reaches no Prototype and asks this nothing.
fn store_reaches_unbuilt_prototype(
    object: ObjectRef,
    name: &[u16],
    heap: &GenerationalHeap,
) -> bool {
    let is = |candidate: &str| candidate.encode_utf16().eq(name.iter().copied());
    if is("__proto__") {
        return true;
    }
    let Some(kind) = heap.get_object(object).map(|entry| entry.kind.clone()) else {
        return false;
    };
    // 10.4.2.1 gives an Array's `length` a `[[DefineOwnProperty]]` of its own,
    // which a write under a key only the run time knows does not reach.
    if matches!(kind, ObjectKind::Array { .. }) && is("length") {
        return true;
    }
    // Every other name the object owns is written by 10.1.9.1 on the object
    // itself and reaches no Prototype at all.
    if heap
        .strings
        .lookup_interned_units(name)
        .map(PropertyKey::String)
        .and_then(|key| heap.own_named_flags(object, key).ok().flatten())
        .is_some()
    {
        return false;
    }
    match kind {
        ObjectKind::Function { .. } | ObjectKind::NativeFunction { .. } => {
            is("length") || is("name")
        }
        _ => false,
    }
}

/// The Reference half of 13.5.1.2: the base goes through `ToObject`, the name
/// through `[[Delete]]`, and a strict Reference that was refused throws.
fn delete_reference(
    base: Value,
    name: PropertyKey,
    index: Option<u32>,
    strict: bool,
    heap: &mut GenerationalHeap,
    realm: &Realm,
) -> Result<Value, VMError> {
    let Some(object) = base.as_object() else {
        return Err(property_base_error(base, heap, realm));
    };
    if delete_property(object, name, index, heap)? {
        return Ok(VALUE_TRUE);
    }
    if strict {
        return Err(type_error(
            heap,
            realm,
            "delete of a property that is not configurable",
        ));
    }
    Ok(VALUE_FALSE)
}

/// `[[Delete]]` of an ordinary object (10.1.10.1) and of the element store an
/// Array exotic object keeps its indices in (10.4.2).
///
/// A property that is not there was deleted, and one that is not configurable
/// was not. The Shape is a transition tree, so removing a name builds the
/// chain again without it: O(n) in the own properties of the object.
fn delete_property(
    object: ObjectRef,
    name: PropertyKey,
    index: Option<u32>,
    heap: &mut GenerationalHeap,
) -> Result<bool, VMError> {
    let entry = heap.get_object(object).ok_or(VMError::TypeError)?;
    let elements = entry.elements;
    let shape = entry.shape_id;
    if let (Some(index), Some(elements)) = (index, elements) {
        // 10.1.6.3 moves an index that carries its own attributes out of the
        // store and into the Shape, and the delete of one belongs there: the
        // store holds a hole for it and answering from there would leave the
        // name on the object.
        if !RegisterVM::shape_holds(object, name, heap)? {
            return Ok(heap.delete_element(elements, index)?);
        }
        heap.delete_element(elements, index)?;
    }
    // 10.4.2.1 gives an Array its own `length`, which no Shape carries and
    // which is never configurable.
    if heap.array_length(object).is_some()
        && heap
            .strings
            .lookup_interned_units(&LENGTH_NAME)
            .is_some_and(|length| PropertyKey::String(length) == name)
    {
        return Ok(false);
    }
    // 10.4.3.1 gives a String exotic object own names that are never
    // configurable, and 10.1.10 answers false for one of those.
    if RegisterVM::owns_string_exotic(object, name, heap)? {
        return Ok(false);
    }
    let Some(location) = heap.shapes.lookup(shape, name) else {
        return Ok(true);
    };
    if !location.flags.configurable {
        return Ok(false);
    }
    let kept = heap.shapes.own_properties(shape);
    let mut moved = Vec::with_capacity(kept.len());
    let mut next = heap.shapes.root_shape();
    for (key, flags, old) in kept.into_iter().filter(|(key, _, _)| *key != name) {
        let value = heap
            .get_object(object)
            .and_then(|entry| entry.get_slot(old))
            .ok_or(VMError::TypeError)?;
        let (shape, slot) = heap.shapes.transition(next, key, flags);
        next = shape;
        moved.push((slot, value));
    }
    heap.set_object_shape(object, next)?;
    for (slot, value) in moved {
        heap.set_object_slot(object, slot, value)?;
    }
    Ok(true)
}

/// What a property assignment on a base that is no Object answers.
///
/// 6.2.5.5 sends the base of the Reference through `ToObject` before it
/// writes, and 7.1.18 refuses undefined and null there. Every other primitive
/// gets a wrapper Object this engine has not built.
fn property_store_error(base: Value, heap: &mut GenerationalHeap, realm: &Realm) -> VMError {
    if base.is_undefined() || base.is_null() {
        return type_error(heap, realm, "property assignment on null or undefined");
    }
    VMError::Unsupported("ToObject of a primitive for a property assignment")
}

/// The Array length a Number denotes, if it is one (23.1.1.1).
///
/// `ToUint32` and `SameValueZero` together take the Numbers that name a length
/// and no others: a fraction, a negative, a NaN and anything past 2^32 - 1
/// name none.
fn exact_array_length(number: f64) -> Option<u32> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the comparison answers for exactly the values this converts"
    )]
    let length = number as u32;
    #[expect(clippy::float_cmp, reason = "SameValueZero of an integral Number")]
    (f64::from(length) == number).then_some(length)
}

/// The Array index a property name denotes, if it is one (10.4.2.1).
///
/// Only the canonical decimal of an index is one: a name with a leading zero,
/// a sign or a fraction names an ordinary property.
fn array_index_units(units: &[u16]) -> Option<u32> {
    let (first, rest) = units.split_first()?;
    if *first == 0x30 {
        return rest.is_empty().then_some(0);
    }
    let mut index: u32 = 0;
    for unit in units {
        let digit = u32::from(*unit)
            .checked_sub(0x30)
            .filter(|digit| *digit < 10)?;
        index = index.checked_mul(10)?.checked_add(digit)?;
    }
    Some(index)
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
    // A Shape looks its names up by reference, so a String the Script made is
    // not the key the Shape holds until it is interned.
    if let Some(name) = value.as_heap_string()
        && heap.strings.is_interned(name)
    {
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

/// `SameValue` of 7.2.11: strict equality, except that NaN matches NaN and
/// the two zeroes do not match each other.
fn same_value(left: Value, right: Value, heap: &GenerationalHeap) -> Result<bool, VMError> {
    if let (Some(left), Some(right)) = (left.as_f64(), right.as_f64()) {
        if left.is_nan() && right.is_nan() {
            return Ok(true);
        }
        // 6.1.6.1.14 tells +0 from -0, which strict equality does not.
        if left == 0.0 && right == 0.0 {
            return Ok(left.is_sign_positive() == right.is_sign_positive());
        }
    }
    RegisterVM::strictly_equals(left, right, heap)
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

/// Raises a native error whose message is built at run time.
///
/// The error object carries the message, so the embedding reads it from there
/// instead of from a fixed diagnostic beside it.
fn raise_message(
    heap: &mut GenerationalHeap,
    realm: &Realm,
    kind: super::realm::NativeErrorKind,
    message: &str,
) -> VMError {
    match realm.create_native_error(heap, kind, message) {
        Ok(error) => VMError::Thrown(Value::from_object(error), None),
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
fn integer_argument(
    value: Value,
    heap: &mut GenerationalHeap,
    realm: &Realm,
) -> Result<i64, VMError> {
    // 7.1.5 goes through 7.1.4, which step 2 refuses for a Symbol. The
    // conversion carries the Realm so that refusal is the TypeError it is and
    // not a gap.
    if value.is_symbol() {
        return Err(type_error(heap, realm, "cannot convert Symbol to a number"));
    }
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

/// The `prototype` 10.2.5 gives a constructor, as code units.
const PROTOTYPE_NAME: [u16; 9] = [0x70, 0x72, 0x6F, 0x74, 0x6F, 0x74, 0x79, 0x70, 0x65];

/// `description`, which 20.4.3.2 gives a Symbol.
const DESCRIPTION_NAME: [u16; 11] = [
    0x64, 0x65, 0x73, 0x63, 0x72, 0x69, 0x70, 0x74, 0x69, 0x6F, 0x6E,
];

/// `callee`, which 10.4.4 gives the arguments object.
const CALLEE_NAME: [u16; 6] = [0x63, 0x61, 0x6C, 0x6C, 0x65, 0x65];

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

/// `ToString` of 7.1.17 for a value that needs no method of the Script.
///
/// An Object would go through `ToPrimitive` (7.1.1), which runs a `valueOf` or
/// a `toString` this function cannot enter, so it names that as a gap rather
/// than answering the text of an object it did not ask.
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
    if value.is_symbol() {
        // 7.1.17 step 2: a Symbol has no String of its own, which is a
        // `TypeError` this function has no Realm to raise.
        return Err(VMError::Unsupported("ToString of a Symbol"));
    }
    Err(VMError::Unsupported("ToString of an Object"))
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
            strict: false,
            define: false,
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
    fn primitive_to_number_takes_neither_a_symbol_nor_a_bigint_nor_an_object() {
        let mut code = BytecodeFunction::new(1, 1);
        code.emit(Instruction::Ldar(Reg(0)));
        code.emit(Instruction::ToNumber);
        code.emit(Instruction::Return);
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let object = heap
            .allocate_object(heap.shapes.root_shape(), VALUE_NULL)
            .unwrap();

        for (value, expected) in [
            (
                Value::from_symbol(super::super::value::SymbolRef(0)),
                VMError::Unsupported("ToNumber of a Symbol"),
            ),
            (
                Value::from_bigint(super::super::value::BigIntRef(0)),
                VMError::TypeError,
            ),
            // 7.1.4 sends an Object through ToPrimitive; this conversion has
            // no frame to run a `valueOf` of the Script in.
            (Value::from_object(object), NUMERIC_CONVERSION_GAP),
        ] {
            let mut feedback = FeedbackVector::for_code(&code);
            let mut vm = RegisterVM::new(100);
            assert_eq!(
                vm.run_with_arguments(&code, &[value], &mut feedback, &mut heap, &realm),
                Err(expected)
            );
        }
    }

    #[test]
    fn primitive_loose_equality_takes_neither_an_object_nor_a_bigint() {
        let mut code = BytecodeFunction::new(2, 2);
        code.emit(Instruction::Ldar(Reg(0)));
        code.emit(Instruction::TestEqual(Reg(1)));
        code.emit(Instruction::Return);
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let object = heap
            .allocate_object(heap.shapes.root_shape(), VALUE_NULL)
            .unwrap();

        // 7.2.14 step 12 answers an Object against undefined without
        // converting it; every other Object needs a frame this has not.
        for (left, right, expected) in [
            (
                VALUE_UNDEFINED,
                Value::from_bigint(super::super::value::BigIntRef(0)),
                VMError::TypeError,
            ),
            (
                Value::from_smi(1),
                Value::from_object(object),
                NUMERIC_CONVERSION_GAP,
            ),
        ] {
            let mut feedback = FeedbackVector::for_code(&code);
            let mut vm = RegisterVM::new(100);
            assert_eq!(
                vm.run_with_arguments(&code, &[left, right], &mut feedback, &mut heap, &realm),
                Err(expected)
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
            strict: false,
            obj: Reg(0),
            key: Reg(1),
            slot: 0,
            define: false,
        });
        code.emit(Instruction::LdaSmi(1));
        code.emit(Instruction::Star(Reg(1)));
        code.emit(Instruction::LdaSmi(2));
        code.emit(Instruction::SetByValue {
            strict: false,
            obj: Reg(0),
            key: Reg(1),
            slot: 1,
            define: false,
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
            strict: false,
            obj: Reg(0),
            key: Reg(1),
            slot: 0,
            define: false,
        });
        code.emit(Instruction::LdaSmi(-2));
        code.emit(Instruction::Star(Reg(1)));
        code.emit(Instruction::LdaSmi(8));
        code.emit(Instruction::SetByValue {
            strict: false,
            obj: Reg(0),
            key: Reg(1),
            slot: 0,
            define: false,
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
            lhs: Reg(0),
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
            strict: false,
            define: false,
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
        // An Object key needs ToPrimitive, which this function cannot run, so
        // it names the gap rather than answering a text it did not ask for.
        let object = realm.ordinary_object(&mut heap).unwrap();
        assert_eq!(
            property_key(Value::from_object(object), &mut heap),
            Err(VMError::Unsupported("ToString of an Object"))
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
            strict: false,
            obj: array,
            key: method,
            slot: get,
            define: false,
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
        let empty = BytecodeFunction::new(1, 1);
        let roots = [&empty];
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap).unwrap();
        let mut vm = RegisterVM::with_limits(1000, 8, 8);

        // A primitive is not an iterator.
        vm.write_reg(Reg(0), Value::from_smi(1)).unwrap();
        assert!(matches!(
            vm.iterator_next(
                Reg(0),
                CodeUnits {
                    table: CodeTable::new(&roots),
                    active: &empty,
                },
                &mut heap,
                &realm
            ),
            Err(VMError::Thrown(
                _,
                Some((super::super::realm::NativeErrorKind::TypeError, _))
            ))
        ));

        // An object without a callable `next` is not one either.
        let plain = realm.ordinary_object(&mut heap).unwrap();
        vm.write_reg(Reg(0), Value::from_object(plain)).unwrap();
        assert!(matches!(
            vm.iterator_next(
                Reg(0),
                CodeUnits {
                    table: CodeTable::new(&roots),
                    active: &empty,
                },
                &mut heap,
                &realm
            ),
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
            vm.iterator_next(
                Reg(0),
                CodeUnits {
                    table: CodeTable::new(&roots),
                    active: &empty,
                },
                &mut heap,
                &realm
            ),
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
        let empty = BytecodeFunction::new(1, 0);
        let roots = alloc::vec![&empty];
        let units = CodeUnits {
            table: CodeTable::new(&roots),
            active: &empty,
        };
        let call = Call {
            receiver: VALUE_UNDEFINED,
            func: Reg(0),
            arg_start: Reg(0),
            arg_count: 0,
            slot: 0,
            return_pc: 0,
            resume: None,
            caller_code_id: None,
            construct: None,
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
                units,
                &mut heap,
                &realm
            ),
            Err(VMError::InvalidFeedbackVector)
        );
    }
}
