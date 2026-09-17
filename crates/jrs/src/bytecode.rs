// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Only the compiler creates bytecode. Bindings resolve to stable slot indices;
//! entering a lexical scope resets its slots to the temporal dead zone.

use crate::{
    Error, Limits, Value,
    parser::{self, Binary, BindingPattern, Expr, ExprKind, Function, Stmt, Unary},
};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    string::String,
    vec::Vec,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    Local(usize),
    Capture(usize),
}

#[derive(Clone, Debug)]
pub(crate) enum Op {
    Nop,
    Constant(Value),
    ToString,
    Await,
    Argument(usize),
    RestArguments(usize),
    EndParameters,
    ForInInit(usize),
    ForInNext(usize, usize),
    ForInEnd(usize),
    ForOfInit(usize),
    IteratorValue(usize),
    IteratorSkip(usize),
    IteratorRest(usize),
    IteratorGuard(usize, usize),
    IteratorEnd(usize),
    ObjectBindingStart,
    ObjectBindingGet(usize),
    ObjectAssignmentGet {
        excluded: usize,
        target_slots: usize,
    },
    ObjectBindingRest(usize),
    ObjectAssignmentRest {
        excluded: usize,
        target_slots: usize,
    },
    ObjectBindingEnd(usize),
    KeyBelow,
    RotateKey,
    Regex(Rc<crate::regexp::RegExp>),
    Load(Access),
    Store(Access),
    Init(usize),
    Reset(usize),
    Missing(String),
    GetGlobal(String, bool),
    SetGlobal(String, bool),
    InitGlobal(String),
    UpdateGlobal(String, bool, bool, bool),
    DeleteGlobal(String),
    Unary(Unary),
    Binary(Binary),
    Dup,
    Pop,
    Result,
    Jump(usize),
    Branch(usize, Branch),
    Call(usize),
    CallExpanded,
    Eval(usize, bool),
    EvalExpanded(bool),
    ConstructExpanded,
    ArgumentAppend(bool),
    ArrayAppend(bool),
    ArrayHole,
    Construct(usize),
    Closure(usize),
    Return,
    Renew(usize),
    Update(Access, bool, bool),
    Object,
    Array(u32),
    Define(bool),
    Accessor(bool),
    Key,
    Get(bool),
    Set(bool),
    Delete(bool),
    UpdateProperty(bool, bool, bool),
    DupPair,
    This,
    NewTarget,
    SuperBase,
    SuperCall(usize),
    PrepareSuperCall,
    SuperCallExpanded,
    DefaultSuper,
    SuperGet(bool),
    SuperSet,
    SuperUpdate(bool, bool),
    Class(bool),
    ClassMethod(bool, Option<bool>),
    MethodHome,
    Global,
    Math,
    Json,
    Reflect,
    Handler {
        catch: Option<usize>,
        finally: Option<usize>,
        end: usize,
    },
    EndTry,
    EndFinally,
    Throw,
    AbruptJump(usize),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Branch {
    False,
    True,
    NotNullish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Builtin {
    JsonParse,
    ReflectApply,
    ReflectConstruct,
    ReflectGet,
    ReflectSet,
    ReflectHas,
    ReflectDeleteProperty,
    ReflectGetPrototypeOf,
    ReflectSetPrototypeOf,
    ReflectIsExtensible,
    ReflectPreventExtensions,
    ReflectGetOwnPropertyDescriptor,
    ReflectDefineProperty,
    ReflectOwnKeys,
    JsonStringify,
    JsonRaw,
    JsonIsRaw,
    Event,
    CustomEvent,
    DomException,
    CustomEventInit,
    EventTarget,
    EventAdd,
    EventRemove,
    EventDispatch,
    EventPrevent,
    EventStop,
    EventStopImmediate,
    EventPath,
    EventInit,
    AbortController,
    AbortSignal,
    Abort,
    SignalAbort,
    SignalAny,
    SignalThrow,
    AbortHandler,
    ArrayKeys,
    ArrayValues,
    ArrayEntries,
    StringIterator,
    ArrayIteratorNext,
    StringIteratorNext,
    IteratorIdentity,
    Symbol,
    SymbolFor,
    SymbolKeyFor,
    SymbolValueOf,
    SymbolToPrimitive,
    SymbolToString,
    SymbolDescription,
    GetOwnPropertySymbols,
    WeakMap,
    WeakMapConstruct,
    WeakMapGet,
    WeakMapSet,
    WeakMapHas,
    WeakMapDelete,
    WeakMapGetOrInsert,
    WeakMapGetOrInsertComputed,
    Print,
    Number,
    Boolean,
    String,
    NumberConstruct,
    BooleanConstruct,
    StringConstruct,
    NumberValueOf,
    BooleanValueOf,
    StringValueOf,
    NumberToString,
    BooleanToString,
    StringToString,
    IsNaN,
    IsFinite,
    ParseInt,
    ParseFloat,
    SetTimeout,
    SetInterval,
    ClearTimeout,
    SignalTimeout,
    NumberIsFinite,
    NumberIsInteger,
    NumberIsNaN,
    NumberIsSafeInteger,
    Object,
    ObjectCreate,
    GetPrototypeOf,
    SetPrototypeOf,
    DefineProperty,
    DefineProperties,
    GetOwnPropertyDescriptor,
    HasOwn,
    HasOwnProperty,
    PropertyIsEnumerable,
    IsPrototypeOf,
    PreventExtensions,
    IsExtensible,
    Freeze,
    Seal,
    ObjectIs,
    IsFrozen,
    IsSealed,
    FunctionCall,
    FunctionBind,
    FunctionHasInstance,
    FunctionApply,
    Function,
    StringSplit,
    StringReplace,
    StringFromCharCode,
    StringAt,
    StringCharAt,
    StringCharCodeAt,
    StringCodePointAt,
    StringConcat,
    StringIndexOf,
    StringLastIndexOf,
    StringIncludes,
    StringStartsWith,
    StringEndsWith,
    StringSlice,
    StringSubstring,
    StringRepeat,
    StringPadStart,
    StringPadEnd,
    StringTrim,
    StringTrimStart,
    StringTrimEnd,
    StringIsWellFormed,
    StringToWellFormed,
    Error,
    TypeError,
    RangeError,
    ReferenceError,
    SyntaxError,
    EvalError,
    URIError,
    Proxy,
    Eval,
    ErrorToString,
    ErrorIsError,
    ThrowTypeError,
    ObjectToString,
    ObjectValueOf,
    FunctionToString,
    Array,
    ArrayConcat,
    ArraySpecies,
    ArrayIsArray,
    ArrayOf,
    ArrayFrom,
    ArrayPush,
    ArrayPop,
    ArrayJoin,
    ArrayToString,
    ArrayForEach,
    ArrayMap,
    ArrayFilter,
    ArraySome,
    ArrayEvery,
    ArrayReduce,
    ArrayIndexOf,
    ArrayLastIndexOf,
    ArrayReverse,
    ArrayFill,
    ArrayCopyWithin,
    ArraySplice,
    ArrayAt,
    ArrayFind,
    ArrayFindIndex,
    ArrayFindLast,
    ArrayFindLastIndex,
    ArrayIncludes,
    ArraySlice,
    ArraySort,
    ArrayToSorted,
    ArrayToReversed,
    ArrayWith,
    ArrayToSpliced,
    ArrayFlat,
    ArrayFlatMap,
    ObjectKeys,
    GetOwnPropertyNames,
    ObjectEntries,
    ObjectValues,
    InternalCall,
    InternalDefine,
    InternalHas,
    InternalReduceEmpty,
    Promise,
    PromiseCapability,
    PromiseSpecies,
    InternalCallable,
    PromiseResolve,
    PromiseReject,
    PromiseThen,
    PromiseCatch,
    PromiseFinally,
    PromiseAll,
    PromiseAllSettled,
    PromiseRace,
    InternalPromiseResolve,
    PromiseWithResolvers,
    MathMax,
    MathPow,
    MathMin,
    MathAbs,
    MathFloor,
    MathCeil,
    MathRound,
    MathTrunc,
    MathSqrt,
    MathSign,
    MathSin,
    MathClz32,
    RegExp,
    RegExpExec,
    RegExpTest,
    RegExpToString,
    StringMatch,
    StringSearch,
    RegExpMatch,
    RegExpSearch,
    RegExpSplit,
    RegExpReplace,
}

#[derive(Clone, Debug)]
pub(crate) struct Slot {
    pub(crate) name: String,
    pub(crate) mutable: bool,
    pub(crate) captured: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct FunctionCode {
    pub(crate) source: Option<parser::Source>,
    pub(crate) program: Program,
    pub(crate) captures: Vec<Access>,
    pub(crate) parameters: Vec<usize>,
    pub(crate) self_slot: Option<usize>,
    pub(crate) arrow: bool,
    pub(crate) strict: bool,
    pub(crate) constructible: bool,
    pub(crate) initialization: ParameterInitialization,
    pub(crate) length: usize,
    pub(crate) async_kind: parser::AsyncKind,
    pub(crate) arguments_slot: Option<usize>,
    pub(crate) name: String,
    pub(crate) constructor_kind: parser::ConstructorKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParameterInitialization {
    Direct,
    Bytecode,
}

/// Immutable compiled script. It can be reused across isolated executions.
#[derive(Clone, Debug)]
pub struct Program {
    pub(crate) code: Vec<Op>,
    pub(crate) slots: Vec<Slot>,
    pub(crate) functions: Vec<Rc<FunctionCode>>,
    total_instructions: usize,
    pub(crate) globals: Vec<GlobalDecl>,
    pub(crate) register_code: Option<Rc<crate::engine::bytecode::BytecodeFunction>>,
    /// The construct the register lowering would not take, when it took none.
    pub(crate) register_refusal: Option<&'static str>,
    /// The construct the stack backend would not take, when it took none and
    /// the register lowering did. It goes away with the legacy backend.
    pub(crate) stack_refusal: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub(crate) struct GlobalDecl {
    pub(crate) name: String,
    pub(crate) kind: GlobalKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GlobalKind {
    Var,
    Function,
    Lexical(bool),
}

impl Program {
    pub(crate) const fn empty() -> Self {
        Self {
            code: Vec::new(),
            slots: Vec::new(),
            functions: Vec::new(),
            globals: Vec::new(),
            register_code: None,
            register_refusal: None,
            stack_refusal: None,
            total_instructions: 0,
        }
    }
    /// Number of bytecode instructions, including scope initialization.
    #[must_use]
    pub const fn instruction_count(&self) -> usize {
        self.total_instructions
    }
    /// Number of statically allocated binding slots.
    #[must_use]
    pub fn binding_count(&self) -> usize {
        if let Some(code) = &self.register_code {
            usize::from(code.binding_count)
        } else {
            self.slots.len()
        }
    }

    /// Returns whether this Program executes through verified Register bytecode.
    #[must_use]
    pub const fn uses_register_backend(&self) -> bool {
        self.register_code.is_some()
    }

    /// The same Program with the Register backend withheld, so that the legacy
    /// stack backend executes it.
    ///
    /// This exists for the differential testing the backend migration needs:
    /// the two backends must answer a source identically. It goes away with the
    /// legacy backend.
    #[must_use]
    pub fn legacy_only(&self) -> Self {
        Self {
            register_code: None,
            register_refusal: None,
            stack_refusal: None,
            ..self.clone()
        }
    }
}

/// Parses and compiles a script without executing any host operation.
///
/// # Errors
/// Returns [`Error::Syntax`] for invalid syntax and duplicate declarations,
/// [`Error::Unsupported`] for recognized but unavailable language features, or
/// [`Error::Limit`] when a compilation budget is exceeded.
pub fn compile(source: &str, limits: Limits) -> Result<Program, Error> {
    compile_mode(source, limits, false)
}
pub(crate) fn compile_realm(source: &str, limits: Limits) -> Result<Program, Error> {
    compile_mode(source, limits, true)
}
/// Compiled global Script, reusable in independent realms with identical limits.
/// It owns no runtime values or heap identities.
pub struct Script {
    pub(crate) program: Program,
    pub(crate) limits: Limits,
}
/// Compiles a global Script without instantiating declarations or executing code.
///
/// # Errors
/// Syntax and early errors, unsupported grammar, or compilation resource exhaustion.
pub fn compile_script(source: &str, limits: Limits) -> Result<Script, Error> {
    Ok(Script {
        program: compile_realm(source, limits)?,
        limits,
    })
}
fn compile_mode(source: &str, limits: Limits, realm: bool) -> Result<Program, Error> {
    let body = parser::parse(source, limits)?;
    compile_parsed(&body, limits, realm)
}

pub(crate) fn compile_eval(
    source: &str,
    limits: Limits,
    strict_caller: bool,
) -> Result<Program, Error> {
    let body = parser::parse_eval(source, limits, strict_caller)?;
    compile_parsed(&body, limits, true)
}

fn compile_parsed(body: &[Stmt], limits: Limits, realm: bool) -> Result<Program, Error> {
    let mut compiler = Compiler {
        program: Program {
            code: Vec::new(),
            slots: Vec::new(),
            functions: Vec::new(),
            total_instructions: 0,
            globals: Vec::new(),
            register_code: None,
            register_refusal: None,
            stack_refusal: None,
        },
        scopes: Vec::new(),
        loops: Vec::new(),
        limits,
        outer: BTreeMap::new(),
        captures: Vec::new(),
        capture_names: BTreeMap::new(),
        enumerations: 0,
        realm,
    };
    compiler.scopes.push(BTreeMap::new());
    let outcome = if realm {
        compiler.global_body(body)
    } else {
        compiler.root_body(body)
    };
    // A feature of the new engine that the stack backend has no value for
    // leaves that backend without code. The Script still runs, on the engine
    // the migration is heading for, and the stack backend names the gap where
    // it is asked to run.
    let stack_refusal = match outcome {
        Ok(()) => None,
        Err(Error::Unsupported { feature }) => Some(feature),
        Err(error) => return Err(error),
    };
    compiler.finish();
    let (register_code, register_refusal) = lower_register_script(
        body,
        realm,
        u64::try_from(compiler.program.total_instructions).unwrap_or(u64::MAX),
        limits.properties,
    );
    if stack_refusal.is_some() && register_code.is_none() {
        return Err(Error::Unsupported {
            feature: stack_refusal.unwrap_or("a Script neither backend takes"),
        });
    }
    compiler.program.register_code = register_code.map(Rc::new);
    compiler.program.register_refusal = register_refusal;
    compiler.program.stack_refusal = stack_refusal;
    Ok(compiler.program)
}

/// Where `@@iterator` stands in [`crate::engine::realm::WellKnownSymbol::ALL`],
/// which is the index `GetWellKnown` takes.
const WELL_KNOWN_ITERATOR: usize = 3;

/// The binding a frame holds its `this` value in.
///
/// No program can declare it: `this` is a keyword, so the name cannot collide
/// with one a Script writes.
const THIS_BINDING: &str = "this";

/// The binding that holds the `[[HomeObject]]` of the running function, which
/// 13.3.7.3 reads the Prototype of. No Script can name it.
const HOME_BINDING: &str = "*home";

/// The binding that holds the `[[NewTarget]]` of the call (9.4.3). No Script
/// can name it.
const NEW_TARGET_BINDING: &str = "*newTarget";

/// The binding that holds the function object of the running call, which 9.4.4
/// reads the Prototype of. No Script can name it.
const CALLEE_BINDING: &str = "*callee";

/// The name the register of the capability of 27.7.5.2 is declared under, so
/// the frame reserves a slot for it that no name of a Script can reach.
const PROMISE_BINDING: &str = "*promise";

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegisterType {
    Array(u32),
    Function(u32),
    NativeFunction(crate::engine::realm::Intrinsic),
    Number,
    NumberOrUndefined,
    Boolean,
    Null,
    Object(u32),
    Primitive,
    String,
    Unknown,
    Undefined,
}

impl RegisterType {
    const fn is_primitive(self) -> bool {
        !matches!(
            self,
            Self::Array(_)
                | Self::Function(_)
                | Self::NativeFunction(_)
                | Self::Object(_)
                | Self::Unknown
        )
    }

    const fn is_object(self) -> bool {
        matches!(self, Self::Array(_) | Self::Function(_) | Self::Object(_))
    }

    /// Whether a value of this type may leave a function.
    ///
    /// `Return` carries the accumulator whatever it holds, so a type the
    /// lowering could not name is returnable too: the call site receives it as
    /// `Unknown`, which is what a call it could not name already produces.
    const fn is_returnable(self) -> bool {
        self.is_primitive() || self.is_object() || matches!(self, Self::Unknown)
    }

    const fn is_numeric_primitive(self) -> bool {
        matches!(self, Self::Number | Self::NumberOrUndefined)
    }

    fn accepts(self, actual: Self) -> bool {
        match self {
            Self::Primitive => actual.is_primitive(),
            Self::Unknown => true,
            _ => self == actual,
        }
    }

    /// Whether a conversion that wants a primitive may be given this value.
    ///
    /// A type the lowering could not name may be an Object, which 7.1.4 and
    /// 7.2.14 send through `ToPrimitive`. The conversion names that as a gap
    /// where it happens, so the lowering does not refuse the whole Script for
    /// a value that is a primitive in every run that reaches it.
    const fn converts_to_primitive(self) -> bool {
        self.is_primitive() || matches!(self, Self::Unknown)
    }

    /// The identifier of the layout this lowering tracks for the value, if it
    /// tracks one.
    const fn object_id(self) -> Option<u32> {
        match self {
            Self::Array(id) | Self::Object(id) => Some(id),
            _ => None,
        }
    }

    fn merge(self, other: Self) -> Self {
        if self == other {
            self
        } else if matches!(
            self,
            Self::Number | Self::NumberOrUndefined | Self::Undefined
        ) && matches!(
            other,
            Self::Number | Self::NumberOrUndefined | Self::Undefined
        ) {
            Self::NumberOrUndefined
        } else if self.is_primitive() && other.is_primitive() {
            Self::Primitive
        } else {
            Self::Unknown
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum RegisterObjectLayout {
    Ordinary {
        properties: BTreeMap<Vec<u16>, RegisterType>,
        order: Vec<Vec<u16>>,
        dynamic: Option<RegisterType>,
    },
    Array {
        length: Option<u32>,
        elements: BTreeMap<u32, RegisterType>,
        dynamic: Option<RegisterType>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RegisterBinding {
    storage: RegisterBindingStorage,
    value_type: Option<RegisterType>,
    mutable: bool,
    stable_function_identity: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegisterBindingStorage {
    Register(crate::engine::bytecode::Reg),
    Context { depth: u16, slot: u16 },
}

struct RegisterMemberAssignment {
    object: crate::engine::bytecode::Reg,
    base_type: RegisterType,
    key: RegisterMemberKey,
}

enum RegisterMemberKey {
    Named {
        constant: u16,
        name: Vec<u16>,
    },
    ArrayKeyed {
        register: crate::engine::bytecode::Reg,
        array_index: Option<u32>,
    },
    ObjectKeyed(crate::engine::bytecode::Reg, Option<Vec<u16>>),
}

enum RegisterPreparedAssignment {
    NestedPattern,
    Name,
    Member(RegisterMemberAssignment),
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag names one independent property of the lowering"
)]
struct RegisterLowerer {
    code: crate::engine::bytecode::BytecodeFunction,
    function_table_base: u32,
    next_register: u16,
    register_count: u16,
    local_count: u16,
    active_binding_count: u16,
    max_binding_count: u16,
    bindings: BTreeMap<String, RegisterBinding>,
    loops: Vec<RegisterLoop>,
    /// The names each Block scope in flight introduced, innermost last.
    block_scoped: Vec<BTreeSet<String>>,
    /// Every code id 10.2.5 made a constructor.
    constructible: BTreeSet<u32>,
    /// Which types a loop head starts from. Widened only for the second
    /// attempt of [`Self::lower_loop`].
    loop_head_types: RegisterLoopHead,
    completions: Vec<crate::engine::bytecode::Reg>,
    /// Types thrown lexically inside each enclosing protected range.
    thrown: Vec<Vec<RegisterType>>,
    next_object_id: u32,
    object_layouts: BTreeMap<u32, RegisterObjectLayout>,
    property_limit: usize,
    function_returns: BTreeMap<u32, RegisterType>,
    function_parameters: BTreeMap<u32, Vec<RegisterType>>,
    function_capture_effects: BTreeMap<u32, BTreeMap<String, RegisterType>>,
    function_layout_effects: BTreeMap<u32, BTreeMap<u32, RegisterObjectLayout>>,
    binding_type_hints: BTreeMap<String, RegisterType>,
    allow_return: bool,
    /// The Finally Blocks a `return` of 14.15.3 has to run before it leaves,
    /// innermost last.
    finallies: Vec<RegisterFinally>,
    /// The iterators of the enclosing `for`-`of` statements, innermost last,
    /// which 7.4.9 closes where a `return` leaves them. Each pair is the
    /// iterator and the register the close reads its `return` method into.
    open_iterators: Vec<(crate::engine::bytecode::Reg, crate::engine::bytecode::Reg)>,
    /// The binding 10.4.4 made for `arguments`, when this body reads it.
    arguments_binding: Option<RegisterBinding>,
    /// How many formal parameters 10.4.4.7 could map the indices of the
    /// arguments object onto.
    mapped_parameters: usize,
    /// Whether 10.4.4.7 builds the map of this function, which the body
    /// prologue puts the parameters in the own context for.
    maps_arguments: bool,
    /// Strictness of the Reference the assignment being lowered names, which
    /// 10.1.9.1 reads to decide whether a write it refuses throws.
    assignment_strict: bool,
    /// Whether a pattern being bound declares lexical bindings of the Global
    /// Environment Record, which 16.1.7 initializes rather than writes.
    initializing_global_lexical: bool,
    /// Set while the base of a property read is lowered, which is the one
    /// place `arguments` may be read: the mapping of 10.4.4.7 is not built,
    /// so a body that could observe it is not lowered.
    reading_member_base: bool,
    return_type: Option<RegisterType>,
    /// Whether a name no binding covers is resolved on the Global Environment
    /// Record. A function of a Realm Script resolves its free names there too,
    /// so this is inherited by the lowering of every function it contains.
    realm: bool,
    /// Whether the top-level `var` names of this unit belong to the Global
    /// Environment Record rather than to the unit. Only a Realm Script does;
    /// a function of one keeps its own var scope.
    script_globals: bool,
    /// The innermost construct this lowering would not take. The first one
    /// recorded is the one that stopped it; an enclosing node fails only
    /// because this one did, so it does not overwrite the name.
    refusal: Option<&'static str>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegisterFlow {
    Empty,
    Value(RegisterType),
    Abrupt,
}

/// An Array layout read out: its identifier, its length, the types of its
/// indexed elements, and the type an index it does not hold yields.
type RegisterArrayLayout<'a> = (
    u32,
    Option<u32>,
    &'a BTreeMap<u32, RegisterType>,
    Option<RegisterType>,
);

/// The binding a `for`-`in` head writes each iteration.
#[derive(Clone, Copy)]
enum ForInHead<'a> {
    /// A lexical head: 14.7.5.5 makes one binding per iteration.
    PerIteration { name: &'a str, mutable: bool },
    /// A `var` head: the one binding the declaration made, in its register.
    Var {
        name: &'a str,
        register: crate::engine::bytecode::Reg,
        declared_type: RegisterType,
    },
    /// A `var` head of a Realm Script, whose binding 16.1.7 made on the
    /// Global Environment Record.
    Global { name: &'a str },
    /// A head that is a binding pattern, which 8.6.2 binds out of the value
    /// of each step.
    Pattern {
        /// The pattern the head declared.
        pattern: &'a BindingPattern,
        /// Whether the head is lexical, and if so whether it is mutable.
        lexical: Option<bool>,
    },
    /// A head that declares nothing: 14.7.5.6 step 7.g evaluates the target as
    /// a Reference of its own and writes the value of each step through it.
    Target(&'a parser::AssignmentTarget),
}

/// What a `for`-`in` or `for`-`of` head leaves for its body.
#[derive(Clone, Copy)]
struct IterationHead<'a> {
    /// Offset the back edge returns to.
    head: usize,
    /// Jump taken when the step produced a value.
    enter: usize,
    /// Jump taken when it did not.
    exit: usize,
    /// Name of the Global Environment Record the loop variable is written to
    /// as well, when the head declared one there.
    global: Option<u16>,
    /// The pattern the head declared, which 8.6.2 binds out of the value of
    /// each step.
    pattern: Option<&'a BindingPattern>,
    /// The target a head that declares nothing writes each step through.
    target: Option<&'a parser::AssignmentTarget>,
    /// Register the loop variable is written to.
    variable: crate::engine::bytecode::Reg,
    /// Register holding the loop's completion value.
    result: crate::engine::bytecode::Reg,
    /// Register the produced value is read from, when it is not the accumulator.
    source: Option<crate::engine::bytecode::Reg>,
    /// Layout the loop variable's type was read from, which the body may not
    /// change.
    guarded_layout: Option<u32>,
    /// The iterator 7.4.9 closes where a `break` leaves the loop, and a
    /// register the close reads the `return` method into.
    close: Option<(crate::engine::bytecode::Reg, crate::engine::bytecode::Reg)>,
}

struct RegisterLoop {
    /// A `switch` is a break target but never a continue target (14.12).
    is_switch: bool,
    breaks: Vec<usize>,
    continues: Vec<usize>,
    result_register: crate::engine::bytecode::Reg,
    bindings: BTreeMap<String, RegisterBinding>,
    completion_depth: usize,
    object_layouts: BTreeMap<u32, RegisterObjectLayout>,
}

/// Which types the head of a loop starts its bindings from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RegisterLoopHead {
    /// The types the bindings hold where the loop begins.
    Declared,
    /// Those merged with the types the body's assignments produce.
    Widened,
    /// Those, and every Object a binding names has left, so the body reads
    /// and writes it at run time and no layout has to match at the back edge.
    Escaped,
}

/// One `try` statement with a Finally Block, while its protected Block or its
/// Catch Block is lowered.
///
/// 14.15.3 runs the Finally Block on the path a `return` takes out of the
/// statement, so the `return` writes its value and a flag and jumps to the
/// Block instead of leaving.
#[derive(Clone)]
struct RegisterFinally {
    /// Holds the value the `return` answers.
    value: crate::engine::bytecode::Reg,
    /// Holds 1 on the path of a `return` and 0 on every other one.
    returning: crate::engine::bytecode::Reg,
    /// The jumps to the first instruction of the Finally Block.
    jumps: Vec<usize>,
}

#[derive(Clone)]
struct RegisterSnapshot {
    instructions: usize,
    block_scoped: usize,
    constants: usize,
    string_constants: usize,
    next_register: u16,
    register_count: u16,
    active_binding_count: u16,
    max_binding_count: u16,
    bindings: BTreeMap<String, RegisterBinding>,
    next_object_id: u32,
    object_layouts: BTreeMap<u32, RegisterObjectLayout>,
    functions: usize,
    own_context_slot_count: Option<u16>,
    outer_context_slot_counts: Vec<u16>,
    function_returns: BTreeMap<u32, RegisterType>,
    function_parameters: BTreeMap<u32, Vec<RegisterType>>,
    function_capture_effects: BTreeMap<u32, BTreeMap<String, RegisterType>>,
    function_layout_effects: BTreeMap<u32, BTreeMap<u32, RegisterObjectLayout>>,
    constructible: BTreeSet<u32>,
    binding_type_hints: BTreeMap<String, RegisterType>,
    return_type: Option<RegisterType>,
}

/// The name 10.2.5 gives the object a constructor carries.
const PROTOTYPE_UNITS: [u16; 9] = [0x70, 0x72, 0x6F, 0x74, 0x6F, 0x74, 0x79, 0x70, 0x65];

impl RegisterLowerer {
    const fn new(
        entry_fuel_cost: u64,
        entry_stack_requirement: usize,
        property_limit: usize,
        function_table_base: u32,
    ) -> Self {
        let mut code = crate::engine::bytecode::BytecodeFunction::new(0, 0);
        code.entry_fuel_cost = entry_fuel_cost;
        code.entry_stack_requirement = entry_stack_requirement;
        Self {
            code,
            function_table_base,
            next_register: 0,
            register_count: 0,
            local_count: 0,
            active_binding_count: 0,
            max_binding_count: 0,
            bindings: BTreeMap::new(),
            loops: Vec::new(),
            block_scoped: Vec::new(),
            completions: Vec::new(),
            constructible: BTreeSet::new(),
            loop_head_types: RegisterLoopHead::Declared,
            thrown: Vec::new(),
            next_object_id: 0,
            object_layouts: BTreeMap::new(),
            property_limit,
            function_returns: BTreeMap::new(),
            function_parameters: BTreeMap::new(),
            function_capture_effects: BTreeMap::new(),
            function_layout_effects: BTreeMap::new(),
            binding_type_hints: BTreeMap::new(),
            allow_return: false,
            finallies: Vec::new(),
            open_iterators: Vec::new(),
            arguments_binding: None,
            mapped_parameters: 0,
            maps_arguments: false,
            assignment_strict: false,
            initializing_global_lexical: false,
            reading_member_base: false,
            return_type: None,
            realm: false,
            script_globals: false,
            refusal: None,
        }
    }

    fn declare(&mut self, name: &str, mutable: bool) -> Option<()> {
        if self.bindings.contains_key(name) {
            return None;
        }
        let register = crate::engine::bytecode::Reg(self.local_count);
        self.local_count = self.local_count.checked_add(1)?;
        self.active_binding_count = self.active_binding_count.checked_add(1)?;
        self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
        self.next_register = self.local_count;
        self.register_count = self.register_count.max(self.local_count);
        self.bindings.insert(
            String::from(name),
            RegisterBinding {
                storage: RegisterBindingStorage::Register(register),
                value_type: None,
                mutable,
                stable_function_identity: false,
            },
        );
        Some(())
    }

    fn initialize(&mut self, name: &str, expression: Option<&Expr>) -> Option<()> {
        let value_type = if let Some(expression) = expression {
            self.lower(expression)?
        } else {
            self.code
                .emit(crate::engine::bytecode::Instruction::LdaUndefined);
            RegisterType::Undefined
        };
        let binding = *self.bindings.get(name)?;
        self.store_binding(binding);
        self.bindings.get_mut(name)?.value_type = Some(value_type);
        Some(())
    }

    fn initialize_vars(
        &mut self,
        bindings: &[(parser::BindingPattern, Option<Expr>)],
    ) -> Option<()> {
        for (pattern, initializer) in bindings {
            let Some(initializer) = initializer else {
                continue;
            };
            self.initialize_pattern(pattern, initializer)?;
        }
        Some(())
    }

    fn initialize_pattern(
        &mut self,
        pattern: &parser::BindingPattern,
        expression: &Expr,
    ) -> Option<()> {
        if let Some(name) = pattern.identifier() {
            let Some(binding) = self.bindings.get(name).copied() else {
                // 16.1.7 created the binding on the Global Environment Record,
                // so 14.3.2.1 writes the initializer there.
                if !self.realm {
                    return None;
                }
                let units: Vec<u16> = name.encode_utf16().collect();
                let constant = self.string_constant(&units)?;
                self.lower_named(expression, &units)?;
                self.code
                    .emit(crate::engine::bytecode::Instruction::StaGlobal {
                        name: constant,
                        strict: expression.strict,
                    });
                return Some(());
            };
            if binding.stable_function_identity {
                return None;
            }
            let units: Vec<u16> = name.encode_utf16().collect();
            let value_type = self.lower_named(expression, &units)?;
            self.store_binding(binding);
            self.bindings.get_mut(name)?.value_type = Some(value_type);
            return Some(());
        }
        let value_type = self.lower(expression)?;
        self.bind_pattern(value_type, pattern)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one function names every shape a binding pattern takes"
    )]
    fn bind_pattern(
        &mut self,
        value_type: RegisterType,
        pattern: &parser::BindingPattern,
    ) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        match pattern {
            parser::BindingPattern::Name(name) => {
                let Some(binding) = self.bindings.get(name).copied() else {
                    // 16.1.7 made the binding on the Global Environment
                    // Record, so 8.6.2 writes it there.
                    if !self.realm {
                        return None;
                    }
                    let units: Vec<u16> = name.encode_utf16().collect();
                    let constant = self.string_constant(&units)?;
                    // A lexical binding of that Record is initialized and not
                    // written, which is what takes it out of its dead zone.
                    if self.initializing_global_lexical {
                        self.code.emit(
                            crate::engine::bytecode::Instruction::InitializeGlobalLexical(constant),
                        );
                        return Some(());
                    }
                    self.code
                        .emit(crate::engine::bytecode::Instruction::StaGlobal {
                            name: constant,
                            strict: false,
                        });
                    return Some(());
                };
                if binding.stable_function_identity {
                    return None;
                }
                self.store_binding(binding);
                self.bindings.get_mut(name)?.value_type = Some(value_type);
            }
            parser::BindingPattern::Object(object) => {
                // A layout the lowering tracks answers each property from
                // what it knows; every other value is read at run time.
                let tracked = value_type.is_object();
                if object.rest.is_some()
                    && tracked
                    && !matches!(value_type, RegisterType::Object(_))
                {
                    return None;
                }
                if !tracked {
                    self.code.emit(Instruction::Require(
                        crate::engine::bytecode::RequireKind::ObjectCoercible,
                    ));
                }
                let source = self.allocate_register()?;
                self.code.emit(Instruction::Star(source));
                let mut excluded = Vec::new();
                for property in &object.properties {
                    if object.rest.is_some() {
                        excluded.push(Self::binding_property_name(property)?);
                    }
                    let mut property_type = if tracked {
                        self.lower_property_from_register(
                            source,
                            value_type,
                            &property.key,
                            property.computed,
                            true,
                        )?
                    } else {
                        self.lower_unknown_property_from_register(
                            source,
                            &property.key,
                            property.computed,
                        )?
                    };
                    if let Some(initializer) = &property.initializer {
                        let bound: Option<Vec<u16>> = property
                            .pattern
                            .identifier()
                            .map(|name| name.encode_utf16().collect());
                        property_type = self.lower_binding_default_named(
                            property_type,
                            initializer,
                            bound.as_deref(),
                        )?;
                    }
                    self.bind_pattern(property_type, &property.pattern)?;
                }
                if let Some(rest) = &object.rest {
                    let (rest_type, rest_object) = if tracked {
                        self.lower_object_rest_from_register(source, value_type, &excluded)?
                    } else {
                        // 7.3.25 collects every own enumerable key of a value
                        // the lowering could not name, which only the run time
                        // knows.
                        self.lower_copied_data_properties(source, &excluded)?
                    };
                    self.code.emit(Instruction::Ldar(rest_object));
                    self.bind_pattern(rest_type, &parser::BindingPattern::Name(rest.clone()))?;
                    self.release_register(rest_object)?;
                }
                self.release_register(source)?;
            }
            parser::BindingPattern::Array(array) => {
                if !matches!(value_type, RegisterType::Array(_)) {
                    // 8.6.2 takes the elements from the iterator of the value.
                    return self
                        .lower_array_pattern_by_iterator(&RegisterArrayPattern::Binding(array));
                }
                let source = self.allocate_register()?;
                self.code.emit(Instruction::Star(source));
                for (index, element) in array.elements.iter().enumerate() {
                    let parser::ArrayBindingElement::Element {
                        pattern,
                        initializer,
                    } = element
                    else {
                        continue;
                    };
                    let index = u32::try_from(index).ok()?;
                    let mut element_type =
                        self.lower_array_index_from_register(source, value_type, index)?;
                    if let Some(initializer) = initializer {
                        let bound: Option<Vec<u16>> = pattern
                            .identifier()
                            .map(|name| name.encode_utf16().collect());
                        element_type = self.lower_binding_default_named(
                            element_type,
                            initializer,
                            bound.as_deref(),
                        )?;
                    }
                    self.bind_pattern(element_type, pattern)?;
                }
                if let Some(rest) = &array.rest {
                    let start = u32::try_from(array.elements.len()).ok()?;
                    let (rest_type, rest_array) =
                        self.lower_array_rest_from_register(source, value_type, start)?;
                    self.code.emit(Instruction::Ldar(rest_array));
                    self.bind_pattern(rest_type, rest)?;
                    self.release_register(rest_array)?;
                }
                self.release_register(source)?;
            }
        }
        Some(())
    }

    /// `IteratorBindingInitialization` of 8.6.2 and
    /// `DestructuringAssignmentEvaluation` of 13.15.5.5 for an array pattern
    /// over a value whose layout the lowering does not know.
    ///
    /// 7.4.2 opens the iterator, each element takes one step of 7.4.6, and an
    /// element the iterator no longer answers is undefined. 7.4.9 closes an
    /// iterator the pattern did not exhaust. The two forms differ only in what
    /// one element does with the value, so they share the walk.
    #[expect(
        clippy::too_many_lines,
        reason = "one function emits the whole of the walk for one pattern"
    )]
    fn lower_array_pattern_by_iterator(&mut self, array: &RegisterArrayPattern<'_>) -> Option<()> {
        use crate::engine::bytecode::{FeedbackKind, Instruction};
        let iterable = self.allocate_register()?;
        self.code.emit(Instruction::Star(iterable));
        let iterator_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetWellKnown {
            obj: iterable,
            symbol: u16::try_from(WELL_KNOWN_ITERATOR).ok()?,
            slot: iterator_slot,
        });
        // 7.4.2 refuses a value whose `@@iterator` is undefined before it
        // calls anything.
        self.code.emit(Instruction::Require(
            crate::engine::bytecode::RequireKind::Iterable,
        ));
        let method = self.allocate_register()?;
        self.code.emit(Instruction::Star(method));
        let open = self.feedback_slot(FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver: iterable,
            func: method,
            arg_start: method,
            arg_count: 0,
            slot: open,
        });
        let iterator = self.allocate_register()?;
        self.code.emit(Instruction::Star(iterator));
        // `[[Done]]` of the Iterator Record, which 8.6.2 reads before each
        // element and 7.4.9 reads at the end.
        let done = self.allocate_register()?;
        self.code.emit(Instruction::LdaFalse);
        self.code.emit(Instruction::Star(done));
        let step = self.allocate_register()?;
        let next = self.allocate_register()?;
        for index in 0..array.len() {
            // 13.15.5.5 evaluates the target of an element before the
            // iterator steps, which 8.6.2 has no reference to evaluate.
            let prepared = match array {
                RegisterArrayPattern::Assignment(assignment) => {
                    match assignment.elements.get(index) {
                        Some(parser::AssignmentArrayElement::Element { target, .. }) => {
                            Some(self.prepare_assignment_pattern_target(target)?)
                        }
                        _ => None,
                    }
                }
                RegisterArrayPattern::Binding(_) => None,
            };
            self.code.emit(Instruction::Ldar(done));
            let finished = self.code.emit(Instruction::JumpIfTrue(0));
            let next_name = self.string_constant(&"next".encode_utf16().collect::<Vec<_>>())?;
            let next_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: iterator,
                name: next_name,
                slot: next_slot,
            });
            self.code.emit(Instruction::Star(next));
            let step_slot = self.feedback_slot(FeedbackKind::Call)?;
            self.code.emit(Instruction::CallMethod {
                receiver: iterator,
                func: next,
                arg_start: next,
                arg_count: 0,
                slot: step_slot,
            });
            self.code.emit(Instruction::Star(step));
            let done_name = self.string_constant(&"done".encode_utf16().collect::<Vec<_>>())?;
            let done_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: step,
                name: done_name,
                slot: done_slot,
            });
            let exhausted = self.code.emit(Instruction::JumpIfTrue(0));
            let value_name = self.string_constant(&"value".encode_utf16().collect::<Vec<_>>())?;
            let value_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: step,
                name: value_name,
                slot: value_slot,
            });
            let bound = self.code.emit(Instruction::Jump(0));
            // The iterator answered done, so the record says so and the
            // element is undefined, which is also where a pattern that was
            // already finished lands.
            let mark = self.code.instructions.len();
            self.patch_jump(exhausted, mark)?;
            self.code.emit(Instruction::LdaTrue);
            self.code.emit(Instruction::Star(done));
            let absent = self.code.instructions.len();
            self.patch_jump(finished, absent)?;
            self.code.emit(Instruction::LdaUndefined);
            let after = self.code.instructions.len();
            self.patch_jump(bound, after)?;
            match array {
                RegisterArrayPattern::Binding(binding) => {
                    if let Some(parser::ArrayBindingElement::Element {
                        pattern,
                        initializer,
                    }) = binding.elements.get(index)
                    {
                        let mut element_type = RegisterType::Unknown;
                        if let Some(initializer) = initializer {
                            let bound: Option<Vec<u16>> = pattern
                                .identifier()
                                .map(|name| name.encode_utf16().collect());
                            element_type = self.lower_binding_default_named(
                                element_type,
                                initializer,
                                bound.as_deref(),
                            )?;
                        }
                        self.bind_pattern(element_type, pattern)?;
                    }
                }
                RegisterArrayPattern::Assignment(assignment) => {
                    if let Some(parser::AssignmentArrayElement::Element {
                        target,
                        initializer,
                    }) = assignment.elements.get(index)
                    {
                        let mut element_type = RegisterType::Unknown;
                        if let Some(initializer) = initializer {
                            element_type =
                                self.lower_assignment_default(element_type, initializer, target)?;
                        }
                        self.finish_assignment_pattern_target(element_type, target, prepared?)?;
                    }
                }
            }
        }
        // 8.6.2 collects a rest element by walking the iterator to its end
        // into an Array of its own, whose indices 13.2.5.5 defines.
        if array.has_rest() {
            // 13.15.5.5 evaluates the target of the rest element before it
            // collects anything into the Array.
            let prepared = match array {
                RegisterArrayPattern::Assignment(assignment) => {
                    Some(self.prepare_assignment_pattern_target(assignment.rest.as_deref()?)?)
                }
                RegisterArrayPattern::Binding(_) => None,
            };
            let collected = self.allocate_register()?;
            self.code.emit(Instruction::CreateArray(0));
            self.code.emit(Instruction::Star(collected));
            let count = self.allocate_register()?;
            self.code.emit(Instruction::LdaSmi(0));
            self.code.emit(Instruction::Star(count));
            let one = self.allocate_register()?;
            self.code.emit(Instruction::LdaSmi(1));
            self.code.emit(Instruction::Star(one));
            let head = self.code.instructions.len();
            self.code.emit(Instruction::Ldar(done));
            let leave = self.code.emit(Instruction::JumpIfTrue(0));
            let next_name = self.string_constant(&"next".encode_utf16().collect::<Vec<_>>())?;
            let next_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: iterator,
                name: next_name,
                slot: next_slot,
            });
            self.code.emit(Instruction::Star(next));
            let step_slot = self.feedback_slot(FeedbackKind::Call)?;
            self.code.emit(Instruction::CallMethod {
                receiver: iterator,
                func: next,
                arg_start: next,
                arg_count: 0,
                slot: step_slot,
            });
            self.code.emit(Instruction::Star(step));
            let done_name = self.string_constant(&"done".encode_utf16().collect::<Vec<_>>())?;
            let done_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: step,
                name: done_name,
                slot: done_slot,
            });
            let exhausted = self.code.emit(Instruction::JumpIfTrue(0));
            let value_name = self.string_constant(&"value".encode_utf16().collect::<Vec<_>>())?;
            let value_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: step,
                name: value_name,
                slot: value_slot,
            });
            let write_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::SetByValue {
                obj: collected,
                key: count,
                slot: write_slot,
                define: true,
                strict: true,
            });
            self.code.emit(Instruction::Ldar(count));
            self.code.emit(Instruction::Add(one));
            self.code.emit(Instruction::Star(count));
            let back_edge = self.code.emit(Instruction::Jump(0));
            self.patch_jump(back_edge, head)?;
            let mark = self.code.instructions.len();
            self.patch_jump(exhausted, mark)?;
            self.code.emit(Instruction::LdaTrue);
            self.code.emit(Instruction::Star(done));
            let end = self.code.instructions.len();
            self.patch_jump(leave, end)?;
            self.code.emit(Instruction::Ldar(collected));
            match array {
                RegisterArrayPattern::Binding(binding) => {
                    self.bind_pattern(RegisterType::Unknown, binding.rest.as_deref()?)?;
                }
                RegisterArrayPattern::Assignment(assignment) => {
                    self.finish_assignment_pattern_target(
                        RegisterType::Unknown,
                        assignment.rest.as_deref()?,
                        prepared?,
                    )?;
                }
            }
            self.release_register(one)?;
            self.release_register(count)?;
            self.release_register(collected)?;
        }
        // 7.4.9 closes an iterator that is not done, and an iterator with no
        // `return` is closed by doing nothing.
        self.code.emit(Instruction::Ldar(done));
        let closed = self.code.emit(Instruction::JumpIfTrue(0));
        let return_name = self.string_constant(&"return".encode_utf16().collect::<Vec<_>>())?;
        let return_slot = self.feedback_slot(FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: iterator,
            name: return_name,
            slot: return_slot,
        });
        self.code.emit(Instruction::Star(next));
        let present = self.code.emit(Instruction::JumpIfNotNullish(0));
        let skip = self.code.emit(Instruction::Jump(0));
        let call = self.code.instructions.len();
        self.patch_jump(present, call)?;
        let close_slot = self.feedback_slot(FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver: iterator,
            func: next,
            arg_start: next,
            arg_count: 0,
            slot: close_slot,
        });
        let end = self.code.instructions.len();
        self.patch_jump(closed, end)?;
        self.patch_jump(skip, end)?;
        self.release_register(next)?;
        self.release_register(step)?;
        self.release_register(done)?;
        self.release_register(iterator)?;
        self.release_register(method)?;
        self.release_register(iterable)?;
        Some(())
    }

    /// The Initializer of one element of an assignment pattern, named after
    /// the target where 13.15.5.2 and 13.15.5.4 name it: a target that is a
    /// plain identifier reference and nothing else.
    fn lower_assignment_default(
        &mut self,
        value_type: RegisterType,
        initializer: &Expr,
        target: &parser::AssignmentPattern,
    ) -> Option<RegisterType> {
        let named = Self::assignment_target_name(target);
        self.lower_binding_default_named(value_type, initializer, named.as_deref())
    }

    /// The name of an assignment target that is a plain identifier reference.
    fn assignment_target_name(target: &parser::AssignmentPattern) -> Option<Vec<u16>> {
        fn named(expression: &Expr) -> Option<Vec<u16>> {
            match &expression.kind {
                ExprKind::Group(inner) => named(inner),
                ExprKind::Name(name) => Some(name.encode_utf16().collect()),
                _ => None,
            }
        }
        match target {
            parser::AssignmentPattern::Target(expression) => named(expression),
            parser::AssignmentPattern::Array(_) | parser::AssignmentPattern::Object(_) => None,
        }
    }

    /// The Initializer itself, named after its binding where 8.5.2 names it.
    fn lower_default_value(
        &mut self,
        initializer: &Expr,
        name: Option<&[u16]>,
    ) -> Option<RegisterType> {
        match name {
            Some(name) => self.lower_named(initializer, name),
            None => self.lower(initializer),
        }
    }

    /// The Initializer of one element of a pattern, which 8.5.2 names after
    /// the binding it is for when that binding is a single name.
    fn lower_binding_default_named(
        &mut self,
        value_type: RegisterType,
        initializer: &Expr,
        name: Option<&[u16]>,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        // A value that is undefined always takes the Initializer, so what the
        // Initializer made is there on every path and keeps its layout.
        if value_type == RegisterType::Undefined {
            let bindings_without_default = self.bindings.clone();
            let default_type = self.lower_default_value(initializer, name)?;
            return register_context_bindings_unchanged(&bindings_without_default, &self.bindings)
                .then_some(default_type);
        }
        // A value the lowering cannot name may be undefined, so the check
        // 8.6.2 makes has to be made at run time.
        if !matches!(
            value_type,
            RegisterType::NumberOrUndefined | RegisterType::Primitive | RegisterType::Unknown
        ) {
            return Some(value_type);
        }
        let present = self.code.emit(Instruction::JumpIfNotUndefined(0));
        let bindings_without_default = self.bindings.clone();
        let layouts_without_default = self.object_layouts.clone();
        let default_type = self.lower_default_value(initializer, name)?;
        // The Initializer runs on one path only, so a layout it changed would
        // be wrong on the other.
        if !layouts_without_default
            .iter()
            .all(|(id, layout)| self.object_layouts.get(id) == Some(layout))
            || !register_context_bindings_unchanged(&bindings_without_default, &self.bindings)
        {
            return None;
        }
        // A layout the Initializer made is there on one path only, so nothing
        // may read it: what it answers is the type the lowering cannot name.
        let makes_object = !default_type.is_primitive();
        if makes_object {
            self.object_layouts = layouts_without_default;
        }
        self.bindings = merge_register_bindings(&bindings_without_default, &self.bindings)?;
        let end = self.code.instructions.len();
        self.patch_jump(present, end)?;
        Some(match value_type {
            _ if makes_object => RegisterType::Unknown,
            RegisterType::Primitive => RegisterType::Primitive,
            RegisterType::Unknown => RegisterType::Unknown,
            _ => RegisterType::Number.merge(default_type),
        })
    }

    /// `GlobalDeclarationInstantiation` of 16.1.7 for the `var` names of a
    /// Script of a persistent Realm.
    ///
    /// Every name is verified before any is created, which is the order 16.1.7
    /// gives: a Script that conflicts with an existing lexical declaration
    /// leaves the Realm as it found it.
    fn instantiate_global_declarations(&mut self, body: &[Stmt]) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        let mut variables = Vec::new();
        // 16.1.7 step 3 checks every lexically declared name of the Script
        // before step 16 creates any of them, so a Script that clashes leaves
        // the Realm as it found it.
        let mut lexical: Vec<(String, bool)> = Vec::new();
        for statement in body {
            if let Stmt::Declare(bindings) = statement {
                for (pattern, mutable, _) in bindings {
                    let mut bound = Vec::new();
                    pattern.names(&mut bound);
                    for name in bound {
                        if lexical.iter().any(|(taken, _)| *taken == name) {
                            return None;
                        }
                        lexical.push((name, *mutable));
                    }
                }
                continue;
            }
            var_names(statement, &mut variables);
        }
        variables.dedup();
        // 16.1.7 takes the last declaration of each function name.
        let mut functions: Vec<(&String, &Function)> = Vec::new();
        for statement in body {
            if let Stmt::Function(name, function) = statement {
                if self.bindings.contains_key(name) {
                    return None;
                }
                functions.retain(|(declared, _)| *declared != name);
                functions.push((name, function));
            }
        }
        let function_names: Vec<u16> = functions
            .iter()
            .map(|(name, _)| self.name_constant(name))
            .collect::<Option<_>>()?;
        let variable_names: Vec<u16> = variables
            .iter()
            .map(|name| self.name_constant(name))
            .collect::<Option<_>>()?;
        // Every name is verified before any binding is created, so a Script
        // that conflicts leaves the Realm as it found it.
        let lexical_names: Vec<u16> = lexical
            .iter()
            .map(|(name, _)| self.name_constant(name))
            .collect::<Option<_>>()?;
        for name in &lexical_names {
            self.code.emit(Instruction::VerifyGlobalLexical(*name));
        }
        for name in &variable_names {
            self.code.emit(Instruction::VerifyGlobalVar(*name));
        }
        for name in &function_names {
            self.code.emit(Instruction::VerifyGlobalFunction(*name));
        }
        for ((declared, function), constant) in functions.iter().zip(&function_names) {
            // 10.2.10 names a declaration after the binding it makes, which
            // 16.1.7 puts on the Global Environment Record.
            let units: Vec<u16> = declared.encode_utf16().collect();
            self.lower_callable(function, false, Some(&units))?;
            self.code
                .emit(Instruction::DeclareGlobalFunction(*constant));
        }
        for name in &variable_names {
            self.code.emit(Instruction::DeclareGlobalVar(*name));
        }
        for (constant, (_, mutable)) in lexical_names.iter().zip(&lexical) {
            self.code.emit(Instruction::DeclareGlobalLexical {
                name: *constant,
                mutable: *mutable,
            });
        }
        Some(())
    }

    /// Gives a lexical binding of a Realm Script the value its declaration
    /// names (9.1.1.4.4).
    ///
    /// 16.1.7 created the binding before the Script ran, so this only writes
    /// it. A declaration without an initializer writes undefined, which is
    /// what `let x;` binds.
    fn initialize_global_lexical(
        &mut self,
        pattern: &parser::BindingPattern,
        initializer: Option<&Expr>,
    ) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        let Some(name) = pattern.identifier() else {
            // 14.3.1.2: a lexical declaration that is a pattern always has an
            // Initializer, and 8.6.2 binds every name it names.
            let value_type = self.lower(initializer?)?;
            let held = core::mem::replace(&mut self.initializing_global_lexical, true);
            let bound = self.bind_pattern(RegisterType::Unknown, pattern);
            self.initializing_global_lexical = held;
            bound?;
            self.escape(&[value_type]);
            return Some(());
        };
        let value_type = if let Some(initializer) = initializer {
            // 14.3.1.2 step 4: an anonymous function takes the name the
            // declaration binds it to, with 8.5.2.
            let units: Vec<u16> = name.encode_utf16().collect();
            self.lower_named(initializer, &units)?
        } else {
            self.code.emit(Instruction::LdaUndefined);
            RegisterType::Undefined
        };
        let constant = self.name_constant(name)?;
        self.code
            .emit(Instruction::InitializeGlobalLexical(constant));
        // Every later Script of this Realm reads the name from the
        // [[DeclarativeRecord]], so the value is no longer only this one's.
        self.escape(&[value_type]);
        Some(())
    }

    /// The string constant of one declared name.
    fn name_constant(&mut self, name: &str) -> Option<u16> {
        let units: Vec<u16> = name.encode_utf16().collect();
        self.string_constant(&units)
    }

    fn prepare_var_bindings(&mut self, body: &[Stmt]) -> Option<()> {
        if self.script_globals {
            // 16.1.7 creates a top-level `var` on the Global Environment
            // Record, which outlives this Script, so it is no binding of it.
            return Some(());
        }
        let names = register_body_var_names(body)?;
        let initialized_names = register_body_initialized_var_names(body)?;
        for (index, statement) in body.iter().enumerate() {
            let Stmt::Function(declared_name, function) = statement else {
                continue;
            };
            let scope = register_function_scope(function)?;
            let later_function_names: BTreeSet<_> = body
                .iter()
                .skip(index.saturating_add(1))
                .filter_map(|statement| match statement {
                    Stmt::Function(name, _) => Some(name),
                    _ => None,
                })
                .collect();
            if scope
                .free_names
                .iter()
                .any(|name| name != declared_name && later_function_names.contains(name))
            {
                return None;
            }
        }
        for name in &names {
            if !self.bindings.contains_key(name) {
                self.declare(name, true)?;
                self.bindings.get_mut(name)?.value_type = Some(RegisterType::Undefined);
            } else if self
                .bindings
                .get(name)
                .is_some_and(|binding| !binding.mutable)
            {
                return None;
            } else if self
                .bindings
                .get(name)
                .is_some_and(|binding| binding.mutable && binding.value_type.is_none())
            {
                self.bindings.get_mut(name)?.value_type = Some(RegisterType::Undefined);
            }
        }
        let local_names = self.bindings.keys().cloned().collect();
        let captured_names = register_body_scope(body, &local_names)?.captured_names;
        let captured_vars: BTreeSet<_> = captured_names.intersection(&names).cloned().collect();
        // A captured reader is compiled against the type its binding carries,
        // and an assignment anywhere — in this body or in a function it holds
        // — can make that type wrong. The name the lowering cannot give is
        // what the top of the lattice is: a captured var that is written
        // carries it, and every read of it takes the generic path.
        let mut written = BTreeSet::new();
        for name in &captured_vars {
            let one: BTreeSet<_> = core::iter::once(name.clone()).collect();
            for statement in body {
                if register_statement_writes_names(statement, &one)? {
                    written.insert(name.clone());
                    break;
                }
            }
        }
        for name in &written {
            self.bindings.get_mut(name)?.value_type = Some(RegisterType::Unknown);
        }
        // A capture moves the value out of its register where it stands, so a
        // path that does not reach the capture would leave the slot empty, and
        // a closure made before it declares an outer context the body does not
        // have yet. Both go away when every var a nested function reads stands
        // in a slot before the body runs.
        for name in &captured_names {
            // 10.4.4 makes the arguments object where the body starts, which
            // is after this, so the capture of it belongs to that step.
            if name != ARGUMENTS && self.bindings.contains_key(name) {
                self.capture_binding(name)?;
            }
        }
        let mut inferred = self.bindings.clone();
        infer_register_body_var_types_to_fixed_point(body, &mut inferred)?;
        for name in initialized_names {
            let hint = if written.contains(&name) {
                RegisterType::Unknown
            } else {
                inferred.get(&name)?.value_type?
            };
            self.binding_type_hints.insert(name, hint);
        }
        Some(())
    }

    fn load_binding(&mut self, binding: RegisterBinding) {
        use crate::engine::bytecode::Instruction;
        // 9.4.5 refuses the `this` binding of a derived constructor until
        // 13.3.7.1 has made it, whichever expression reads it.
        if self.code.derived
            && let RegisterBindingStorage::Register(register) = binding.storage
            && self.code.this_register == Some(register)
        {
            self.code.emit(Instruction::ThisBinding { register });
            return;
        }
        self.code.emit(match binding.storage {
            RegisterBindingStorage::Register(register) => Instruction::Ldar(register),
            RegisterBindingStorage::Context { depth, slot } => {
                Instruction::LoadContext { depth, slot }
            }
        });
    }

    fn store_binding(&mut self, binding: RegisterBinding) {
        use crate::engine::bytecode::Instruction;
        self.code.emit(match binding.storage {
            RegisterBindingStorage::Register(register) => Instruction::Star(register),
            RegisterBindingStorage::Context { depth, slot } => {
                Instruction::StoreContext { depth, slot }
            }
        });
    }

    fn capture_binding(&mut self, name: &str) -> Option<RegisterBinding> {
        // 14.7.4.8 gives a Block binding inside a loop a copy per iteration,
        // and one slot of the context holds one value, so the closures of two
        // iterations would read the same binding.
        if !self.loops.is_empty() && self.block_scoped.iter().any(|scope| scope.contains(name)) {
            self.refuse("a Block binding of a loop, read by a nested function");
            return None;
        }
        let binding = *self.bindings.get(name)?;
        if matches!(binding.storage, RegisterBindingStorage::Context { .. }) {
            return Some(binding);
        }
        let RegisterBindingStorage::Register(register) = binding.storage else {
            return None;
        };
        let slot_count = self.code.own_context_slot_count.unwrap_or(0);
        let next = slot_count.checked_add(1)?;
        self.code.own_context_slot_count = Some(next);
        self.code
            .emit(crate::engine::bytecode::Instruction::Ldar(register));
        self.code
            .emit(crate::engine::bytecode::Instruction::StoreContext {
                depth: 0,
                slot: slot_count,
            });
        let captured = RegisterBinding {
            storage: RegisterBindingStorage::Context {
                depth: 0,
                slot: slot_count,
            },
            ..binding
        };
        self.bindings.insert(String::from(name), captured);
        Some(captured)
    }

    /// Puts the parameters 10.4.4.7 maps in consecutive slots of the own
    /// context, where the arguments object reaches them for as long as it
    /// lives.
    fn capture_parameters(&mut self, function: &Function) -> Option<()> {
        let base = self.code.own_context_slot_count.unwrap_or(0);
        let mut mask = 0u64;
        for (index, parameter) in function.parameters.iter().enumerate() {
            let parser::BindingPattern::Name(name) = &parameter.pattern else {
                return None;
            };
            let name = name.clone();
            let binding = self.capture_binding(&name)?;
            let RegisterBindingStorage::Context { depth: 0, slot } = binding.storage else {
                return None;
            };
            if slot != base.checked_add(u16::try_from(index).ok()?)? {
                return None;
            }
            mask |= 1u64.checked_shl(u32::try_from(index).ok()?)?;
        }
        self.code.arguments_map = Some(crate::engine::bytecode::ParameterMap { slot: base, mask });
        Some(())
    }

    fn snapshot(&self) -> RegisterSnapshot {
        RegisterSnapshot {
            instructions: self.code.instructions.len(),
            block_scoped: self.block_scoped.len(),
            constants: self.code.constants.len(),
            string_constants: self.code.string_constants.len(),
            next_register: self.next_register,
            register_count: self.register_count,
            active_binding_count: self.active_binding_count,
            max_binding_count: self.max_binding_count,
            bindings: self.bindings.clone(),
            next_object_id: self.next_object_id,
            object_layouts: self.object_layouts.clone(),
            functions: self.code.functions.len(),
            own_context_slot_count: self.code.own_context_slot_count,
            outer_context_slot_counts: self.code.outer_context_slot_counts.clone(),
            function_returns: self.function_returns.clone(),
            function_parameters: self.function_parameters.clone(),
            function_capture_effects: self.function_capture_effects.clone(),
            function_layout_effects: self.function_layout_effects.clone(),
            constructible: self.constructible.clone(),
            binding_type_hints: self.binding_type_hints.clone(),
            return_type: self.return_type,
        }
    }

    fn restore(&mut self, snapshot: RegisterSnapshot) {
        self.block_scoped.truncate(snapshot.block_scoped);
        self.code.instructions.truncate(snapshot.instructions);
        self.code.constants.truncate(snapshot.constants);
        self.code
            .string_constants
            .truncate(snapshot.string_constants);
        self.next_register = snapshot.next_register;
        self.register_count = snapshot.register_count;
        self.active_binding_count = snapshot.active_binding_count;
        self.max_binding_count = snapshot.max_binding_count;
        self.bindings = snapshot.bindings;
        self.next_object_id = snapshot.next_object_id;
        self.object_layouts = snapshot.object_layouts;
        self.code.functions.truncate(snapshot.functions);
        self.code.own_context_slot_count = snapshot.own_context_slot_count;
        self.code.outer_context_slot_counts = snapshot.outer_context_slot_counts;
        self.function_returns = snapshot.function_returns;
        self.function_parameters = snapshot.function_parameters;
        self.function_capture_effects = snapshot.function_capture_effects;
        self.function_layout_effects = snapshot.function_layout_effects;
        self.constructible = snapshot.constructible;
        self.binding_type_hints = snapshot.binding_type_hints;
        self.return_type = snapshot.return_type;
    }

    fn lower(&mut self, expression: &Expr) -> Option<RegisterType> {
        let lowered = self.lower_kind(expression);
        if lowered.is_none() {
            self.refuse(expression_refusal(&expression.kind));
        }
        lowered
    }

    /// Records the construct that stopped this lowering, if none is recorded.
    const fn refuse(&mut self, name: &'static str) {
        if self.refusal.is_none() {
            self.refusal = Some(name);
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "expression lowering keeps type propagation beside emitted operations"
    )]
    fn lower_kind(&mut self, expression: &Expr) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        // The flag covers this expression alone, so a base that is itself a
        // call or an assignment does not pass it on to what it contains.
        let member_base = core::mem::take(&mut self.reading_member_base);
        let result = match &expression.kind {
            // 12.9.3 makes the value of a BigInt literal where it stands, in
            // the arena of the Agent, from the digits the lexer kept.
            ExprKind::BigInt(digits, radix) => {
                let units: Vec<u16> = digits.encode_utf16().collect();
                let value = crate::engine::bigint::BigIntValue::from_digits(&units, *radix, false)?;
                let index = u16::try_from(self.code.bigint_constants.len()).ok()?;
                self.code.bigint_constants.push(value);
                self.code.emit(Instruction::LdaBigInt(index));
                RegisterType::Unknown
            }
            ExprKind::Literal(value) => match value {
                Value::Number(number) => {
                    if let Some(smi) = smi_literal(*number) {
                        self.code.emit(Instruction::LdaSmi(smi));
                    } else {
                        let index =
                            self.constant(crate::engine::value::Value::from_f64(*number))?;
                        self.code.emit(Instruction::LdaConstant(index));
                    }
                    RegisterType::Number
                }
                Value::Boolean(true) => {
                    self.code.emit(Instruction::LdaTrue);
                    RegisterType::Boolean
                }
                Value::Boolean(false) => {
                    self.code.emit(Instruction::LdaFalse);
                    RegisterType::Boolean
                }
                Value::Null => {
                    self.code.emit(Instruction::LdaNull);
                    RegisterType::Null
                }
                Value::Undefined => {
                    self.code.emit(Instruction::LdaUndefined);
                    RegisterType::Undefined
                }
                Value::String(units) => {
                    let index = self.string_constant(units)?;
                    self.code.emit(Instruction::LdaString(index));
                    RegisterType::String
                }
                Value::Symbol(_) | Value::Function(_) | Value::Object(_) => {
                    return None;
                }
            },
            // 9.4.5 resolves `this` on the Function Environment Record of the
            // call, which the frame carries in a register of its own. A Script
            // has no such record, so its `this` is not this binding.
            ExprKind::This => {
                // 9.4.2 resolves `this` on the Environment Record that has
                // one. At the top level of a Script that is the Global
                // Environment Record, whose [[GlobalThisValue]] is the global
                // object; 10.2.1.2 bound every other one to a register.
                match self.bindings.get(THIS_BINDING).copied() {
                    Some(binding) => {
                        let value_type = binding.value_type?;
                        self.load_binding(binding);
                        value_type
                    }
                    None if self.realm => {
                        self.code.emit(Instruction::LdaGlobalThis);
                        RegisterType::Unknown
                    }
                    None => return None,
                }
            }
            // 15.7.14 step 10: the default constructor of a derived class is
            // a super call of the arguments its own call was given.
            ExprKind::DefaultSuper => {
                let this_register = self.code.this_register?;
                let register = self.allocate_register()?;
                let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
                self.code.emit(Instruction::SuperCall {
                    arg_start: register,
                    arg_count: 0,
                    forwarded: true,
                    slot,
                });
                self.code.emit(Instruction::Star(this_register));
                self.release_register(register)?;
                RegisterType::Unknown
            }
            // 9.4.3 answers the `[[NewTarget]]` of the call, which the frame
            // holds in a register of its own.
            ExprKind::NewTarget => {
                let register = self.code.new_target_register?;
                self.code.emit(Instruction::Ldar(register));
                RegisterType::Unknown
            }
            ExprKind::Name(name) => {
                if let Some(binding) = self.bindings.get(name).copied() {
                    let value_type = binding.value_type?;
                    self.load_binding(binding);
                    value_type
                } else {
                    match name.as_str() {
                        "undefined" => {
                            self.code.emit(Instruction::LdaUndefined);
                            RegisterType::Undefined
                        }
                        "NaN" => {
                            let index = self.constant(crate::engine::value::VALUE_NAN)?;
                            self.code.emit(Instruction::LdaConstant(index));
                            RegisterType::Number
                        }
                        "Infinity" => {
                            let index = self
                                .constant(crate::engine::value::Value::from_f64(f64::INFINITY))?;
                            self.code.emit(Instruction::LdaConstant(index));
                            RegisterType::Number
                        }
                        // 10.4.4 binds `arguments` in every ordinary function,
                        // so inside one it is never the Realm's global of that
                        // name. The object is only read as the base of a
                        // property access: 10.4.4.7 maps its indices onto the
                        // parameters, and a body that could observe the
                        // mapping is not lowered.
                        // 10.4.4.7 maps the indices of a sloppy function's
                        // arguments object onto its parameters, which this
                        // engine does not do, so there it is only read as the
                        // base of a property access. A strict function has no
                        // mapping to observe.
                        "arguments" if self.allow_return => {
                            // A function with no formal parameter has an empty
                            // mapping, so its object carries nothing that could
                            // be observed anywhere.
                            let unmapped = self.code.strict
                                || self.mapped_parameters == 0
                                || self.code.arguments_map.is_some();
                            let binding = self
                                .bindings
                                .get(ARGUMENTS)
                                .copied()
                                .or(self.arguments_binding)
                                .filter(|_| member_base || unmapped)?;
                            self.load_binding(binding);
                            RegisterType::Unknown
                        }
                        // 9.1.1.4.6 resolves every other name on the Realm's
                        // Global Environment Record. A name of clause 19 this
                        // Realm has not built is reported there as a gap, so
                        // no read of one can answer wrongly.
                        _ => {
                            let units: Vec<u16> = name.encode_utf16().collect();
                            let index = self.string_constant(&units)?;
                            self.code.emit(Instruction::LdaGlobal(index));
                            RegisterType::Unknown
                        }
                    }
                }
            }
            ExprKind::Group(inner) => self.lower(inner)?,
            ExprKind::Sequence(left, right) => {
                self.lower(left)?;
                self.lower(right)?
            }
            ExprKind::Unary(Unary::Delete, inner) => self.lower_delete(inner, expression.strict)?,
            ExprKind::Unary(operator, inner) => {
                // 13.5.3 reads the operand of `typeof` without GetValue, so an
                // unresolvable name answers undefined instead of throwing.
                let inner_type = match (operator, self.global_name(inner)) {
                    (Unary::Typeof, Some(index)) => {
                        self.code.emit(Instruction::LdaGlobalForTypeOf(index));
                        RegisterType::Unknown
                    }
                    _ => self.lower(inner)?,
                };
                match operator {
                    Unary::Plus | Unary::Minus | Unary::BitNot => {
                        if inner_type != RegisterType::Number {
                            // 7.1.4 of an Object runs a method of the Script,
                            // which the instruction names where it runs.
                            let held = self.allocate_register()?;
                            self.code.emit(Instruction::Star(held));
                            // 13.5.4 asks ToNumber, which has no Number for a
                            // BigInt, where 13.5.5 and 13.5.6 ask ToNumeric.
                            self.code.emit(if matches!(operator, Unary::Plus) {
                                Instruction::NumberOnly(held)
                            } else {
                                Instruction::ToNumeric(held)
                            });
                            self.release_register(held)?;
                        }
                        if matches!(operator, Unary::Minus) {
                            self.code.emit(Instruction::Negate);
                        } else if matches!(operator, Unary::BitNot) {
                            self.code.emit(Instruction::BitNot);
                        }
                    }
                    Unary::Not => {
                        self.code.emit(Instruction::LogicalNot);
                    }
                    Unary::Void => {
                        self.code.emit(Instruction::ToUndefined);
                    }
                    Unary::Typeof => {
                        self.code.emit(Instruction::TypeOf);
                    }
                    Unary::Delete => return None,
                }
                match operator {
                    Unary::Plus => RegisterType::Number,
                    // 6.1.6.2.1 and 6.1.6.2.2 answer a BigInt for one.
                    Unary::Minus | Unary::BitNot if inner_type == RegisterType::Number => {
                        RegisterType::Number
                    }
                    Unary::Minus | Unary::BitNot => RegisterType::Unknown,
                    Unary::Not => RegisterType::Boolean,
                    Unary::Void => RegisterType::Undefined,
                    Unary::Typeof => RegisterType::String,
                    Unary::Delete => return None,
                }
            }
            ExprKind::Binary(
                operator @ (Binary::And | Binary::Or | Binary::Nullish),
                left,
                right,
            ) => self.lower_short_circuit(*operator, left, right)?,
            ExprKind::Binary(operator, left, right) => self.lower_binary(*operator, left, right)?,
            ExprKind::Assign(name, operator, right) => {
                self.lower_assignment(name, *operator, right, expression.strict)?
            }
            // 27.7.5.3: the operand is evaluated and the body waits for it.
            ExprKind::Await(inner) => {
                self.lower(inner)?;
                self.code.emit(crate::engine::bytecode::Instruction::Await);
                RegisterType::Unknown
            }
            ExprKind::Destructure(pattern, right) => {
                self.lower_destructuring_assignment(pattern, right)?
            }
            ExprKind::Update(name, add, prefix) => self.lower_update(name, *add, *prefix)?,
            ExprKind::Conditional(condition, yes, no) => {
                self.lower_conditional(condition, yes, no)?
            }
            ExprKind::Object(properties) => self.lower_object(properties)?,
            ExprKind::Array(items) => self.lower_array(items)?,
            ExprKind::Function(function) => self.lower_function(function)?,
            ExprKind::Class(class) => self.lower_class(class)?,
            ExprKind::Call(callee, arguments) => self.lower_call(callee, arguments)?,
            ExprKind::Member(base, key) => self.lower_member(base, key)?,
            ExprKind::Construct(callee, arguments) => self.lower_construct(callee, arguments)?,
            ExprKind::SetMember(target, operator, value, strict) => {
                self.assignment_strict = *strict;
                self.lower_member_assignment(target, *operator, value)?
            }
            ExprKind::UpdateMember(target, add, prefix, strict) => {
                self.assignment_strict = *strict;
                self.lower_member_update(target, *add, *prefix)?
            }
            ExprKind::Regex(pattern, flags) => self.lower_regexp(pattern, flags)?,
            ExprKind::Template(head, parts) => self.lower_template(head, parts)?,
            _ => return None,
        };
        Some(result)
    }

    /// `SubstitutionTemplate` of 13.2.8.6: each substitution goes through
    /// 7.1.17 and the parts are concatenated left to right.
    fn lower_template(&mut self, head: &Value, parts: &[(Expr, Value)]) -> Option<RegisterType> {
        use crate::engine::bytecode::{BinaryOp, Instruction};
        let text = |value: &Value| match value {
            Value::String(units) => Some(units.clone()),
            _ => None,
        };
        let constant = self.string_constant(&text(head)?)?;
        let result = self.allocate_register()?;
        self.code.emit(Instruction::LdaString(constant));
        self.code.emit(Instruction::Star(result));
        let part = self.allocate_register()?;
        for (value, tail) in parts {
            self.lower(value)?;
            self.code.emit(Instruction::Star(part));
            self.code.emit(Instruction::ToText(part));
            self.code.emit(Instruction::Star(part));
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::BinaryOp)?;
            self.code.emit(Instruction::Binary {
                op: BinaryOp::Add,
                lhs: result,
                rhs: part,
                slot,
            });
            self.code.emit(Instruction::Star(result));
            let constant = self.string_constant(&text(tail)?)?;
            self.code.emit(Instruction::LdaString(constant));
            self.code.emit(Instruction::Star(part));
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::BinaryOp)?;
            self.code.emit(Instruction::Binary {
                op: BinaryOp::Add,
                lhs: result,
                rhs: part,
                slot,
            });
            self.code.emit(Instruction::Star(result));
        }
        self.code.emit(Instruction::Ldar(result));
        self.release_register(part)?;
        self.release_register(result)?;
        Some(RegisterType::String)
    }

    fn lower_destructuring_assignment(
        &mut self,
        pattern: &parser::AssignmentPattern,
        right: &Expr,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if !register_assignment_pattern_supported(pattern) {
            return None;
        }
        let value_type = self.lower(right)?;
        // 13.15.5.2 takes the value as it is: an array pattern reaches it
        // through 7.4.2, which throws for a value that carries no
        // `@@iterator`, and an object pattern through 7.3.5, which 13.15.5.5
        // step 1 guards with `RequireObjectCoercible`. Both run on a primitive
        // as they run on an Object.
        let result = self.allocate_register()?;
        self.code.emit(Instruction::Star(result));
        self.assign_pattern(value_type, pattern)?;
        self.code.emit(Instruction::Ldar(result));
        self.release_register(result)?;
        Some(value_type)
    }

    fn assign_pattern(
        &mut self,
        value_type: RegisterType,
        pattern: &parser::AssignmentPattern,
    ) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        match pattern {
            parser::AssignmentPattern::Target(target) => {
                self.assign_name(value_type, target)?;
            }
            parser::AssignmentPattern::Array(array) => {
                if !matches!(value_type, RegisterType::Array(_)) {
                    // 13.15.5.5 takes the elements from the iterator of the
                    // value.
                    return self
                        .lower_array_pattern_by_iterator(&RegisterArrayPattern::Assignment(array));
                }
                let source = self.allocate_register()?;
                self.code.emit(Instruction::Star(source));
                for (index, element) in array.elements.iter().enumerate() {
                    let parser::AssignmentArrayElement::Element {
                        target,
                        initializer,
                    } = element
                    else {
                        continue;
                    };
                    let prepared = self.prepare_assignment_pattern_target(target)?;
                    let index = u32::try_from(index).ok()?;
                    let mut element_type =
                        self.lower_array_index_from_register(source, value_type, index)?;
                    if let Some(initializer) = initializer {
                        element_type =
                            self.lower_assignment_default(element_type, initializer, target)?;
                    }
                    self.finish_assignment_pattern_target(element_type, target, prepared)?;
                }
                if let Some(rest) = &array.rest {
                    let prepared = self.prepare_assignment_pattern_target(rest)?;
                    let start = u32::try_from(array.elements.len()).ok()?;
                    let (rest_type, rest_array) =
                        self.lower_array_rest_from_register(source, value_type, start)?;
                    self.code.emit(Instruction::Ldar(rest_array));
                    self.release_register(rest_array)?;
                    self.finish_assignment_pattern_target(rest_type, rest, prepared)?;
                }
                self.release_register(source)?;
            }
            parser::AssignmentPattern::Object(object) => {
                let tracked = matches!(value_type, RegisterType::Object(_));
                if !value_type.is_object() {
                    // 13.15.5.5 step 1 refuses undefined and null before it
                    // reads any property.
                    self.code.emit(Instruction::Require(
                        crate::engine::bytecode::RequireKind::ObjectCoercible,
                    ));
                }
                let source = self.allocate_register()?;
                self.code.emit(Instruction::Star(source));
                let mut excluded = Vec::new();
                for property in &object.properties {
                    let prepared = self.prepare_assignment_pattern_target(&property.target)?;
                    if object.rest.is_some() {
                        excluded.push(Self::static_property_key_units(&property.key)?);
                    }
                    let keyed = Self::static_property_name(&property.key).is_none();
                    let mut property_type = if value_type.is_object() {
                        self.lower_property_from_register(
                            source,
                            value_type,
                            &property.key,
                            keyed,
                            true,
                        )?
                    } else {
                        self.lower_unknown_property_from_register(source, &property.key, keyed)?
                    };
                    if let Some(initializer) = &property.initializer {
                        property_type = self.lower_assignment_default(
                            property_type,
                            initializer,
                            &property.target,
                        )?;
                    }
                    self.finish_assignment_pattern_target(
                        property_type,
                        &property.target,
                        prepared,
                    )?;
                }
                if let Some(rest) = &object.rest {
                    let prepared = self.prepare_assignment_reference(rest)?;
                    let (rest_type, rest_object) = if tracked {
                        self.lower_object_rest_from_register(source, value_type, &excluded)?
                    } else {
                        self.lower_copied_data_properties(source, &excluded)?
                    };
                    self.code.emit(Instruction::Ldar(rest_object));
                    self.release_register(rest_object)?;
                    self.finish_assignment_reference(rest_type, rest, prepared)?;
                }
                self.release_register(source)?;
            }
        }
        Some(())
    }

    fn prepare_assignment_pattern_target(
        &mut self,
        pattern: &parser::AssignmentPattern,
    ) -> Option<RegisterPreparedAssignment> {
        let parser::AssignmentPattern::Target(target) = pattern else {
            return Some(RegisterPreparedAssignment::NestedPattern);
        };
        self.prepare_assignment_reference(target)
    }

    fn finish_assignment_pattern_target(
        &mut self,
        value_type: RegisterType,
        pattern: &parser::AssignmentPattern,
        prepared: RegisterPreparedAssignment,
    ) -> Option<()> {
        match (pattern, prepared) {
            (
                parser::AssignmentPattern::Target(target),
                prepared @ (RegisterPreparedAssignment::Name
                | RegisterPreparedAssignment::Member(_)),
            ) => self.finish_assignment_reference(value_type, target, prepared),
            (
                parser::AssignmentPattern::Array(_) | parser::AssignmentPattern::Object(_),
                RegisterPreparedAssignment::NestedPattern,
            ) => self.assign_pattern(value_type, pattern),
            (parser::AssignmentPattern::Target(_), RegisterPreparedAssignment::NestedPattern)
            | (
                parser::AssignmentPattern::Array(_) | parser::AssignmentPattern::Object(_),
                RegisterPreparedAssignment::Name | RegisterPreparedAssignment::Member(_),
            ) => None,
        }
    }

    fn prepare_assignment_reference(
        &mut self,
        target: &Expr,
    ) -> Option<RegisterPreparedAssignment> {
        if target.reference_name().is_some() {
            return Some(RegisterPreparedAssignment::Name);
        }
        // 10.1.9.1 reads the strictness of the Reference a member target is,
        // which is the strictness of the target and not of whatever assignment
        // was lowered before it.
        self.assignment_strict = target.strict;
        self.prepare_member_assignment(target)
            .map(RegisterPreparedAssignment::Member)
    }

    fn finish_assignment_reference(
        &mut self,
        value_type: RegisterType,
        target: &Expr,
        prepared: RegisterPreparedAssignment,
    ) -> Option<()> {
        match prepared {
            RegisterPreparedAssignment::Name => self.assign_name(value_type, target),
            RegisterPreparedAssignment::Member(prepared) => {
                target.member()?;
                self.finish_member_assignment(prepared, value_type)
            }
            RegisterPreparedAssignment::NestedPattern => None,
        }
    }

    /// Writes the accumulator to the Reference a name of a pattern is.
    fn assign_name(&mut self, value_type: RegisterType, target: &Expr) -> Option<()> {
        let name = target.reference_name()?;
        let Some(binding) = self.bindings.get(name).copied() else {
            // 9.1.1.4.5 writes a name no binding of this Script covers on the
            // Global Environment Record. Outside a Realm there is no such
            // Record to write to.
            if !self.realm {
                return None;
            }
            let constant = self.global_name(target)?;
            self.code
                .emit(crate::engine::bytecode::Instruction::StaGlobal {
                    name: constant,
                    strict: target.strict,
                });
            // Every later call can read the name from the Global Environment
            // Record, so the value is no longer only this Script's.
            self.escape(&[value_type]);
            return Some(());
        };
        if !binding.mutable || binding.value_type.is_none() || binding.stable_function_identity {
            return None;
        }
        self.store_binding(binding);
        self.bindings.get_mut(name)?.value_type = Some(value_type);
        Some(())
    }

    fn lower_conditional(
        &mut self,
        condition: &Expr,
        yes: &Expr,
        no: &Expr,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        self.lower(condition)?;
        let branch = self.code.emit(Instruction::JumpIfFalse(0));
        let bindings_before = self.bindings.clone();
        let properties_before = self.object_layouts.clone();
        let yes_type = self.lower(yes)?;
        let bindings_after_yes = self.bindings.clone();
        let properties_after_yes = self.object_layouts.clone();
        let jump = self.code.emit(Instruction::Jump(0));
        let no_start = self.code.instructions.len();
        self.bindings = bindings_before;
        self.object_layouts = properties_before;
        let no_type = self.lower(no)?;
        if self.bindings != bindings_after_yes || self.object_layouts != properties_after_yes {
            return None;
        }
        let end = self.code.instructions.len();
        self.patch_jump(branch, no_start)?;
        self.patch_jump(jump, end)?;
        Some(yes_type.merge(no_type))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one function emits every property definition of 13.2.5"
    )]
    fn lower_object(&mut self, properties: &[parser::ObjectProperty]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let object_id = self.next_object_id;
        self.next_object_id = self.next_object_id.checked_add(1)?;
        self.object_layouts.insert(
            object_id,
            RegisterObjectLayout::Ordinary {
                properties: BTreeMap::new(),
                order: Vec::new(),
                dynamic: None,
            },
        );
        self.code.emit(Instruction::CreateObject);
        let object = self.allocate_register()?;
        self.code.emit(Instruction::Star(object));
        for property in properties {
            if property.prototype {
                return None;
            }
            // 13.2.5.1 defines an accessor property, whose two halves meet on
            // the object. A computed name would have to reach the same
            // property, which needs the key at run time.
            if let Some(setter) = property.accessor {
                if property.computed {
                    // 13.2.5.1 with a key only the run time knows: 7.1.19
                    // makes it, and 10.2.10 names the half after it.
                    if !self.lower(&property.key)?.converts_to_primitive() {
                        return None;
                    }
                    let register = self.allocate_register()?;
                    self.code.emit(Instruction::Star(register));
                    self.lower(&property.value)?;
                    self.code.emit(Instruction::DefineAccessorByValue {
                        obj: object,
                        key: register,
                        setter,
                        enumerable: true,
                    });
                    self.release_register(register)?;
                    self.record_ordinary_property_write(object_id, None, RegisterType::Unknown)?;
                    continue;
                }
                let name = Self::static_property_name(&property.key)?.to_vec();
                let constant = self.string_constant(&name)?;
                // 10.2.10 names a getter "get x" and a setter "set x".
                self.lower_named(&property.value, &accessor_name(setter, &name))?;
                self.code.emit(Instruction::DefineAccessor {
                    obj: object,
                    name: constant,
                    setter,
                    enumerable: true,
                });
                // A read of the property is a call of its getter, so the
                // layout knows the name and not what it answers.
                self.record_ordinary_property_write(object_id, Some(name), RegisterType::Unknown)?;
                continue;
            }
            // 13.2.5.5 names the function a definition holds after the key it
            // is given, which for a computed one 10.2.10 does at run time.
            if property.computed && register_names_itself_after_its_key(&property.value) {
                if !self.lower(&property.key)?.converts_to_primitive() {
                    return None;
                }
                let register = self.allocate_register()?;
                self.code.emit(Instruction::Star(register));
                self.lower(&property.value)?;
                self.code.emit(Instruction::DefineMethodByValue {
                    obj: object,
                    key: register,
                    enumerable: true,
                });
                self.release_register(register)?;
                self.record_ordinary_property_write(object_id, None, RegisterType::Unknown)?;
                continue;
            }
            let key = if property.computed {
                let static_name = Self::static_property_key_units(&property.key);
                if !self.lower(&property.key)?.converts_to_primitive() {
                    return None;
                }
                let register = self.allocate_register()?;
                self.code.emit(Instruction::Star(register));
                RegisterMemberKey::ObjectKeyed(register, static_name)
            } else {
                let name = Self::static_property_name(&property.key)?;
                let name = name.to_vec();
                RegisterMemberKey::Named {
                    constant: self.string_constant(&name)?,
                    name,
                }
            };
            // 13.2.5.5 names the function a property definition holds after
            // the key it is given (8.5.2).
            let value_type = match &key {
                RegisterMemberKey::Named { name, .. } => {
                    let name = name.clone();
                    self.lower_named(&property.value, &name)?
                }
                _ => self.lower(&property.value)?,
            };
            if matches!(value_type, RegisterType::NativeFunction(_)) {
                return None;
            }
            // 13.2.5.5 makes every method of a literal a method of the object;
            // only a body that reads `super` can tell it from a function the
            // property merely holds.
            if register_is_method_reading_super(&property.value) {
                self.code.emit(Instruction::MakeMethod { home: object });
            }
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            let property_name = match key {
                RegisterMemberKey::Named { constant, name } => {
                    self.code.emit(Instruction::SetNamed {
                        obj: object,
                        name: constant,
                        slot,
                        strict: false,
                        define: true,
                    });
                    Some(name)
                }
                RegisterMemberKey::ObjectKeyed(register, name) => {
                    self.code.emit(Instruction::SetByValue {
                        obj: object,
                        key: register,
                        slot,
                        define: true,
                        strict: false,
                    });
                    self.release_register(register)?;
                    name
                }
                RegisterMemberKey::ArrayKeyed { .. } => return None,
            };
            self.record_ordinary_property_write(object_id, property_name, value_type)?;
        }
        self.code.emit(Instruction::Ldar(object));
        self.release_register(object)?;
        Some(RegisterType::Object(object_id))
    }

    fn record_ordinary_property_write(
        &mut self,
        object_id: u32,
        property_name: Option<Vec<u16>>,
        value_type: RegisterType,
    ) -> Option<()> {
        let RegisterObjectLayout::Ordinary {
            properties,
            order,
            dynamic,
        } = self.object_layouts.get_mut(&object_id)?
        else {
            return None;
        };
        if let Some(name) = property_name {
            let is_new = !properties.contains_key(&name);
            if is_new && properties.len() >= self.property_limit {
                return None;
            }
            if is_new {
                order.push(name.clone());
            }
            properties.insert(name, value_type);
        } else {
            *dynamic = Some(dynamic.map_or(value_type, |current| current.merge(value_type)));
        }
        Some(())
    }

    fn lower_array(&mut self, items: &[Option<Expr>]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let object_id = self.next_object_id;
        self.next_object_id = self.next_object_id.checked_add(1)?;
        let length = u32::try_from(items.len()).ok()?;
        self.object_layouts.insert(
            object_id,
            RegisterObjectLayout::Array {
                length: Some(length),
                elements: BTreeMap::new(),
                dynamic: None,
            },
        );
        let property_count = items
            .iter()
            .filter(|item| item.is_some())
            .count()
            .saturating_add(1);
        if property_count > self.property_limit {
            return None;
        }
        if items
            .iter()
            .flatten()
            .any(|item| matches!(item.kind, ExprKind::Spread(_)))
        {
            return self.lower_spread_array(items);
        }
        self.code.emit(Instruction::CreateArray(length));
        let array = self.allocate_register()?;
        self.code.emit(Instruction::Star(array));
        for (index, item) in items.iter().enumerate() {
            let Some(item) = item else {
                continue;
            };

            let index = u32::try_from(index).ok()?;
            self.emit_array_index(index)?;
            let key = self.allocate_register()?;
            self.code.emit(Instruction::Star(key));
            let value_type = self.lower(item)?;
            if matches!(value_type, RegisterType::NativeFunction(_)) {
                return None;
            }
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::SetByValue {
                obj: array,
                key,
                slot,
                define: true,
                strict: false,
            });
            let RegisterObjectLayout::Array { elements, .. } =
                self.object_layouts.get_mut(&object_id)?
            else {
                return None;
            };
            elements.insert(index, value_type);
            self.release_register(key)?;
        }
        self.code.emit(Instruction::Ldar(array));
        self.release_register(array)?;
        Some(RegisterType::Array(object_id))
    }

    /// Whether an argument list or an element list holds a spread element,
    /// which 13.3.8 turns into a List the run time decides the length of.
    fn holds_a_spread(items: &[Expr]) -> bool {
        items
            .iter()
            .any(|item| matches!(item.kind, ExprKind::Spread(_)))
    }

    /// The List of 13.3.8, as the Array the call carries.
    ///
    /// The caller releases the register this answers, which holds the Array,
    /// and the one after it, which holds the index the writes stand at.
    fn lower_argument_list(
        &mut self,
        arguments: &[Expr],
    ) -> Option<(
        crate::engine::bytecode::Reg,
        crate::engine::bytecode::Reg,
        crate::engine::bytecode::Reg,
    )> {
        use crate::engine::bytecode::Instruction;
        let array = self.allocate_register()?;
        let index = self.allocate_register()?;
        let one = self.allocate_register()?;
        self.code.emit(Instruction::CreateArray(0));
        self.code.emit(Instruction::Star(array));
        self.code.emit(Instruction::LdaSmi(0));
        self.code.emit(Instruction::Star(index));
        self.code.emit(Instruction::LdaSmi(1));
        self.code.emit(Instruction::Star(one));
        for argument in arguments {
            if let ExprKind::Spread(inner) = &argument.kind {
                self.lower_spread_operand(inner, array, index, one)?;
                continue;
            }
            self.lower(argument)?;
            self.append_to_list(array, index, one)?;
        }
        Some((array, index, one))
    }

    /// Writes the accumulator at the index the List stands at and moves it on.
    fn append_to_list(
        &mut self,
        array: crate::engine::bytecode::Reg,
        index: crate::engine::bytecode::Reg,
        one: crate::engine::bytecode::Reg,
    ) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::SetByValue {
            obj: array,
            key: index,
            slot,
            define: true,
            strict: false,
        });
        self.code.emit(Instruction::Ldar(index));
        self.code.emit(Instruction::Add(one));
        self.code.emit(Instruction::Star(index));
        Some(())
    }

    /// 13.2.4.2 and 13.3.8.1: every value of the iterator of the operand goes
    /// into the List, in the order 7.4.6 answers them.
    fn lower_spread_operand(
        &mut self,
        operand: &Expr,
        array: crate::engine::bytecode::Reg,
        index: crate::engine::bytecode::Reg,
        one: crate::engine::bytecode::Reg,
    ) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        self.lower(operand)?;
        let iterable = self.allocate_register()?;
        self.code.emit(Instruction::Star(iterable));
        let iterator_slot =
            self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetWellKnown {
            obj: iterable,
            symbol: u16::try_from(WELL_KNOWN_ITERATOR).ok()?,
            slot: iterator_slot,
        });
        let method = self.allocate_register()?;
        self.code.emit(Instruction::Star(method));
        let open = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver: iterable,
            func: method,
            arg_start: method,
            arg_count: 0,
            slot: open,
        });
        let iterator = self.allocate_register()?;
        self.code.emit(Instruction::Star(iterator));
        let next = self.allocate_register()?;
        let step = self.allocate_register()?;
        let head = self.code.instructions.len();
        let next_name = self.string_constant(&"next".encode_utf16().collect::<Vec<_>>())?;
        let next_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: iterator,
            name: next_name,
            slot: next_slot,
        });
        self.code.emit(Instruction::Star(next));
        let step_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver: iterator,
            func: next,
            arg_start: next,
            arg_count: 0,
            slot: step_slot,
        });
        self.code.emit(Instruction::Star(step));
        let done_name = self.string_constant(&"done".encode_utf16().collect::<Vec<_>>())?;
        let done_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: step,
            name: done_name,
            slot: done_slot,
        });
        // 7.4.6 answers done by ToBoolean, and 7.4.7 reads `value` only where
        // it is false, which is what the order of these two says.
        let exit = self.code.emit(Instruction::JumpIfTrue(0));
        let value_name = self.string_constant(&"value".encode_utf16().collect::<Vec<_>>())?;
        let value_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: step,
            name: value_name,
            slot: value_slot,
        });
        self.append_to_list(array, index, one)?;
        let back = self.code.emit(Instruction::Jump(0));
        self.patch_jump(back, head)?;
        let done = self.code.instructions.len();
        self.patch_jump(exit, done)?;
        self.release_register(step)?;
        self.release_register(next)?;
        self.release_register(iterator)?;
        self.release_register(method)?;
        self.release_register(iterable)?;
        Some(())
    }

    /// 13.3.6.2 and 13.3.8: a call whose argument list holds a spread element.
    ///
    /// A computed member callee is a named gap: 13.3.3 evaluates the key
    /// before the arguments, which this path has no register window for.
    fn lower_spread_call(&mut self, callee: &Expr, arguments: &[Expr]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let receiver = self.allocate_register()?;
        let function = self.allocate_register()?;
        if let ExprKind::Member(base, key) = &callee.kind {
            let name = Self::static_property_name(key)?.to_vec();
            self.lower(base)?;
            self.code.emit(Instruction::Star(receiver));
            let constant = self.string_constant(&name)?;
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: receiver,
                name: constant,
                slot,
            });
            self.code.emit(Instruction::Star(function));
        } else {
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(receiver));
            self.lower(callee)?;
            self.code.emit(Instruction::Star(function));
        }
        let (array, index, one) = self.lower_argument_list(arguments)?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallSpread {
            receiver,
            func: function,
            list: array,
            slot,
        });
        self.release_register(one)?;
        self.release_register(index)?;
        self.release_register(array)?;
        self.release_register(function)?;
        self.release_register(receiver)?;
        self.escape(&[RegisterType::Unknown]);
        Some(RegisterType::Unknown)
    }

    /// 13.3.5.1: `new` whose argument list holds a spread element.
    fn lower_spread_construct(
        &mut self,
        callee: &Expr,
        arguments: &[Expr],
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let function = self.allocate_register()?;
        let target = self.allocate_register()?;
        self.lower(callee)?;
        self.code.emit(Instruction::Star(function));
        let (array, index, one) = self.lower_argument_list(arguments)?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::ConstructSpread {
            func: function,
            target,
            list: array,
            slot,
        });
        self.release_register(one)?;
        self.release_register(index)?;
        self.release_register(array)?;
        self.release_register(target)?;
        self.release_register(function)?;
        self.escape(&[RegisterType::Unknown]);
        Some(RegisterType::Unknown)
    }

    /// 13.2.4.2: an Array literal that holds a spread element.
    fn lower_spread_array(&mut self, items: &[Option<Expr>]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let array = self.allocate_register()?;
        let index = self.allocate_register()?;
        let one = self.allocate_register()?;
        self.code.emit(Instruction::CreateArray(0));
        self.code.emit(Instruction::Star(array));
        self.code.emit(Instruction::LdaSmi(0));
        self.code.emit(Instruction::Star(index));
        self.code.emit(Instruction::LdaSmi(1));
        self.code.emit(Instruction::Star(one));
        for item in items {
            let Some(item) = item else {
                // 13.2.4.2: an elision moves the index on and defines nothing.
                self.code.emit(Instruction::Ldar(index));
                self.code.emit(Instruction::Add(one));
                self.code.emit(Instruction::Star(index));
                continue;
            };
            if let ExprKind::Spread(inner) = &item.kind {
                self.lower_spread_operand(inner, array, index, one)?;
                continue;
            }
            self.lower(item)?;
            self.append_to_list(array, index, one)?;
        }
        // 13.2.4.2 step 5 sets the length, which an elision at the end needs.
        let length = self.string_constant(&"length".encode_utf16().collect::<Vec<_>>())?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::Ldar(index));
        self.code.emit(Instruction::SetNamed {
            obj: array,
            name: length,
            slot,
            define: false,
            strict: false,
        });
        self.code.emit(Instruction::Ldar(array));
        self.release_register(one)?;
        self.release_register(index)?;
        self.release_register(array)?;
        self.escape(&[RegisterType::Unknown]);
        Some(RegisterType::Unknown)
    }

    fn lower_function(&mut self, function: &Function) -> Option<RegisterType> {
        self.lower_callable(function, false, None)
    }

    /// `NamedEvaluation` of 8.5.2: an anonymous function or class takes the
    /// name the expression is being given.
    fn lower_named(&mut self, expression: &Expr, name: &[u16]) -> Option<RegisterType> {
        match &expression.kind {
            ExprKind::Group(inner) => self.lower_named(inner, name),
            ExprKind::Function(function) if function.name.is_none() => {
                self.lower_callable(function, false, Some(name))
            }
            ExprKind::Class(class) if class.name.is_none() => self.lower_class_named(class, name),
            _ => self.lower(expression),
        }
    }

    /// Lowers a function body into a unit of its own and leaves the closure
    /// 10.2.4 makes of it in the accumulator.
    ///
    /// `class` says the body is the constructor of a class, which 15.7.14
    /// gives a `prototype` no ordinary function's attributes match.
    #[expect(
        clippy::too_many_lines,
        reason = "one function carries a body from its scope to its unit"
    )]
    fn lower_callable(
        &mut self,
        function: &Function,
        class: bool,
        name: Option<&[u16]>,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if let Some(refusal) = Self::function_refusal(function, class) {
            self.refuse(refusal);
            return None;
        }
        let code_id = u32::try_from(self.code.functions.len())
            .ok()?
            .checked_add(self.function_table_base)?;
        let first_child_object_id = self.next_object_id;
        let inherited_layouts = self.object_layouts.clone();
        // The scan of the names a body reads and writes stops at a construct
        // it does not walk, and the lowering would report nothing of it.
        let Some(scope) = register_function_scope(function) else {
            self.refuse("a name of a function body the scan does not reach");
            return None;
        };
        let mut captures = BTreeMap::new();
        for name in &scope.free_names {
            if self.bindings.contains_key(name) {
                captures.insert(name.clone(), self.capture_binding(name)?);
            }
        }
        // 10.2.1.1 gives an arrow no Function Environment Record, so 9.4.2
        // answers its `this` out of the one this function has. The binding is
        // captured like any other name the arrow reads. A Realm Script has
        // none, and 9.4.2 answers the `[[GlobalThisValue]]` there.
        if function.arrow
            && register_body_reads_this(&function.body)
            && self.bindings.contains_key(THIS_BINDING)
        {
            // 9.4.5 refuses the `this` binding of a derived constructor until
            // 13.3.7.1 has made it. The register carries that state and a copy
            // of it into the context does not, so an arrow that reads it there
            // is a named gap.
            if self.code.derived {
                self.refuse("the `this` of a derived constructor, read by an arrow");
                return None;
            }
            captures.insert(
                String::from(THIS_BINDING),
                self.capture_binding(THIS_BINDING)?,
            );
        }
        let Some((mut child, self_register)) =
            self.register_function_child(function, code_id, &captures, &scope.captured_names)
        else {
            self.refuse("a frame of a call the lowering does not prepare");
            return None;
        };
        if !captures.is_empty() {
            child.code.outer_context_slot_counts = self.context_slot_counts()?;
        }
        // 10.2.5 gives an ordinary function a `[[Construct]]` and a `prototype`,
        // and withholds both from a method and an arrow. The body is told
        // before it is lowered, because a constructor may construct itself.
        // 27.7.4 gives an async function no `[[Construct]]` and no
        // `prototype`.
        let child_constructible = function.constructible
            && !function.arrow
            && function.async_kind == parser::AsyncKind::Sync;
        child.constructible.clone_from(&self.constructible);
        if child_constructible {
            child.constructible.insert(code_id);
        }
        // The body is lowered in its own unit, so the construct it stopped at
        // is recorded there and would be lost with it.
        let Some(return_type) = Self::lower_function_body(&mut child, function, &function.body)
        else {
            if let Some(refusal) = child.refusal {
                self.refuse(refusal);
            }
            return None;
        };
        let capture_effects = captures
            .iter()
            .map(|(name, captured)| {
                let initial_type = captured
                    .value_type
                    .or_else(|| self.binding_type_hints.get(name).copied())?;
                let final_type = child.bindings.get(name)?.value_type?;
                Some((name.clone(), initial_type.merge(final_type)))
            })
            .collect::<Option<BTreeMap<_, _>>>()?;
        let layout_effects = inherited_layouts
            .iter()
            .filter_map(|(id, inherited)| {
                let final_layout = child.object_layouts.get(id)?;
                (final_layout != inherited).then(|| (*id, final_layout.clone()))
            })
            .collect();
        if !return_type.is_returnable() {
            // The value of a `return` carries a type the lowering named
            // and cannot hand to a caller, which is the lowering's own gap.
            self.refuse("a return of a value the lowering cannot type");
            return None;
        }
        self.next_object_id = child.next_object_id;
        self.object_layouts.extend(
            child
                .object_layouts
                .iter()
                .filter(|(id, _)| **id >= first_child_object_id)
                .map(|(id, layout)| (*id, layout.clone())),
        );
        child.code.register_count = child.register_count;
        child.code.parameter_count = u16::try_from(function.parameters.len()).ok()?;
        // 15.1.5 stops counting at the first Initializer and at a rest
        // parameter, which is what 10.2.9 gives the function as its `length`.
        let expected = function
            .parameters
            .iter()
            .position(|parameter| parameter.default.is_some() || parameter.rest)
            .unwrap_or(function.parameters.len());
        child.code.expected_arguments = u16::try_from(expected).ok()?;
        child.code.binding_count = child.max_binding_count;
        child.code.self_register = self_register;
        child.code.constructible = child_constructible;
        child.code.strict = function.strict;
        child.code.class_constructor = class;
        // 10.2.10 gives the function the name it carries, and 8.5.2 the one
        // the expression it stands in is being given.
        let own: Option<Vec<u16>> = function
            .name
            .as_ref()
            .map(|name| name.encode_utf16().collect());
        if let Some(units) = own.as_deref().or(name) {
            child.code.name = Some(child.string_constant(units)?);
        }
        // 20.2.3.5 answers the source text of the grammar node the function
        // was written as, which the parser kept beside it.
        if let Some(source) = &function.source {
            let text: Vec<u16> = source
                .text
                .get(source.range.clone())?
                .encode_utf16()
                .collect();
            child.code.source = Some(child.string_constant(&text)?);
        }
        let nested_functions = core::mem::take(&mut child.code.functions);
        self.code.functions.push(child.code);
        self.code.functions.extend(nested_functions);
        self.function_returns.extend(child.function_returns);
        self.function_parameters.extend(child.function_parameters);
        self.function_capture_effects
            .extend(child.function_capture_effects);
        self.function_layout_effects
            .extend(child.function_layout_effects);
        if child_constructible {
            self.constructible.insert(code_id);
        }
        self.constructible
            .extend(child.constructible.iter().copied());
        self.function_returns.insert(code_id, return_type);
        self.function_parameters.insert(
            code_id,
            alloc::vec![RegisterType::Unknown; function.parameters.len()],
        );
        self.function_capture_effects
            .insert(code_id, capture_effects);
        self.function_layout_effects.insert(code_id, layout_effects);
        self.code.emit(if class {
            Instruction::CreateClass(code_id)
        } else {
            Instruction::CreateClosure(code_id)
        });
        Some(RegisterType::Function(code_id))
    }

    /// Lowers a class body (15.7.14), leaving its constructor in the
    /// accumulator.
    fn lower_class(&mut self, class: &parser::Class) -> Option<RegisterType> {
        let name: Option<Vec<u16>> = class
            .name
            .as_ref()
            .map(|name| name.encode_utf16().collect());
        self.lower_class_with(class, name.as_deref())
    }

    /// A class expression 8.5.2 gives a name it does not carry itself.
    fn lower_class_named(&mut self, class: &parser::Class, name: &[u16]) -> Option<RegisterType> {
        self.lower_class_with(class, Some(name))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one function carries a class from its heritage to its methods"
    )]
    fn lower_class_with(
        &mut self,
        class: &parser::Class,
        name: Option<&[u16]>,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        // 15.7.14 step 5 evaluates the heritage before the constructor is
        // made, and the register keeps it where the collector sees it. The
        // binding of the class name is in its Temporal Dead Zone there, which
        // the lowering answers by not carrying it yet.
        let heritage = match &class.heritage {
            Some(heritage) => {
                self.lower(heritage)?;
                let register = self.allocate_register()?;
                self.code.emit(Instruction::Star(register));
                Some(register)
            }
            None => None,
        };
        // 15.7.14 steps 3 and 4 give the class body a binding of its own for
        // the class name, which the constructor and every method reach and
        // nothing can write.
        let inner = class.name.clone();
        let inner = match &inner {
            Some(inner) => {
                let register = self.allocate_register()?;
                self.active_binding_count = self.active_binding_count.checked_add(1)?;
                self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
                let shadowed = self.bindings.insert(
                    inner.clone(),
                    RegisterBinding {
                        storage: RegisterBindingStorage::Register(register),
                        value_type: Some(RegisterType::Unknown),
                        mutable: false,
                        stable_function_identity: false,
                    },
                );
                Some((inner.clone(), register, shadowed))
            }
            None => None,
        };
        let value_type = self.lower_callable(&class.constructor, true, name)?;
        // 15.7.14 steps 6 through 8 tie the class to its heritage while the
        // class is still in the accumulator.
        if let Some(heritage) = heritage {
            self.code.emit(Instruction::DeriveClass { heritage });
        }
        let constructor = self.allocate_register()?;
        self.code.emit(Instruction::Star(constructor));
        // 15.7.14 step 17 initializes the binding of the class name with the
        // class, which every method and every computed key after this reads.
        // The constructor may already have captured it, which moved it out of
        // the register and into a context slot, so the write goes where the
        // binding is now and not where it started.
        if let Some((name, _, _)) = &inner {
            let binding = *self.bindings.get(name)?;
            self.code.emit(Instruction::Ldar(constructor));
            self.store_binding(binding);
        }
        // 15.7.14 puts every method the body defines on the prototype the
        // constructor carries, and a static one on the constructor itself.
        let prototype = self.allocate_register()?;
        let name = self.string_constant(&PROTOTYPE_UNITS)?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: constructor,
            name,
            slot,
        });
        self.code.emit(Instruction::Star(prototype));
        for (is_static, method) in &class.methods {
            let target = if *is_static { constructor } else { prototype };
            // 15.7.14 with a key only the run time knows: 7.1.19 makes it, and
            // 10.2.10 names the method after it.
            if method.computed {
                if !self.lower(&method.key)?.converts_to_primitive() {
                    return None;
                }
                let register = self.allocate_register()?;
                self.code.emit(Instruction::Star(register));
                self.lower(&method.value)?;
                self.code.emit(match method.accessor {
                    Some(setter) => Instruction::DefineAccessorByValue {
                        obj: target,
                        key: register,
                        setter,
                        enumerable: false,
                    },
                    None => Instruction::DefineMethodByValue {
                        obj: target,
                        key: register,
                        enumerable: false,
                    },
                });
                self.release_register(register)?;
                continue;
            }
            let name = Self::static_property_name(&method.key)?.to_vec();
            let constant = self.string_constant(&name)?;
            let given = match method.accessor {
                Some(setter) => accessor_name(setter, &name),
                None => name.clone(),
            };
            self.lower_named(&method.value, &given)?;
            self.code.emit(match method.accessor {
                Some(setter) => Instruction::DefineAccessor {
                    obj: target,
                    name: constant,
                    setter,
                    enumerable: false,
                },
                None => Instruction::DefineMethod {
                    obj: target,
                    name: constant,
                },
            });
        }
        self.code.emit(Instruction::Ldar(constructor));
        self.release_register(prototype)?;
        self.release_register(constructor)?;
        if let Some((name, register, shadowed)) = inner {
            match shadowed {
                Some(shadowed) => self.bindings.insert(name, shadowed),
                None => self.bindings.remove(&name),
            };
            self.active_binding_count = self.active_binding_count.checked_sub(1)?;
            self.release_register(register)?;
        }
        if let Some(heritage) = heritage {
            self.release_register(heritage)?;
        }
        Some(value_type)
    }

    fn lower_function_declaration(
        &mut self,
        name: &str,
        function: &Function,
    ) -> Option<RegisterType> {
        let code_id = u32::try_from(self.code.functions.len())
            .ok()?
            .checked_add(self.function_table_base)?;
        let scope = register_function_scope(function)?;
        if scope.free_names.contains(name) {
            let binding = self.bindings.get_mut(name)?;
            if !binding.mutable {
                return None;
            }
            binding.value_type = Some(RegisterType::Function(code_id));
            binding.stable_function_identity = true;
            self.function_returns
                .insert(code_id, RegisterType::Primitive);
            self.function_parameters.insert(
                code_id,
                alloc::vec![RegisterType::Unknown; function.parameters.len()],
            );
            // A declaration names itself in its own body, so a constructor that
            // constructs itself has to know it is one before the body is
            // lowered (10.2.5).
            if function.constructible && !function.arrow {
                self.constructible.insert(code_id);
            }
        }
        // 10.2.10 names a declaration after the binding it makes.
        let units: Vec<u16> = name.encode_utf16().collect();
        self.lower_callable(function, false, Some(&units))
    }

    /// What 10.2.11 would have to do for this function that the lowering does
    /// not, so that the refusal names the parameter list or the kind and not
    /// the expression the function was written as.
    fn function_refusal(function: &Function, class: bool) -> Option<&'static str> {
        let kind_allowed = match function.constructor_kind {
            parser::ConstructorKind::Ordinary => true,
            parser::ConstructorKind::BaseClass | parser::ConstructorKind::DerivedClass => class,
        };
        if !kind_allowed {
            return Some("a class constructor");
        }
        // 10.2.11 instantiates each of these differently, and they are three
        // pieces of work, so the refusal says which one it stands on.
        // 8.6.3 binds the Array to a name; a pattern of 8.6.2 over it is a
        // second piece of work.
        if function.parameters.iter().any(|parameter| {
            parameter.rest && !matches!(parameter.pattern, parser::BindingPattern::Name(_))
        }) {
            return Some("a rest parameter that is a pattern");
        }
        // A rest parameter takes the arguments the call passed beyond it, so
        // an Initializer of its own would never run.
        if function
            .parameters
            .iter()
            .any(|parameter| parameter.rest && parameter.default.is_some())
        {
            return Some("a rest parameter with an Initializer");
        }
        // 15.1.5: only the last parameter is a rest one.
        if function
            .parameters
            .iter()
            .rev()
            .skip(1)
            .any(|parameter| parameter.rest)
        {
            return Some("a rest parameter that is not the last");
        }
        let duplicated = !function.parameters.iter().all(|parameter| {
            let Some(name) = parameter.pattern.identifier() else {
                return true;
            };
            function
                .parameters
                .iter()
                .filter(|candidate| candidate.pattern.identifier() == Some(name))
                .count()
                == 1
        });
        if duplicated {
            return Some("a duplicated parameter name");
        }
        // 10.2.11 initializes the parameters in order, so an Initializer that
        // reads the parameter it binds, or one the list binds after it, reads
        // a binding of its temporal dead zone. The registers of the frame hold
        // undefined there and say nothing of the two apart.
        register_initializer_reads_a_later_parameter(function)
            .then_some("a parameter Initializer that reads a parameter of its dead zone")
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one function prepares every binding a call frame starts with"
    )]
    fn register_function_child(
        &self,
        function: &Function,
        code_id: u32,
        captures: &BTreeMap<String, RegisterBinding>,
        captured_names: &BTreeSet<String>,
    ) -> Option<(Self, Option<crate::engine::bytecode::Reg>)> {
        let mut child = Self::new(
            0,
            function.body.iter().fold(1usize, |maximum, statement| {
                maximum.max(register_statement_stack_requirement(statement))
            }),
            self.property_limit,
            code_id.checked_add(1)?,
        );
        child.allow_return = true;
        // The body is lowered before the unit is finished, and 10.4.4 reads
        // the strictness while it runs.
        child.code.strict = function.strict;
        child.realm = self.realm;
        child.function_returns = self.function_returns.clone();
        child.function_parameters = self.function_parameters.clone();
        child.function_capture_effects = self.function_capture_effects.clone();
        child.function_layout_effects = self.function_layout_effects.clone();
        child.next_object_id = self.next_object_id;
        child.object_layouts = self.object_layouts.clone();
        // 10.4.4.7 builds the map for a sloppy function whose parameter list
        // is simple; 10.4.4.6 builds the object of every other one, which maps
        // nothing. A function that maps holds its parameters in a context of
        // its own, which every inherited binding is one step further out in.
        let maps = !function.strict
            && !function.arrow
            && register_maps_its_parameters(function)
            && register_body_reads_arguments(&function.body)?;
        let depth_shift = u16::from(!captured_names.is_empty() || maps);
        for (name, binding) in captures {
            let RegisterBindingStorage::Context { depth, slot } = binding.storage else {
                return None;
            };
            child.bindings.insert(
                name.clone(),
                RegisterBinding {
                    storage: RegisterBindingStorage::Context {
                        depth: depth.checked_add(depth_shift)?,
                        slot,
                    },
                    value_type: merge_optional_register_types(
                        binding.value_type,
                        self.binding_type_hints.get(name).copied(),
                    ),
                    ..*binding
                },
            );
        }
        // 10.2.11 binds the argument itself, whatever it is, so a parameter
        // has the type of a value the lowering cannot name. A parameter that
        // is a pattern needs a binding for the argument as well, because the
        // call fills the register and no name of the Script reaches it.
        for (index, parameter) in function.parameters.iter().enumerate() {
            let name = match &parameter.pattern {
                parser::BindingPattern::Name(name) => name.clone(),
                _ => register_argument_name(index),
            };
            child.declare(&name, true)?;
            child.bindings.get_mut(&name)?.value_type = Some(RegisterType::Unknown);
        }
        for parameter in &function.parameters {
            if matches!(parameter.pattern, parser::BindingPattern::Name(_)) {
                continue;
            }
            let mut bound = Vec::new();
            parameter.pattern.names(&mut bound);
            for name in bound {
                child.declare(&name, true)?;
                child.bindings.get_mut(&name)?.value_type = Some(RegisterType::Unknown);
            }
        }
        // 9.4.4 answers the Prototype of the running function, which 13.3.7.1
        // constructs; the entry fills the register with the callee.
        let derived = function.constructor_kind == parser::ConstructorKind::DerivedClass;
        child.code.derived = derived;
        let self_register = if derived && function.name.is_none() {
            child.declare(CALLEE_BINDING, false)?;
            let RegisterBindingStorage::Register(register) =
                child.bindings.get(CALLEE_BINDING)?.storage
            else {
                return None;
            };
            child.bindings.remove(CALLEE_BINDING);
            Some(register)
        } else if let Some(name) = &function.name {
            if function
                .parameters
                .iter()
                .any(|parameter| parameter.pattern.identifier() == Some(name))
            {
                return None;
            }
            child.declare(name, false)?;
            let RegisterBindingStorage::Register(register) = child.bindings.get(name)?.storage
            else {
                return None;
            };
            child.bindings.get_mut(name)?.value_type = Some(RegisterType::Function(code_id));
            child
                .function_returns
                .insert(code_id, RegisterType::Primitive);
            child.function_parameters.insert(
                code_id,
                alloc::vec![RegisterType::Unknown; function.parameters.len()],
            );
            Some(register)
        } else {
            None
        };
        // 10.4.4 binds `arguments` in every ordinary function. A strict
        // function's object carries no mapping at all; a sloppy one's mapping
        // of 10.4.4.7 is only unobservable while no parameter is assigned.
        if !function.arrow
            && register_body_reads_arguments(&function.body)?
            && (maps
                || function.strict
                || !register_body_writes_parameters(&function.body, function)?)
        {
            child.declare(ARGUMENTS, false)?;
            let RegisterBindingStorage::Register(register) = child.bindings.get(ARGUMENTS)?.storage
            else {
                return None;
            };
            child.bindings.get_mut(ARGUMENTS)?.value_type = Some(RegisterType::Unknown);
            child.arguments_binding = child.bindings.get(ARGUMENTS).copied();
            // 10.4.4.7 maps the indices of a sloppy function's object onto its
            // parameters, and a nested function that reads the object could
            // observe the mapping, which this engine does not build.
            let captures_arguments = captured_names.contains(ARGUMENTS);
            if captures_arguments && !(function.strict || function.parameters.is_empty() || maps) {
                return None;
            }
            if !captures_arguments {
                child.bindings.remove(ARGUMENTS);
            }
            child.mapped_parameters = function.parameters.len();
            child.maps_arguments = maps;
            child.code.arguments_register = Some(register);
        }
        // 10.2.2 and 13.3.7.1 read the `[[NewTarget]]` of the call and the
        // `this` binding, whatever the body names.
        // 9.4.3 answers the `[[NewTarget]]` of the call, which an arrow takes
        // from the function it was made in; 15.3.4 gives it none of its own.
        // 27.7.5.2 makes the capability before the body runs and keeps it in
        // a register of the frame, where the collector sees it and where a
        // continuation of 27.7.5.3 takes it along.
        if function.async_kind != parser::AsyncKind::Sync {
            child.code.asynchronous = true;
            child.declare(PROMISE_BINDING, false)?;
            let RegisterBindingStorage::Register(register) =
                child.bindings.get(PROMISE_BINDING)?.storage
            else {
                return None;
            };
            child.bindings.remove(PROMISE_BINDING);
            child.code.promise_register = Some(register);
        }
        if derived || register_body_reads_new_target(&function.body) {
            if function.arrow {
                return None;
            }
            child.declare(NEW_TARGET_BINDING, false)?;
            let RegisterBindingStorage::Register(register) =
                child.bindings.get(NEW_TARGET_BINDING)?.storage
            else {
                return None;
            };
            child.bindings.remove(NEW_TARGET_BINDING);
            child.code.new_target_register = Some(register);
        }
        // 13.3.7.3 reads the `[[HomeObject]]` of the running function, which
        // an arrow has none of: 15.3.4 gives it the `super` of the function it
        // was made in, and this engine has not built that.
        if register_body_reads_super(&function.body) {
            if function.arrow {
                return None;
            }
            child.declare(HOME_BINDING, false)?;
            let RegisterBindingStorage::Register(register) =
                child.bindings.get(HOME_BINDING)?.storage
            else {
                return None;
            };
            child.bindings.get_mut(HOME_BINDING)?.value_type = Some(RegisterType::Unknown);
            child.bindings.remove(HOME_BINDING);
            child.code.home_register = Some(register);
        }
        // 10.2.1.1 gives an arrow no Function Environment Record of its own,
        // so its `this` is the binding the enclosing function captured into
        // the context and not the receiver of its call.
        if (derived || register_body_reads_this(&function.body)) && !function.arrow {
            child.declare(THIS_BINDING, false)?;
            let RegisterBindingStorage::Register(register) =
                child.bindings.get(THIS_BINDING)?.storage
            else {
                return None;
            };
            // The receiver of the call; the lowering can name no type for it.
            child.bindings.get_mut(THIS_BINDING)?.value_type = Some(RegisterType::Unknown);
            child.code.this_register = Some(register);
        }
        for statement in &function.body {
            match statement {
                Stmt::Declare(bindings) => {
                    for (pattern, mutable, _) in bindings {
                        let mut names = Vec::new();
                        pattern.names(&mut names);
                        for name in names {
                            child.declare(&name, *mutable)?;
                        }
                    }
                }
                Stmt::Function(name, _) => {
                    if function.name.as_ref() == Some(name) {
                        // A named FunctionExpression's immutable self binding
                        // is outside the call's parameter/var environment. A
                        // body FunctionDeclaration with the same name shadows
                        // it with a distinct mutable binding.
                        return None;
                    }
                    if !child.bindings.contains_key(name) {
                        child.declare(name, true)?;
                    }
                }
                _ => {}
            }
        }
        child.prepare_var_bindings(&function.body)?;
        child.infer_binding_type_hints(&function.body);
        // The map names consecutive slots, so the parameters take the first
        // ones the context has.
        if child.maps_arguments {
            child.capture_parameters(function)?;
        }
        // A name of the body takes its context slot before the body runs; one
        // a Block or a `for` head binds gets its own when the Block is
        // entered, so only the first kind is here to capture.
        for name in captured_names {
            if name != ARGUMENTS && child.bindings.contains_key(name) {
                child.capture_binding(name)?;
            }
        }
        Some((child, self_register))
    }

    fn infer_binding_type_hints(&mut self, body: &[Stmt]) {
        let mut bindings = self.bindings.clone();
        for statement in body {
            let Stmt::Declare(declarations) = statement else {
                continue;
            };
            for (pattern, _, initializer) in declarations {
                let Some(name) = pattern.identifier() else {
                    continue;
                };
                let value_type = if let Some(initializer) = initializer {
                    let Some(value_type) = register_expression_type(initializer, &bindings) else {
                        continue;
                    };
                    value_type
                } else {
                    RegisterType::Undefined
                };
                // A lexical declaration of a Realm Script binds on the
                // [[DeclarativeRecord]] and not here, so there is no binding
                // of this lowering to give a type to.
                let Some(binding) = bindings.get_mut(name) else {
                    continue;
                };
                binding.value_type = Some(value_type);
                self.binding_type_hints
                    .insert(String::from(name), value_type);
            }
        }
    }

    fn context_slot_counts(&self) -> Option<Vec<u16>> {
        let mut counts = Vec::new();
        if let Some(own) = self.code.own_context_slot_count {
            counts.push(own);
        }
        counts.extend(self.code.outer_context_slot_counts.iter().copied());
        (!counts.is_empty()).then_some(counts)
    }

    /// The Initializer of each parameter that has one (10.2.11, and
    /// `IteratorBindingInitialization` of 8.6.2).
    ///
    /// The argument is already in the register the parameter binds, so the
    /// Initializer runs only where the call passed undefined, which is what
    /// the clause says and what a call that passed too few arguments leaves.
    /// They run left to right, so a later one reads what an earlier one bound.
    fn initialize_parameter_defaults(&mut self, function: &Function) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        for (index, parameter) in function.parameters.iter().enumerate() {
            let named = matches!(parameter.pattern, parser::BindingPattern::Name(_));
            // A parameter that is a name and takes no Initializer holds the
            // argument as it arrived, wherever its binding lives.
            if named && parameter.default.is_none() {
                continue;
            }
            let name = match &parameter.pattern {
                parser::BindingPattern::Name(name) => name.clone(),
                _ => register_argument_name(index),
            };
            // An Initializer of a later parameter reads this one, which makes
            // it a captured name of 10.2.11, so the binding may live in the
            // own context rather than in a register.
            let binding = *self.bindings.get(&name)?;
            if let Some(default) = &parameter.default {
                self.load_binding(binding);
                let present = self.code.emit(Instruction::JumpIfNotUndefined(0));
                let value_type = self.lower(default)?;
                self.store_binding(binding);
                let after = self.code.instructions.len();
                self.patch_jump(present, after)?;
                // The parameter holds either the argument, whose type the
                // lowering cannot name, or the Initializer's answer.
                self.bindings.get_mut(&name)?.value_type =
                    Some(RegisterType::Unknown.merge(value_type));
            }
            // 8.6.2 then binds the names the pattern names, out of the
            // argument the binding holds.
            if !named {
                self.load_binding(binding);
                self.bind_pattern(RegisterType::Unknown, &parameter.pattern)?;
            }
        }
        Some(())
    }

    fn lower_function_body(
        child: &mut Self,
        function: &Function,
        body: &[Stmt],
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if let Some(register) = child.code.arguments_register {
            child.code.emit(Instruction::CreateArguments(register));
            // A nested function reads the object out of the context, which
            // only holds it once 10.4.4 has made it.
            if child.bindings.contains_key(ARGUMENTS) {
                child.capture_binding(ARGUMENTS)?;
                child.arguments_binding = child.bindings.get(ARGUMENTS).copied();
            }
        }
        // 10.2.11 step 28: the rest parameter binds the Array 8.6.3 makes of
        // the arguments beyond the parameters before it.
        if let Some(index) = function
            .parameters
            .iter()
            .position(|parameter| parameter.rest)
            && let parser::BindingPattern::Name(name) = &function.parameters.get(index)?.pattern
        {
            let skip = u16::try_from(index).ok()?;
            let binding = *child.bindings.get(name)?;
            match binding.storage {
                RegisterBindingStorage::Register(target) => {
                    child.code.emit(Instruction::CreateRest { target, skip });
                }
                RegisterBindingStorage::Context { .. } => {
                    let target = child.allocate_register()?;
                    child.code.emit(Instruction::CreateRest { target, skip });
                    child.code.emit(Instruction::Ldar(target));
                    child.store_binding(binding);
                    child.release_register(target)?;
                }
            }
            child.bindings.get_mut(name)?.value_type = Some(RegisterType::Unknown);
        }
        child.initialize_parameter_defaults(function)?;
        for statement in body {
            if let Stmt::Function(name, function) = statement {
                let value_type = child.lower_function_declaration(name, function)?;
                let binding = *child.bindings.get(name)?;
                child.store_binding(binding);
                child.bindings.get_mut(name)?.value_type = Some(value_type);
            }
        }
        let mut flow = RegisterFlow::Empty;
        for statement in body {
            flow = match statement {
                Stmt::Declare(bindings) => {
                    if register_lexical_dead_zone_read(bindings)? {
                        return None;
                    }
                    for (pattern, _, initializer) in bindings {
                        if let Some(initializer) = initializer {
                            child.initialize_pattern(pattern, initializer)?;
                        } else {
                            child.initialize(pattern.identifier()?, None)?;
                        }
                    }
                    RegisterFlow::Empty
                }
                Stmt::Var(bindings) => {
                    child.initialize_vars(bindings)?;
                    RegisterFlow::Empty
                }
                Stmt::Function(_, _) => RegisterFlow::Empty,
                _ => child.lower_statement(statement)?,
            };
            if flow == RegisterFlow::Abrupt {
                break;
            }
        }
        if flow != RegisterFlow::Abrupt {
            child.code.emit(Instruction::LdaUndefined);
            child.code.emit(Instruction::Return);
            child.return_type = Some(
                child
                    .return_type
                    .map_or(RegisterType::Undefined, |current| {
                        current.merge(RegisterType::Undefined)
                    }),
            );
        }
        Some(child.return_type.unwrap_or(RegisterType::Undefined))
    }

    /// Gives up the layouts of the values a call hands to user code.
    ///
    /// 10.2.11 binds the argument itself, so the callee reaches the Object and
    /// may change what it holds. From here the value keeps only the type of a
    /// value the lowering cannot name, and every access to it goes to 10.1.8.1
    /// at run time.
    fn escape(&mut self, escaped: &[RegisterType]) {
        let mut pending: Vec<RegisterType> = escaped.to_vec();
        let mut followed: BTreeSet<u32> = BTreeSet::new();
        while let Some(value_type) = pending.pop() {
            // A function leaves with everything its closure can reach: the
            // callee may call it, and the call writes the captured bindings.
            // Following one twice adds nothing, because the first pass left
            // every binding it captures with a type this lowering cannot name.
            // A function that captures itself would be followed forever, so
            // each one is followed once.
            if let RegisterType::Function(code_id) = value_type {
                if !followed.insert(code_id) {
                    continue;
                }
                let captured: Vec<String> = self
                    .function_capture_effects
                    .get(&code_id)
                    .map(|effects| effects.keys().cloned().collect())
                    .unwrap_or_default();
                pending.extend(
                    captured
                        .iter()
                        .filter_map(|name| self.bindings.get(name)?.value_type),
                );
                continue;
            }
            let Some(id) = value_type.object_id() else {
                continue;
            };
            // Everything the Object holds is reachable through it, so it
            // leaves with it.
            let Some(layout) = self.object_layouts.remove(&id) else {
                continue;
            };
            match &layout {
                RegisterObjectLayout::Ordinary {
                    properties,
                    dynamic,
                    ..
                } => pending.extend(properties.values().copied().chain(*dynamic)),
                RegisterObjectLayout::Array {
                    elements, dynamic, ..
                } => pending.extend(elements.values().copied().chain(*dynamic)),
            }
            for binding in self.bindings.values_mut() {
                if binding.value_type == Some(value_type) {
                    binding.value_type = Some(RegisterType::Unknown);
                }
            }
            for hint in self.binding_type_hints.values_mut() {
                if *hint == value_type {
                    *hint = RegisterType::Unknown;
                }
            }
            for layout in self.object_layouts.values_mut() {
                let (held, dynamic) = match layout {
                    RegisterObjectLayout::Ordinary {
                        properties,
                        dynamic,
                        ..
                    } => (properties.values_mut().collect::<Vec<_>>(), dynamic),
                    RegisterObjectLayout::Array {
                        elements, dynamic, ..
                    } => (elements.values_mut().collect::<Vec<_>>(), dynamic),
                };
                for entry in held {
                    if *entry == value_type {
                        *entry = RegisterType::Unknown;
                    }
                }
                if *dynamic == Some(value_type) {
                    *dynamic = Some(RegisterType::Unknown);
                }
            }
        }
    }

    fn lower_call(&mut self, callee: &Expr, arguments: &[Expr]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        // 13.3.7.1 constructs the Prototype of the running function and binds
        // the answer as the `this` of the derived constructor.
        if matches!(callee.kind, ExprKind::Super) {
            return self.lower_super_construct(arguments);
        }
        if Self::holds_a_spread(arguments) {
            return self.lower_spread_call(callee, arguments);
        }
        if let ExprKind::Member(base, key) = &callee.kind {
            if matches!(base.kind, ExprKind::Super) {
                return self.lower_super_call(key, arguments);
            }
            return self.lower_method_call(base, key, arguments);
        }
        let callee_type = self.lower(callee)?;
        let RegisterType::Function(code_id) = callee_type else {
            // 7.3.14 dispatches on the callee at run time, which the call
            // instruction does for a value the lowering could not name.
            if callee_type != RegisterType::Unknown {
                return None;
            }
            // 13.3.6.1: a call of the name `eval` is a direct eval, which
            // shares the variable environment of the function it stands in.
            return self.lower_dynamic_call(arguments, callee.reference_name() == Some(EVAL_NAME));
        };
        if self
            .function_capture_effects
            .get(&code_id)
            .is_some_and(|effects| {
                effects.keys().any(|name| {
                    self.bindings
                        .get(name)
                        .is_some_and(|binding| binding.value_type.is_none())
                })
            })
        {
            return None;
        }
        let function = self.allocate_register()?;
        self.code.emit(Instruction::Star(function));
        let parameter_types = self.function_parameters.get(&code_id)?.clone();
        let mut argument_registers = Vec::new();
        let mut argument_types = Vec::new();
        for (index, argument) in arguments.iter().enumerate() {
            let argument_type = self.lower(argument)?;
            if parameter_types
                .get(index)
                .is_some_and(|parameter| !parameter.accepts(argument_type))
            {
                return None;
            }
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
            argument_types.push(argument_type);
        }
        let dummy = if argument_registers.is_empty() {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(register));
            Some(register)
        } else {
            None
        };
        let argument_start = argument_registers.first().copied().or(dummy)?;
        let argument_count = u16::try_from(arguments.len()).ok()?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::Call {
            func: function,
            arg_start: argument_start,
            arg_count: argument_count,
            slot,
        });
        if let Some(dummy) = dummy {
            self.release_register(dummy)?;
        }
        for register in argument_registers.into_iter().rev() {
            self.release_register(register)?;
        }
        self.release_register(function)?;
        if let Some(effects) = self.function_capture_effects.get(&code_id).cloned() {
            for (name, effect) in effects {
                if let Some(binding) = self.bindings.get_mut(&name) {
                    binding.value_type = Some(binding.value_type?.merge(effect));
                }
            }
        }
        if let Some(effects) = self.function_layout_effects.get(&code_id).cloned() {
            self.object_layouts.extend(effects);
        }
        self.escape(&argument_types);
        self.function_returns.get(&code_id).copied()
    }

    /// Lowers `new` (13.3.5.1), whose callee must be a function of this unit
    /// that 10.2.5 made a constructor.
    ///
    /// The object 10.1.13 creates is kept in a register of this frame, where
    /// the collector sees it while the constructor runs.
    fn lower_construct(&mut self, callee: &Expr, arguments: &[Expr]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if Self::holds_a_spread(arguments) {
            return self.lower_spread_construct(callee, arguments);
        }
        let callee_type = self.lower(callee)?;
        // In a Realm a function declaration is a binding of the Global
        // Environment Record, so its name reads as a type the lowering cannot
        // give. 7.3.15 resolves the constructor at run time either way.
        if callee_type == RegisterType::Unknown {
            return self.lower_dynamic_construct(arguments);
        }
        let RegisterType::Function(code_id) = callee_type else {
            return None;
        };
        if !self.constructible.contains(&code_id) {
            return None;
        }
        if self
            .function_capture_effects
            .get(&code_id)
            .is_some_and(|effects| {
                effects.keys().any(|name| {
                    self.bindings
                        .get(name)
                        .is_some_and(|binding| binding.value_type.is_none())
                })
            })
        {
            return None;
        }
        let function = self.allocate_register()?;
        self.code.emit(Instruction::Star(function));
        let target = self.allocate_register()?;
        let parameter_types = self.function_parameters.get(&code_id)?.clone();
        let mut argument_registers = Vec::new();
        let mut argument_types = Vec::new();
        for (index, argument) in arguments.iter().enumerate() {
            let argument_type = self.lower(argument)?;
            if parameter_types
                .get(index)
                .is_some_and(|parameter| !parameter.accepts(argument_type))
            {
                return None;
            }
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
            argument_types.push(argument_type);
        }
        let dummy = if argument_registers.is_empty() {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(register));
            Some(register)
        } else {
            None
        };
        let arg_start = argument_registers.first().copied().or(dummy)?;
        let arg_count = u16::try_from(arguments.len()).ok()?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::Construct {
            func: function,
            target,
            arg_start,
            arg_count,
            slot,
        });
        if let Some(dummy) = dummy {
            self.release_register(dummy)?;
        }
        for register in argument_registers.into_iter().rev() {
            self.release_register(register)?;
        }
        self.release_register(target)?;
        self.release_register(function)?;
        if let Some(effects) = self.function_capture_effects.get(&code_id).cloned() {
            for (name, effect) in effects {
                if let Some(binding) = self.bindings.get_mut(&name) {
                    binding.value_type = Some(binding.value_type?.merge(effect));
                }
            }
        }
        if let Some(effects) = self.function_layout_effects.get(&code_id).cloned() {
            self.object_layouts.extend(effects);
        }
        self.escape(&argument_types);
        // The result is the created object or whatever the constructor answered
        // instead; the lowering can name neither.
        Some(RegisterType::Unknown)
    }

    /// Lowers a call whose callee is a property of an object, evaluating the
    /// base once and passing it as the `this` value (13.3.6.1).
    /// Lowers a call whose callee the lowering could not name.
    ///
    /// The callee is already in the accumulator. 7.3.14 refuses a value that
    /// is not callable at run time, which the call instruction does.
    fn lower_dynamic_call(
        &mut self,
        arguments: &[Expr],
        direct_eval: bool,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let function = self.allocate_register()?;
        self.code.emit(Instruction::Star(function));
        let mut argument_registers = Vec::new();
        let mut argument_types = Vec::new();
        for argument in arguments {
            let argument_type = self.lower(argument)?;
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
            argument_types.push(argument_type);
        }
        let dummy = if argument_registers.is_empty() {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(register));
            Some(register)
        } else {
            None
        };
        let arg_start = argument_registers.first().copied().or(dummy)?;
        let arg_count = u16::try_from(arguments.len()).ok()?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(if direct_eval {
            Instruction::CallDirectEval {
                func: function,
                arg_start,
                arg_count,
                slot,
            }
        } else {
            Instruction::Call {
                func: function,
                arg_start,
                arg_count,
                slot,
            }
        });
        if let Some(dummy) = dummy {
            self.release_register(dummy)?;
        }
        for register in argument_registers.into_iter().rev() {
            self.release_register(register)?;
        }
        self.release_register(function)?;
        self.escape(&argument_types);
        Some(RegisterType::Unknown)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one function emits 13.3.6.1 for every shape of a callee"
    )]
    fn lower_method_call(
        &mut self,
        base: &Expr,
        key: &Expr,
        arguments: &[Expr],
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let base_type = self.lower(base)?;
        // 13.3.6.1 sends a primitive base through 7.1.18, whose Prototype
        // carries the method. The engine does that at run time, so the
        // lowering only has to name which Prototype it is.
        let primitive_holder = match base_type {
            RegisterType::String => Some(crate::engine::realm::IntrinsicHolder::StringPrototype),
            RegisterType::Number => Some(crate::engine::realm::IntrinsicHolder::NumberPrototype),
            RegisterType::Boolean => Some(crate::engine::realm::IntrinsicHolder::BooleanPrototype),
            _ => None,
        };
        if !base_type.is_object()
            && primitive_holder.is_none()
            && !matches!(
                base_type,
                RegisterType::Unknown | RegisterType::NativeFunction(_)
            )
        {
            return None;
        }
        let receiver = self.allocate_register()?;
        self.code.emit(Instruction::Star(receiver));
        // A base the lowering could not name carries no layout, so the callee
        // is read at run time and 7.3.14 dispatches on whatever it is. A
        // function of the Script and an intrinsic both carry the methods of
        // 20.2.3, which are read the same way.
        if matches!(
            base_type,
            RegisterType::Unknown | RegisterType::Function(_) | RegisterType::NativeFunction(_)
        ) {
            self.code.emit(Instruction::Ldar(receiver));
            self.lower_unknown_member(key)?;
            let result = self.lower_dynamic_method_call(receiver, arguments)?;
            self.release_register(receiver)?;
            return Some(result);
        }
        let intrinsic = if let Some(holder) = primitive_holder {
            // 22.1.3, 21.1.3 and 20.3.3: the method is resolved on the
            // Prototype of the primitive.
            let name = Self::static_property_name(key)
                .map(<[u16]>::to_vec)
                .or_else(|| self.static_key_units(key))?;
            let intrinsic = crate::engine::realm::holder_intrinsic(holder, &name)?;
            let constant = self.string_constant(&name)?;
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: receiver,
                name: constant,
                slot,
            });
            intrinsic
        } else {
            let keyed = matches!(base_type, RegisterType::Object(_))
                && Self::static_property_name(key).is_none();
            let callee_type =
                self.lower_property_from_register(receiver, base_type, key, keyed, true)?;
            match callee_type {
                RegisterType::NativeFunction(intrinsic) => intrinsic,
                // 13.3.6.1 passes the base as the `this` value of the call,
                // which is what a bytecode callee reading `this` needs.
                RegisterType::Function(code_id) => {
                    let result = self.lower_method_call_bytecode(receiver, code_id, arguments)?;
                    self.release_register(receiver)?;
                    self.escape(&[base_type]);
                    return Some(result);
                }
                // A property the layout answers with a type it cannot name is
                // a callee 7.3.14 dispatches on at run time.
                RegisterType::Unknown => {
                    let result = self.lower_dynamic_method_call(receiver, arguments)?;
                    self.release_register(receiver)?;
                    self.escape(&[base_type]);
                    return Some(result);
                }
                _ => return None,
            }
        };
        let function = self.allocate_register()?;
        self.code.emit(Instruction::Star(function));
        // An intrinsic reads its arguments as values, so any lowered
        // expression may be one; only a bytecode callee needs typed parameters.
        let mut argument_registers = Vec::new();
        let mut argument_types = Vec::new();
        for (index, argument) in arguments.iter().enumerate() {
            let argument_type = self.lower(argument)?;
            // An Object in a position the native converts is taken as it is:
            // the native leaves to run 7.1.1 and comes back.
            let index = u16::try_from(index).ok()?;
            if !argument_type.converts_to_primitive()
                && intrinsic.coerces_argument(index)
                && !intrinsic.converts_argument(index)
            {
                return None;
            }
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
            argument_types.push(argument_type);
        }
        let dummy = if argument_registers.is_empty() {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(register));
            Some(register)
        } else {
            None
        };
        let arg_start = argument_registers.first().copied().or(dummy)?;
        let arg_count = u16::try_from(arguments.len()).ok()?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver,
            func: function,
            arg_start,
            arg_count,
            slot,
        });
        if let Some(dummy) = dummy {
            self.release_register(dummy)?;
        }
        for register in argument_registers.into_iter().rev() {
            self.release_register(register)?;
        }
        self.release_register(function)?;
        self.release_register(receiver)?;
        let result = self.intrinsic_call_result(intrinsic, base_type, &argument_types);
        self.escape(&argument_types);
        result
    }

    /// Lowers `new` whose constructor only the run time knows.
    ///
    /// The constructor is in the accumulator. 7.3.15 refuses a value without a
    /// `[[Construct]]` there, which the instruction does. Every argument must
    /// be a primitive: the lowering cannot say which function answers, and it
    /// compiled every candidate under the assumption that its parameters are.
    fn lower_dynamic_construct(&mut self, arguments: &[Expr]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let function = self.allocate_register()?;
        self.code.emit(Instruction::Star(function));
        let target = self.allocate_register()?;
        let mut argument_registers = Vec::new();
        let mut argument_types = Vec::new();
        for argument in arguments {
            let argument_type = self.lower(argument)?;
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
            argument_types.push(argument_type);
        }
        let dummy = if argument_registers.is_empty() {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(register));
            Some(register)
        } else {
            None
        };
        let arg_start = argument_registers.first().copied().or(dummy)?;
        let arg_count = u16::try_from(arguments.len()).ok()?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::Construct {
            func: function,
            target,
            arg_start,
            arg_count,
            slot,
        });
        if let Some(dummy) = dummy {
            self.release_register(dummy)?;
        }
        for register in argument_registers.into_iter().rev() {
            self.release_register(register)?;
        }
        self.release_register(target)?;
        self.release_register(function)?;
        self.escape(&argument_types);
        Some(RegisterType::Unknown)
    }

    /// Lowers a method call whose callee only the run time knows.
    ///
    /// The callee is in the accumulator and `receiver` holds the base, which
    /// 13.3.6.1 passes as the `this` value. Every argument must be a primitive:
    /// the lowering cannot say which function answers, and it compiled every
    /// candidate under the assumption that its parameters are primitives.
    /// `SuperCall` of 13.3.7.1, whose answer 13.3.7.1 step 8 binds as `this`.
    fn lower_super_construct(&mut self, arguments: &[Expr]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let this_register = self.code.this_register?;
        let mut argument_registers = Vec::new();
        let mut argument_types = Vec::new();
        for argument in arguments {
            let argument_type = self.lower(argument)?;
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
            argument_types.push(argument_type);
        }
        let dummy = if argument_registers.is_empty() {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(register));
            Some(register)
        } else {
            None
        };
        let arg_start = argument_registers.first().copied().or(dummy)?;
        let arg_count = u16::try_from(arguments.len()).ok()?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::SuperCall {
            arg_start,
            arg_count,
            forwarded: false,
            slot,
        });
        self.code.emit(Instruction::Star(this_register));
        if let Some(dummy) = dummy {
            self.release_register(dummy)?;
        }
        for register in argument_registers.into_iter().rev() {
            self.release_register(register)?;
        }
        self.escape(&argument_types);
        Some(RegisterType::Unknown)
    }

    /// `MakeSuperPropertyReference` of 13.3.7.3, in a register of its own.
    fn super_base(&mut self) -> Option<crate::engine::bytecode::Reg> {
        use crate::engine::bytecode::Instruction;
        let register = self.allocate_register()?;
        self.code.emit(Instruction::SuperBase { target: register });
        Some(register)
    }

    /// `super.name` and `super[key]` of 13.3.7, read through `this`.
    fn lower_super_property(
        &mut self,
        base: crate::engine::bytecode::Reg,
        key: &Expr,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if let Some(name) = Self::static_property_name(key) {
            let name = self.string_constant(name)?;
            self.code.emit(Instruction::GetSuper { base, name });
            return Some(RegisterType::Unknown);
        }
        if !self.lower(key)?.converts_to_primitive() {
            return None;
        }
        let register = self.allocate_register()?;
        self.code.emit(Instruction::Star(register));
        self.code.emit(Instruction::GetSuperByValue {
            base,
            key: register,
        });
        self.release_register(register)?;
        Some(RegisterType::Unknown)
    }

    /// `super.name(...)` of 13.3.7, whose receiver 13.3.6.1 makes the `this`
    /// value of the call and not the base the method was found on.
    fn lower_super_call(&mut self, key: &Expr, arguments: &[Expr]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let base = self.super_base()?;
        self.lower_super_property(base, key)?;
        self.release_register(base)?;
        let receiver = self.allocate_register()?;
        let this = self.bindings.get(THIS_BINDING).copied()?;
        let function = self.allocate_register()?;
        self.code.emit(Instruction::Star(function));
        self.load_binding(this);
        self.code.emit(Instruction::Star(receiver));
        let result = self.lower_dynamic_method_call_with(receiver, function, arguments)?;
        self.release_register(function)?;
        self.release_register(receiver)?;
        Some(result)
    }

    fn lower_dynamic_method_call(
        &mut self,
        receiver: crate::engine::bytecode::Reg,
        arguments: &[Expr],
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let function = self.allocate_register()?;
        self.code.emit(Instruction::Star(function));
        let result = self.lower_dynamic_method_call_with(receiver, function, arguments);
        self.release_register(function)?;
        result
    }

    /// The same call with the callee already in a register of its own.
    fn lower_dynamic_method_call_with(
        &mut self,
        receiver: crate::engine::bytecode::Reg,
        function: crate::engine::bytecode::Reg,
        arguments: &[Expr],
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let mut argument_registers = Vec::new();
        let mut argument_types = Vec::new();
        for argument in arguments {
            let argument_type = self.lower(argument)?;
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
            argument_types.push(argument_type);
        }
        let dummy = if argument_registers.is_empty() {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(register));
            Some(register)
        } else {
            None
        };
        let arg_start = argument_registers.first().copied().or(dummy)?;
        let arg_count = u16::try_from(arguments.len()).ok()?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver,
            func: function,
            arg_start,
            arg_count,
            slot,
        });
        if let Some(dummy) = dummy {
            self.release_register(dummy)?;
        }
        for register in argument_registers.into_iter().rev() {
            self.release_register(register)?;
        }
        self.escape(&argument_types);
        Some(RegisterType::Unknown)
    }

    /// Lowers a method call whose callee is a function of this unit.
    ///
    /// The callee is in the accumulator and `receiver` holds the base, which
    /// 13.3.6.1 passes as the `this` value of the call.
    fn lower_method_call_bytecode(
        &mut self,
        receiver: crate::engine::bytecode::Reg,
        code_id: u32,
        arguments: &[Expr],
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if self
            .function_capture_effects
            .get(&code_id)
            .is_some_and(|effects| {
                effects.keys().any(|name| {
                    self.bindings
                        .get(name)
                        .is_some_and(|binding| binding.value_type.is_none())
                })
            })
        {
            return None;
        }
        let function = self.allocate_register()?;
        self.code.emit(Instruction::Star(function));
        let parameter_types = self.function_parameters.get(&code_id)?.clone();
        let mut argument_registers = Vec::new();
        let mut argument_types = Vec::new();
        for (index, argument) in arguments.iter().enumerate() {
            let argument_type = self.lower(argument)?;
            if parameter_types
                .get(index)
                .is_some_and(|parameter| !parameter.accepts(argument_type))
            {
                return None;
            }
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
            argument_types.push(argument_type);
        }
        let dummy = if argument_registers.is_empty() {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::LdaUndefined);
            self.code.emit(Instruction::Star(register));
            Some(register)
        } else {
            None
        };
        let arg_start = argument_registers.first().copied().or(dummy)?;
        let arg_count = u16::try_from(arguments.len()).ok()?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver,
            func: function,
            arg_start,
            arg_count,
            slot,
        });
        if let Some(dummy) = dummy {
            self.release_register(dummy)?;
        }
        for register in argument_registers.into_iter().rev() {
            self.release_register(register)?;
        }
        self.release_register(function)?;
        if let Some(effects) = self.function_capture_effects.get(&code_id).cloned() {
            for (name, effect) in effects {
                if let Some(binding) = self.bindings.get_mut(&name) {
                    binding.value_type = Some(binding.value_type?.merge(effect));
                }
            }
        }
        if let Some(effects) = self.function_layout_effects.get(&code_id).cloned() {
            self.object_layouts.extend(effects);
        }
        self.escape(&argument_types);
        self.function_returns.get(&code_id).copied()
    }

    /// The type an intrinsic call answers, once the arguments have been
    /// lowered, and the change it leaves on the receiver's layout.
    ///
    /// Most intrinsics answer a type the intrinsic alone fixes. The ones that
    /// read or move elements answer a type only the receiver's layout carries,
    /// and 23.1.3.22 and 23.1.3.23 change that layout.
    fn intrinsic_call_result(
        &mut self,
        intrinsic: crate::engine::realm::Intrinsic,
        base_type: RegisterType,
        arguments: &[RegisterType],
    ) -> Option<RegisterType> {
        use crate::engine::realm::Intrinsic;
        let property_limit = self.property_limit;
        match intrinsic {
            // 23.1.3.1 answers an element of the receiver.
            Intrinsic::ArrayPrototypeAt => self.array_element_type(base_type),
            // 23.1.3.23 writes each argument at the length reached so far, so
            // only a layout that knows that length knows where they land.
            Intrinsic::ArrayPrototypePush => {
                let (object_id, ..) = self.array_layout(base_type)?;
                let RegisterObjectLayout::Array {
                    length, elements, ..
                } = self.object_layouts.get_mut(&object_id)?
                else {
                    return None;
                };
                if elements.len().saturating_add(arguments.len()) > property_limit {
                    return None;
                }
                let mut next = (*length)?;
                for argument in arguments {
                    elements.insert(next, *argument);
                    next = next.checked_add(1)?;
                }
                *length = Some(next);
                Some(intrinsic_result_type(intrinsic))
            }
            // 23.1.3.28 answers a new Array, whose length the arguments fix
            // only at run time and whose elements come from the receiver.
            Intrinsic::ArrayPrototypeSlice => {
                let element = self.array_element_type(base_type)?;
                let object_id = self.next_object_id;
                self.next_object_id = self.next_object_id.checked_add(1)?;
                self.object_layouts.insert(
                    object_id,
                    RegisterObjectLayout::Array {
                        length: None,
                        elements: BTreeMap::new(),
                        dynamic: Some(element),
                    },
                );
                Some(RegisterType::Array(object_id))
            }
            // 23.1.3.26 answers the receiver with its indices mirrored.
            Intrinsic::ArrayPrototypeReverse => {
                let (object_id, ..) = self.array_layout(base_type)?;
                let RegisterObjectLayout::Array {
                    length, elements, ..
                } = self.object_layouts.get_mut(&object_id)?
                else {
                    return None;
                };
                let Some(last) = (*length)?.checked_sub(1) else {
                    return Some(base_type);
                };
                *elements = core::mem::take(elements)
                    .into_iter()
                    .map(|(index, value_type)| Some((last.checked_sub(index)?, value_type)))
                    .collect::<Option<_>>()?;
                Some(base_type)
            }
            // 23.1.3.22 takes the last element away, which leaves the layout
            // one element shorter.
            Intrinsic::ArrayPrototypePop => {
                let (object_id, ..) = self.array_layout(base_type)?;
                let RegisterObjectLayout::Array {
                    length, elements, ..
                } = self.object_layouts.get_mut(&object_id)?
                else {
                    return None;
                };
                let Some(last) = (*length)?.checked_sub(1) else {
                    return Some(RegisterType::Undefined);
                };
                let element = elements.remove(&last).unwrap_or(RegisterType::Undefined);
                *length = Some(last);
                Some(element)
            }
            // 23.1.3.18 applies `ToString` to every element, which for an
            // Object is 7.1.1 with the hint `string`: the engine runs the
            // methods of the Realm for it and names the gap for one of the
            // Script.
            _ => Some(intrinsic_result_type(intrinsic)),
        }
    }

    fn lower_member(&mut self, base: &Expr, key: &Expr) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if matches!(base.kind, ExprKind::Super) {
            let register = self.super_base()?;
            let result = self.lower_super_property(register, key)?;
            self.release_register(register)?;
            return Some(result);
        }
        self.reading_member_base = true;
        let base_type = self.lower(base)?;
        if base_type == RegisterType::String {
            return self.lower_string_member(key);
        }
        // A function object carries no layout this lowering tracks, so its
        // properties are read the way a base it could not name is read. That
        // holds for an intrinsic as much as for a function of the Script:
        // `[].slice.call` reads `call` off `%Array.prototype%.slice`.
        if matches!(
            base_type,
            RegisterType::Unknown | RegisterType::Function(_) | RegisterType::NativeFunction(_)
        ) {
            return self.lower_unknown_member(key);
        }
        if !base_type.is_object() {
            return None;
        }
        let object = self.allocate_register()?;
        self.code.emit(Instruction::Star(object));
        let keyed = matches!(base_type, RegisterType::Object(_))
            && Self::static_property_name(key).is_none();
        let result = self.lower_property_from_register(object, base_type, key, keyed, true)?;
        self.release_register(object)?;
        Some(result)
    }

    /// Lowers `delete` (13.5.1.2).
    ///
    /// A Reference to a property goes through `[[Delete]]` where it happens.
    /// An operand that makes no Reference is evaluated for its effect and
    /// answers true, and a name a declaration bound answers false, because
    /// 9.1.1.1 makes no binding of one configurable. A free name is a
    /// property of the global object, which this lowering does not reach.
    fn lower_delete(&mut self, inner: &Expr, strict: bool) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if let Some((base, key)) = inner.member() {
            self.reading_member_base = true;
            let base_type = self.lower(base)?;
            // The object loses the layout this lowering tracked: what a
            // `[[Delete]]` took off it is known where it runs, not here.
            self.escape(&[base_type]);
            let object = self.allocate_register()?;
            self.code.emit(Instruction::Star(object));
            let static_name = Self::static_property_name(key)
                .map(<[u16]>::to_vec)
                .or_else(|| self.static_key_units(key));
            if let Some(name) = static_name.as_deref() {
                let name = self.string_constant(name)?;
                self.code.emit(Instruction::DeleteNamed {
                    obj: object,
                    name,
                    strict,
                });
            } else {
                // 7.1.19 step 2 sends an Object key through 7.1.1, which
                // the engine runs where the access stands.
                self.lower(key)?;
                let key = self.allocate_register()?;
                self.code.emit(Instruction::Star(key));
                self.code.emit(Instruction::DeleteByValue {
                    obj: object,
                    key,
                    strict,
                });
                self.release_register(key)?;
            }
            self.release_register(object)?;
            return Some(RegisterType::Boolean);
        }
        if let Some(name) = inner.reference_name() {
            if !self.bindings.contains_key(name) {
                return None;
            }
            self.code.emit(Instruction::LdaFalse);
            return Some(RegisterType::Boolean);
        }
        self.lower(inner)?;
        self.code.emit(Instruction::LdaTrue);
        Some(RegisterType::Boolean)
    }

    /// Lowers a property read whose base the lowering could not name.
    ///
    /// 10.1.8.1 walks the Prototype Chain at run time, which the instruction
    /// does: a base that is not an `Object` is a `TypeError` there, and a name
    /// no object of the chain has is `undefined` or, where the chain reaches a
    /// Prototype this Realm has not built, a gap.
    fn lower_unknown_member(&mut self, key: &Expr) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let object = self.allocate_register()?;
        self.code.emit(Instruction::Star(object));
        let static_name = Self::static_property_name(key)
            .map(<[u16]>::to_vec)
            .or_else(|| self.static_key_units(key));
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        if let Some(name) = static_name.as_deref() {
            let name = self.string_constant(name)?;
            self.code.emit(Instruction::GetNamed {
                obj: object,
                name,
                slot,
            });
        } else {
            // 7.1.19 step 2 sends an Object key through 7.1.1, which the
            // engine runs where the access stands.
            self.lower(key)?;
            let key = self.allocate_register()?;
            self.code.emit(Instruction::Star(key));
            self.code.emit(Instruction::GetByValue {
                obj: object,
                key,
                slot,
            });
            self.release_register(key)?;
        }
        self.release_register(object)?;
        Some(RegisterType::Unknown)
    }

    /// Lowers a property read whose base is a String.
    ///
    /// 10.4.3 gives the String exotic object `ToObject` produces an own
    /// `"length"` and an own property per code unit; every other name is
    /// resolved on %String.prototype%, which the instruction walks. A name
    /// that Prototype owns answers whatever it carries, so the type is not
    /// known here.
    fn lower_string_member(&mut self, key: &Expr) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let object = self.allocate_register()?;
        self.code.emit(Instruction::Star(object));
        let static_name = Self::static_property_name(key)
            .map(<[u16]>::to_vec)
            .or_else(|| self.static_key_units(key));
        let result_type = match static_name.as_deref() {
            Some(name) if Self::is_length(name) => RegisterType::Number,
            Some(name) if crate::engine::realm::string_prototype_owns(name) => {
                RegisterType::Unknown
            }
            // A run-time key can name a method of %String.prototype%.
            None if self.key_reaches_prototype(key) => RegisterType::Unknown,
            // A name that is neither "length" nor an index is undefined, and an
            // index is a one-unit String or undefined past the end.
            Some(_) | None => RegisterType::Primitive,
        };
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        if let Some(name) = static_name.as_deref() {
            let name = self.string_constant(name)?;
            self.code.emit(Instruction::GetNamed {
                obj: object,
                name,
                slot,
            });
        } else {
            // 7.1.19 step 2 sends an Object key through 7.1.1, which the
            // engine runs where the access stands.
            self.lower(key)?;
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            self.code.emit(Instruction::GetByValue {
                obj: object,
                key: register,
                slot,
            });
            self.release_register(register)?;
        }
        self.release_register(object)?;
        Some(result_type)
    }

    fn lower_property_from_register(
        &mut self,
        object: crate::engine::bytecode::Reg,
        base_type: RegisterType,
        key: &Expr,
        keyed: bool,
        missing_is_undefined: bool,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if !keyed && matches!(base_type, RegisterType::Array(_)) && Self::is_length_name(key) {
            self.code.emit(Instruction::GetArrayLength { obj: object });
            return Some(RegisterType::Number);
        }
        if matches!(base_type, RegisterType::Array(_)) {
            self.lower_array_property_from_register(object, base_type, key, keyed)
        } else {
            self.lower_ordinary_property_from_register(
                object,
                base_type,
                key,
                keyed,
                missing_is_undefined,
            )
        }
    }

    fn lower_array_property_from_register(
        &mut self,
        object: crate::engine::bytecode::Reg,
        base_type: RegisterType,
        key: &Expr,
        keyed: bool,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let RegisterType::Array(object_id) = base_type else {
            return None;
        };
        let RegisterObjectLayout::Array {
            elements, dynamic, ..
        } = self.object_layouts.get(&object_id)?
        else {
            return None;
        };
        let static_index = Self::static_array_index(key);
        let static_name = if keyed {
            self.static_key_units(key)
        } else {
            Self::static_property_name(key).map(<[u16]>::to_vec)
        };
        // A name that is neither "length" nor an index is resolved on the
        // Prototype Chain, and a key known only at run time can name one.
        let result_type = if static_name.as_deref().is_some_and(Self::is_length) {
            RegisterType::Number
        } else if let Some(index) = static_index {
            let static_type = elements
                .get(&index)
                .copied()
                .unwrap_or(RegisterType::Undefined);
            dynamic.map_or(static_type, |dynamic| static_type.merge(dynamic))
        } else if let Some(name) = static_name.as_deref() {
            if crate::engine::realm::array_prototype_owns(name) {
                // The name is resolved on %Array.prototype%, so it is the
                // intrinsic when one is implemented and unsupported otherwise.
                // An own property cannot shadow it: a key this lowering writes
                // to an Array is a Number, whose ToPropertyKey never spells a
                // method name.
                let intrinsic = crate::engine::realm::array_prototype_intrinsic(name)?;
                let slot =
                    self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
                let name = self.string_constant(name)?;
                self.code.emit(Instruction::GetNamed {
                    obj: object,
                    name,
                    slot,
                });
                return Some(RegisterType::NativeFunction(intrinsic));
            }
            elements
                .values()
                .copied()
                .chain(dynamic.iter().copied())
                .reduce(RegisterType::merge)
                .unwrap_or(RegisterType::Undefined)
                .merge(RegisterType::Number)
                .merge(RegisterType::Undefined)
        } else {
            elements
                .values()
                .copied()
                .chain(dynamic.iter().copied())
                .reduce(RegisterType::merge)
                .unwrap_or(RegisterType::Undefined)
                .merge(RegisterType::Number)
                .merge(RegisterType::Undefined)
                .merge(if self.key_reaches_prototype(key) {
                    RegisterType::Unknown
                } else {
                    RegisterType::Undefined
                })
        };
        if keyed {
            // 7.1.19 step 2 sends an Object key through 7.1.1, which the
            // engine runs where the access stands.
            self.lower(key)?;
        } else if let Some(index) = static_index {
            self.emit_array_index(index)?;
        } else if !self.lower(key)?.converts_to_primitive() {
            return None;
        }
        let key = self.allocate_register()?;
        self.code.emit(Instruction::Star(key));
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetByValue {
            obj: object,
            key,
            slot,
        });
        self.release_register(key)?;
        Some(result_type)
    }

    /// The Array layout `base_type` names: its identifier, its length, the
    /// types of its indexed elements, and the type an index it does not hold
    /// yields.
    fn array_layout(&self, base_type: RegisterType) -> Option<RegisterArrayLayout<'_>> {
        let RegisterType::Array(object_id) = base_type else {
            return None;
        };
        let RegisterObjectLayout::Array {
            length,
            elements,
            dynamic,
        } = self.object_layouts.get(&object_id)?
        else {
            return None;
        };
        Some((object_id, *length, elements, *dynamic))
    }

    /// The type one element of the Array `base_type` names has during the
    /// iteration of 14.7.5.
    ///
    /// Unlike a read by index, an iteration only reaches the indices below the
    /// length, so undefined joins the type only for a layout that leaves one
    /// of them open.
    fn array_iteration_type(&self, base_type: RegisterType) -> Option<RegisterType> {
        let (_, length, elements, dynamic) = self.array_layout(base_type)?;
        let complete = length.is_some_and(|length| usize::try_from(length) == Ok(elements.len()))
            && dynamic.is_none();
        let element = elements
            .values()
            .copied()
            .chain(dynamic.iter().copied())
            .reduce(RegisterType::merge)
            .unwrap_or(RegisterType::Undefined);
        Some(if complete {
            element
        } else {
            element.merge(RegisterType::Undefined)
        })
    }

    /// The type an indexed read of the Array `base_type` names answers, which
    /// is undefined for an index the layout does not hold.
    fn array_element_type(&self, base_type: RegisterType) -> Option<RegisterType> {
        let (_, _, elements, dynamic) = self.array_layout(base_type)?;
        Some(
            elements
                .values()
                .copied()
                .chain(dynamic.iter().copied())
                .reduce(RegisterType::merge)
                .unwrap_or(RegisterType::Undefined)
                .merge(RegisterType::Undefined),
        )
    }

    fn lower_array_index_from_register(
        &mut self,
        object: crate::engine::bytecode::Reg,
        base_type: RegisterType,
        index: u32,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let (_, _, elements, dynamic) = self.array_layout(base_type)?;
        let static_type = elements
            .get(&index)
            .copied()
            .unwrap_or(RegisterType::Undefined);
        let result_type = dynamic.map_or(static_type, |dynamic| static_type.merge(dynamic));
        self.emit_array_index(index)?;
        let key = self.allocate_register()?;
        self.code.emit(Instruction::Star(key));
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetByValue {
            obj: object,
            key,
            slot,
        });
        self.release_register(key)?;
        Some(result_type)
    }

    fn lower_array_rest_from_register(
        &mut self,
        source: crate::engine::bytecode::Reg,
        source_type: RegisterType,
        start: u32,
    ) -> Option<(RegisterType, crate::engine::bytecode::Reg)> {
        use crate::engine::bytecode::Instruction;
        let RegisterType::Array(source_id) = source_type else {
            return None;
        };
        let RegisterObjectLayout::Array {
            length: Some(length),
            ..
        } = self.object_layouts.get(&source_id)?
        else {
            return None;
        };
        let length = *length;
        let rest_length = length.saturating_sub(start);
        if usize::try_from(rest_length).ok()?.saturating_add(1) > self.property_limit {
            return None;
        }
        let rest_id = self.next_object_id;
        self.next_object_id = self.next_object_id.checked_add(1)?;
        self.object_layouts.insert(
            rest_id,
            RegisterObjectLayout::Array {
                length: Some(rest_length),
                elements: BTreeMap::new(),
                dynamic: None,
            },
        );
        self.code.emit(Instruction::CreateArray(rest_length));
        let rest_array = self.allocate_register()?;
        self.code.emit(Instruction::Star(rest_array));
        for offset in 0..rest_length {
            let source_index = start.checked_add(offset)?;
            let value_type =
                self.lower_array_index_from_register(source, source_type, source_index)?;
            let value = self.allocate_register()?;
            self.code.emit(Instruction::Star(value));
            self.emit_array_index(offset)?;
            let key = self.allocate_register()?;
            self.code.emit(Instruction::Star(key));
            self.code.emit(Instruction::Ldar(value));
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::SetByValue {
                obj: rest_array,
                key,
                slot,
                define: true,
                strict: false,
            });
            self.release_register(key)?;
            self.release_register(value)?;
            let RegisterObjectLayout::Array { elements, .. } =
                self.object_layouts.get_mut(&rest_id)?
            else {
                return None;
            };
            elements.insert(offset, value_type);
        }
        Some((RegisterType::Array(rest_id), rest_array))
    }

    /// `CopyDataProperties` of 7.3.25 over a value whose layout the lowering
    /// does not know: the names the pattern already took go in registers of
    /// their own, which the instruction reads.
    fn lower_copied_data_properties(
        &mut self,
        source: crate::engine::bytecode::Reg,
        excluded: &[Vec<u16>],
    ) -> Option<(RegisterType, crate::engine::bytecode::Reg)> {
        use crate::engine::bytecode::Instruction;
        let count = u16::try_from(excluded.len()).ok()?;
        let mut registers = Vec::with_capacity(excluded.len());
        for name in excluded {
            let constant = self.string_constant(name)?;
            self.code.emit(Instruction::LdaString(constant));
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            registers.push(register);
        }
        let first = registers.first().copied().unwrap_or(source);
        self.code.emit(Instruction::CopyDataProperties {
            source,
            excluded: first,
            count,
        });
        // The names are read before the object is made, so their registers go
        // back before the one that holds it is taken.
        for register in registers.into_iter().rev() {
            self.release_register(register)?;
        }
        let rest_object = self.allocate_register()?;
        self.code.emit(Instruction::Star(rest_object));
        Some((RegisterType::Unknown, rest_object))
    }

    fn lower_object_rest_from_register(
        &mut self,
        source: crate::engine::bytecode::Reg,
        source_type: RegisterType,
        excluded: &[Vec<u16>],
    ) -> Option<(RegisterType, crate::engine::bytecode::Reg)> {
        use crate::engine::bytecode::Instruction;
        let RegisterType::Object(source_id) = source_type else {
            return None;
        };
        let RegisterObjectLayout::Ordinary {
            properties,
            order,
            dynamic,
        } = self.object_layouts.get(&source_id)?
        else {
            return None;
        };
        if dynamic.is_some() {
            return None;
        }
        let copied = order
            .iter()
            .filter(|name| !excluded.contains(name))
            .map(|name| Some((name.clone(), *properties.get(name)?)))
            .collect::<Option<Vec<_>>>()?;
        if copied.len() > self.property_limit {
            return None;
        }
        let rest_id = self.next_object_id;
        self.next_object_id = self.next_object_id.checked_add(1)?;
        self.object_layouts.insert(
            rest_id,
            RegisterObjectLayout::Ordinary {
                properties: BTreeMap::new(),
                order: Vec::new(),
                dynamic: None,
            },
        );
        self.code.emit(Instruction::CreateObject);
        let rest_object = self.allocate_register()?;
        self.code.emit(Instruction::Star(rest_object));
        for (name, value_type) in copied {
            let property_name = name.clone();
            let name = self.string_constant(&property_name)?;
            let get_slot =
                self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::GetNamed {
                obj: source,
                name,
                slot: get_slot,
            });
            let set_slot =
                self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::SetNamed {
                obj: rest_object,
                name,
                slot: set_slot,
                strict: false,
                define: true,
            });
            let RegisterObjectLayout::Ordinary {
                properties, order, ..
            } = self.object_layouts.get_mut(&rest_id)?
            else {
                return None;
            };
            order.push(property_name.clone());
            properties.insert(property_name, value_type);
        }
        Some((RegisterType::Object(rest_id), rest_object))
    }

    /// Reads one property of a value the lowering cannot name, which is what
    /// 14.3.3.3 does for a binding pattern over an argument.
    fn lower_unknown_property_from_register(
        &mut self,
        object: crate::engine::bytecode::Reg,
        key: &Expr,
        keyed: bool,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        if keyed {
            // 7.1.19 step 2 sends an Object key through 7.1.1, which the
            // engine runs where the access stands.
            self.lower(key)?;
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            self.code.emit(Instruction::GetByValue {
                obj: object,
                key: register,
                slot,
            });
            self.release_register(register)?;
        } else {
            let name = Self::static_property_name(key)?.to_vec();
            let constant = self.string_constant(&name)?;
            self.code.emit(Instruction::GetNamed {
                obj: object,
                name: constant,
                slot,
            });
        }
        Some(RegisterType::Unknown)
    }

    fn lower_ordinary_property_from_register(
        &mut self,
        object: crate::engine::bytecode::Reg,
        base_type: RegisterType,
        key: &Expr,
        keyed: bool,
        missing_is_undefined: bool,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let RegisterType::Object(object_id) = base_type else {
            return None;
        };
        let RegisterObjectLayout::Ordinary {
            properties,
            dynamic,
            ..
        } = self.object_layouts.get(&object_id)?
        else {
            return None;
        };
        let static_name = if keyed {
            self.static_key_units(key)
        } else {
            Some(Self::static_property_name(key)?.to_vec())
        };
        // A key the own layout does not carry is resolved on the Prototype
        // Chain. A name %Object.prototype% owns is therefore not undefined, and
        // a key only known at run time can reach one of those names.
        let result_type = if let Some(name) = static_name.as_deref() {
            let known = properties.get(name).copied();
            if known.is_none() && crate::engine::realm::object_prototype_owns(name) {
                // The name is resolved on %Object.prototype%, so it is the
                // intrinsic when one is implemented and unsupported otherwise.
                let intrinsic = crate::engine::realm::object_prototype_intrinsic(name)?;
                if dynamic.is_some() {
                    return None;
                }
                let slot =
                    self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
                let name = self.string_constant(name)?;
                self.code.emit(Instruction::GetNamed {
                    obj: object,
                    name,
                    slot,
                });
                return Some(RegisterType::NativeFunction(intrinsic));
            }
            match (known, dynamic) {
                (Some(known), Some(dynamic)) => known.merge(*dynamic),
                (Some(known), None) => known,
                (None, Some(dynamic)) => RegisterType::Undefined.merge(*dynamic),
                (None, None) if missing_is_undefined => RegisterType::Undefined,
                (None, None) => return None,
            }
        } else {
            properties
                .values()
                .copied()
                .chain(dynamic.iter().copied())
                .reduce(RegisterType::merge)
                .unwrap_or(RegisterType::Undefined)
                .merge(RegisterType::Undefined)
                .merge(if self.key_reaches_prototype(key) {
                    RegisterType::Unknown
                } else {
                    RegisterType::Undefined
                })
        };
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        if keyed {
            // 7.1.19 step 2 sends an Object key through 7.1.1, which the
            // engine runs where the access stands.
            self.lower(key)?;
            let key = self.allocate_register()?;
            self.code.emit(Instruction::Star(key));
            self.code.emit(Instruction::GetByValue {
                obj: object,
                key,
                slot,
            });
            self.release_register(key)?;
        } else {
            let name = self.string_constant(static_name.as_deref()?)?;
            self.code.emit(Instruction::GetNamed {
                obj: object,
                name,
                slot,
            });
        }
        Some(result_type)
    }

    fn lower_member_assignment(
        &mut self,
        target: &Expr,
        operator: Option<Binary>,
        value: &Expr,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let (base, key) = target.member()?;
        if operator.is_none()
            && let Some(value_type) = self.lower_unknown_member_assignment(base, key, value)?
        {
            return Some(value_type);
        }
        let prepared = self.prepare_member_assignment(target)?;
        let Some(operator) = operator else {
            let value_type = self.lower(value)?;
            self.finish_member_assignment(prepared, value_type)?;
            return Some(value_type);
        };
        // 13.15.2 evaluates the Reference once and reads through it before the
        // right side is evaluated: the base and the key are in registers
        // already, so the read and the write below reach the same property
        // whatever the right side does to the expressions that named it.
        let left_type = self.read_prepared_member(&prepared)?;
        let left_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(left_register));
        let right_type = self.lower(value)?;
        let value_type = self.emit_compound(operator, left_type, left_register, right_type)?;
        self.finish_member_assignment(prepared, value_type)?;
        Some(value_type)
    }

    /// A regular-expression literal, which 22.2.4.1 makes an object of.
    ///
    /// The pattern is compiled here, once, and the literal only makes the
    /// object each time it is evaluated.
    fn lower_regexp(&mut self, pattern: &str, flags: &str) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let compiled = crate::regexp::RegExp::compile(pattern.encode_utf16().collect(), flags)
            .ok()
            .map(alloc::rc::Rc::new)?;
        let index = u16::try_from(self.code.regex_constants.len()).ok()?;
        self.code.regex_constants.push(compiled);
        self.code.emit(Instruction::CreateRegExp(index));
        Some(RegisterType::Unknown)
    }

    /// `13.4.4.1` on a property reference: the base and the key are evaluated
    /// once, and the read and the write reach the same property.
    fn lower_member_update(
        &mut self,
        target: &Expr,
        add: bool,
        prefix: bool,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        // The old value outlives the write, so its register is allocated
        // before the ones the assignment takes and released after them.
        let numeric = self.allocate_register()?;
        let prepared = self.prepare_member_assignment(target)?;
        self.read_prepared_member(&prepared)?;
        // 13.4.4.1 takes `ToNumeric` of the old value first, so a postfix
        // update answers that Number and not what the property held. An
        // Object reaches 7.1.1 there, which the instruction names.
        self.code.emit(Instruction::Star(numeric));
        self.code.emit(Instruction::ToNumeric(numeric));
        self.code.emit(Instruction::Star(numeric));
        self.code.emit(Instruction::Ldar(numeric));
        self.code.emit(if add {
            Instruction::Increment
        } else {
            Instruction::Decrement
        });
        self.finish_member_assignment(prepared, RegisterType::Unknown)?;
        if !prefix {
            self.code.emit(Instruction::Ldar(numeric));
        }
        self.release_register(numeric)?;
        Some(RegisterType::Unknown)
    }

    /// Reads the property a prepared assignment names, through the registers
    /// that already hold its base and its key.
    ///
    /// The instruction answers what 10.1.8.1 answers, walking the Prototype
    /// Chain and naming the gap where the chain reaches a Prototype this Realm
    /// has not built, so the read needs no type of its own to be right.
    fn read_prepared_member(
        &mut self,
        prepared: &RegisterMemberAssignment,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        match &prepared.key {
            RegisterMemberKey::Named { constant, .. } => {
                self.code.emit(Instruction::GetNamed {
                    obj: prepared.object,
                    name: *constant,
                    slot,
                });
            }
            RegisterMemberKey::ArrayKeyed { register, .. }
            | RegisterMemberKey::ObjectKeyed(register, _) => {
                self.code.emit(Instruction::GetByValue {
                    obj: prepared.object,
                    key: *register,
                    slot,
                });
            }
        }
        Some(RegisterType::Unknown)
    }

    /// Lowers a write to a property of a base this lowering could not name.
    ///
    /// Answers `None` for a base it did name, so the layout-tracking path takes
    /// it instead; `Some(None)` never occurs, and an error in either direction
    /// refuses the Script as before.
    ///
    /// A name that a Prototype this Realm has not built would own is refused:
    /// 10.1.9.1 consults the chain before it creates an own property, and the
    /// missing part of the chain could hold an accessor or a property that is
    /// not writable.
    #[expect(clippy::option_option, reason = "the outer None means a typed base")]
    fn lower_unknown_member_assignment(
        &mut self,
        base: &Expr,
        key: &Expr,
        value: &Expr,
    ) -> Option<Option<RegisterType>> {
        use crate::engine::bytecode::Instruction;
        let snapshot = self.snapshot();
        let base_type = self.lower(base)?;
        if !matches!(base_type, RegisterType::Unknown | RegisterType::Function(_)) {
            self.restore(snapshot);
            return Some(None);
        }
        let object = self.allocate_register()?;
        self.code.emit(Instruction::Star(object));
        let static_name = Self::static_property_name(key)
            .map(<[u16]>::to_vec)
            .or_else(|| self.static_key_units(key));
        let keyed = match static_name.as_deref() {
            Some(name) if Self::names_an_unbuilt_prototype(name) => return None,
            Some(name) => {
                let constant = self.string_constant(name)?;
                Some(constant)
            }
            // A key only the run time knows can name one of those too, and the
            // instruction names the gap there instead of here.
            None => None,
        };
        let key_register = if keyed.is_some() {
            None
        } else {
            // 7.1.19 step 2 sends an Object key through 7.1.1, which the
            // engine runs where the access stands.
            self.lower(key)?;
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            Some(register)
        };
        let value_type = self.lower(value)?;
        if matches!(value_type, RegisterType::NativeFunction(_)) {
            return None;
        }
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        if let Some(name) = keyed {
            self.code.emit(Instruction::SetNamed {
                obj: object,
                name,
                slot,
                strict: self.assignment_strict,
                define: false,
            });
        } else {
            let register = key_register?;
            self.code.emit(Instruction::SetByValue {
                obj: object,
                key: register,
                slot,
                define: false,
                strict: self.assignment_strict,
            });
            self.release_register(register)?;
        }
        self.release_register(object)?;
        Some(Some(value_type))
    }

    /// Whether writing this name could do something other than create an own
    /// property, on whichever kind of object the base turns out to be.
    ///
    /// 10.1.9.1 creates an own property only when the Prototype Chain holds no
    /// setter and nothing that refuses the write. Every name a Prototype of
    /// this Realm would own is a writable data property, so shadowing one is
    /// what a Script may do — except `__proto__`, which B.2.2.1 makes an
    /// accessor of a Prototype this Realm has not built, so a write of that
    /// name is refused rather than guessed. A property that is not writable
    /// refuses the write where it runs (10.1.9.1).
    fn names_an_unbuilt_prototype(name: &[u16]) -> bool {
        ["__proto__"]
            .into_iter()
            .any(|refused| refused.encode_utf16().eq(name.iter().copied()))
    }

    fn prepare_member_assignment(&mut self, target: &Expr) -> Option<RegisterMemberAssignment> {
        use crate::engine::bytecode::Instruction;
        let (base, key) = target.member()?;
        let base_type = self.lower(base)?;
        // A value the lowering cannot name is written at run time, which is
        // what 13.15.2 does anyway; only a layout it tracks needs more.
        if !base_type.is_object() && base_type != RegisterType::Unknown {
            return None;
        }
        let object = self.allocate_register()?;
        self.code.emit(Instruction::Star(object));
        let array_length = matches!(base_type, RegisterType::Array(_)) && Self::is_length_name(key);
        let key = if array_length {
            // 10.4.2.4 takes the name and sets the Array's own length.
            let name = Self::static_property_name(key)?.to_vec();
            RegisterMemberKey::Named {
                constant: self.string_constant(&name)?,
                name,
            }
        } else if matches!(base_type, RegisterType::Array(_)) {
            let array_index = Self::static_array_index(key);
            if let Some(index) = array_index {
                self.emit_array_index(index)?;
            } else if self.lower(key)? != RegisterType::Number {
                return None;
            }
            let key = self.allocate_register()?;
            self.code.emit(Instruction::Star(key));
            RegisterMemberKey::ArrayKeyed {
                register: key,
                array_index,
            }
        } else if let Some(name) = Self::static_property_name(key) {
            let name = name.to_vec();
            RegisterMemberKey::Named {
                constant: self.string_constant(&name)?,
                name,
            }
        } else {
            // 7.1.19 step 2 sends an Object key through 7.1.1, which the
            // engine runs where the access stands.
            self.lower(key)?;
            let key = self.allocate_register()?;
            self.code.emit(Instruction::Star(key));
            RegisterMemberKey::ObjectKeyed(key, None)
        };
        Some(RegisterMemberAssignment {
            object,
            base_type,
            key,
        })
    }

    fn finish_member_assignment(
        &mut self,
        prepared: RegisterMemberAssignment,
        value_type: RegisterType,
    ) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        if matches!(value_type, RegisterType::NativeFunction(_)) {
            return None;
        }
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        match prepared.key {
            RegisterMemberKey::ArrayKeyed {
                register,
                array_index,
            } => {
                self.code.emit(Instruction::SetByValue {
                    obj: prepared.object,
                    key: register,
                    slot,
                    define: false,
                    strict: self.assignment_strict,
                });
                self.release_register(register)?;
                let RegisterType::Array(object_id) = prepared.base_type else {
                    return None;
                };
                let RegisterObjectLayout::Array {
                    length,
                    elements,
                    dynamic,
                } = self.object_layouts.get_mut(&object_id)?
                else {
                    return None;
                };
                if let Some(index) = array_index {
                    let is_new = !elements.contains_key(&index);
                    if is_new && elements.len().saturating_add(1) >= self.property_limit {
                        return None;
                    }
                    elements.insert(index, value_type);
                    *length = Some((*length)?.max(index.checked_add(1)?));
                } else {
                    *dynamic =
                        Some(dynamic.map_or(value_type, |current| current.merge(value_type)));
                    *length = None;
                }
            }
            RegisterMemberKey::ObjectKeyed(register, name) => {
                self.code.emit(Instruction::SetByValue {
                    obj: prepared.object,
                    key: register,
                    slot,
                    define: false,
                    strict: self.assignment_strict,
                });
                self.release_register(register)?;
                // Nothing is recorded for a base the lowering cannot name:
                // there is no layout of it to record into.
                if prepared.base_type == RegisterType::Unknown {
                    return self.release_register(prepared.object);
                }
                let RegisterType::Object(object_id) = prepared.base_type else {
                    return None;
                };
                self.record_ordinary_property_write(object_id, name, value_type)?;
            }
            RegisterMemberKey::Named { constant, name } => {
                self.code.emit(Instruction::SetNamed {
                    obj: prepared.object,
                    name: constant,
                    slot,
                    strict: self.assignment_strict,
                    define: false,
                });
                if prepared.base_type == RegisterType::Unknown {
                    return self.release_register(prepared.object);
                }
                // 10.4.2.4 deletes every index at or above the new length, and
                // the lowering cannot name which those are, so the layout
                // keeps only that its elements are no longer known.
                if let RegisterType::Array(object_id) = prepared.base_type {
                    let RegisterObjectLayout::Array {
                        length,
                        elements,
                        dynamic,
                    } = self.object_layouts.get_mut(&object_id)?
                    else {
                        return None;
                    };
                    let merged = elements
                        .values()
                        .copied()
                        .chain(*dynamic)
                        .chain(core::iter::once(RegisterType::Undefined))
                        .reduce(RegisterType::merge);
                    *dynamic = merged;
                    elements.clear();
                    *length = None;
                    return self.release_register(prepared.object);
                }
                let RegisterType::Object(object_id) = prepared.base_type else {
                    return None;
                };
                self.record_ordinary_property_write(object_id, Some(name), value_type)?;
            }
        }
        self.release_register(prepared.object)
    }

    fn static_property_name(expression: &Expr) -> Option<&[u16]> {
        match &expression.kind {
            ExprKind::Literal(Value::String(units)) => Some(units),
            ExprKind::Group(inner) => Self::static_property_name(inner),
            _ => None,
        }
    }

    fn is_length_name(expression: &Expr) -> bool {
        const LENGTH: [u16; 6] = [0x6C, 0x65, 0x6E, 0x67, 0x74, 0x68];
        Self::static_property_name(expression).is_some_and(|name| name == LENGTH)
    }

    fn is_length(units: &[u16]) -> bool {
        const LENGTH: [u16; 6] = [0x6C, 0x65, 0x6E, 0x67, 0x74, 0x68];
        units == LENGTH
    }

    /// Whether a key expression can denote a name %Object.prototype% owns.
    ///
    /// Only a String key can: every other primitive converts to a name the
    /// prototype does not own.
    fn key_reaches_prototype(&self, key: &Expr) -> bool {
        !matches!(
            register_expression_type(key, &self.bindings),
            Some(
                RegisterType::Number
                    | RegisterType::NumberOrUndefined
                    | RegisterType::Boolean
                    | RegisterType::Null
                    | RegisterType::Undefined
            )
        )
    }

    /// The property name a key expression denotes at compile time, including
    /// the `undefined` an unshadowed name denotes.
    fn static_key_units(&self, expression: &Expr) -> Option<Vec<u16>> {
        if let ExprKind::Name(name) = &expression.kind
            && name == "undefined"
            && !self.bindings.contains_key(name)
        {
            return Some("undefined".encode_utf16().collect());
        }
        if let ExprKind::Group(inner) = &expression.kind {
            return self.static_key_units(inner);
        }
        Self::static_property_key_units(expression)
    }

    fn static_property_key_units(expression: &Expr) -> Option<Vec<u16>> {
        match &expression.kind {
            ExprKind::Literal(Value::String(units)) => Some(units.to_vec()),
            ExprKind::Literal(Value::Number(number)) => Some(
                crate::number::decimal_string(*number)
                    .encode_utf16()
                    .collect(),
            ),
            ExprKind::Literal(Value::Boolean(value)) => Some(
                if *value { "true" } else { "false" }
                    .encode_utf16()
                    .collect(),
            ),
            ExprKind::Literal(Value::Null) => Some("null".encode_utf16().collect()),
            ExprKind::Literal(Value::Undefined) => Some("undefined".encode_utf16().collect()),
            ExprKind::Group(inner) => Self::static_property_key_units(inner),
            _ => None,
        }
    }

    fn binding_property_name(property: &parser::ObjectBindingProperty) -> Option<Vec<u16>> {
        if property.computed {
            Self::static_property_key_units(&property.key)
        } else {
            Some(Self::static_property_name(&property.key)?.to_vec())
        }
    }

    fn static_array_index(expression: &Expr) -> Option<u32> {
        let number = match &expression.kind {
            ExprKind::Literal(Value::Number(number)) => *number,
            ExprKind::Literal(Value::String(units)) => return parse_array_index(units),
            ExprKind::Group(inner) => return Self::static_array_index(inner),
            _ => return None,
        };
        if number == 0.0 {
            return Some(0);
        }
        if !number.is_finite() || number < 0.0 || number >= f64::from(u32::MAX) {
            return None;
        }
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "finite literal was bounded to the Array-index range"
        )]
        let index = number as u32;
        #[expect(
            clippy::float_cmp,
            reason = "Array indices require exact integral binary64 values"
        )]
        (f64::from(index) == number).then_some(index)
    }

    fn emit_array_index(&mut self, index: u32) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        if let Ok(index) = i32::try_from(index) {
            self.code.emit(Instruction::LdaSmi(index));
        } else {
            let constant =
                self.constant(crate::engine::value::Value::from_f64(f64::from(index)))?;
            self.code.emit(Instruction::LdaConstant(constant));
        }
        Some(())
    }

    fn lower_statement(&mut self, statement: &Stmt) -> Option<RegisterFlow> {
        let lowered = self.lower_statement_kind(statement);
        if lowered.is_none() {
            self.refuse(statement_refusal(statement));
        }
        lowered
    }

    fn lower_statement_kind(&mut self, statement: &Stmt) -> Option<RegisterFlow> {
        let flow = match statement {
            Stmt::Expr(expression) => RegisterFlow::Value(self.lower(expression)?),
            Stmt::If(condition, yes, no) => self.lower_if(condition, yes, no.as_deref())?,
            Stmt::Block(body) => self.lower_block(body)?,
            Stmt::Var(bindings) => {
                self.initialize_vars(bindings)?;
                RegisterFlow::Empty
            }
            Stmt::Break => {
                self.lower_loop_jump(true)?;
                RegisterFlow::Abrupt
            }
            Stmt::Continue => {
                self.lower_loop_jump(false)?;
                RegisterFlow::Abrupt
            }
            Stmt::Return(value) if self.allow_return => {
                let return_type = if let Some(value) = value {
                    self.lower(value)?
                } else {
                    self.code
                        .emit(crate::engine::bytecode::Instruction::LdaUndefined);
                    RegisterType::Undefined
                };
                if !return_type.is_returnable() {
                    self.refuse("a return of a value the lowering cannot type");
                    return None;
                }
                self.return_type = Some(
                    self.return_type
                        .map_or(return_type, |current| current.merge(return_type)),
                );
                self.close_open_iterators()?;
                self.leave_with_return()?;
                RegisterFlow::Abrupt
            }
            Stmt::Throw(value) => {
                let value_type = self.lower(value)?;
                // An Object may be thrown. One that leaves the Script has no
                // identity the embedding can hold, which the boundary reports
                // as a gap; one a handler of this Script catches never reaches
                // it.
                if !value_type.is_returnable() {
                    return None;
                }
                if let Some(thrown) = self.thrown.last_mut() {
                    thrown.push(value_type);
                }
                self.code.emit(crate::engine::bytecode::Instruction::Throw);
                RegisterFlow::Abrupt
            }
            Stmt::Try {
                body,
                catch,
                finally,
            } => self.lower_try(body, catch.as_ref(), finally.as_deref())?,
            Stmt::Switch(discriminant, clauses) => self.lower_switch(discriminant, clauses)?,
            Stmt::While(..)
            | Stmt::DoWhile(..)
            | Stmt::For(..)
            | Stmt::ForOf { .. }
            | Stmt::ForIn { .. } => RegisterFlow::Value(self.lower_loop(statement)?),
            Stmt::Empty => RegisterFlow::Empty,
            _ => return None,
        };
        Some(flow)
    }

    /// Lowers a `try` statement of 14.15 in its three forms.
    ///
    /// Without a Finally Block the value is the try Block's, or the Catch
    /// Block's when a value was thrown (14.15.3). With one, the Finally Block
    /// runs on both paths and a completion token in a register carries whether
    /// a value is still to be rethrown after it.
    #[expect(
        clippy::too_many_lines,
        reason = "one function keeps the three try forms and their handler ranges together"
    )]
    fn lower_try(
        &mut self,
        body: &[Stmt],
        catch: Option<&(Option<BindingPattern>, Vec<Stmt>)>,
        finally: Option<&[Stmt]>,
    ) -> Option<RegisterFlow> {
        use crate::engine::bytecode::{ExceptionHandler, Instruction};
        // 14.15.3: the Catch Parameter is a binding of a Declarative
        // Environment Record of its own, which 8.6.2 fills from the thrown
        // value. A single name takes the register the value is already in.
        let parameter = catch.and_then(|(parameter, _)| parameter.as_ref());
        let name = parameter.and_then(BindingPattern::identifier);
        let destructured = parameter.is_some() && name.is_none();
        // 14.15.3 runs the Finally Block on the path a `break` or a `continue`
        // takes out of the statement, which this lowering does not do. A
        // `return` takes the path below. An open iterator of an enclosing
        // `for`-`of` would be closed before the Block rather than after it,
        // which is the other order, so a return inside one is refused too.
        if finally.is_some()
            && (try_statements(body, catch, finally).any(register_statement_breaks_control)
                || !self.open_iterators.is_empty()
                    && try_statements(body, catch, finally)
                        .any(register_statement_transfers_control))
        {
            self.refuse("a jump out of a try with a Finally Block");
            return None;
        }
        let result_register = self.allocate_register()?;
        let exception_register = self.allocate_register()?;
        let token_register = match finally {
            Some(_) => Some(self.allocate_register()?),
            None => None,
        };
        // 14.15.3 keeps the value a `return` of the protected Block or of the
        // Catch Block answers until the Finally Block has run.
        let returning = match finally {
            Some(_) => Some((self.allocate_register()?, self.allocate_register()?)),
            None => None,
        };
        self.code.emit(Instruction::LdaUndefined);
        self.code.emit(Instruction::Star(result_register));
        if let Some(token_register) = token_register {
            self.code.emit(Instruction::LdaSmi(0));
            self.code.emit(Instruction::Star(token_register));
        }
        if let Some((value, flag)) = returning {
            self.code.emit(Instruction::LdaSmi(0));
            self.code.emit(Instruction::Star(flag));
            self.finallies.push(RegisterFinally {
                value,
                returning: flag,
                jumps: Vec::new(),
            });
        }

        // An exception can be thrown at any point of a protected range, so a
        // binding whose tracked type the Block changes has no single type
        // afterwards. Such a body is not lowered.
        let bindings_before = self.bindings.clone();
        let layouts_before = self.object_layouts.clone();
        // The Block starts from an empty completion, not from the token or the
        // value of the statement before the `try`.
        self.code.emit(Instruction::LdaUndefined);
        let start = self.code.instructions.len();
        self.thrown.push(Vec::new());
        let body_flow = self.lower_block(body);
        let thrown = self.thrown.pop()?;
        let body_flow = body_flow?;
        let end = self.code.instructions.len();
        let bindings_after_body = self.bindings.clone();
        let layouts_after_body = self.object_layouts.clone();
        // An exception can be thrown at any point of a protected range, so a
        // binding the Block writes carries no single type at the handler: it
        // takes the top of the lattice there, and so does one that names an
        // Object whose layout the Block changed. A Finally Block runs on every
        // path out of the statement, including the ones a widening would have
        // to describe together, so a Block beside one is not lowered when it
        // changes a type at all.
        let mut lost: BTreeSet<u32> = BTreeSet::new();
        for (id, layout) in &layouts_before {
            if layouts_after_body.get(id) != Some(layout) {
                lost.insert(*id);
            }
        }
        let mut handler_bindings = bindings_before.clone();
        if finally.is_none() {
            let names: Vec<String> = handler_bindings.keys().cloned().collect();
            for name in names {
                let one: BTreeSet<String> = core::iter::once(name.clone()).collect();
                let mut writes = false;
                for statement in body {
                    writes = writes || register_statement_writes_names(statement, &one)?;
                }
                let binding = handler_bindings.get_mut(&name)?;
                let escaped = binding
                    .value_type
                    .and_then(RegisterType::object_id)
                    .is_some_and(|id| lost.contains(&id));
                if (writes || escaped) && binding.value_type.is_some() {
                    binding.value_type = Some(RegisterType::Unknown);
                }
            }
        }
        // A binding whose type the Block changed without this lowering seeing
        // a write to it would leave the handler compiled against a type the
        // Block may already have left.
        for (name, binding) in &bindings_after_body {
            if bindings_before.get(name) == Some(binding) {
                continue;
            }
            if handler_bindings.get(name).map(|widened| widened.value_type)
                != Some(Some(RegisterType::Unknown))
            {
                self.refuse("a try body that changes a tracked type");
                return None;
            }
        }
        self.object_layouts = layouts_before.clone();
        self.object_layouts.retain(|id, _| !lost.contains(id));
        self.bindings = handler_bindings;
        let opaque = self.range_throws_opaque(start, end)?;
        if body_flow != RegisterFlow::Abrupt {
            self.code.emit(Instruction::Star(result_register));
        }
        let mut exits = alloc::vec![];
        if body_flow != RegisterFlow::Abrupt {
            exits.push(self.code.emit(Instruction::Jump(0)));
        }

        let handler_pc = self.code.instructions.len();
        // A callee's thrown type is not tracked, so a call in the protected
        // range widens the catch parameter to the top type.
        let value_type = thrown
            .into_iter()
            .chain(opaque.then_some(RegisterType::Unknown))
            .reduce(RegisterType::merge)
            .unwrap_or(RegisterType::Primitive);
        let mut handler_flow = RegisterFlow::Abrupt;
        let mut bindings_after_handler = bindings_after_body.clone();
        let mut layouts_after_handler = layouts_after_body.clone();
        if let Some((_, handler)) = catch {
            let catch_start = self.code.instructions.len();
            let previous = name.map(|name| {
                self.bindings.insert(
                    String::from(name),
                    RegisterBinding {
                        storage: RegisterBindingStorage::Register(exception_register),
                        value_type: Some(value_type),
                        mutable: true,
                        stable_function_identity: false,
                    },
                )
            });
            if name.is_some() {
                self.active_binding_count = self.active_binding_count.checked_add(1)?;
                self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
            }
            let mut shadowed: Vec<(String, Option<RegisterBinding>)> = alloc::vec![];
            if destructured {
                let pattern = parameter?;
                let mut names = alloc::vec![];
                pattern.names(&mut names);
                for bound in names {
                    self.active_binding_count = self.active_binding_count.checked_add(1)?;
                    self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
                    let register = self.allocate_register()?;
                    let previous = self.bindings.insert(
                        bound.clone(),
                        RegisterBinding {
                            storage: RegisterBindingStorage::Register(register),
                            value_type: Some(RegisterType::Unknown),
                            mutable: true,
                            stable_function_identity: false,
                        },
                    );
                    shadowed.push((bound, previous));
                }
                self.code.emit(Instruction::Ldar(exception_register));
                self.bind_pattern(value_type, pattern)?;
            }
            self.code.emit(Instruction::LdaUndefined);
            self.thrown.push(Vec::new());
            let flow = self.lower_block(handler);
            let handler_thrown = self.thrown.pop()?;
            if let (Some(outer), true) = (self.thrown.last_mut(), finally.is_none()) {
                // Without a Finally Block a value thrown by the Catch Block
                // leaves this statement, so an enclosing range observes it.
                outer.extend(handler_thrown);
            }
            // The registers were taken in the order the names came, so they
            // are given back in the other one.
            for (bound, previous) in shadowed.into_iter().rev() {
                let binding = self.bindings.remove(&bound)?;
                if let Some(previous) = previous {
                    self.bindings.insert(bound, previous);
                }
                self.active_binding_count = self.active_binding_count.checked_sub(1)?;
                if let RegisterBindingStorage::Register(register) = binding.storage {
                    self.release_register(register)?;
                }
            }
            if let (Some(name), Some(previous)) = (name, previous) {
                self.bindings.remove(name)?;
                if let Some(previous) = previous {
                    self.bindings.insert(String::from(name), previous);
                }
                self.active_binding_count = self.active_binding_count.checked_sub(1)?;
            }
            handler_flow = flow?;
            let catch_end = self.code.instructions.len();
            if handler_flow != RegisterFlow::Abrupt {
                self.code.emit(Instruction::Star(result_register));
                exits.push(self.code.emit(Instruction::Jump(0)));
            }
            bindings_after_handler = self.bindings.clone();
            layouts_after_handler = self.object_layouts.clone();
            if let Some(token_register) = token_register {
                // 14.15.3: the Finally Block also runs when the Catch Block
                // throws, and that value is rethrown after it.
                let rethrow = self.code.instructions.len();
                self.code.emit(Instruction::LdaSmi(1));
                self.code.emit(Instruction::Star(token_register));
                self.code.handlers.push(ExceptionHandler {
                    start_pc: u32::try_from(catch_start).ok()?,
                    end_pc: u32::try_from(catch_end).ok()?,
                    handler_pc: u32::try_from(rethrow).ok()?,
                    exception: exception_register,
                });
            }
        } else if let Some(token_register) = token_register {
            self.code.emit(Instruction::LdaSmi(1));
            self.code.emit(Instruction::Star(token_register));
        }
        self.code.handlers.push(ExceptionHandler {
            start_pc: u32::try_from(start).ok()?,
            end_pc: u32::try_from(end).ok()?,
            handler_pc: u32::try_from(handler_pc).ok()?,
            exception: exception_register,
        });

        let routed = match returning {
            Some(_) => self.finallies.pop()?,
            None => RegisterFinally {
                value: result_register,
                returning: result_register,
                jumps: Vec::new(),
            },
        };
        let finally_start = self.code.instructions.len();
        for exit in exits.into_iter().chain(routed.jumps.iter().copied()) {
            self.patch_jump(exit, finally_start)?;
        }
        if let Some(finally) = finally {
            self.bindings = bindings_before.clone();
            self.object_layouts = layouts_before.clone();
            self.code.emit(Instruction::LdaUndefined);
            // 14.15.3: a normal Finally completion is discarded and the try or
            // Catch completion is kept.
            self.lower_block(finally)?;
            if self.bindings != bindings_before || self.object_layouts != layouts_before {
                self.refuse("a Finally Block that changes a tracked type");
                return None;
            }
            // 14.15.3 keeps the completion the Block was reached with: a
            // `return` leaves with its value, a throw is raised again, and
            // every other path goes on after the statement. A Block reached
            // only by a throw or a `return` has no path that goes on, and the
            // test for one would leave the function able to run off its end.
            let carries_on = body_flow != RegisterFlow::Abrupt
                || catch.is_some() && handler_flow != RegisterFlow::Abrupt;
            self.code.emit(Instruction::Ldar(routed.returning));
            let leaving = self.code.emit(Instruction::JumpIfTrue(0));
            let normal = carries_on.then(|| {
                self.code
                    .emit(Instruction::Ldar(token_register.unwrap_or(routed.value)));
                self.code.emit(Instruction::JumpIfFalse(0))
            });
            self.code.emit(Instruction::Ldar(exception_register));
            self.code.emit(Instruction::Throw);
            let leave = self.code.instructions.len();
            self.patch_jump(leaving, leave)?;
            self.code.emit(Instruction::Ldar(routed.value));
            self.leave_with_return()?;
            let after = self.code.instructions.len();
            if let Some(normal) = normal {
                self.patch_jump(normal, after)?;
            }
        }
        self.object_layouts = layouts_after_body;
        self.object_layouts.retain(|id, layout| {
            !lost.contains(id) && layouts_after_handler.get(id) == Some(layout)
        });
        self.bindings = merge_register_bindings(&bindings_after_body, &bindings_after_handler)?;
        self.code.emit(Instruction::Ldar(result_register));
        if let Some((value, flag)) = returning {
            self.release_register(flag)?;
            self.release_register(value)?;
        }
        if let Some(token_register) = token_register {
            self.release_register(token_register)?;
        }
        self.release_register(exception_register)?;
        self.release_register(result_register)?;
        // 14.15.3 ends in UpdateEmpty(C, undefined): a `try` statement never
        // completes empty, so an empty Block contributes undefined and not the
        // value of the statement before it.
        let completion = |flow| match flow {
            RegisterFlow::Value(value) => Some(value),
            RegisterFlow::Empty => Some(RegisterType::Undefined),
            RegisterFlow::Abrupt => None,
        };
        let reachable = [
            completion(body_flow),
            catch.and_then(|_| completion(handler_flow)),
        ];
        Some(
            reachable
                .into_iter()
                .flatten()
                .reduce(RegisterType::merge)
                .map_or(RegisterFlow::Abrupt, RegisterFlow::Value),
        )
    }

    /// Whether a protected range calls a function, whose thrown value the
    /// lowerer cannot type.
    /// Whether a protected range can throw a value this lowering cannot type.
    ///
    /// Only `Throw` carries a value the lowering saw. Every other instruction
    /// that can throw raises an error object of the Realm, whose type the
    /// lowering does not know, so the catch parameter has to widen to the top
    /// type. The list below is therefore the instructions that cannot throw a
    /// value at all: the typed arithmetic and comparison forms belong to it
    /// because the lowering only emits them over operands it typed as
    /// primitives. `TestEqual` does not: 7.2.15 converts, and an Object
    /// operand reaches a method. Anything else makes the range opaque.
    fn range_throws_opaque(&self, start: usize, end: usize) -> Option<bool> {
        use crate::engine::bytecode::Instruction;
        Some(
            self.code
                .instructions
                .get(start..end)?
                .iter()
                .any(|instruction| {
                    !matches!(
                        instruction,
                        Instruction::LdaSmi(_)
                            | Instruction::LdaConstant(_)
                            | Instruction::LdaString(_)
                            | Instruction::LdaUndefined
                            | Instruction::LdaNull
                            | Instruction::LdaTrue
                            | Instruction::LdaFalse
                            | Instruction::LogicalNot
                            | Instruction::ToUndefined
                            | Instruction::TypeOf
                            | Instruction::Ldar(_)
                            | Instruction::Star(_)
                            | Instruction::Mov { .. }
                            | Instruction::LoadContext { .. }
                            | Instruction::StoreContext { .. }
                            | Instruction::Jump(_)
                            | Instruction::JumpIfTrue(_)
                            | Instruction::JumpIfFalse(_)
                            | Instruction::JumpIfNotNullish(_)
                            | Instruction::JumpIfNotUndefined(_)
                            | Instruction::CreateObject
                            | Instruction::CreateArray(_)
                            | Instruction::CreateClosure(_)
                            | Instruction::GetArrayLength { .. }
                            | Instruction::Negate
                            | Instruction::ToNumber
                            | Instruction::BitNot
                            | Instruction::Add(_)
                            | Instruction::Sub(_)
                            | Instruction::Mul(_)
                            | Instruction::Pow(_)
                            | Instruction::Div(_)
                            | Instruction::Mod(_)
                            | Instruction::BitAnd(_)
                            | Instruction::BitOr(_)
                            | Instruction::BitXor(_)
                            | Instruction::Shl(_)
                            | Instruction::Shr(_)
                            | Instruction::Ushr(_)
                            | Instruction::TestStrictEqual(_)
                            | Instruction::TestLessThan(_)
                            | Instruction::TestLessThanOrEqual(_)
                            | Instruction::TestGreaterThan(_)
                            | Instruction::TestGreaterThanOrEqual(_)
                            | Instruction::Throw
                            | Instruction::Return
                    )
                }),
        )
    }

    fn lower_block(&mut self, body: &[Stmt]) -> Option<RegisterFlow> {
        use crate::engine::bytecode::Instruction;
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));
        let scoped_bindings = self.enter_block_scope(body)?;
        // 14.2.3 step 1 instantiates the functions of the Block before its
        // first statement runs. B.3.2.1 gives a sloppy one a `var` binding of
        // the enclosing function as well, which this lowering does not make,
        // so only a strict Block takes one.
        if body
            .iter()
            .any(|statement| matches!(statement, Stmt::Function(_, _)))
        {
            if !self.code.strict {
                self.refuse("a function declaration in a sloppy Block");
                return None;
            }
            for statement in body {
                if let Stmt::Function(name, function) = statement {
                    let value_type = self.lower_function_declaration(name, function)?;
                    let binding = *self.bindings.get(name)?;
                    self.store_binding(binding);
                    self.bindings.get_mut(name)?.value_type = Some(value_type);
                }
            }
        }
        self.completions.push(result_register);
        let mut result_type = None;
        let mut flow = RegisterFlow::Empty;
        for statement in body {
            flow = match statement {
                Stmt::Declare(bindings) => {
                    if register_lexical_dead_zone_read(bindings)? {
                        return None;
                    }
                    for (pattern, _, initializer) in bindings {
                        if let Some(initializer) = initializer {
                            self.initialize_pattern(pattern, initializer)?;
                        } else {
                            self.initialize(pattern.identifier()?, None)?;
                        }
                        // 14.2.3 leaves the block with the binding, so the
                        // only thing the lowering needs of it is a type.
                        let mut names = Vec::new();
                        pattern.names(&mut names);
                        if names.iter().any(|name| {
                            self.bindings
                                .get(name)
                                .and_then(|binding| binding.value_type)
                                .is_none()
                        }) {
                            return None;
                        }
                    }
                    RegisterFlow::Empty
                }
                // 14.2.3 step 1 already instantiated these.
                Stmt::Function(_, _) => RegisterFlow::Empty,
                _ => self.lower_statement(statement)?,
            };
            match flow {
                RegisterFlow::Empty => {}
                RegisterFlow::Value(value_type) => {
                    self.code.emit(Instruction::Star(result_register));
                    result_type = Some(value_type);
                }
                RegisterFlow::Abrupt => break,
            }
        }
        self.completions.pop()?;
        self.leave_block_scope(scoped_bindings)?;
        if flow == RegisterFlow::Abrupt {
            self.release_register(result_register)?;
            return Some(RegisterFlow::Abrupt);
        }
        self.code.emit(Instruction::Ldar(result_register));
        self.release_register(result_register)?;
        Some(result_type.map_or(RegisterFlow::Empty, RegisterFlow::Value))
    }

    fn enter_block_scope(
        &mut self,
        body: &[Stmt],
    ) -> Option<
        Vec<(
            String,
            crate::engine::bytecode::Reg,
            Option<RegisterBinding>,
        )>,
    > {
        let names = register_block_local_names(body)?;
        self.block_scoped
            .push(names.keys().cloned().collect::<BTreeSet<String>>());
        let mut scoped = Vec::new();
        for (name, mutable) in names {
            let register = self.allocate_register()?;
            self.active_binding_count = self.active_binding_count.checked_add(1)?;
            self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
            let previous = self.bindings.insert(
                name.clone(),
                RegisterBinding {
                    storage: RegisterBindingStorage::Register(register),
                    value_type: None,
                    mutable,
                    stable_function_identity: false,
                },
            );
            scoped.push((name, register, previous));
        }
        Some(scoped)
    }

    fn leave_block_scope(
        &mut self,
        scoped: Vec<(
            String,
            crate::engine::bytecode::Reg,
            Option<RegisterBinding>,
        )>,
    ) -> Option<()> {
        self.block_scoped.pop()?;
        for (name, register, previous) in scoped.into_iter().rev() {
            self.bindings.remove(&name)?;
            if let Some(previous) = previous {
                self.bindings.insert(name, previous);
            }
            self.release_register(register)?;
            self.active_binding_count = self.active_binding_count.checked_sub(1)?;
        }
        Some(())
    }

    fn lower_loop_jump(&mut self, is_break: bool) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        // 14.12: `break` leaves the innermost breakable statement, `continue`
        // the innermost iteration statement, which a `switch` is not.
        let index = if is_break {
            self.loops.len().checked_sub(1)?
        } else {
            self.loops.iter().rposition(|frame| !frame.is_switch)?
        };
        let loop_state = self.loops.get(index)?;
        if !register_bindings_reach(&self.bindings, &loop_state.bindings)
            || !self.loop_layouts_match(&loop_state.object_layouts)
        {
            return None;
        }
        let result_register = loop_state.result_register;
        if let Some(&completion) = self.completions.get(loop_state.completion_depth..)?.last() {
            self.code.emit(Instruction::Ldar(completion));
        }
        self.code.emit(Instruction::Star(result_register));
        let jump = self.code.emit(Instruction::Jump(0));
        let loop_state = self.loops.get_mut(index)?;
        if is_break {
            loop_state.breaks.push(jump);
        } else {
            loop_state.continues.push(jump);
        }
        Some(())
    }

    /// Lowers `switch (Expression) CaseBlock` of 14.12.
    ///
    /// Selectors are evaluated in source order — the clauses before the
    /// `DefaultClause` first and the ones after it second — and only until one is
    /// strictly equal to the discriminant. Execution then falls through the
    /// remaining clause bodies in source order, accumulating the completion
    /// value, which 14.12.2 updates to undefined when it stays empty.
    fn lower_switch(
        &mut self,
        discriminant: &Expr,
        clauses: &[(Option<Expr>, Vec<Stmt>)],
    ) -> Option<RegisterFlow> {
        use crate::engine::bytecode::Instruction;
        // A lexical declaration in a CaseBlock is visible in every clause but
        // is in its Temporal Dead Zone until its own clause runs, which the
        // register lowering does not model.
        if clauses.iter().any(|(_, body)| {
            body.iter()
                .any(|statement| matches!(statement, Stmt::Declare(_) | Stmt::Function(_, _)))
        }) {
            return None;
        }
        let result_register = self.allocate_register()?;
        let input_register = self.allocate_register()?;
        let selector_register = self.allocate_register()?;
        self.code.emit(Instruction::LdaUndefined);
        self.code.emit(Instruction::Star(result_register));
        self.lower(discriminant)?;
        self.code.emit(Instruction::Star(input_register));

        let bindings_before = self.bindings.clone();
        let layouts_before = self.object_layouts.clone();
        let mut selected = Vec::new();
        for (index, (test, _)) in clauses.iter().enumerate() {
            let Some(test) = test else {
                continue;
            };
            self.lower(test)?;
            if self.bindings != bindings_before || self.object_layouts != layouts_before {
                return None;
            }
            self.code.emit(Instruction::Star(selector_register));
            self.code.emit(Instruction::Ldar(input_register));
            self.code
                .emit(Instruction::TestStrictEqual(selector_register));
            selected.push((index, self.code.emit(Instruction::JumpIfTrue(0))));
        }
        let unmatched = self.code.emit(Instruction::Jump(0));

        self.loops.push(RegisterLoop {
            is_switch: true,
            breaks: Vec::new(),
            continues: Vec::new(),
            result_register,
            bindings: bindings_before.clone(),
            completion_depth: self.completions.len(),
            object_layouts: layouts_before.clone(),
        });
        self.completions.push(result_register);
        let mut starts = Vec::new();
        let mut value_type = RegisterType::Undefined;
        for (_, body) in clauses {
            starts.push(self.code.instructions.len());
            // Every clause is an entry point of its own, which the dispatch
            // reaches with the bindings the statement began with.
            self.bindings.clone_from(&bindings_before);
            self.object_layouts.clone_from(&layouts_before);
            // A clause is entered by a jump as well as by fallthrough, and in
            // both cases the accumulator has to hold the value accumulated so
            // far, not the discriminant the dispatch left behind.
            self.code.emit(Instruction::Ldar(result_register));
            let mut abrupt = false;
            for statement in body {
                match self.lower_statement(statement)? {
                    RegisterFlow::Value(clause_type) => {
                        value_type = value_type.merge(clause_type);
                        self.code.emit(Instruction::Star(result_register));
                    }
                    RegisterFlow::Empty => {}
                    RegisterFlow::Abrupt => {
                        abrupt = true;
                        break;
                    }
                }
            }
            // 14.12.4 falls through to the next clause, which the jumps of the
            // dispatch reach with the bindings of the statement: what falls
            // through has to fit them. A clause that ends abruptly falls
            // through to nothing.
            if !abrupt
                && (!register_bindings_fit(&self.bindings, &bindings_before)
                    || !self.loop_layouts_match(&layouts_before))
            {
                return None;
            }
        }
        self.bindings.clone_from(&bindings_before);
        self.object_layouts.clone_from(&layouts_before);
        self.completions.pop()?;
        let loop_state = self.loops.pop()?;

        let done = self.code.instructions.len();
        self.code.emit(Instruction::Ldar(result_register));
        for (index, jump) in selected {
            self.patch_jump(jump, *starts.get(index)?)?;
        }
        let default = clauses.iter().position(|(test, _)| test.is_none());
        let fallback = match default {
            Some(index) => *starts.get(index)?,
            None => done,
        };
        self.patch_jump(unmatched, fallback)?;
        for jump in loop_state.breaks {
            self.patch_jump(jump, done)?;
        }
        if !loop_state.continues.is_empty() {
            return None;
        }
        self.release_register(selector_register)?;
        self.release_register(input_register)?;
        self.release_register(result_register)?;
        // 14.12.2 ends in UpdateEmpty(R, undefined), so the statement never
        // completes empty and never keeps the value before it.
        Some(RegisterFlow::Value(value_type))
    }

    /// Lowers a loop, and once more from the types its body's assignments
    /// produce when the first attempt does not hold its bindings across the
    /// back edge.
    ///
    /// The narrow types are what most loops need and what keeps their
    /// operations specialized. The second attempt admits a loop whose binding a
    /// call the lowering could not name widens. A loop that fails both is
    /// refused, as before.
    /// Lowers a loop, giving up what the body turned out to change.
    ///
    /// The first pass takes the types the bindings hold where the loop begins.
    /// The second merges them with what the body's assignments produce. The
    /// third also lets every Object a binding names leave, because a body that
    /// edits one leaves a layout the back edge cannot match, and the run time
    /// reads and writes an Object that left.
    fn lower_loop(&mut self, statement: &Stmt) -> Option<RegisterType> {
        if self.loop_head_types != RegisterLoopHead::Declared {
            return self.lower_loop_once(statement);
        }
        let snapshot = self.snapshot();
        let loops = self.loops.len();
        let completions = self.completions.len();
        if let Some(value) = self.lower_loop_once(statement) {
            return Some(value);
        }
        for head in [RegisterLoopHead::Widened, RegisterLoopHead::Escaped] {
            self.restore(snapshot.clone());
            self.loops.truncate(loops);
            self.completions.truncate(completions);
            if head == RegisterLoopHead::Escaped {
                self.escape_every_binding();
            }
            self.loop_head_types = head;
            let value = self.lower_loop_once(statement);
            self.loop_head_types = RegisterLoopHead::Declared;
            if value.is_some() {
                return value;
            }
        }
        None
    }

    /// Lets every Object a binding names leave, which drops the layout the
    /// lowering tracked for it.
    fn escape_every_binding(&mut self) {
        let named: Vec<RegisterType> = self
            .bindings
            .values()
            .filter_map(|binding| binding.value_type)
            .collect();
        self.escape(&named);
    }

    fn lower_loop_once(&mut self, statement: &Stmt) -> Option<RegisterType> {
        match statement {
            Stmt::While(condition, body) => self.lower_while(condition, body),
            Stmt::DoWhile(body, condition) => self.lower_do_while(body, condition),
            Stmt::For(initializer, condition, step, body) => {
                self.lower_for(initializer, condition.as_ref(), step.as_ref(), body)
            }
            Stmt::ForOf {
                binding,
                target,
                object,
                body,
            } => self.lower_for_of(binding.as_ref(), target.as_ref(), object, body),
            Stmt::ForIn {
                binding,
                target,
                object,
                body,
            } => self.lower_for_in(binding.as_ref(), target.as_ref(), object, body),
            _ => None,
        }
    }

    /// Lowers the condition a loop reaches on every iteration, widening the
    /// types its head starts from until the condition leaves them as it found
    /// them.
    ///
    /// An assignment in the condition writes a binding, and the second
    /// iteration starts from what the first one left, so the head carries the
    /// merge of the two rather than refusing the loop. Answers where the
    /// condition begins.
    fn lower_loop_condition(
        &mut self,
        condition: &Expr,
        bindings_at_head: &mut BTreeMap<String, RegisterBinding>,
    ) -> Option<usize> {
        // The type lattice is three steps deep, so a head that has not settled
        // by then carries a binding whose storage the merge cannot reconcile.
        for _ in 0..4u8 {
            let snapshot = self.snapshot();
            self.bindings = bindings_at_head.clone();
            let head = self.code.instructions.len();
            self.lower(condition)?;
            let merged = merge_register_bindings(&self.bindings, bindings_at_head)?;
            // The head carries what every iteration may start from, and the
            // body starts from the narrower types the condition just left.
            if merged == *bindings_at_head {
                return Some(head);
            }
            self.restore(snapshot);
            *bindings_at_head = merged;
        }
        None
    }

    fn lower_while(&mut self, condition: &Expr, body: &Stmt) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        self.code.emit(Instruction::LdaUndefined);
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));
        let mut bindings_at_head = self.bindings.clone();
        infer_register_var_types_to_fixed_point(
            body,
            &mut bindings_at_head,
            self.loop_head_types != RegisterLoopHead::Declared,
        )?;
        let head = self.lower_loop_condition(condition, &mut bindings_at_head)?;
        let branch = self.code.emit(Instruction::JumpIfFalse(0));
        self.code.emit(Instruction::Ldar(result_register));
        self.loops.push(RegisterLoop {
            is_switch: false,
            breaks: Vec::new(),
            continues: Vec::new(),
            result_register,
            bindings: bindings_at_head.clone(),
            completion_depth: self.completions.len(),
            object_layouts: bindings_at_head
                .values()
                .filter_map(|binding| match binding.value_type {
                    Some(RegisterType::Object(id) | RegisterType::Array(id)) => self
                        .object_layouts
                        .get(&id)
                        .cloned()
                        .map(|layout| (id, layout)),
                    _ => None,
                })
                .collect(),
        });
        let flow = self.lower_statement(body)?;
        let loop_state = self.loops.pop()?;
        if flow != RegisterFlow::Abrupt {
            if !register_bindings_fit(&self.bindings, &bindings_at_head)
                || !self.loop_layouts_match(&loop_state.object_layouts)
            {
                return None;
            }
            self.code.emit(Instruction::Star(result_register));
        }
        let back_edge = self.code.emit(Instruction::Jump(0));
        let done = self.code.instructions.len();
        self.code.emit(Instruction::Ldar(result_register));
        self.patch_jump(branch, done)?;
        self.patch_jump(back_edge, head)?;
        for jump in loop_state.breaks {
            self.patch_jump(jump, done)?;
        }
        for jump in loop_state.continues {
            self.patch_jump(jump, head)?;
        }
        self.bindings = bindings_at_head;
        self.release_register(result_register)?;
        Some(match flow {
            RegisterFlow::Value(value_type) => RegisterType::Undefined.merge(value_type),
            RegisterFlow::Empty | RegisterFlow::Abrupt => RegisterType::Undefined,
        })
    }

    fn lower_do_while(&mut self, body: &Stmt, condition: &Expr) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        self.code.emit(Instruction::LdaUndefined);
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));
        let mut bindings_at_head = self.bindings.clone();
        infer_register_var_types_to_fixed_point(
            body,
            &mut bindings_at_head,
            self.loop_head_types != RegisterLoopHead::Declared,
        )?;
        self.bindings = bindings_at_head.clone();
        let head = self.code.instructions.len();
        self.code.emit(Instruction::Ldar(result_register));
        self.loops.push(RegisterLoop {
            is_switch: false,
            breaks: Vec::new(),
            continues: Vec::new(),
            result_register,
            bindings: bindings_at_head.clone(),
            completion_depth: self.completions.len(),
            object_layouts: bindings_at_head
                .values()
                .filter_map(|binding| match binding.value_type {
                    Some(RegisterType::Object(id) | RegisterType::Array(id)) => self
                        .object_layouts
                        .get(&id)
                        .cloned()
                        .map(|layout| (id, layout)),
                    _ => None,
                })
                .collect(),
        });
        let flow = self.lower_statement(body)?;
        let loop_state = self.loops.pop()?;
        if flow != RegisterFlow::Abrupt {
            if !register_bindings_fit(&self.bindings, &bindings_at_head)
                || !self.loop_layouts_match(&loop_state.object_layouts)
            {
                return None;
            }
            self.code.emit(Instruction::Star(result_register));
        }
        let condition_start = self.code.instructions.len();
        for jump in loop_state.continues {
            self.patch_jump(jump, condition_start)?;
        }
        self.bindings = bindings_at_head.clone();
        self.lower(condition)?;
        if self.bindings != bindings_at_head {
            return None;
        }
        let back_edge = self.code.emit(Instruction::JumpIfTrue(0));
        let done = self.code.instructions.len();
        self.code.emit(Instruction::Ldar(result_register));
        self.patch_jump(back_edge, head)?;
        for jump in loop_state.breaks {
            self.patch_jump(jump, done)?;
        }
        self.bindings = bindings_at_head;
        self.release_register(result_register)?;
        Some(match flow {
            RegisterFlow::Value(value_type) => value_type,
            RegisterFlow::Empty | RegisterFlow::Abrupt => RegisterType::Undefined,
        })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "for lowering keeps lexical scope, completion and object-shape state atomic"
    )]
    fn lower_for(
        &mut self,
        initializer: &Stmt,
        condition: Option<&Expr>,
        step: Option<&Expr>,
        body: &Stmt,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let mut scoped_registers = Vec::new();
        match initializer {
            Stmt::Declare(bindings) => {
                let mut declared_names = BTreeSet::new();
                for (pattern, mutable, _) in bindings {
                    let mut names = Vec::new();
                    pattern.names(&mut names);
                    for name in names {
                        if self.bindings.contains_key(&name) || !declared_names.insert(name.clone())
                        {
                            return None;
                        }
                        let register = self.allocate_register()?;
                        self.active_binding_count = self.active_binding_count.checked_add(1)?;
                        self.max_binding_count =
                            self.max_binding_count.max(self.active_binding_count);
                        self.bindings.insert(
                            name.clone(),
                            RegisterBinding {
                                storage: RegisterBindingStorage::Register(register),
                                value_type: None,
                                mutable: *mutable,
                                stable_function_identity: false,
                            },
                        );
                        scoped_registers.push((name, register));
                    }
                }
                if register_lexical_dead_zone_read(bindings)? {
                    return None;
                }
                for (pattern, _, expression) in bindings {
                    if let Some(expression) = expression {
                        self.initialize_pattern(pattern, expression)?;
                    } else {
                        self.initialize(pattern.identifier()?, None)?;
                    }
                }
            }
            Stmt::Expr(expression) => {
                self.lower(expression)?;
            }
            Stmt::Var(bindings) => {
                self.initialize_vars(bindings)?;
            }
            Stmt::Empty => {}
            _ => return None,
        }

        self.code.emit(Instruction::LdaUndefined);
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));
        let mut bindings_at_head = self.bindings.clone();
        infer_register_var_types_to_fixed_point(
            body,
            &mut bindings_at_head,
            self.loop_head_types != RegisterLoopHead::Declared,
        )?;
        self.bindings = bindings_at_head.clone();
        let mut head = self.code.instructions.len();
        let branch = if let Some(condition) = condition {
            head = self.lower_loop_condition(condition, &mut bindings_at_head)?;
            Some(self.code.emit(Instruction::JumpIfFalse(0)))
        } else {
            None
        };
        self.code.emit(Instruction::Ldar(result_register));
        self.loops.push(RegisterLoop {
            is_switch: false,
            breaks: Vec::new(),
            continues: Vec::new(),
            result_register,
            bindings: bindings_at_head.clone(),
            completion_depth: self.completions.len(),
            object_layouts: bindings_at_head
                .values()
                .filter_map(|binding| match binding.value_type {
                    Some(RegisterType::Object(id) | RegisterType::Array(id)) => self
                        .object_layouts
                        .get(&id)
                        .cloned()
                        .map(|layout| (id, layout)),
                    _ => None,
                })
                .collect(),
        });
        let flow = self.lower_statement(body)?;
        let loop_state = self.loops.pop()?;
        if flow != RegisterFlow::Abrupt {
            if !register_bindings_fit(&self.bindings, &bindings_at_head)
                || !self.loop_layouts_match(&loop_state.object_layouts)
            {
                return None;
            }
            self.code.emit(Instruction::Star(result_register));
        }
        let step_start = self.code.instructions.len();
        for jump in loop_state.continues {
            self.patch_jump(jump, step_start)?;
        }
        self.bindings = bindings_at_head.clone();
        if let Some(step) = step {
            self.lower(step)?;
            if self.bindings != bindings_at_head {
                return None;
            }
        }
        let back_edge = self.code.emit(Instruction::Jump(0));
        let done = self.code.instructions.len();
        self.code.emit(Instruction::Ldar(result_register));
        if let Some(branch) = branch {
            self.patch_jump(branch, done)?;
        }
        self.patch_jump(back_edge, head)?;
        for jump in loop_state.breaks {
            self.patch_jump(jump, done)?;
        }
        self.bindings = bindings_at_head;
        self.release_register(result_register)?;
        for (name, register) in scoped_registers.into_iter().rev() {
            self.bindings.remove(&name)?;
            self.release_register(register)?;
            self.active_binding_count = self.active_binding_count.checked_sub(1)?;
        }
        Some(match flow {
            RegisterFlow::Value(value_type) => RegisterType::Undefined.merge(value_type),
            RegisterFlow::Empty | RegisterFlow::Abrupt => RegisterType::Undefined,
        })
    }

    /// The loop variable of a `for`-`in` or `for`-`of` head (14.7.5), when the
    /// head is one this lowering can hold in a register.
    ///
    /// An assignment target is not lowered. Neither is a `var` head: it shares
    /// one function-scoped binding whose inferred type the loop would have to
    /// widen. A per-iteration binding captured by a closure needs a fresh
    /// context per step, which this lowering does not create.
    /// The binding a `for`-`in` head writes.
    ///
    /// 14.7.5.5 gives a lexical head a binding of its own in every iteration.
    /// A `var` head has none: it writes the one function-scoped binding the
    /// declaration already made, which the frame keeps in a register.
    fn for_in_head_binding<'a>(
        &self,
        binding: Option<&'a (BindingPattern, Option<bool>)>,
        target: Option<&'a parser::AssignmentTarget>,
        body: &Stmt,
    ) -> Option<ForInHead<'a>> {
        if let Some(target) = target {
            // 14.7.5.6 step 7.g evaluates the target once per iteration, so
            // every name it writes carries the top of the lattice from the
            // head on.
            if !register_assignment_target_supported(target) {
                return None;
            }
            return Some(ForInHead::Target(target));
        }
        let (pattern, lexical) = binding?;
        // 8.6.2 binds the names a pattern head names out of the value of each
        // step; every other head names one binding.
        if !matches!(pattern, BindingPattern::Name(_)) {
            return Some(ForInHead::Pattern {
                pattern,
                lexical: *lexical,
            });
        }
        if lexical.is_some() {
            return self
                .iteration_binding(binding, target, body)
                .map(|(name, mutable)| ForInHead::PerIteration { name, mutable });
        }
        let name = pattern.identifier()?;
        let mut direct = BTreeSet::new();
        let mut nested = BTreeSet::new();
        let mut captured = BTreeSet::new();
        register_statement_references(body, &mut direct, &mut nested, &mut captured)?;
        if nested.contains(name) {
            return None;
        }
        let Some(declared) = self.bindings.get(name).copied() else {
            // 16.1.7 made the binding on the Global Environment Record, so
            // the loop writes it there.
            if !self.realm {
                return None;
            }
            return Some(ForInHead::Global { name });
        };
        if !declared.mutable || declared.stable_function_identity {
            return None;
        }
        let RegisterBindingStorage::Register(register) = declared.storage else {
            return None;
        };
        Some(ForInHead::Var {
            name,
            register,
            declared_type: declared.value_type?,
        })
    }

    /// Declares the names a pattern head binds, which 14.7.5.5 makes one of
    /// per iteration for a lexical head and 14.7.5.6 leaves to the
    /// declaration for a `var`.
    fn declare_iteration_pattern(
        &mut self,
        pattern: &BindingPattern,
        lexical: Option<bool>,
    ) -> Option<()> {
        let mut names = Vec::new();
        pattern.names(&mut names);
        let Some(mutable) = lexical else {
            // 14.7.5.6 step 7.g writes every name a `var` head binds, which
            // the declaration only hoisted, so each carries the top of the
            // lattice from the head on.
            for name in names {
                if let Some(binding) = self.bindings.get_mut(&name) {
                    binding.value_type = Some(RegisterType::Unknown);
                }
            }
            return Some(());
        };
        for name in names {
            self.active_binding_count = self.active_binding_count.checked_add(1)?;
            self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
            let register = self.allocate_register()?;
            self.bindings.insert(
                name,
                RegisterBinding {
                    storage: RegisterBindingStorage::Register(register),
                    value_type: Some(RegisterType::Unknown),
                    mutable,
                    stable_function_identity: false,
                },
            );
        }
        Some(())
    }

    /// Gives every name an assignment head writes the top of the lattice,
    /// which is what it carries from the head on.
    fn widen_assignment_target(&mut self, target: &parser::AssignmentTarget) {
        let mut names = Vec::new();
        register_assignment_target_names(target, &mut names);
        for name in names {
            if let Some(binding) = self.bindings.get_mut(name)
                && binding.value_type.is_some()
            {
                binding.value_type = Some(RegisterType::Unknown);
            }
        }
    }

    /// Gives back what a pattern head took.
    fn close_iteration_pattern(
        &mut self,
        pattern: &BindingPattern,
        lexical: Option<bool>,
    ) -> Option<()> {
        if lexical.is_none() {
            return Some(());
        }
        let mut names = Vec::new();
        pattern.names(&mut names);
        // The registers were taken in the order the names came, so they are
        // given back in the other one.
        for name in names.into_iter().rev() {
            let binding = self.bindings.remove(&name)?;
            self.active_binding_count = self.active_binding_count.checked_sub(1)?;
            if let RegisterBindingStorage::Register(register) = binding.storage {
                self.release_register(register)?;
            }
        }
        Some(())
    }

    fn iteration_binding<'a>(
        &self,
        binding: Option<&'a (BindingPattern, Option<bool>)>,
        target: Option<&parser::AssignmentTarget>,
        body: &Stmt,
    ) -> Option<(&'a str, bool)> {
        if target.is_some() {
            return None;
        }
        let (pattern, Some(mutable)) = binding? else {
            return None;
        };
        let name = pattern.identifier()?;
        if self.bindings.contains_key(name) {
            return None;
        }
        let mut direct = BTreeSet::new();
        let mut nested = BTreeSet::new();
        let mut captured = BTreeSet::new();
        register_statement_references(body, &mut direct, &mut nested, &mut captured)?;
        if nested.contains(name) {
            return None;
        }
        Some((name, *mutable))
    }

    /// Lowers `for (ForDeclaration in Expression) Statement` of 14.7.5.
    ///
    /// The enumeration keeps its state in four consecutive registers that
    /// `ForInNext` advances: the object of the current Prototype Chain level,
    /// that level's own-key Array, the index reached in it, and the object
    /// recording the keys already visited.
    #[expect(
        clippy::too_many_lines,
        reason = "one function emits the whole of a loop head and its close"
    )]
    fn lower_for_in(
        &mut self,
        binding: Option<&(BindingPattern, Option<bool>)>,
        target: Option<&parser::AssignmentTarget>,
        object: &Expr,
        body: &Stmt,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let head_binding = self.for_in_head_binding(binding, target, body)?;
        // 14.7.5.6 decides at run time what the head is: undefined and null
        // enumerate nothing, an Object enumerates its chain, and a primitive
        // needs the ToObject this engine cannot build. The instruction names
        // that where it happens, so any head is lowered.
        self.lower(object)?;

        let [state, keys, index, visited] = self.allocate_register_window()?;
        self.code.emit(Instruction::Star(state));
        self.code.emit(Instruction::LdaUndefined);
        self.code.emit(Instruction::Star(keys));
        self.code.emit(Instruction::LdaSmi(0));
        self.code.emit(Instruction::Star(index));
        self.code.emit(Instruction::CreateObject);
        self.code.emit(Instruction::Star(visited));

        self.code.emit(Instruction::LdaUndefined);
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));

        let key_register = match head_binding {
            ForInHead::PerIteration { name, mutable } => {
                let key_register = self.allocate_register()?;
                self.active_binding_count = self.active_binding_count.checked_add(1)?;
                self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
                self.bindings.insert(
                    String::from(name),
                    RegisterBinding {
                        storage: RegisterBindingStorage::Register(key_register),
                        value_type: Some(RegisterType::String),
                        mutable,
                        stable_function_identity: false,
                    },
                );
                key_register
            }
            // The declaration already made the binding; the loop only writes
            // it, and inside the body it holds the key.
            ForInHead::Var { name, register, .. } => {
                self.bindings.get_mut(name)?.value_type = Some(RegisterType::String);
                register
            }
            // The binding is a property of the global object, or the names a
            // pattern binds, and the loop keeps the step in a register of its
            // own to write it from.
            ForInHead::Global { .. } => self.allocate_register()?,
            ForInHead::Pattern { pattern, lexical } => {
                let register = self.allocate_register()?;
                self.declare_iteration_pattern(pattern, lexical)?;
                register
            }
            ForInHead::Target(target) => {
                let register = self.allocate_register()?;
                self.widen_assignment_target(target);
                register
            }
        };
        let head_global = match head_binding {
            ForInHead::Global { name } => {
                let units: Vec<u16> = name.encode_utf16().collect();
                Some(self.string_constant(&units)?)
            }
            _ => None,
        };
        let head_pattern = match head_binding {
            ForInHead::Pattern { pattern, .. } => Some(pattern),
            _ => None,
        };
        let head_target = match head_binding {
            ForInHead::Target(target) => Some(target),
            _ => None,
        };

        let mut bindings_at_head = self.bindings.clone();
        infer_register_var_types_to_fixed_point(
            body,
            &mut bindings_at_head,
            self.loop_head_types != RegisterLoopHead::Declared,
        )?;
        self.bindings = bindings_at_head.clone();
        let head = self.code.instructions.len();
        self.code.emit(Instruction::ForInNext { state });
        let enter = self.code.emit(Instruction::JumpIfNotUndefined(0));
        let exit = self.code.emit(Instruction::Jump(0));
        let flow = self.lower_iteration_body(
            body,
            IterationHead {
                head,
                enter,
                exit,
                variable: key_register,
                global: head_global,
                pattern: head_pattern,
                target: head_target,
                result: result_register,
                source: None,
                guarded_layout: None,
                close: None,
            },
            &bindings_at_head,
        )?;
        self.bindings = bindings_at_head;
        match head_binding {
            ForInHead::PerIteration { name, .. } => {
                self.bindings.remove(name)?;
                self.active_binding_count = self.active_binding_count.checked_sub(1)?;
                self.release_register(key_register)?;
            }
            // An enumeration that ran left a key in the binding; one that
            // enumerated nothing left what the declaration put there.
            ForInHead::Var {
                name,
                declared_type,
                ..
            } => {
                self.bindings.get_mut(name)?.value_type =
                    Some(declared_type.merge(RegisterType::String));
            }
            // The binding lives on the Global Environment Record, or is the
            // names a pattern bound, so nothing of this frame holds the step
            // after the loop.
            ForInHead::Global { .. } | ForInHead::Target(_) => {
                self.release_register(key_register)?;
            }
            ForInHead::Pattern { pattern, lexical } => {
                self.close_iteration_pattern(pattern, lexical)?;
                self.release_register(key_register)?;
            }
        }
        self.release_register(result_register)?;
        self.release_register(visited)?;
        self.release_register(index)?;
        self.release_register(keys)?;
        self.release_register(state)?;
        Some(match flow {
            RegisterFlow::Value(value_type) => RegisterType::Undefined.merge(value_type),
            RegisterFlow::Empty | RegisterFlow::Abrupt => RegisterType::Undefined,
        })
    }

    /// Lowers the body of a `for`-`in` or `for`-`of` loop and patches the jumps
    /// around it.
    ///
    /// The head has already emitted its step and the two jumps that leave it:
    /// `enter` is taken when the step produced a value and `exit` when it did
    /// not. The loop variable is written from `source`, or from the
    /// accumulator when there is none.
    fn lower_iteration_body(
        &mut self,
        body: &Stmt,
        loop_head: IterationHead<'_>,
        bindings_at_head: &BTreeMap<String, RegisterBinding>,
    ) -> Option<RegisterFlow> {
        use crate::engine::bytecode::Instruction;
        let guarded = loop_head
            .guarded_layout
            .and_then(|id| self.object_layouts.get(&id).cloned());
        let body_start = self.code.instructions.len();
        if let Some(source) = loop_head.source {
            self.code.emit(Instruction::Ldar(source));
        }
        self.code.emit(Instruction::Star(loop_head.variable));
        // 14.7.5.6 writes the binding the head declared, which for a `var` of
        // a Realm Script is a property of the global object and for a pattern
        // is whatever 8.6.2 binds out of the value.
        if let Some(name) = loop_head.global {
            self.code.emit(Instruction::StaGlobal {
                name,
                strict: false,
            });
        }
        if let Some(pattern) = loop_head.pattern {
            self.code.emit(Instruction::Ldar(loop_head.variable));
            self.bind_pattern(RegisterType::Unknown, pattern)?;
        }
        // 14.7.5.6 step 7.g evaluates the target as a Reference of its own,
        // after the step produced the value it writes through it.
        if let Some(target) = loop_head.target {
            match target {
                parser::AssignmentTarget::Reference(reference) => {
                    let prepared = self.prepare_assignment_reference(reference)?;
                    self.code.emit(Instruction::Ldar(loop_head.variable));
                    self.finish_assignment_reference(RegisterType::Unknown, reference, prepared)?;
                }
                parser::AssignmentTarget::Pattern(pattern) => {
                    self.code.emit(Instruction::Ldar(loop_head.variable));
                    self.assign_pattern(RegisterType::Unknown, pattern)?;
                }
            }
        }
        self.code.emit(Instruction::Ldar(loop_head.result));
        self.loops.push(RegisterLoop {
            is_switch: false,
            breaks: Vec::new(),
            continues: Vec::new(),
            result_register: loop_head.result,
            bindings: bindings_at_head.clone(),
            completion_depth: self.completions.len(),
            object_layouts: bindings_at_head
                .values()
                .filter_map(|binding| match binding.value_type {
                    Some(RegisterType::Object(id) | RegisterType::Array(id)) => self
                        .object_layouts
                        .get(&id)
                        .cloned()
                        .map(|layout| (id, layout)),
                    _ => None,
                })
                .collect(),
        });
        let flow = self.lower_statement(body)?;
        let loop_state = self.loops.pop()?;
        if flow != RegisterFlow::Abrupt {
            if !register_bindings_fit(&self.bindings, bindings_at_head)
                || !self.loop_layouts_match(&loop_state.object_layouts)
            {
                return None;
            }
            self.code.emit(Instruction::Star(loop_head.result));
        }
        // The loop variable's type was read before the body ran, so a body that
        // changes the layout it came from invalidates it.
        if guarded
            != loop_head
                .guarded_layout
                .and_then(|id| self.object_layouts.get(&id).cloned())
        {
            return None;
        }
        let back_edge = self.code.emit(Instruction::Jump(0));
        // 7.4.9 closes an iterator a `break` left before its end; the normal
        // exit reached that end and closes nothing.
        let closing = self.code.instructions.len();
        if let Some((iterator, scratch)) = loop_head.close
            && !loop_state.breaks.is_empty()
        {
            self.lower_iterator_close(iterator, scratch)?;
        }
        let done = self.code.instructions.len();
        self.code.emit(Instruction::Ldar(loop_head.result));
        self.patch_jump(loop_head.enter, body_start)?;
        self.patch_jump(loop_head.exit, done)?;
        self.patch_jump(back_edge, loop_head.head)?;
        for jump in loop_state.breaks {
            self.patch_jump(jump, closing)?;
        }
        for jump in loop_state.continues {
            self.patch_jump(jump, loop_head.head)?;
        }
        Some(flow)
    }

    /// Lowers `for (ForDeclaration of Expression) Statement` of 14.7.5 over an
    /// Array.
    ///
    /// The iterator is the one `%Array.prototype%[@@iterator]` produces, and
    /// `IteratorNext` advances it, so the loop follows 7.4.8 rather than
    /// indexing the Array itself.
    /// Opens the iterator of 14.7.5 step 1 on an Array.
    ///
    /// 23.1.3.40 makes `values` the same function object as `@@iterator`, so
    /// the method is read by that name; every other iterable resolves the
    /// Symbol to a method this lowering cannot name.
    fn open_array_iterator(
        &mut self,
    ) -> Option<(crate::engine::bytecode::Reg, crate::engine::bytecode::Reg)> {
        use crate::engine::bytecode::Instruction;
        let iterable = self.allocate_register()?;
        self.code.emit(Instruction::Star(iterable));
        let values = self.string_constant(&"values".encode_utf16().collect::<Vec<_>>())?;
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: iterable,
            name: values,
            slot,
        });
        let method = self.allocate_register()?;
        self.code.emit(Instruction::Star(method));
        let call = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver: iterable,
            func: method,
            arg_start: method,
            arg_count: 0,
            slot: call,
        });
        Some((iterable, method))
    }

    /// Walks an iterable by the protocol of 7.4, for a `for`-`of` whose
    /// operand is not an Array this lowering typed.
    ///
    /// 7.4.2 reads `@@iterator` and calls it, 7.4.6 calls `next` and asks the
    /// result whether it is `done`, and 7.4.7 reads `value` only when it is
    /// not. Each of those is a call or a property read the engine already has.
    ///
    /// A body that leaves by `break` or `return` would have to close the
    /// iterator (7.4.9), which the lowering does not emit, so it names that
    /// rather than leaving a `return` method uncalled.
    #[expect(
        clippy::too_many_lines,
        reason = "one function emits the whole of a loop head and its close"
    )]
    fn lower_iterated_for_of(
        &mut self,
        head_binding: ForInHead<'_>,
        body: &Stmt,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let iterable = self.allocate_register()?;
        self.code.emit(Instruction::Star(iterable));
        let iterator_slot =
            self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetWellKnown {
            obj: iterable,
            symbol: u16::try_from(WELL_KNOWN_ITERATOR).ok()?,
            slot: iterator_slot,
        });
        let method = self.allocate_register()?;
        self.code.emit(Instruction::Star(method));
        let open = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver: iterable,
            func: method,
            arg_start: method,
            arg_count: 0,
            slot: open,
        });
        let iterator = self.allocate_register()?;
        self.code.emit(Instruction::Star(iterator));

        self.code.emit(Instruction::LdaUndefined);
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));
        let step = self.allocate_register()?;
        let next = self.allocate_register()?;
        let variable = self.iteration_variable(head_binding, RegisterType::Unknown)?;
        let head_global = match head_binding {
            ForInHead::Global { name } => {
                let units: Vec<u16> = name.encode_utf16().collect();
                Some(self.string_constant(&units)?)
            }
            _ => None,
        };
        let head_pattern = match head_binding {
            ForInHead::Pattern { pattern, .. } => Some(pattern),
            _ => None,
        };
        let head_target = match head_binding {
            ForInHead::Target(target) => Some(target),
            _ => None,
        };

        let mut bindings_at_head = self.bindings.clone();
        infer_register_var_types_to_fixed_point(
            body,
            &mut bindings_at_head,
            self.loop_head_types != RegisterLoopHead::Declared,
        )?;
        self.bindings = bindings_at_head.clone();

        let head = self.code.instructions.len();
        let next_name = self.string_constant(&"next".encode_utf16().collect::<Vec<_>>())?;
        let next_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: iterator,
            name: next_name,
            slot: next_slot,
        });
        self.code.emit(Instruction::Star(next));
        let step_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver: iterator,
            func: next,
            arg_start: next,
            arg_count: 0,
            slot: step_slot,
        });
        self.code.emit(Instruction::Star(step));
        let done_name = self.string_constant(&"done".encode_utf16().collect::<Vec<_>>())?;
        let done_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: step,
            name: done_name,
            slot: done_slot,
        });
        // 7.4.6 answers done by ToBoolean, and 7.4.7 reads `value` only where
        // it is false, which is what the order of these two says.
        let exit = self.code.emit(Instruction::JumpIfTrue(0));
        let value_name = self.string_constant(&"value".encode_utf16().collect::<Vec<_>>())?;
        let value_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: step,
            name: value_name,
            slot: value_slot,
        });
        let enter = self.code.emit(Instruction::Jump(0));
        // 7.4.9 closes an iterator the loop leaves early. A `break` reaches
        // the close this emits after the body; a `return` leaves the frame, so
        // it emits a close of its own for every loop it leaves.
        self.open_iterators.push((iterator, next));
        let flow = self.lower_iteration_body(
            body,
            IterationHead {
                head,
                enter,
                exit,
                variable,
                global: head_global,
                pattern: head_pattern,
                target: head_target,
                result: result_register,
                source: None,
                guarded_layout: None,
                close: Some((iterator, next)),
            },
            &bindings_at_head,
        );
        self.open_iterators.pop();
        let flow = flow?;
        self.bindings = bindings_at_head;
        self.close_iteration_variable(head_binding, variable, RegisterType::Unknown)?;
        self.release_register(next)?;
        self.release_register(step)?;
        self.release_register(result_register)?;
        self.release_register(iterator)?;
        self.release_register(method)?;
        self.release_register(iterable)?;
        Some(match flow {
            RegisterFlow::Value(value_type) => RegisterType::Undefined.merge(value_type),
            RegisterFlow::Empty | RegisterFlow::Abrupt => RegisterType::Undefined,
        })
    }

    /// Closes every `for`-`of` iterator a `return` leaves, innermost first
    /// (7.4.9 through 14.7.5.6).
    ///
    /// The accumulator holds the value the return answers, which the close
    /// overwrites, so it waits in a register of its own.
    /// Leaves the running function with the value in the accumulator.
    ///
    /// 14.15.3 runs every Finally Block the `return` stands in before it
    /// leaves, so inside one the value and a flag are written and the jump
    /// goes to that Block, which leaves in turn.
    fn leave_with_return(&mut self) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        let Some(finally) = self.finallies.last().cloned() else {
            self.code.emit(Instruction::Return);
            return Some(());
        };
        self.code.emit(Instruction::Star(finally.value));
        self.code.emit(Instruction::LdaSmi(1));
        self.code.emit(Instruction::Star(finally.returning));
        let jump = self.code.emit(Instruction::Jump(0));
        self.finallies.last_mut()?.jumps.push(jump);
        Some(())
    }

    fn close_open_iterators(&mut self) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        if self.open_iterators.is_empty() {
            return Some(());
        }
        let value = self.allocate_register()?;
        self.code.emit(Instruction::Star(value));
        for (iterator, scratch) in self.open_iterators.clone().into_iter().rev() {
            self.lower_iterator_close(iterator, scratch)?;
        }
        self.code.emit(Instruction::Ldar(value));
        self.release_register(value)?;
        Some(())
    }

    /// `IteratorClose` of 7.4.9: an iterator with no `return` is closed by
    /// doing nothing, and one that has it is called with no argument.
    fn lower_iterator_close(
        &mut self,
        iterator: crate::engine::bytecode::Reg,
        scratch: crate::engine::bytecode::Reg,
    ) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        let return_name = self.string_constant(&"return".encode_utf16().collect::<Vec<_>>())?;
        let return_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        self.code.emit(Instruction::GetNamed {
            obj: iterator,
            name: return_name,
            slot: return_slot,
        });
        self.code.emit(Instruction::Star(scratch));
        let present = self.code.emit(Instruction::JumpIfNotNullish(0));
        let skip = self.code.emit(Instruction::Jump(0));
        let call = self.code.instructions.len();
        self.patch_jump(present, call)?;
        let close_slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::Call)?;
        self.code.emit(Instruction::CallMethod {
            receiver: iterator,
            func: scratch,
            arg_start: scratch,
            arg_count: 0,
            slot: close_slot,
        });
        let after = self.code.instructions.len();
        self.patch_jump(skip, after)?;
        Some(())
    }

    /// The register a `for`-`in` or `for`-`of` head writes each iteration.
    fn iteration_variable(
        &mut self,
        head_binding: ForInHead<'_>,
        element_type: RegisterType,
    ) -> Option<crate::engine::bytecode::Reg> {
        match head_binding {
            ForInHead::PerIteration { name, mutable } => {
                let register = self.allocate_register()?;
                self.active_binding_count = self.active_binding_count.checked_add(1)?;
                self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
                self.bindings.insert(
                    String::from(name),
                    RegisterBinding {
                        storage: RegisterBindingStorage::Register(register),
                        value_type: Some(element_type),
                        mutable,
                        stable_function_identity: false,
                    },
                );
                Some(register)
            }
            // The declaration already made the binding; the loop only writes
            // it, and inside the body it holds the element.
            ForInHead::Var { name, register, .. } => {
                self.bindings.get_mut(name)?.value_type = Some(element_type);
                Some(register)
            }
            // The binding is a property of the global object, or the names a
            // pattern binds, and the loop keeps the element in a register of
            // its own to write it from.
            ForInHead::Global { .. } => self.allocate_register(),
            ForInHead::Pattern { pattern, lexical } => {
                let register = self.allocate_register()?;
                self.declare_iteration_pattern(pattern, lexical)?;
                Some(register)
            }
            ForInHead::Target(target) => {
                let register = self.allocate_register()?;
                self.widen_assignment_target(target);
                Some(register)
            }
        }
    }

    /// Gives back what the head took, and says what the binding holds after a
    /// loop that may have run no iteration at all.
    fn close_iteration_variable(
        &mut self,
        head_binding: ForInHead<'_>,
        variable: crate::engine::bytecode::Reg,
        element_type: RegisterType,
    ) -> Option<()> {
        match head_binding {
            ForInHead::PerIteration { name, .. } => {
                self.bindings.remove(name)?;
                self.active_binding_count = self.active_binding_count.checked_sub(1)?;
                self.release_register(variable)?;
            }
            ForInHead::Var {
                name,
                declared_type,
                ..
            } => {
                self.bindings.get_mut(name)?.value_type = Some(declared_type.merge(element_type));
            }
            ForInHead::Global { .. } | ForInHead::Target(_) => {
                self.release_register(variable)?;
            }
            ForInHead::Pattern { pattern, lexical } => {
                self.close_iteration_pattern(pattern, lexical)?;
                self.release_register(variable)?;
            }
        }
        Some(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one function emits the whole of a loop head and its close"
    )]
    fn lower_for_of(
        &mut self,
        binding: Option<&(BindingPattern, Option<bool>)>,
        target: Option<&parser::AssignmentTarget>,
        object: &Expr,
        body: &Stmt,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        // 14.7.5 makes one binding per iteration for a lexical head and writes
        // the one the declaration made for a `var` head, which is the same
        // difference a `for`-`in` has.
        let head_binding = self.for_in_head_binding(binding, target, body)?;
        let object_type = self.lower(object)?;
        // An Array is stepped by `IteratorNext`, which needs no call. Every
        // other iterable is walked by the protocol of 7.4 itself, which is
        // calls and property reads the lowering already emits.
        let Some((object_id, ..)) = self.array_layout(object_type) else {
            return self.lower_iterated_for_of(head_binding, body);
        };
        let Some(element_type) = self
            .array_iteration_type(object_type)
            .filter(|element| element.is_primitive())
        else {
            return self.lower_iterated_for_of(head_binding, body);
        };

        let (iterable, method) = self.open_array_iterator()?;
        let [iterator, value] = self.allocate_register_window()?;
        self.code.emit(Instruction::Star(iterator));
        self.code.emit(Instruction::LdaUndefined);
        self.code.emit(Instruction::Star(value));

        self.code.emit(Instruction::LdaUndefined);
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));
        let key_register = match head_binding {
            ForInHead::PerIteration { name, mutable } => {
                let key_register = self.allocate_register()?;
                self.active_binding_count = self.active_binding_count.checked_add(1)?;
                self.max_binding_count = self.max_binding_count.max(self.active_binding_count);
                self.bindings.insert(
                    String::from(name),
                    RegisterBinding {
                        storage: RegisterBindingStorage::Register(key_register),
                        value_type: Some(element_type),
                        mutable,
                        stable_function_identity: false,
                    },
                );
                key_register
            }
            // The declaration already made the binding; the loop only writes
            // it, and inside the body it holds the element.
            ForInHead::Var { name, register, .. } => {
                self.bindings.get_mut(name)?.value_type = Some(element_type);
                register
            }
            ForInHead::Global { .. } => self.allocate_register()?,
            ForInHead::Pattern { pattern, lexical } => {
                let register = self.allocate_register()?;
                self.declare_iteration_pattern(pattern, lexical)?;
                register
            }
            ForInHead::Target(target) => {
                let register = self.allocate_register()?;
                self.widen_assignment_target(target);
                register
            }
        };
        let head_global = match head_binding {
            ForInHead::Global { name } => {
                let units: Vec<u16> = name.encode_utf16().collect();
                Some(self.string_constant(&units)?)
            }
            _ => None,
        };
        let head_pattern = match head_binding {
            ForInHead::Pattern { pattern, .. } => Some(pattern),
            _ => None,
        };
        let head_target = match head_binding {
            ForInHead::Target(target) => Some(target),
            _ => None,
        };

        let mut bindings_at_head = self.bindings.clone();
        infer_register_var_types_to_fixed_point(
            body,
            &mut bindings_at_head,
            self.loop_head_types != RegisterLoopHead::Declared,
        )?;
        self.bindings = bindings_at_head.clone();
        let head = self.code.instructions.len();
        self.code
            .emit(Instruction::IteratorNext { state: iterator });
        let enter = self.code.emit(Instruction::JumpIfTrue(0));
        let exit = self.code.emit(Instruction::Jump(0));
        let flow = self.lower_iteration_body(
            body,
            IterationHead {
                head,
                enter,
                exit,
                variable: key_register,
                global: head_global,
                pattern: head_pattern,
                target: head_target,
                result: result_register,
                source: Some(value),
                guarded_layout: Some(object_id),
                close: None,
            },
            &bindings_at_head,
        )?;
        self.bindings = bindings_at_head;
        match head_binding {
            ForInHead::PerIteration { name, .. } => {
                self.bindings.remove(name)?;
                self.active_binding_count = self.active_binding_count.checked_sub(1)?;
                self.release_register(key_register)?;
            }
            // An iteration that ran left an element in the binding; one that
            // iterated nothing left what the declaration put there.
            ForInHead::Var {
                name,
                declared_type,
                ..
            } => {
                self.bindings.get_mut(name)?.value_type = Some(declared_type.merge(element_type));
            }
            // The binding lives on the Global Environment Record, or is the
            // names a pattern bound, so nothing of this frame holds the step
            // after the loop.
            ForInHead::Global { .. } | ForInHead::Target(_) => {
                self.release_register(key_register)?;
            }
            ForInHead::Pattern { pattern, lexical } => {
                self.close_iteration_pattern(pattern, lexical)?;
                self.release_register(key_register)?;
            }
        }
        self.release_register(result_register)?;
        self.release_register(value)?;
        self.release_register(iterator)?;
        self.release_register(method)?;
        self.release_register(iterable)?;
        Some(match flow {
            RegisterFlow::Value(value_type) => RegisterType::Undefined.merge(value_type),
            RegisterFlow::Empty | RegisterFlow::Abrupt => RegisterType::Undefined,
        })
    }

    fn lower_if(
        &mut self,
        condition: &Expr,
        yes: &Stmt,
        no: Option<&Stmt>,
    ) -> Option<RegisterFlow> {
        use crate::engine::bytecode::Instruction;
        self.lower(condition)?;
        let branch = self.code.emit(Instruction::JumpIfFalse(0));
        let bindings_before = self.bindings.clone();
        let properties_before = self.object_layouts.clone();
        // 14.6.2 answers UpdateEmpty(stmtCompletion, undefined), so a branch
        // starts from undefined and an abrupt completion inside it carries
        // that, not the value the enclosing statement list reached.
        let completion = self.allocate_register()?;
        self.code.emit(Instruction::LdaUndefined);
        self.code.emit(Instruction::Star(completion));
        self.completions.push(completion);
        let yes_flow = self.lower_statement(yes)?;
        self.completions.pop()?;
        let mut bindings_after_yes = self.bindings.clone();
        let mut properties_after_yes = self.object_layouts.clone();
        let jump = (yes_flow != RegisterFlow::Abrupt).then(|| self.code.emit(Instruction::Jump(0)));
        let no_start = self.code.instructions.len();
        self.bindings = bindings_before;
        self.object_layouts = properties_before.clone();
        self.code.emit(Instruction::LdaUndefined);
        self.code.emit(Instruction::Star(completion));
        self.completions.push(completion);
        let no_flow = no.map_or(Some(RegisterFlow::Value(RegisterType::Undefined)), |no| {
            self.lower_statement(no)
        })?;
        self.completions.pop()?;
        self.release_register(completion)?;
        // An Object one branch handed to user code is no longer this
        // lowering's after the join, whichever branch ran, so the branch that
        // kept its layout gives it up too. One the two branches shaped
        // differently has no single shape to name and gives it up as well.
        let lost: Vec<u32> = properties_before
            .keys()
            .filter(|id| {
                !properties_after_yes.contains_key(id) || !self.object_layouts.contains_key(id)
            })
            .copied()
            .collect();
        let reshaped: Vec<u32> = properties_after_yes
            .iter()
            .filter(|(id, layout)| {
                self.object_layouts
                    .get(id)
                    .is_some_and(|other| other != *layout)
            })
            .map(|(id, _)| *id)
            .collect();
        let escaped: Vec<RegisterType> = lost
            .into_iter()
            .chain(reshaped)
            .flat_map(|id| [RegisterType::Object(id), RegisterType::Array(id)])
            .collect();
        if !escaped.is_empty() {
            self.escape(&escaped);
            let bindings = core::mem::replace(&mut self.bindings, bindings_after_yes);
            let layouts = core::mem::replace(&mut self.object_layouts, properties_after_yes);
            self.escape(&escaped);
            bindings_after_yes = core::mem::replace(&mut self.bindings, bindings);
            properties_after_yes = core::mem::replace(&mut self.object_layouts, layouts);
        }
        let bindings_after_no = self.bindings.clone();
        let end = self.code.instructions.len();
        self.patch_jump(branch, no_start)?;
        if let Some(jump) = jump {
            self.patch_jump(jump, end)?;
        }
        self.bindings = match (yes_flow, no_flow) {
            (RegisterFlow::Abrupt, RegisterFlow::Abrupt) => {
                return Some(RegisterFlow::Abrupt);
            }
            (RegisterFlow::Abrupt, _) => bindings_after_no,
            (_, RegisterFlow::Abrupt) => bindings_after_yes,
            _ => {
                self.object_layouts =
                    merge_register_layouts(&properties_after_yes, &self.object_layouts);
                let mut merged = merge_register_bindings(&bindings_after_yes, &bindings_after_no)?;
                // A value whose layout the join could not keep is one this
                // lowering can no longer name, so every access to it goes to
                // 10.1.8.1 and none to a shape that holds on one path only.
                for binding in merged.values_mut() {
                    if binding
                        .value_type
                        .and_then(RegisterType::object_id)
                        .is_some_and(|id| !self.object_layouts.contains_key(&id))
                    {
                        binding.value_type = Some(RegisterType::Unknown);
                    }
                }
                merged
            }
        };
        let value_type = match (yes_flow, no_flow) {
            (RegisterFlow::Value(yes), RegisterFlow::Value(no)) => yes.merge(no),
            (RegisterFlow::Value(value_type), RegisterFlow::Abrupt)
            | (RegisterFlow::Abrupt, RegisterFlow::Value(value_type)) => value_type,
            (RegisterFlow::Empty, RegisterFlow::Value(value_type))
            | (RegisterFlow::Value(value_type), RegisterFlow::Empty) => {
                RegisterType::Undefined.merge(value_type)
            }
            (RegisterFlow::Empty, RegisterFlow::Empty | RegisterFlow::Abrupt)
            | (RegisterFlow::Abrupt, RegisterFlow::Empty) => RegisterType::Undefined,
            (RegisterFlow::Abrupt, RegisterFlow::Abrupt) => return Some(RegisterFlow::Abrupt),
        };
        Some(RegisterFlow::Value(value_type))
    }

    fn loop_layouts_match(&self, expected: &BTreeMap<u32, RegisterObjectLayout>) -> bool {
        expected.iter().all(|(id, expected)| {
            self.object_layouts
                .get(id)
                .is_some_and(|actual| match (expected, actual) {
                    (
                        RegisterObjectLayout::Array {
                            length: expected_length,
                            elements: expected_elements,
                            dynamic: expected_dynamic,
                        },
                        RegisterObjectLayout::Array {
                            length: actual_length,
                            elements: actual_elements,
                            dynamic: actual_dynamic,
                        },
                    ) => {
                        (expected_length == actual_length
                            || expected_length.is_some() && actual_length.is_none())
                            && expected_elements == actual_elements
                            && (expected_dynamic == actual_dynamic
                                || expected_dynamic.is_none() && actual_dynamic.is_some())
                    }
                    _ => expected == actual,
                })
        })
    }

    fn lower_short_circuit(
        &mut self,
        operator: Binary,
        left: &Expr,
        right: &Expr,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let left_type = self.lower(left)?;
        let branch = self.code.emit(match operator {
            Binary::And => Instruction::JumpIfFalse(0),
            Binary::Or => Instruction::JumpIfTrue(0),
            Binary::Nullish => Instruction::JumpIfNotNullish(0),
            _ => return None,
        });
        let bindings_after_left = self.bindings.clone();
        let object_layouts_after_left = self.object_layouts.clone();
        let right_type = self.lower(right)?;
        let bindings_after_right = self.bindings.clone();
        // A layout the right side only adds belongs to an object the right
        // side makes, and no type names that object once the two sides merge.
        // A layout the right side changes or drops belongs to an object the
        // left side may leave as it was, which the lowering refuses.
        if object_layouts_after_left
            .iter()
            .any(|(id, layout)| self.object_layouts.get(id) != Some(layout))
        {
            return None;
        }
        self.bindings = merge_register_bindings(&bindings_after_left, &bindings_after_right)?;
        let end = self.code.instructions.len();
        self.patch_jump(branch, end)?;
        Some(left_type.merge(right_type))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one table names every operator beside the form it takes"
    )]
    fn lower_binary(
        &mut self,
        operator: Binary,
        left: &Expr,
        right: &Expr,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let left_type = self.lower(left)?;
        let left_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(left_register));
        let right_type = self.lower(right)?;
        let right_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(right_register));
        self.code.emit(Instruction::Ldar(left_register));
        let (instruction, result_type) = match operator {
            // 13.10.2 reaches 7.3.22 for every object of this Realm, because
            // none of them carries an `@@hasInstance` yet.
            // 13.10.2: `HasProperty` on the key the left side makes, with a
            // right side that has to be an Object.
            Binary::In => (Instruction::TestIn(right_register), RegisterType::Boolean),
            Binary::InstanceOf => (
                Instruction::TestInstanceOf(right_register),
                RegisterType::Boolean,
            ),
            Binary::Add | Binary::Sub | Binary::Mul | Binary::Pow | Binary::Div | Binary::Rem
                if left_type.is_numeric_primitive() && right_type.is_numeric_primitive() =>
            {
                let instruction = Self::number_instruction(operator, right_register)?;
                (instruction, RegisterType::Number)
            }
            Binary::Add
                if left_type == RegisterType::String && right_type == RegisterType::String =>
            {
                (Instruction::Add(right_register), RegisterType::String)
            }
            Binary::Lt | Binary::Le | Binary::Gt | Binary::Ge
                if left_type == RegisterType::Number && right_type == RegisterType::Number =>
            {
                let instruction = match operator {
                    Binary::Lt => Instruction::TestLessThan(right_register),
                    Binary::Le => Instruction::TestLessThanOrEqual(right_register),
                    Binary::Gt => Instruction::TestGreaterThan(right_register),
                    Binary::Ge => Instruction::TestGreaterThanOrEqual(right_register),
                    _ => return None,
                };
                (instruction, RegisterType::Boolean)
            }
            Binary::BitAnd
            | Binary::BitOr
            | Binary::BitXor
            | Binary::Shl
            | Binary::Shr
            | Binary::Ushr
                if left_type.is_primitive() && right_type.is_primitive() =>
            {
                let instruction = match operator {
                    Binary::BitAnd => Instruction::BitAnd(right_register),
                    Binary::BitOr => Instruction::BitOr(right_register),
                    Binary::BitXor => Instruction::BitXor(right_register),
                    Binary::Shl => Instruction::Shl(right_register),
                    Binary::Shr => Instruction::Shr(right_register),
                    Binary::Ushr => Instruction::Ushr(right_register),
                    _ => return None,
                };
                (instruction, RegisterType::Number)
            }
            Binary::StrictEq => (
                Instruction::TestStrictEqual(right_register),
                RegisterType::Boolean,
            ),
            Binary::Eq if equality_operands_supported(left_type, right_type) => (
                Instruction::TestEqual(right_register),
                RegisterType::Boolean,
            ),
            Binary::Ne if equality_operands_supported(left_type, right_type) => {
                self.code.emit(Instruction::TestEqual(right_register));
                self.code.emit(Instruction::LogicalNot);
                self.release_register(right_register)?;
                self.release_register(left_register)?;
                return Some(RegisterType::Boolean);
            }
            // 13.11.1 answers the negation of the same comparison, and the
            // comparison converts an Object operand.
            Binary::Ne => {
                let (instruction, _) =
                    self.feedback_binary(operator, left_register, right_register)?;
                self.code.emit(instruction);
                self.code.emit(Instruction::LogicalNot);
                self.release_register(right_register)?;
                self.release_register(left_register)?;
                return Some(RegisterType::Boolean);
            }
            // 7.1.1 converts an Object operand at run time, so the feedback
            // dispatch takes every operand the typed forms above do not. It is
            // also where 13.12, 13.9 and 13.11.1 land.
            Binary::Add
            | Binary::Sub
            | Binary::Mul
            | Binary::Pow
            | Binary::Div
            | Binary::Rem
            | Binary::Lt
            | Binary::Le
            | Binary::Gt
            | Binary::Ge
            | Binary::BitAnd
            | Binary::BitOr
            | Binary::BitXor
            | Binary::Shl
            | Binary::Shr
            | Binary::Ushr
            | Binary::Eq => self.feedback_binary(operator, left_register, right_register)?,
            Binary::StrictNe => {
                self.code.emit(Instruction::TestStrictEqual(right_register));
                self.code.emit(Instruction::LogicalNot);
                self.release_register(right_register)?;
                self.release_register(left_register)?;
                return Some(RegisterType::Boolean);
            }
            _ => return None,
        };
        self.code.emit(instruction);
        self.release_register(right_register)?;
        self.release_register(left_register)?;
        Some(result_type)
    }

    const fn number_instruction(
        operator: Binary,
        rhs: crate::engine::bytecode::Reg,
    ) -> Option<crate::engine::bytecode::Instruction> {
        use crate::engine::bytecode::Instruction;
        Some(match operator {
            Binary::Add => Instruction::Add(rhs),
            Binary::Sub => Instruction::Sub(rhs),
            Binary::Mul => Instruction::Mul(rhs),
            Binary::Pow => Instruction::Pow(rhs),
            Binary::Div => Instruction::Div(rhs),
            Binary::Rem => Instruction::Mod(rhs),
            _ => return None,
        })
    }

    fn feedback_binary(
        &mut self,
        operator: Binary,
        lhs: crate::engine::bytecode::Reg,
        rhs: crate::engine::bytecode::Reg,
    ) -> Option<(crate::engine::bytecode::Instruction, RegisterType)> {
        use crate::engine::bytecode::{BinaryOp, FeedbackKind, Instruction};
        let (op, result_type) = match operator {
            Binary::Add => (BinaryOp::Add, RegisterType::Primitive),
            Binary::Sub => (BinaryOp::Sub, RegisterType::Number),
            Binary::Mul => (BinaryOp::Mul, RegisterType::Number),
            Binary::Pow => (BinaryOp::Pow, RegisterType::Number),
            Binary::Div => (BinaryOp::Div, RegisterType::Number),
            Binary::Rem => (BinaryOp::Mod, RegisterType::Number),
            Binary::BitAnd => (BinaryOp::BitAnd, RegisterType::Number),
            Binary::BitOr => (BinaryOp::BitOr, RegisterType::Number),
            Binary::BitXor => (BinaryOp::BitXor, RegisterType::Number),
            Binary::Shl => (BinaryOp::ShiftLeft, RegisterType::Number),
            Binary::Shr => (BinaryOp::ShiftRight, RegisterType::Number),
            Binary::Ushr => (BinaryOp::UnsignedShiftRight, RegisterType::Number),
            Binary::Eq | Binary::Ne => (BinaryOp::Equals, RegisterType::Boolean),
            Binary::Lt => (BinaryOp::LessThan, RegisterType::Boolean),
            Binary::Le => (BinaryOp::LessThanOrEqual, RegisterType::Boolean),
            Binary::Gt => (BinaryOp::GreaterThan, RegisterType::Boolean),
            Binary::Ge => (BinaryOp::GreaterThanOrEqual, RegisterType::Boolean),
            _ => return None,
        };
        let slot = self.feedback_slot(FeedbackKind::BinaryOp)?;
        Some((Instruction::Binary { op, lhs, rhs, slot }, result_type))
    }

    /// The string constant of an identifier this Script resolves on the Global
    /// Environment Record, if `expression` is one.
    fn global_name(&mut self, expression: &Expr) -> Option<u16> {
        let ExprKind::Name(name) = &expression.kind else {
            return None;
        };
        if self.bindings.contains_key(name)
            || matches!(name.as_str(), "undefined" | "NaN" | "Infinity")
            // 10.4.4 binds `arguments` in every ordinary function, so inside
            // one it never names the Realm's global of that name.
            || (name == "arguments" && self.allow_return)
        {
            return None;
        }
        let units: Vec<u16> = name.encode_utf16().collect();
        self.string_constant(&units)
    }

    fn lower_assignment(
        &mut self,
        name: &str,
        operator: Option<Binary>,
        right: &Expr,
        strict: bool,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let Some(binding) = self.bindings.get(name).copied() else {
            // 9.1.1.4.5 writes a name no binding of this Script covers on the
            // Global Environment Record. Outside a Realm there is no such
            // Record to write to.
            if !self.realm {
                return None;
            }
            let units: Vec<u16> = name.encode_utf16().collect();
            let constant = self.string_constant(&units)?;
            let value_type = if let Some(operator) = operator {
                // 13.15.2 reads the Reference before it evaluates the right
                // side, and 9.1.1.4.6 resolves this name on the Global
                // Environment Record, where a later Script may have put
                // anything, so the read has no type of its own.
                self.code.emit(Instruction::LdaGlobal(constant));
                let left_register = self.allocate_register()?;
                self.code.emit(Instruction::Star(left_register));
                let right_type = self.lower(right)?;
                self.emit_compound(operator, RegisterType::Unknown, left_register, right_type)?
            } else {
                self.lower(right)?
            };
            self.code.emit(Instruction::StaGlobal {
                name: constant,
                strict,
            });
            // Every later call can read the name from the Global Environment
            // Record, so the value is no longer only this Script's.
            self.escape(&[value_type]);
            return Some(if value_type.object_id().is_some() {
                RegisterType::Unknown
            } else {
                value_type
            });
        };
        if !binding.mutable || binding.value_type.is_none() || binding.stable_function_identity {
            return None;
        }
        let result_type = if let Some(operator) = operator {
            let left_type = binding.value_type?;
            self.load_binding(binding);
            let left_register = self.allocate_register()?;
            self.code.emit(Instruction::Star(left_register));
            let right_type = self.lower(right)?;
            self.emit_compound(operator, left_type, left_register, right_type)?
        } else {
            self.lower(right)?
        };
        self.store_binding(binding);
        self.bindings.get_mut(name)?.value_type = Some(result_type);
        Some(result_type)
    }

    /// Applies the operator of a compound assignment (13.15.3) to the value in
    /// `left_register` and the one in the accumulator, leaving the answer in
    /// the accumulator.
    ///
    /// This is the operator of 13.15.2 step 1.e, which is the same operator
    /// the expression form applies, so the two reach the same instruction and
    /// the same feedback dispatch for the operands a typed one does not cover.
    fn emit_compound(
        &mut self,
        operator: Binary,
        left_type: RegisterType,
        left_register: crate::engine::bytecode::Reg,
        right_type: RegisterType,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        {
            let result_type = match operator {
                Binary::Add
                    if left_type == RegisterType::String && right_type == RegisterType::String =>
                {
                    RegisterType::String
                }
                Binary::Add | Binary::Sub | Binary::Mul | Binary::Div | Binary::Rem
                    if left_type.is_numeric_primitive() && right_type.is_numeric_primitive() =>
                {
                    RegisterType::Number
                }
                // The generic dispatch below carries these; 7.1.1 converts an
                // Object operand there, so the type is the one a run-time
                // dispatch answers with.
                Binary::Add => RegisterType::Primitive,
                Binary::Sub
                | Binary::Mul
                | Binary::Div
                | Binary::Rem
                | Binary::Pow
                | Binary::BitAnd
                | Binary::BitOr
                | Binary::BitXor
                | Binary::Shl
                | Binary::Shr
                | Binary::Ushr => RegisterType::Number,
                _ => return None,
            };
            let right_register = self.allocate_register()?;
            self.code.emit(Instruction::Star(right_register));
            self.code.emit(Instruction::Ldar(left_register));
            // Two operands the typed instruction does not cover are dispatched
            // at run time through a feedback slot, exactly as the same
            // operator is outside an assignment. Only 13.15.3 concatenates two
            // Strings; every other operator applies ToNumeric to them first.
            let concatenates = operator == Binary::Add
                && left_type == RegisterType::String
                && right_type == RegisterType::String;
            let integer = matches!(
                operator,
                Binary::BitAnd
                    | Binary::BitOr
                    | Binary::BitXor
                    | Binary::Shl
                    | Binary::Shr
                    | Binary::Ushr
            ) && !(left_type.is_primitive() && right_type.is_primitive());
            let generic = integer
                || matches!(
                    operator,
                    Binary::Add | Binary::Sub | Binary::Mul | Binary::Div | Binary::Rem
                ) && !(left_type.is_numeric_primitive() && right_type.is_numeric_primitive())
                    && !concatenates;
            if generic {
                let (instruction, generic_type) =
                    self.feedback_binary(operator, left_register, right_register)?;
                self.code.emit(instruction);
                self.release_register(right_register)?;
                self.release_register(left_register)?;
                return Some(generic_type);
            }
            let instruction = match operator {
                Binary::Add => Instruction::Add(right_register),
                Binary::Sub => Instruction::Sub(right_register),
                Binary::Mul => Instruction::Mul(right_register),
                Binary::Pow => {
                    let slot =
                        self.feedback_slot(crate::engine::bytecode::FeedbackKind::BinaryOp)?;
                    Instruction::Binary {
                        op: crate::engine::bytecode::BinaryOp::Pow,
                        lhs: left_register,
                        rhs: right_register,
                        slot,
                    }
                }
                Binary::Div => Instruction::Div(right_register),
                Binary::Rem => Instruction::Mod(right_register),
                Binary::BitAnd => Instruction::BitAnd(right_register),
                Binary::BitOr => Instruction::BitOr(right_register),
                Binary::BitXor => Instruction::BitXor(right_register),
                Binary::Shl => Instruction::Shl(right_register),
                Binary::Shr => Instruction::Shr(right_register),
                Binary::Ushr => Instruction::Ushr(right_register),
                _ => return None,
            };
            self.code.emit(instruction);
            self.release_register(right_register)?;
            self.release_register(left_register)?;
            Some(result_type)
        }
    }

    fn lower_update(&mut self, name: &str, add: bool, prefix: bool) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        // 9.1.1.4 resolves a name this Script does not bind on the Global
        // Environment Record, which 13.4.4.1 reads and writes the same way.
        let Some(binding) = self.bindings.get(name).copied() else {
            if !self.realm || matches!(name, "undefined" | "NaN" | "Infinity") {
                return None;
            }
            return self.lower_global_update(name, add, prefix);
        };
        if !binding.mutable {
            return None;
        }
        // 13.4.4.1 takes `ToNumeric` of the old value first, so the operand of
        // the addition is a Number whatever the binding held, and the answer a
        // postfix update gives is that Number and not what was there before.
        // An Object reaches the `ToPrimitive` of 7.1.1 there.
        self.load_binding(binding);
        let numeric = self.allocate_register()?;
        self.code.emit(Instruction::Star(numeric));
        self.code.emit(Instruction::ToNumeric(numeric));
        self.code.emit(Instruction::Star(numeric));
        self.code.emit(Instruction::Ldar(numeric));
        self.code.emit(if add {
            Instruction::Increment
        } else {
            Instruction::Decrement
        });
        self.store_binding(binding);
        // 13.4.4.1 answers a BigInt for a BigInt operand, so only an operand
        // the lowering knows to be a Number leaves a Number behind.
        let answer = if binding
            .value_type
            .is_some_and(RegisterType::is_numeric_primitive)
        {
            RegisterType::Number
        } else {
            RegisterType::Unknown
        };
        self.bindings.get_mut(name)?.value_type = Some(answer);
        if !prefix {
            self.code.emit(Instruction::Ldar(numeric));
        }
        self.release_register(numeric)?;
        Some(answer)
    }

    /// `13.4.4.1` on a name the Global Environment Record binds.
    fn lower_global_update(&mut self, name: &str, add: bool, prefix: bool) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let units: Vec<u16> = name.encode_utf16().collect();
        let constant = self.string_constant(&units)?;
        self.code.emit(Instruction::LdaGlobal(constant));
        let numeric = self.allocate_register()?;
        self.code.emit(Instruction::Star(numeric));
        self.code.emit(Instruction::ToNumeric(numeric));
        self.code.emit(Instruction::Star(numeric));
        self.code.emit(Instruction::Ldar(numeric));
        self.code.emit(if add {
            Instruction::Increment
        } else {
            Instruction::Decrement
        });
        self.code.emit(Instruction::StaGlobal {
            name: constant,
            strict: false,
        });
        if !prefix {
            self.code.emit(Instruction::Ldar(numeric));
        }
        self.release_register(numeric)?;
        Some(RegisterType::Unknown)
    }

    fn allocate_register(&mut self) -> Option<crate::engine::bytecode::Reg> {
        let register = crate::engine::bytecode::Reg(self.next_register);
        self.next_register = self.next_register.checked_add(1)?;
        self.register_count = self.register_count.max(self.next_register);
        Some(register)
    }

    /// Allocates `N` registers as one contiguous window, in allocation order.
    /// An instruction addressing a register range needs its operands adjacent,
    /// which the bump allocator gives by construction.
    fn allocate_register_window<const N: usize>(
        &mut self,
    ) -> Option<[crate::engine::bytecode::Reg; N]> {
        let mut window = [crate::engine::bytecode::Reg(0); N];
        for slot in &mut window {
            *slot = self.allocate_register()?;
        }
        Some(window)
    }

    fn release_register(&mut self, register: crate::engine::bytecode::Reg) -> Option<()> {
        self.next_register = self.next_register.checked_sub(1)?;
        if register.0 != self.next_register {
            return None;
        }
        Some(())
    }

    fn constant(&mut self, value: crate::engine::value::Value) -> Option<u16> {
        let index = u16::try_from(self.code.constants.len()).ok()?;
        self.code.constants.push(value);
        Some(index)
    }

    fn string_constant(&mut self, units: &[u16]) -> Option<u16> {
        let index = u16::try_from(self.code.string_constants.len()).ok()?;
        self.code.string_constants.push(units.to_vec());
        Some(index)
    }

    fn feedback_slot(&mut self, kind: crate::engine::bytecode::FeedbackKind) -> Option<u16> {
        if self.code.feedback_slots.len() >= usize::from(u16::MAX) {
            return None;
        }
        Some(self.code.allocate_feedback_slot(kind))
    }

    fn patch_jump(&mut self, at: usize, target: usize) -> Option<()> {
        let next = at.checked_add(1)?;
        let target = isize::try_from(target).ok()?;
        let next = isize::try_from(next).ok()?;
        let offset = i32::try_from(target.checked_sub(next)?).ok()?;
        match self.code.instructions.get_mut(at)? {
            crate::engine::bytecode::Instruction::Jump(value)
            | crate::engine::bytecode::Instruction::JumpIfFalse(value)
            | crate::engine::bytecode::Instruction::JumpIfTrue(value)
            | crate::engine::bytecode::Instruction::JumpIfNotNullish(value)
            | crate::engine::bytecode::Instruction::JumpIfNotUndefined(value) => *value = offset,
            _ => return None,
        }
        Some(())
    }
}

/// Whether 13.11.1 needs no conversion of either operand.
///
/// A type the lowering does not know may be an Object, and 7.2.14 sends an
/// Object through 7.1.1, so only two types it does know answer here.
const fn equality_operands_supported(left: RegisterType, right: RegisterType) -> bool {
    left.is_primitive() && right.is_primitive() || left.is_object() && right.is_object()
}

fn smi_literal(number: f64) -> Option<i32> {
    if !number.is_finite()
        || number < f64::from(i32::MIN)
        || number > f64::from(i32::MAX)
        || number == 0.0 && number.is_sign_negative()
    {
        return None;
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "finite Number literal was bounded to the signed 32-bit range"
    )]
    let integer = number as i32;
    #[expect(
        clippy::float_cmp,
        reason = "Smi literals require exact integral binary64 values"
    )]
    (f64::from(integer) == number).then_some(integer)
}

fn parse_array_index(units: &[u16]) -> Option<u32> {
    if units.is_empty()
        || units.len() > 10
        || units.len() > 1 && units.first() == Some(&u16::from(b'0'))
    {
        return None;
    }
    let mut index = 0u32;
    for unit in units {
        let digit = unit
            .checked_sub(u16::from(b'0'))
            .filter(|digit| *digit < 10)?;
        index = index.checked_mul(10)?.checked_add(u32::from(digit))?;
    }
    (index != u32::MAX).then_some(index)
}

fn register_expression_type(
    expression: &Expr,
    bindings: &BTreeMap<String, RegisterBinding>,
) -> Option<RegisterType> {
    let value_type = match &expression.kind {
        ExprKind::Literal(Value::Number(_)) => RegisterType::Number,
        ExprKind::Literal(Value::Boolean(_)) => RegisterType::Boolean,
        ExprKind::Literal(Value::Null) => RegisterType::Null,
        ExprKind::Literal(Value::Undefined) => RegisterType::Undefined,
        ExprKind::Literal(Value::String(_)) => RegisterType::String,
        ExprKind::Name(name) => bindings
            .get(name)
            .and_then(|binding| binding.value_type)
            .or(match name.as_str() {
                "undefined" => Some(RegisterType::Undefined),
                "NaN" | "Infinity" => Some(RegisterType::Number),
                _ => None,
            })?,
        ExprKind::Update(name, _, _) => bindings.get(name)?.value_type?,
        ExprKind::Group(inner) => register_expression_type(inner, bindings)?,
        ExprKind::Sequence(_, right) | ExprKind::Assign(_, _, right) => {
            register_expression_type(right, bindings)?
        }
        ExprKind::Unary(operator, inner) => {
            let inner = register_expression_type(inner, bindings)?;
            match operator {
                Unary::Plus => RegisterType::Number,
                Unary::Minus | Unary::BitNot if inner == RegisterType::Number => {
                    RegisterType::Number
                }
                Unary::Not | Unary::Delete => RegisterType::Boolean,
                Unary::Void => RegisterType::Undefined,
                Unary::Typeof => RegisterType::String,
                Unary::Minus | Unary::BitNot => {
                    return None;
                }
            }
        }
        ExprKind::Binary(operator, left, right) => {
            let left = register_expression_type(left, bindings)?;
            let right = register_expression_type(right, bindings)?;
            match operator {
                Binary::And | Binary::Or | Binary::Nullish => left.merge(right),
                Binary::Add if left == RegisterType::String && right == RegisterType::String => {
                    RegisterType::String
                }
                Binary::Add | Binary::Sub | Binary::Mul | Binary::Div | Binary::Rem
                    if left.is_numeric_primitive() && right.is_numeric_primitive() =>
                {
                    RegisterType::Number
                }
                Binary::Pow
                | Binary::BitAnd
                | Binary::BitOr
                | Binary::BitXor
                | Binary::Shl
                | Binary::Shr
                | Binary::Ushr => {
                    if !left.is_primitive() || !right.is_primitive() {
                        return None;
                    }
                    RegisterType::Number
                }
                Binary::Lt
                | Binary::Le
                | Binary::Gt
                | Binary::Ge
                | Binary::Eq
                | Binary::Ne
                | Binary::StrictEq
                | Binary::StrictNe => RegisterType::Boolean,
                _ => return None,
            }
        }
        ExprKind::Conditional(_, yes, no) => {
            register_expression_type(yes, bindings)?.merge(register_expression_type(no, bindings)?)
        }
        _ => return None,
    };
    Some(value_type)
}

/// The type one intrinsic returns.
#[expect(
    clippy::too_many_lines,
    reason = "one table names every intrinsic beside the type it answers"
)]
const fn intrinsic_result_type(intrinsic: crate::engine::realm::Intrinsic) -> RegisterType {
    match intrinsic {
        crate::engine::realm::Intrinsic::ObjectPrototypeHasOwnProperty
        | crate::engine::realm::Intrinsic::ObjectPrototypeIsPrototypeOf
        | crate::engine::realm::Intrinsic::ObjectPrototypePropertyIsEnumerable
        | crate::engine::realm::Intrinsic::StringPrototypeEndsWith
        | crate::engine::realm::Intrinsic::StringPrototypeIncludes
        | crate::engine::realm::Intrinsic::StringPrototypeStartsWith
        | crate::engine::realm::Intrinsic::ArrayPrototypeIncludes
        | crate::engine::realm::Intrinsic::ObjectIs
        | crate::engine::realm::Intrinsic::ObjectHasOwn
        | crate::engine::realm::Intrinsic::ReflectSet
        | crate::engine::realm::Intrinsic::ObjectIsExtensible
        | crate::engine::realm::Intrinsic::ObjectIsSealed
        | crate::engine::realm::Intrinsic::ObjectIsFrozen
        | crate::engine::realm::Intrinsic::IsNaN
        | crate::engine::realm::Intrinsic::IsFinite
        // 28.1.3, 28.1.4, 28.1.8, 28.1.9 and 28.1.11 answer whether the
        // operation they name succeeded.
        | crate::engine::realm::Intrinsic::ReflectDefineProperty
        | crate::engine::realm::Intrinsic::ReflectDeleteProperty
        | crate::engine::realm::Intrinsic::ReflectHas
        | crate::engine::realm::Intrinsic::ReflectIsExtensible
        | crate::engine::realm::Intrinsic::ReflectPreventExtensions
        // 21.1.2.2 to 21.1.2.5 each answer a Boolean about one argument.
        | crate::engine::realm::Intrinsic::NumberIsFinite
        | crate::engine::realm::Intrinsic::NumberIsInteger
        | crate::engine::realm::Intrinsic::NumberIsNaN
        | crate::engine::realm::Intrinsic::NumberIsSafeInteger
        // 20.3.1.1 answers the Boolean ToBoolean makes of its argument.
        | crate::engine::realm::Intrinsic::BooleanConstructor
        | crate::engine::realm::Intrinsic::ArrayPrototypeEvery
        | crate::engine::realm::Intrinsic::ArrayPrototypeSome
        | crate::engine::realm::Intrinsic::BooleanPrototypeValueOf
        | crate::engine::realm::Intrinsic::RegExpPrototypeTest
        // 22.1.3.9 answers whether the text has a lone surrogate.
        | crate::engine::realm::Intrinsic::StringPrototypeIsWellFormed
        // 24.1.3.7, 24.1.3.3, 24.2.3.8 and 24.2.3.4 answer a Boolean.
        | crate::engine::realm::Intrinsic::MapPrototypeHas
        | crate::engine::realm::Intrinsic::MapPrototypeDelete
        | crate::engine::realm::Intrinsic::SetPrototypeHas
        | crate::engine::realm::Intrinsic::SetPrototypeDelete
        // 24.3.3.4, 24.3.3.2, 24.4.3.4 and 24.4.3.3 answer a Boolean.
        | crate::engine::realm::Intrinsic::WeakMapPrototypeHas
        | crate::engine::realm::Intrinsic::WeakMapPrototypeDelete
        // The rawJSON proposal answers a Boolean from `isRawJSON`.
        | crate::engine::realm::Intrinsic::JsonIsRawJson
        // 25.1.5.1, 25.1.6.3 and 25.1.6.6 answer a Boolean.
        | crate::engine::realm::Intrinsic::ArrayBufferIsView
        | crate::engine::realm::Intrinsic::ArrayBufferPrototypeDetached
        | crate::engine::realm::Intrinsic::ArrayBufferPrototypeResizable
        | crate::engine::realm::Intrinsic::WeakSetPrototypeHas
        | crate::engine::realm::Intrinsic::WeakSetPrototypeDelete
        // 24.2.3.12, 24.2.3.13 and 24.2.3.11 answer a Boolean.
        | crate::engine::realm::Intrinsic::SetPrototypeIsSubsetOf
        | crate::engine::realm::Intrinsic::SetPrototypeIsSupersetOf
        | crate::engine::realm::Intrinsic::SetPrototypeIsDisjointFrom
        | crate::engine::realm::Intrinsic::ArrayIsArray
        // 25.2.5.3 and 25.4.7 answer a Boolean.
        | crate::engine::realm::Intrinsic::SharedArrayBufferPrototypeGrowable
        | crate::engine::realm::Intrinsic::AtomicsIsLockFree => RegisterType::Boolean,
        // 22.1.1.1 answers a String whichever argument it took; `new` answers
        // no value at all, because the exotic object it would make is a gap.
        crate::engine::realm::Intrinsic::StringConstructor
        // 22.1.2.1, 22.1.2.2 and 22.1.2.4 answer a String of what they read.
        | crate::engine::realm::Intrinsic::StringFromCharCode
        // 19.2.6 answers the text it built.
        | crate::engine::realm::Intrinsic::EncodeUri
        | crate::engine::realm::Intrinsic::EncodeUriComponent
        | crate::engine::realm::Intrinsic::DecodeUri
        | crate::engine::realm::Intrinsic::DecodeUriComponent
        // B.2.1 answers a text too.
        | crate::engine::realm::Intrinsic::Escape
        | crate::engine::realm::Intrinsic::Unescape
        // 21.4.4.43 and 21.4.4.42 answer a text.
        | crate::engine::realm::Intrinsic::DatePrototypeToIsoString
        | crate::engine::realm::Intrinsic::DatePrototypeToString
        | crate::engine::realm::Intrinsic::DatePrototypeToDateString
        | crate::engine::realm::Intrinsic::DatePrototypeToTimeString
        | crate::engine::realm::Intrinsic::DatePrototypeToUtcString
        | crate::engine::realm::Intrinsic::DatePrototypeToLocaleString
        | crate::engine::realm::Intrinsic::DatePrototypeToLocaleDateString
        | crate::engine::realm::Intrinsic::DatePrototypeToLocaleTimeString
        | crate::engine::realm::Intrinsic::StringFromCodePoint
        | crate::engine::realm::Intrinsic::StringRaw
        // 22.2.6.4 and 22.2.6.13 answer a String for every receiver they take.
        | crate::engine::realm::Intrinsic::RegExpPrototypeFlags
        | crate::engine::realm::Intrinsic::RegExpPrototypeSource
        | crate::engine::realm::Intrinsic::ObjectPrototypeToString
        | crate::engine::realm::Intrinsic::NumberPrototypeToString
        | crate::engine::realm::Intrinsic::BooleanPrototypeToString
        | crate::engine::realm::Intrinsic::RegExpPrototypeToString
        | crate::engine::realm::Intrinsic::JsonStringify
        // 22.1.3.29 and B.2.2 answer a String of the text.
        | crate::engine::realm::Intrinsic::StringPrototypeToWellFormed
        | crate::engine::realm::Intrinsic::StringPrototypeSubstr
        | crate::engine::realm::Intrinsic::StringPrototypeAnchor
        | crate::engine::realm::Intrinsic::StringPrototypeBig
        | crate::engine::realm::Intrinsic::StringPrototypeBlink
        | crate::engine::realm::Intrinsic::StringPrototypeBold
        | crate::engine::realm::Intrinsic::StringPrototypeFixed
        | crate::engine::realm::Intrinsic::StringPrototypeFontcolor
        | crate::engine::realm::Intrinsic::StringPrototypeFontsize
        | crate::engine::realm::Intrinsic::StringPrototypeItalics
        | crate::engine::realm::Intrinsic::StringPrototypeLink
        | crate::engine::realm::Intrinsic::StringPrototypeSmall
        | crate::engine::realm::Intrinsic::StringPrototypeStrike
        | crate::engine::realm::Intrinsic::StringPrototypeSub
        | crate::engine::realm::Intrinsic::StringPrototypeSup
        | crate::engine::realm::Intrinsic::StringPrototypeToString
        | crate::engine::realm::Intrinsic::StringPrototypeValueOf
        | crate::engine::realm::Intrinsic::StringPrototypeCharAt
        | crate::engine::realm::Intrinsic::StringPrototypeConcat
        | crate::engine::realm::Intrinsic::StringPrototypeRepeat
        | crate::engine::realm::Intrinsic::StringPrototypeSlice
        | crate::engine::realm::Intrinsic::StringPrototypeSubstring
        | crate::engine::realm::Intrinsic::StringPrototypePadEnd
        | crate::engine::realm::Intrinsic::StringPrototypePadStart
        | crate::engine::realm::Intrinsic::StringPrototypeTrim
        | crate::engine::realm::Intrinsic::StringPrototypeTrimEnd
        | crate::engine::realm::Intrinsic::StringPrototypeTrimStart
        | crate::engine::realm::Intrinsic::ArrayPrototypeJoin
        | crate::engine::realm::Intrinsic::SymbolPrototypeToString
        | crate::engine::realm::Intrinsic::FunctionPrototypeToString
        | crate::engine::realm::Intrinsic::ErrorPrototypeToString
        | crate::engine::realm::Intrinsic::StringPrototypeReplace
        | crate::engine::realm::Intrinsic::ArrayPrototypeToString
        // 21.2.3.3 and 21.2.3.2 write the BigInt out.
        | crate::engine::realm::Intrinsic::BigIntPrototypeToString
        | crate::engine::realm::Intrinsic::BigIntPrototypeToLocaleString => RegisterType::String,

        // 23.1.3.38 answers an Array Iterator and 23.1.5.2.1 a result object,
        // neither of which has a tracked layout. 23.1.3.1 answers an element,
        // whose type only the receiver's layout carries, so the call site
        // reads it there instead. 23.1.1.1 answers an Array whose elements
        // this lowering did not make and cannot name.
        // 10.2.4.1 answers nothing at all: it throws.
        crate::engine::realm::Intrinsic::ThrowTypeError
        | crate::engine::realm::Intrinsic::SymbolConstructor
        | crate::engine::realm::Intrinsic::RegExpConstructor
        | crate::engine::realm::Intrinsic::RegExpPrototypeExec
        | crate::engine::realm::Intrinsic::JsonParse
        | crate::engine::realm::Intrinsic::SymbolKeyFor
        | crate::engine::realm::Intrinsic::StringPrototypeSplit
        | crate::engine::realm::Intrinsic::StringPrototypeMatch
        | crate::engine::realm::Intrinsic::StringPrototypeSearch
        | crate::engine::realm::Intrinsic::SymbolPrototypeValueOf
        | crate::engine::realm::Intrinsic::SymbolFor
        | crate::engine::realm::Intrinsic::ParseInt
        | crate::engine::realm::Intrinsic::ParseFloat
        | crate::engine::realm::Intrinsic::ArrayConstructor
        | crate::engine::realm::Intrinsic::ObjectConstructor
        | crate::engine::realm::Intrinsic::ObjectDefineProperty
        | crate::engine::realm::Intrinsic::ObjectGetOwnPropertyDescriptor
        | crate::engine::realm::Intrinsic::ObjectGetOwnPropertyNames
        | crate::engine::realm::Intrinsic::ObjectCreate
        | crate::engine::realm::Intrinsic::ObjectDefineProperties
        | crate::engine::realm::Intrinsic::ObjectGetPrototypeOf
        | crate::engine::realm::Intrinsic::ObjectSetPrototypeOf
        | crate::engine::realm::Intrinsic::ReflectSetPrototypeOf
        | crate::engine::realm::Intrinsic::ObjectKeys
        | crate::engine::realm::Intrinsic::ReflectGet
        | crate::engine::realm::Intrinsic::ReflectGetOwnPropertyDescriptor
        | crate::engine::realm::Intrinsic::ReflectGetPrototypeOf
        | crate::engine::realm::Intrinsic::ReflectOwnKeys
        | crate::engine::realm::Intrinsic::ObjectValues
        | crate::engine::realm::Intrinsic::ObjectEntries
        // 20.1.2.20, 20.1.2.22 and 20.1.2.6 answer the object they were given.
        | crate::engine::realm::Intrinsic::ObjectPreventExtensions
        | crate::engine::realm::Intrinsic::ObjectSeal
        | crate::engine::realm::Intrinsic::ObjectFreeze
        | crate::engine::realm::Intrinsic::FunctionConstructor
        | crate::engine::realm::Intrinsic::FunctionPrototypeCall
        // 20.2.3 answers undefined whatever it was given.
        | crate::engine::realm::Intrinsic::FunctionPrototype
        // 20.1.3.7 answers the object it was given.
        | crate::engine::realm::Intrinsic::ObjectPrototypeValueOf
        | crate::engine::realm::Intrinsic::FunctionPrototypeApply
        | crate::engine::realm::Intrinsic::ReflectApply
        | crate::engine::realm::Intrinsic::ReflectConstruct
        | crate::engine::realm::Intrinsic::SpeciesGetter
        // 23.1.2.1 and 23.1.2.3 answer an Array whose elements this lowering
        // did not make.
        | crate::engine::realm::Intrinsic::ArrayOf
        | crate::engine::realm::Intrinsic::ArrayFrom
        // 19.2.1 answers whatever the Script it evaluated did.
        | crate::engine::realm::Intrinsic::Eval
        // 22.2.6.11 answers the String it built, which no lowering names.
        | crate::engine::realm::Intrinsic::RegExpPrototypeReplace
        // 23.1.3.14, 23.1.3.30, 23.1.3.34 and 23.1.3.35 answer an Array whose
        // elements this lowering did not make.
        | crate::engine::realm::Intrinsic::ArrayPrototypeFlat
        | crate::engine::realm::Intrinsic::ArrayPrototypeSort
        | crate::engine::realm::Intrinsic::ArrayPrototypeToSorted
        | crate::engine::realm::Intrinsic::ArrayPrototypeToSpliced
        | crate::engine::realm::Intrinsic::FunctionPrototypeBind
        | crate::engine::realm::Intrinsic::MathPow
        | crate::engine::realm::Intrinsic::ErrorConstructor
        | crate::engine::realm::Intrinsic::EvalErrorConstructor
        | crate::engine::realm::Intrinsic::RangeErrorConstructor
        | crate::engine::realm::Intrinsic::ReferenceErrorConstructor
        | crate::engine::realm::Intrinsic::SyntaxErrorConstructor
        | crate::engine::realm::Intrinsic::TypeErrorConstructor
        | crate::engine::realm::Intrinsic::UriErrorConstructor
        | crate::engine::realm::Intrinsic::ArrayPrototypeValues
        | crate::engine::realm::Intrinsic::ArrayPrototypeKeys
        | crate::engine::realm::Intrinsic::ArrayPrototypeEntries
        | crate::engine::realm::Intrinsic::ArrayIteratorPrototypeNext
        | crate::engine::realm::Intrinsic::ArrayPrototypeAt
        | crate::engine::realm::Intrinsic::ArrayPrototypePop
        | crate::engine::realm::Intrinsic::ArrayPrototypeReverse
        | crate::engine::realm::Intrinsic::ArrayPrototypeSlice
        // 27.1.2.1 answers the receiver, whose layout the call site knows and
        // this table does not.
        | crate::engine::realm::Intrinsic::IteratorPrototypeIterator
        | crate::engine::realm::Intrinsic::ArrayPrototypeShift
        | crate::engine::realm::Intrinsic::ArrayPrototypeSplice
        | crate::engine::realm::Intrinsic::ArrayPrototypeFill
        | crate::engine::realm::Intrinsic::ArrayPrototypeCopyWithin
        | crate::engine::realm::Intrinsic::ArrayPrototypeConcat
        | crate::engine::realm::Intrinsic::ArrayPrototypeWith
        | crate::engine::realm::Intrinsic::ArrayPrototypeToReversed
        // 23.1.3.21 and 23.1.3.8 answer an Array, 23.1.3.9 an element and
        // 23.1.3.24 whatever the callback carried; none has a tracked layout.
        | crate::engine::realm::Intrinsic::ArrayPrototypeMap
        | crate::engine::realm::Intrinsic::ArrayPrototypeFilter
        | crate::engine::realm::Intrinsic::ArrayPrototypeFind
        | crate::engine::realm::Intrinsic::ArrayPrototypeFindLast
        | crate::engine::realm::Intrinsic::ArrayPrototypeFindLastIndex
        | crate::engine::realm::Intrinsic::ArrayPrototypeReduce
        | crate::engine::realm::Intrinsic::ArrayPrototypeReduceRight
        // 23.1.3.15, 24.1.3.5 and 24.2.3.7 answer undefined.
        | crate::engine::realm::Intrinsic::ArrayPrototypeForEach
        | crate::engine::realm::Intrinsic::MapPrototypeForEach
        | crate::engine::realm::Intrinsic::SetPrototypeForEach
        // 20.1.2.1 answers the target, 20.1.2.9 and 20.1.2.11 answer an
        // object and an Array of Symbols, and 28.1.13 answers a Boolean the
        // lowering types below.
        | crate::engine::realm::Intrinsic::ObjectAssign
        | crate::engine::realm::Intrinsic::ObjectGetOwnPropertyDescriptors
        | crate::engine::realm::Intrinsic::ObjectGetOwnPropertySymbols
        | crate::engine::realm::Intrinsic::ArrayPrototypeFlatMap
        | crate::engine::realm::Intrinsic::ObjectFromEntries
        // 27.2 answers a Promise, a settled value, or nothing; none of the
        // four has a tracked layout.
        | crate::engine::realm::Intrinsic::PromiseConstructor
        | crate::engine::realm::Intrinsic::PromiseResolve
        | crate::engine::realm::Intrinsic::PromiseReject
        | crate::engine::realm::Intrinsic::PromisePrototypeThen
        | crate::engine::realm::Intrinsic::PromisePrototypeCatch
        | crate::engine::realm::Intrinsic::PromiseResolveFunction
        | crate::engine::realm::Intrinsic::PromiseRejectFunction
        // The embedding answers nothing for a line it wrote.
        | crate::engine::realm::Intrinsic::Print
        | crate::engine::realm::Intrinsic::PromiseAll
        | crate::engine::realm::Intrinsic::PromiseRace
        | crate::engine::realm::Intrinsic::PromiseAllSettled
        | crate::engine::realm::Intrinsic::PromiseWithResolvers
        | crate::engine::realm::Intrinsic::PromiseAllElement
        | crate::engine::realm::Intrinsic::PromiseAllSettledFulfilled
        | crate::engine::realm::Intrinsic::PromiseAllSettledRejected
        // 22.2.6.8 answers an Array or null, 22.2.6.14 an Array of parts.
        | crate::engine::realm::Intrinsic::RegExpPrototypeMatch
        | crate::engine::realm::Intrinsic::RegExpPrototypeSplit
        // 27.7.5.3 answers nothing: the job queue is its only caller.
        | crate::engine::realm::Intrinsic::AsyncResume
        | crate::engine::realm::Intrinsic::AsyncThrow
        // 24.1.3.6 answers the value an entry holds, and 24.1.3.9, 24.2.3.1
        // and the two constructors answer the collection itself.
        | crate::engine::realm::Intrinsic::MapConstructor
        | crate::engine::realm::Intrinsic::MapPrototypeGet
        | crate::engine::realm::Intrinsic::MapPrototypeSet
        | crate::engine::realm::Intrinsic::SetConstructor
        | crate::engine::realm::Intrinsic::SetPrototypeAdd
        // 24.3.3.3 answers the value an entry holds, and 24.3.3.5, 24.4.3.1
        // and the two constructors answer the collection itself.
        | crate::engine::realm::Intrinsic::WeakMapConstructor
        | crate::engine::realm::Intrinsic::WeakMapPrototypeGet
        | crate::engine::realm::Intrinsic::WeakMapPrototypeSet
        | crate::engine::realm::Intrinsic::WeakSetConstructor
        | crate::engine::realm::Intrinsic::WeakSetPrototypeAdd
        // 24.1.3.7, 24.1.3.8, 24.3.3.4 and 24.3.3.5 answer the value of the
        // entry, which is the argument or what the callback said.
        | crate::engine::realm::Intrinsic::MapPrototypeGetOrInsert
        | crate::engine::realm::Intrinsic::MapPrototypeGetOrInsertComputed
        | crate::engine::realm::Intrinsic::WeakMapPrototypeGetOrInsert
        | crate::engine::realm::Intrinsic::WeakMapPrototypeGetOrInsertComputed
        // `rawJSON` answers the object it made.
        | crate::engine::realm::Intrinsic::JsonRawJson
        // 21.4.2.1 answers the Date it made, and 21.4.4.42 whatever the text
        // of it is.
        | crate::engine::realm::Intrinsic::DateConstructor
        | crate::engine::realm::Intrinsic::DatePrototypeToJson
        // 25.1.4.1 and 25.1.6.7 answer the block they made.
        | crate::engine::realm::Intrinsic::ArrayBufferConstructor
        | crate::engine::realm::Intrinsic::ArrayBufferPrototypeSlice
        // 25.3.3.1 answers the view, 25.3.4.1 the block it looks into, and
        // every write of 25.3.4 answers undefined.
        | crate::engine::realm::Intrinsic::DataViewConstructor
        | crate::engine::realm::Intrinsic::DataViewPrototypeBuffer
        // 23.2.6 answers the array it made, 23.2.3.1 the block it looks into,
        // and 23.2.1.1 refuses every call.
        | crate::engine::realm::Intrinsic::TypedArrayBase
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeAt
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeCopyWithin
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeEntries
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeFill
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeIncludes
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeIndexOf
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeJoin
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeKeys
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeLastIndexOf
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeReverse
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeSet
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeSlice
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeSort
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeSubarray
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeToReversed
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeToSorted
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeValues
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeWith
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeEvery
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeFind
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeFindIndex
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeFindLast
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeFindLastIndex
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeForEach
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeReduce
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeReduceRight
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeSome
        // 21.2.1.1, 21.2.2 and 21.2.3.4 answer a BigInt, which no
        // register type of this lowering names.
        | crate::engine::realm::Intrinsic::BigIntConstructor
        | crate::engine::realm::Intrinsic::BigIntAsIntN
        | crate::engine::realm::Intrinsic::BigIntAsUintN
        | crate::engine::realm::Intrinsic::BigIntPrototypeValueOf
        // 25.2.4 and 25.2.5.4 answer a block, and 25.4.13 one of the
        // three texts of table 76.
        | crate::engine::realm::Intrinsic::SharedArrayBufferConstructor
        | crate::engine::realm::Intrinsic::SharedArrayBufferPrototypeSlice
        | crate::engine::realm::Intrinsic::AtomicsWait
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeToStringTag
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeBuffer
        | crate::engine::realm::Intrinsic::TypedArrayInt8Constructor
        | crate::engine::realm::Intrinsic::TypedArrayUint8Constructor
        | crate::engine::realm::Intrinsic::TypedArrayUint8ClampedConstructor
        | crate::engine::realm::Intrinsic::TypedArrayInt16Constructor
        | crate::engine::realm::Intrinsic::TypedArrayUint16Constructor
        | crate::engine::realm::Intrinsic::TypedArrayInt32Constructor
        | crate::engine::realm::Intrinsic::TypedArrayUint32Constructor
        | crate::engine::realm::Intrinsic::TypedArrayFloat32Constructor
        | crate::engine::realm::Intrinsic::TypedArrayFloat64Constructor
        | crate::engine::realm::Intrinsic::TypedArrayFloat16Constructor
        | crate::engine::realm::Intrinsic::TypedArrayBigInt64Constructor
        | crate::engine::realm::Intrinsic::TypedArrayBigUint64Constructor
        | crate::engine::realm::Intrinsic::DataViewPrototypeSetInt8
        | crate::engine::realm::Intrinsic::DataViewPrototypeSetUint8
        | crate::engine::realm::Intrinsic::DataViewPrototypeSetInt16
        | crate::engine::realm::Intrinsic::DataViewPrototypeSetUint16
        | crate::engine::realm::Intrinsic::DataViewPrototypeSetInt32
        | crate::engine::realm::Intrinsic::DataViewPrototypeSetUint32
        | crate::engine::realm::Intrinsic::DataViewPrototypeSetFloat32
        | crate::engine::realm::Intrinsic::DataViewPrototypeSetFloat64
        // 24.1.5.1 and 24.2.5.1 answer an iterator, and 7.4.14 an ordinary
        // object with a `value` and a `done`.
        | crate::engine::realm::Intrinsic::MapPrototypeEntries
        | crate::engine::realm::Intrinsic::MapPrototypeKeys
        | crate::engine::realm::Intrinsic::MapPrototypeValues
        | crate::engine::realm::Intrinsic::SetPrototypeValues
        | crate::engine::realm::Intrinsic::SetPrototypeEntries
        | crate::engine::realm::Intrinsic::MapIteratorPrototypeNext
        | crate::engine::realm::Intrinsic::SetIteratorPrototypeNext
        // 24.2.3.18, 24.2.3.9, 24.2.3.6 and 24.2.3.15 answer a Set.
        | crate::engine::realm::Intrinsic::SetPrototypeUnion
        | crate::engine::realm::Intrinsic::SetPrototypeIntersection
        | crate::engine::realm::Intrinsic::SetPrototypeDifference
        | crate::engine::realm::Intrinsic::SetPrototypeSymmetricDifference => {
            RegisterType::Unknown
        }
        // 24.1.3.1 and 24.2.3.2 answer undefined.
        crate::engine::realm::Intrinsic::MapPrototypeClear
        | crate::engine::realm::Intrinsic::SetPrototypeClear
        // 25.2.5.2 and 25.4.10 answer undefined.
        | crate::engine::realm::Intrinsic::SharedArrayBufferPrototypeGrow
        | crate::engine::realm::Intrinsic::AtomicsPause => RegisterType::Undefined,
        // 22.2.6.12 answers the index of the match.
        crate::engine::realm::Intrinsic::RegExpPrototypeSearch
        | crate::engine::realm::Intrinsic::StringPrototypeCharCodeAt
        | crate::engine::realm::Intrinsic::StringPrototypeIndexOf
        | crate::engine::realm::Intrinsic::StringPrototypeLastIndexOf
        | crate::engine::realm::Intrinsic::ArrayPrototypeIndexOf
        | crate::engine::realm::Intrinsic::ArrayPrototypeLastIndexOf
        | crate::engine::realm::Intrinsic::ArrayPrototypePush
        | crate::engine::realm::Intrinsic::ArrayPrototypeUnshift
        // 22.1.3.12 answers the order of the two texts.
        | crate::engine::realm::Intrinsic::StringPrototypeLocaleCompare
        | crate::engine::realm::Intrinsic::ArrayPrototypeFindIndex
        // 21.3.2 answers a Number for every one of these.
        | crate::engine::realm::Intrinsic::MathAbs
        | crate::engine::realm::Intrinsic::MathCeil
        | crate::engine::realm::Intrinsic::MathFloor
        | crate::engine::realm::Intrinsic::MathTrunc
        | crate::engine::realm::Intrinsic::MathRound
        | crate::engine::realm::Intrinsic::MathSign
        | crate::engine::realm::Intrinsic::MathMax
        | crate::engine::realm::Intrinsic::MathMin
        | crate::engine::realm::Intrinsic::MathClz32
        | crate::engine::realm::Intrinsic::MathImul
        | crate::engine::realm::Intrinsic::MathFround
        | crate::engine::realm::Intrinsic::MathSin
        | crate::engine::realm::Intrinsic::MathAcos
        | crate::engine::realm::Intrinsic::MathAcosh
        | crate::engine::realm::Intrinsic::MathAsin
        | crate::engine::realm::Intrinsic::MathAsinh
        | crate::engine::realm::Intrinsic::MathAtan
        | crate::engine::realm::Intrinsic::MathAtanh
        | crate::engine::realm::Intrinsic::MathCbrt
        | crate::engine::realm::Intrinsic::MathCos
        | crate::engine::realm::Intrinsic::MathCosh
        | crate::engine::realm::Intrinsic::MathExp
        | crate::engine::realm::Intrinsic::MathExpm1
        | crate::engine::realm::Intrinsic::MathF16round
        | crate::engine::realm::Intrinsic::MathLog
        | crate::engine::realm::Intrinsic::MathLog10
        | crate::engine::realm::Intrinsic::MathLog1p
        | crate::engine::realm::Intrinsic::MathLog2
        | crate::engine::realm::Intrinsic::MathSinh
        | crate::engine::realm::Intrinsic::MathSqrt
        | crate::engine::realm::Intrinsic::MathTan
        | crate::engine::realm::Intrinsic::MathTanh
        // 21.4.3 and 21.4.4 answer a Number, but for the text of 21.4.4.43,
        // the object of 21.4.2.1 and the value of 21.4.4.42.
        | crate::engine::realm::Intrinsic::DateNow
        | crate::engine::realm::Intrinsic::DateUtc
        | crate::engine::realm::Intrinsic::DateParse
        | crate::engine::realm::Intrinsic::DatePrototypeValueOf
        | crate::engine::realm::Intrinsic::DatePrototypeGetTime
        | crate::engine::realm::Intrinsic::DatePrototypeSetTime
        | crate::engine::realm::Intrinsic::DatePrototypeGetTimezoneOffset
        | crate::engine::realm::Intrinsic::DatePrototypeGetFullYear
        | crate::engine::realm::Intrinsic::DatePrototypeGetUtcFullYear
        | crate::engine::realm::Intrinsic::DatePrototypeGetMonth
        | crate::engine::realm::Intrinsic::DatePrototypeGetUtcMonth
        | crate::engine::realm::Intrinsic::DatePrototypeGetDate
        | crate::engine::realm::Intrinsic::DatePrototypeGetUtcDate
        | crate::engine::realm::Intrinsic::DatePrototypeGetDay
        | crate::engine::realm::Intrinsic::DatePrototypeGetUtcDay
        | crate::engine::realm::Intrinsic::DatePrototypeGetHours
        | crate::engine::realm::Intrinsic::DatePrototypeGetUtcHours
        | crate::engine::realm::Intrinsic::DatePrototypeGetMinutes
        | crate::engine::realm::Intrinsic::DatePrototypeGetUtcMinutes
        | crate::engine::realm::Intrinsic::DatePrototypeGetSeconds
        | crate::engine::realm::Intrinsic::DatePrototypeGetUtcSeconds
        | crate::engine::realm::Intrinsic::DatePrototypeGetMilliseconds
        | crate::engine::realm::Intrinsic::DatePrototypeGetUtcMilliseconds
        // 25.1.6.2 and 25.1.6.4 answer a Number.
        | crate::engine::realm::Intrinsic::ArrayBufferPrototypeByteLength
        | crate::engine::realm::Intrinsic::ArrayBufferPrototypeMaxByteLength
        // 25.3.4 answers a Number from every read and from two accessors.
        | crate::engine::realm::Intrinsic::DataViewPrototypeByteLength
        | crate::engine::realm::Intrinsic::DataViewPrototypeByteOffset
        // 23.2.3.2, 23.2.3.3 and 23.2.3.19 answer a Number.
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeByteLength
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeByteOffset
        | crate::engine::realm::Intrinsic::TypedArrayPrototypeLength
        | crate::engine::realm::Intrinsic::DataViewPrototypeGetInt8
        | crate::engine::realm::Intrinsic::DataViewPrototypeGetUint8
        | crate::engine::realm::Intrinsic::DataViewPrototypeGetInt16
        | crate::engine::realm::Intrinsic::DataViewPrototypeGetUint16
        | crate::engine::realm::Intrinsic::DataViewPrototypeGetInt32
        | crate::engine::realm::Intrinsic::DataViewPrototypeGetUint32
        | crate::engine::realm::Intrinsic::DataViewPrototypeGetFloat32
        | crate::engine::realm::Intrinsic::DataViewPrototypeGetFloat64
        | crate::engine::realm::Intrinsic::DatePrototypeSetMilliseconds
        | crate::engine::realm::Intrinsic::DatePrototypeSetUtcMilliseconds
        | crate::engine::realm::Intrinsic::DatePrototypeSetSeconds
        | crate::engine::realm::Intrinsic::DatePrototypeSetUtcSeconds
        | crate::engine::realm::Intrinsic::DatePrototypeSetMinutes
        | crate::engine::realm::Intrinsic::DatePrototypeSetUtcMinutes
        | crate::engine::realm::Intrinsic::DatePrototypeSetHours
        | crate::engine::realm::Intrinsic::DatePrototypeSetUtcHours
        | crate::engine::realm::Intrinsic::DatePrototypeSetDate
        | crate::engine::realm::Intrinsic::DatePrototypeSetUtcDate
        | crate::engine::realm::Intrinsic::DatePrototypeSetMonth
        | crate::engine::realm::Intrinsic::DatePrototypeSetUtcMonth
        | crate::engine::realm::Intrinsic::DatePrototypeSetFullYear
        | crate::engine::realm::Intrinsic::DatePrototypeSetUtcFullYear
        | crate::engine::realm::Intrinsic::DatePrototypeSetYear
        | crate::engine::realm::Intrinsic::MathAtan2
        | crate::engine::realm::Intrinsic::MathHypot
        | crate::engine::realm::Intrinsic::MathRandom
        // 21.1.1.1 answers the Number ToNumber makes of its argument.
        | crate::engine::realm::Intrinsic::NumberPrototypeValueOf
        // 24.1.3.10 and 24.2.3.14 answer the count of entries.
        | crate::engine::realm::Intrinsic::MapPrototypeSize
        | crate::engine::realm::Intrinsic::SetPrototypeSize
        | crate::engine::realm::Intrinsic::NumberConstructor
        // 25.2.5.1 and 25.2.5.5 answer a length, and the reads and
        // writes of 25.4 the element of a row of table 71.
        | crate::engine::realm::Intrinsic::SharedArrayBufferPrototypeByteLength
        | crate::engine::realm::Intrinsic::SharedArrayBufferPrototypeMaxByteLength
        | crate::engine::realm::Intrinsic::AtomicsAdd
        | crate::engine::realm::Intrinsic::AtomicsAnd
        | crate::engine::realm::Intrinsic::AtomicsCompareExchange
        | crate::engine::realm::Intrinsic::AtomicsExchange
        | crate::engine::realm::Intrinsic::AtomicsLoad
        | crate::engine::realm::Intrinsic::AtomicsOr
        | crate::engine::realm::Intrinsic::AtomicsStore
        | crate::engine::realm::Intrinsic::AtomicsSub
        | crate::engine::realm::Intrinsic::AtomicsXor
        | crate::engine::realm::Intrinsic::AtomicsNotify => RegisterType::Number,
        // 22.1.3.1 and 22.1.3.4 answer undefined for an index outside the String.
        crate::engine::realm::Intrinsic::StringPrototypeAt
        | crate::engine::realm::Intrinsic::StringPrototypeCodePointAt
        // 22.2.6 answers a Boolean for a RegExp and undefined for the one
        // object of step 3 that is no RegExp.
        | crate::engine::realm::Intrinsic::RegExpPrototypeHasIndices
        | crate::engine::realm::Intrinsic::RegExpPrototypeGlobal
        | crate::engine::realm::Intrinsic::RegExpPrototypeIgnoreCase
        | crate::engine::realm::Intrinsic::RegExpPrototypeMultiline
        | crate::engine::realm::Intrinsic::RegExpPrototypeDotAll
        | crate::engine::realm::Intrinsic::RegExpPrototypeUnicode
        | crate::engine::realm::Intrinsic::RegExpPrototypeUnicodeSets
        | crate::engine::realm::Intrinsic::RegExpPrototypeSticky
        => RegisterType::Primitive,
    }
}

/// Joins the object layouts of the two branches of an `if` (14.6.2).
///
/// An Object only one branch made exists only where that branch ran, so what
/// its layout says still holds after the join. One both branches describe the
/// same way keeps its layout too. One they describe differently has no single
/// shape to name and keeps none.
fn merge_register_layouts(
    yes: &BTreeMap<u32, RegisterObjectLayout>,
    no: &BTreeMap<u32, RegisterObjectLayout>,
) -> BTreeMap<u32, RegisterObjectLayout> {
    yes.iter()
        .chain(no.iter())
        .filter(|(id, layout)| {
            yes.get(id).is_none_or(|own| own == *layout)
                && no.get(id).is_none_or(|own| own == *layout)
        })
        .map(|(id, layout)| (*id, layout.clone()))
        .collect()
}

fn merge_register_bindings(
    left: &BTreeMap<String, RegisterBinding>,
    right: &BTreeMap<String, RegisterBinding>,
) -> Option<BTreeMap<String, RegisterBinding>> {
    if left.len() != right.len() {
        return None;
    }
    left.iter()
        .map(|(name, left)| {
            let right = right.get(name)?;
            if left.storage != right.storage
                || left.mutable != right.mutable
                || left.stable_function_identity != right.stable_function_identity
            {
                return None;
            }
            let value_type = match (left.value_type, right.value_type) {
                (Some(left), Some(right)) => Some(left.merge(right)),
                (None, None) => None,
                (Some(_), None) | (None, Some(_)) => return None,
            };
            Some((
                name.clone(),
                RegisterBinding {
                    value_type,
                    ..*left
                },
            ))
        })
        .collect()
}

fn register_bindings_fit(
    actual: &BTreeMap<String, RegisterBinding>,
    expected: &BTreeMap<String, RegisterBinding>,
) -> bool {
    merge_register_bindings(actual, expected).as_ref() == Some(expected)
}

/// Whether the bindings a jump leaves carry every binding its target has, with
/// a type that fits it.
///
/// 14.9 and 14.10 leave every block between the jump and its target, so a
/// lexical declaration of one of those blocks is a name the target does not
/// have and the comparison does not carry.
fn register_bindings_reach(
    actual: &BTreeMap<String, RegisterBinding>,
    expected: &BTreeMap<String, RegisterBinding>,
) -> bool {
    expected.iter().all(|(name, expected)| {
        actual.get(name).is_some_and(|actual| {
            actual.storage == expected.storage
                && actual.mutable == expected.mutable
                && actual.stable_function_identity == expected.stable_function_identity
                && match (actual.value_type, expected.value_type) {
                    (Some(actual), Some(expected)) => actual.merge(expected) == expected,
                    (None, None) => true,
                    (Some(_), None) | (None, Some(_)) => false,
                }
        })
    })
}

fn register_context_bindings_unchanged(
    before: &BTreeMap<String, RegisterBinding>,
    after: &BTreeMap<String, RegisterBinding>,
) -> bool {
    before.iter().all(|(name, binding)| {
        !matches!(binding.storage, RegisterBindingStorage::Context { .. })
            || after.get(name) == Some(binding)
    })
}

fn register_body_var_names(body: &[Stmt]) -> Option<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for statement in body {
        register_statement_var_names(statement, &mut names, false)?;
    }
    Some(names)
}

fn register_body_initialized_var_names(body: &[Stmt]) -> Option<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for statement in body {
        register_statement_var_names(statement, &mut names, true)?;
    }
    Some(names)
}

fn register_statement_var_names(
    statement: &Stmt,
    names: &mut BTreeSet<String>,
    initialized_only: bool,
) -> Option<()> {
    match statement {
        Stmt::Var(bindings) => {
            for (pattern, initializer) in bindings {
                if !initialized_only || initializer.is_some() {
                    let mut bound = Vec::new();
                    pattern.names(&mut bound);
                    names.extend(bound);
                }
            }
        }
        Stmt::Block(body) => {
            for statement in body {
                register_statement_var_names(statement, names, initialized_only)?;
            }
        }
        Stmt::If(_, yes, no) => {
            register_statement_var_names(yes, names, initialized_only)?;
            if let Some(no) = no {
                register_statement_var_names(no, names, initialized_only)?;
            }
        }
        Stmt::While(_, body) | Stmt::DoWhile(body, _) => {
            register_statement_var_names(body, names, initialized_only)?;
        }
        Stmt::For(initializer, _, _, body) => {
            register_statement_var_names(initializer, names, initialized_only)?;
            register_statement_var_names(body, names, initialized_only)?;
        }
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            for statement in try_statements(body, catch.as_ref(), finally.as_deref()) {
                register_statement_var_names(statement, names, initialized_only)?;
            }
        }
        Stmt::Switch(_, clauses) => {
            for statement in clauses.iter().flat_map(|(_, body)| body) {
                register_statement_var_names(statement, names, initialized_only)?;
            }
        }
        Stmt::ForIn { binding, body, .. } | Stmt::ForOf { binding, body, .. } => {
            // 8.2.7: a `var` head is a var name of the body. The head writes it
            // once per iteration, so an enumeration that produces nothing
            // leaves it as the declaration did: it is not an initialized name.
            if let Some((pattern, None)) = binding
                && !initialized_only
            {
                let mut bound = Vec::new();
                pattern.names(&mut bound);
                names.extend(bound);
            }
            register_statement_var_names(body, names, initialized_only)?;
        }
        Stmt::Empty
        | Stmt::Expr(_)
        | Stmt::Declare(_)
        | Stmt::Function(_, _)
        | Stmt::Return(_)
        | Stmt::Throw(_)
        | Stmt::Break
        | Stmt::Continue => {}
    }
    Some(())
}

/// Every statement of a `try` statement's three Blocks, in evaluation order.
fn try_statements<'a>(
    body: &'a [Stmt],
    catch: Option<&'a (Option<BindingPattern>, Vec<Stmt>)>,
    finally: Option<&'a [Stmt]>,
) -> impl Iterator<Item = &'a Stmt> {
    body.iter()
        .chain(catch.into_iter().flat_map(|(_, body)| body.iter()))
        .chain(finally.into_iter().flatten())
}

/// Whether a statement holds a `break` or a `continue`, which 14.15.3 would
/// have to run a Finally Block before.
fn register_statement_breaks_control(statement: &Stmt) -> bool {
    match statement {
        Stmt::Break | Stmt::Continue => true,
        Stmt::Block(body) => body.iter().any(register_statement_breaks_control),
        Stmt::If(_, yes, no) => {
            register_statement_breaks_control(yes)
                || no.as_deref().is_some_and(register_statement_breaks_control)
        }
        Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::For(_, _, _, body)
        | Stmt::ForIn { body, .. }
        | Stmt::ForOf { body, .. } => register_statement_breaks_control(body),
        Stmt::Try {
            body,
            catch,
            finally,
        } => try_statements(body, catch.as_ref(), finally.as_deref())
            .any(register_statement_breaks_control),
        Stmt::Switch(_, clauses) => clauses
            .iter()
            .flat_map(|(_, body)| body)
            .any(register_statement_breaks_control),
        _ => false,
    }
}

fn register_statement_transfers_control(statement: &Stmt) -> bool {
    match statement {
        Stmt::Break | Stmt::Continue | Stmt::Return(_) => true,
        Stmt::Block(body) => body.iter().any(register_statement_transfers_control),
        Stmt::If(_, yes, no) => {
            register_statement_transfers_control(yes)
                || no
                    .as_deref()
                    .is_some_and(register_statement_transfers_control)
        }
        Stmt::While(_, body)
        | Stmt::DoWhile(body, _)
        | Stmt::For(_, _, _, body)
        | Stmt::ForIn { body, .. }
        | Stmt::ForOf { body, .. } => register_statement_transfers_control(body),
        Stmt::Try {
            body,
            catch,
            finally,
        } => try_statements(body, catch.as_ref(), finally.as_deref())
            .any(register_statement_transfers_control),
        Stmt::Switch(_, clauses) => clauses
            .iter()
            .flat_map(|(_, body)| body)
            .any(register_statement_transfers_control),
        _ => false,
    }
}

/// Merges the type of every assignment an expression performs into the binding
/// it writes.
///
/// A target this cannot type widens to the unknown type, which every other
/// type merges into, so a binding a loop body writes with a value the lowering
/// could not name keeps that type across the back edge.
fn infer_register_assignment_types(
    expression: &Expr,
    bindings: &mut BTreeMap<String, RegisterBinding>,
) {
    match &expression.kind {
        ExprKind::Assign(name, operator, value) => {
            infer_register_assignment_types(value, bindings);
            // A compound assignment answers what its operator answers, not what
            // its right side holds, so only a plain one is typed here.
            let assigned = if operator.is_none() {
                register_expression_type(value, bindings).unwrap_or(RegisterType::Unknown)
            } else {
                RegisterType::Unknown
            };
            if let Some(binding) = bindings.get_mut(name) {
                binding.value_type =
                    merge_optional_register_types(binding.value_type, Some(assigned));
            }
        }
        ExprKind::Group(inner) | ExprKind::Unary(_, inner) => {
            infer_register_assignment_types(inner, bindings);
        }
        ExprKind::Sequence(left, right) | ExprKind::Binary(_, left, right) => {
            infer_register_assignment_types(left, bindings);
            infer_register_assignment_types(right, bindings);
        }
        ExprKind::Conditional(condition, yes, no) => {
            infer_register_assignment_types(condition, bindings);
            infer_register_assignment_types(yes, bindings);
            infer_register_assignment_types(no, bindings);
        }
        ExprKind::Call(callee, arguments) => {
            infer_register_assignment_types(callee, bindings);
            for argument in arguments {
                infer_register_assignment_types(argument, bindings);
            }
        }
        ExprKind::Member(base, key) => {
            infer_register_assignment_types(base, bindings);
            infer_register_assignment_types(key, bindings);
        }
        ExprKind::SetMember(base, _, value, _) => {
            infer_register_assignment_types(base, bindings);
            infer_register_assignment_types(value, bindings);
        }
        _ => {}
    }
}

/// Widens the tracked type of every `var` a statement writes.
///
/// `widen` additionally follows the assignments of an expression statement, a
/// return and a throw, so that a loop head can start from the types its body
/// produces. It is a hint: the fit check across the back edge is what makes a
/// loop sound, so a hint that is too narrow refuses and never miscompiles.
fn infer_register_var_types(
    statement: &Stmt,
    bindings: &mut BTreeMap<String, RegisterBinding>,
    widen: bool,
) -> Option<()> {
    match statement {
        Stmt::Var(declarations) => {
            for (pattern, initializer) in declarations {
                let Some(initializer) = initializer else {
                    continue;
                };
                let Some(name) = pattern.identifier() else {
                    continue;
                };
                let observed = register_expression_type(initializer, bindings)
                    .unwrap_or(RegisterType::Unknown);
                // 16.1.7 puts a `var` of a Realm Script on the Global
                // Environment Record, where it carries no tracked type and
                // this pass has nothing to widen.
                let Some(binding) = bindings.get_mut(name) else {
                    continue;
                };
                binding.value_type =
                    merge_optional_register_types(binding.value_type, Some(observed));
            }
        }
        Stmt::Block(body) => {
            for statement in body {
                infer_register_var_types(statement, bindings, widen)?;
            }
        }
        Stmt::If(_, yes, no) => {
            infer_register_var_types(yes, bindings, widen)?;
            if let Some(no) = no {
                infer_register_var_types(no, bindings, widen)?;
            }
        }
        Stmt::While(_, body) | Stmt::DoWhile(body, _) => {
            infer_register_var_types(body, bindings, widen)?;
        }
        // 14.7.5.6 step 7.g and 14.7.5.7 write the head of each step, so a
        // `var` head carries the top of the lattice from the head on. The
        // declaration itself is hoisted and says only that the name exists.
        Stmt::ForIn { binding, body, .. } | Stmt::ForOf { binding, body, .. } => {
            if let Some((pattern, None)) = binding {
                let mut names = Vec::new();
                pattern.names(&mut names);
                for name in names {
                    if let Some(declared) = bindings.get_mut(&name) {
                        declared.value_type = Some(RegisterType::Unknown);
                    }
                }
            }
            infer_register_var_types(body, bindings, widen)?;
        }
        Stmt::For(initializer, _, _, body) => {
            infer_register_var_types(initializer, bindings, widen)?;
            infer_register_var_types(body, bindings, widen)?;
        }
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            for statement in try_statements(body, catch.as_ref(), finally.as_deref()) {
                infer_register_var_types(statement, bindings, widen)?;
            }
        }
        Stmt::Switch(_, clauses) => {
            for statement in clauses.iter().flat_map(|(_, body)| body) {
                infer_register_var_types(statement, bindings, widen)?;
            }
        }
        Stmt::Expr(expression) | Stmt::Throw(expression) if widen => {
            infer_register_assignment_types(expression, bindings);
        }
        Stmt::Return(Some(expression)) if widen => {
            infer_register_assignment_types(expression, bindings);
        }
        Stmt::Empty
        | Stmt::Expr(_)
        | Stmt::Declare(_)
        | Stmt::Function(_, _)
        | Stmt::Return(_)
        | Stmt::Throw(_)
        | Stmt::Break
        | Stmt::Continue => {}
    }
    Some(())
}

fn infer_register_body_var_types_to_fixed_point(
    body: &[Stmt],
    bindings: &mut BTreeMap<String, RegisterBinding>,
) -> Option<()> {
    loop {
        let before = bindings.clone();
        for statement in body {
            infer_register_var_types(statement, bindings, false)?;
        }
        if *bindings == before {
            return Some(());
        }
    }
}

fn infer_register_var_types_to_fixed_point(
    statement: &Stmt,
    bindings: &mut BTreeMap<String, RegisterBinding>,
    widen: bool,
) -> Option<()> {
    loop {
        let before = bindings.clone();
        infer_register_var_types(statement, bindings, widen)?;
        if *bindings == before {
            return Some(());
        }
    }
}

fn register_statement_writes_names(statement: &Stmt, names: &BTreeSet<String>) -> Option<bool> {
    Some(match statement {
        Stmt::Expr(expression) | Stmt::Return(Some(expression)) => {
            register_expression_writes_names(expression, names)?
        }
        Stmt::Declare(bindings) => {
            for (pattern, _, initializer) in bindings {
                if pattern.contains_expression() {
                    return None;
                }
                if let Some(initializer) = initializer
                    && register_expression_writes_names(initializer, names)?
                {
                    return Some(true);
                }
            }
            false
        }
        Stmt::Var(bindings) => {
            for (pattern, initializer) in bindings {
                if pattern.contains_expression() {
                    return None;
                }
                if let Some(initializer) = initializer
                    && register_expression_writes_names(initializer, names)?
                {
                    return Some(true);
                }
            }
            false
        }
        Stmt::Block(body) => {
            for statement in body {
                if register_statement_writes_names(statement, names)? {
                    return Some(true);
                }
            }
            false
        }
        Stmt::If(condition, yes, no) => {
            register_expression_writes_names(condition, names)?
                || register_statement_writes_names(yes, names)?
                || if let Some(no) = no {
                    register_statement_writes_names(no, names)?
                } else {
                    false
                }
        }
        Stmt::While(condition, body) => {
            register_expression_writes_names(condition, names)?
                || register_statement_writes_names(body, names)?
        }
        Stmt::DoWhile(body, condition) => {
            register_statement_writes_names(body, names)?
                || register_expression_writes_names(condition, names)?
        }
        Stmt::For(initializer, condition, step, body) => {
            register_statement_writes_names(initializer, names)?
                || if let Some(condition) = condition {
                    register_expression_writes_names(condition, names)?
                } else {
                    false
                }
                || if let Some(step) = step {
                    register_expression_writes_names(step, names)?
                } else {
                    false
                }
                || register_statement_writes_names(body, names)?
        }
        Stmt::Function(_, function) => register_function_writes_names(function, names)?,
        Stmt::Throw(value) => register_expression_writes_names(value, names)?,
        Stmt::Try { .. } | Stmt::Switch(_, _) | Stmt::ForIn { .. } | Stmt::ForOf { .. } => {
            register_scoped_statement_writes_names(statement, names)?
        }
        Stmt::Empty | Stmt::Return(None) | Stmt::Break | Stmt::Continue => false,
    })
}

/// Whether one of the statements that own a scope writes any of `names`.
fn register_scoped_statement_writes_names(
    statement: &Stmt,
    names: &BTreeSet<String>,
) -> Option<bool> {
    Some(match statement {
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            let mut writes = false;
            for statement in try_statements(body, catch.as_ref(), finally.as_deref()) {
                writes = writes || register_statement_writes_names(statement, names)?;
            }
            writes
        }
        Stmt::Switch(discriminant, clauses) => {
            let mut writes = register_expression_writes_names(discriminant, names)?;
            for (test, body) in clauses {
                if let Some(test) = test {
                    writes = writes || register_expression_writes_names(test, names)?;
                }
                for statement in body {
                    writes = writes || register_statement_writes_names(statement, names)?;
                }
            }
            writes
        }
        Stmt::ForIn {
            binding,
            target,
            object,
            body,
        }
        | Stmt::ForOf {
            binding,
            target,
            object,
            body,
        } => {
            // A `var` head writes its binding once per iteration; a lexical
            // head makes one of its own and writes nothing outside the loop.
            if let Some((pattern, None)) = binding {
                let mut bound = Vec::new();
                pattern.names(&mut bound);
                if bound.iter().any(|name| names.contains(name)) {
                    return Some(true);
                }
            }
            // A head that declares nothing writes every name its target does.
            if let Some(target) = target {
                let mut written = Vec::new();
                register_assignment_target_names(target, &mut written);
                if written.iter().any(|name| names.contains(*name)) {
                    return Some(true);
                }
            }
            register_expression_writes_names(object, names)?
                || register_statement_writes_names(body, names)?
        }
        _ => return None,
    })
}

/// Whether a function body resolves `this` on its own Function Environment
/// Record (9.4.5).
///
/// A nested ordinary function has a record of its own, so the walk stops there.
/// An arrow function has none and takes the one of this body, so the walk
/// follows it.
/// The name 10.4.4 binds in every ordinary function.
const ARGUMENTS: &str = "arguments";

/// The name 13.3.6.1 makes a call a direct eval.
const EVAL_NAME: &str = "eval";

/// Whether a body reads `arguments` and no function inside it does.
///
/// An arrow has no `arguments` of its own (10.2.1.1), so one that names it
/// would read the object of this frame from a context this step does not
/// build.
fn register_body_reads_arguments(body: &[Stmt]) -> Option<bool> {
    let mut names = BTreeSet::new();
    let mut nested = BTreeSet::new();
    let mut captured = BTreeSet::new();
    for statement in body {
        register_statement_references(statement, &mut names, &mut nested, &mut captured)?;
    }
    // 10.2.1.1 gives an arrow no object of its own, so one that reads
    // `arguments` reads the object of the function it stands in, which that
    // function then has to build.
    Some(names.contains(ARGUMENTS) || nested.contains(ARGUMENTS))
}

/// Whether an Initializer of 8.6.2 reads the parameter it binds or one the
/// list binds after it, which 10.2.11 leaves in its temporal dead zone.
fn register_initializer_reads_a_later_parameter(function: &Function) -> bool {
    let mut bound: Vec<BTreeSet<String>> = Vec::with_capacity(function.parameters.len());
    for parameter in &function.parameters {
        let mut names = Vec::new();
        parameter.pattern.names(&mut names);
        bound.push(names.into_iter().collect());
    }
    for (index, parameter) in function.parameters.iter().enumerate() {
        let Some(default) = &parameter.default else {
            continue;
        };
        let mut direct = BTreeSet::new();
        let mut nested = BTreeSet::new();
        if register_expression_references(default, &mut direct, &mut nested).is_none() {
            continue;
        }
        let dead = bound.get(index..).unwrap_or_default();
        if dead
            .iter()
            .any(|names| names.iter().any(|name| direct.contains(name)))
        {
            return true;
        }
    }
    false
}

/// Whether 10.4.4.7 maps the indices of this function's arguments object.
///
/// The clause maps a simple parameter list, and this lowering maps one whose
/// names are distinct, so that each parameter holds a slot of its own, and no
/// more than the 64 the object carries one bit each for.
fn register_maps_its_parameters(function: &Function) -> bool {
    if function.parameters.len() > 64 {
        return false;
    }
    let mut seen = BTreeSet::new();
    function.parameters.iter().all(|parameter| {
        let parser::BindingPattern::Name(name) = &parameter.pattern else {
            return false;
        };
        parameter.default.is_none() && !parameter.rest && seen.insert(name.clone())
    })
}

/// Whether a body assigns one of the parameters, which 10.4.4.7 would show in
/// the arguments object.
fn register_body_writes_parameters(body: &[Stmt], function: &Function) -> Option<bool> {
    let mut parameters = BTreeSet::new();
    for parameter in &function.parameters {
        let mut bound = Vec::new();
        parameter.names(&mut bound);
        parameters.extend(bound);
    }
    if parameters.is_empty() {
        return Some(false);
    }
    for statement in body {
        if register_statement_writes_names(statement, &parameters)? {
            return Some(true);
        }
    }
    Some(false)
}

/// Whether the value of a property definition is a method whose body reads
/// `super`, which 10.2.11 gives the object it is defined on.
fn register_is_method_reading_super(value: &Expr) -> bool {
    let ExprKind::Function(function) = &value.kind else {
        return false;
    };
    // 15.4.4 makes a method non-constructible, which is what tells `m(){}`
    // from `m: function(){}`.
    !function.arrow && !function.constructible && register_body_reads_super(&function.body)
}

/// What a scan of a body looks for, because 9.4.5 and 13.3.7.3 read two
/// different values of the same Function Environment Record.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reads {
    /// The `this` value of 9.4.5.
    This,
    /// The `[[HomeObject]]` 13.3.7.3 reads the Prototype of.
    Super,
    /// The `[[NewTarget]]` of 9.4.3.
    NewTarget,
}

fn register_body_reads_this(body: &[Stmt]) -> bool {
    register_body_reads(body, Reads::This)
}

/// Whether a body reads `super`, which 13.3.7.3 answers out of the
/// `[[HomeObject]]` of the running function.
fn register_body_reads_super(body: &[Stmt]) -> bool {
    register_body_reads(body, Reads::Super)
}

/// Whether a body reads `new.target`, which 9.4.3 answers out of the call.
fn register_body_reads_new_target(body: &[Stmt]) -> bool {
    register_body_reads(body, Reads::NewTarget)
}

fn register_body_reads(body: &[Stmt], what: Reads) -> bool {
    body.iter()
        .any(|statement| register_statement_reads(statement, what))
}

fn register_statement_reads(statement: &Stmt, what: Reads) -> bool {
    match statement {
        Stmt::Expr(expression) | Stmt::Throw(expression) => {
            register_expression_reads(expression, what)
        }
        Stmt::Return(expression) => expression
            .as_ref()
            .is_some_and(|expression| register_expression_reads(expression, what)),
        Stmt::Block(body) => register_body_reads(body, what),
        Stmt::Declare(bindings) => bindings
            .iter()
            .filter_map(|(_, _, initializer)| initializer.as_ref())
            .any(|expression| register_expression_reads(expression, what)),
        Stmt::Var(bindings) => bindings
            .iter()
            .filter_map(|(_, initializer)| initializer.as_ref())
            .any(|expression| register_expression_reads(expression, what)),
        Stmt::If(condition, yes, no) => {
            register_expression_reads(condition, what)
                || register_statement_reads(yes, what)
                || no
                    .as_deref()
                    .is_some_and(|statement| register_statement_reads(statement, what))
        }
        Stmt::While(condition, body) | Stmt::DoWhile(body, condition) => {
            register_expression_reads(condition, what) || register_statement_reads(body, what)
        }
        Stmt::For(initializer, condition, step, body) => {
            register_statement_reads(initializer, what)
                || condition
                    .as_ref()
                    .is_some_and(|expression| register_expression_reads(expression, what))
                || step
                    .as_ref()
                    .is_some_and(|expression| register_expression_reads(expression, what))
                || register_statement_reads(body, what)
        }
        Stmt::ForIn { object, body, .. } | Stmt::ForOf { object, body, .. } => {
            register_expression_reads(object, what) || register_statement_reads(body, what)
        }
        Stmt::Switch(discriminant, clauses) => {
            register_expression_reads(discriminant, what)
                || clauses.iter().any(|(test, body)| {
                    test.as_ref()
                        .is_some_and(|expression| register_expression_reads(expression, what))
                        || register_body_reads(body, what)
                })
        }
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            register_body_reads(body, what)
                || catch
                    .as_ref()
                    .is_some_and(|(_, body)| register_body_reads(body, what))
                || finally.as_deref().is_some_and(register_body_reads_this)
        }
        Stmt::Function(_, function) => function.arrow && register_body_reads(&function.body, what),
        Stmt::Empty | Stmt::Break | Stmt::Continue => false,
    }
}

fn register_expression_reads(expression: &Expr, what: Reads) -> bool {
    match &expression.kind {
        // A class body the lowering does not take at all.
        // A class body the lowering takes as a unit of its own, and the
        // default constructor of 15.7.14 reads all three.
        ExprKind::Class(_) | ExprKind::DefaultSuper => true,
        ExprKind::Super => !matches!(what, Reads::NewTarget),
        ExprKind::NewTarget => matches!(what, Reads::This | Reads::NewTarget),
        ExprKind::This => matches!(what, Reads::This),
        ExprKind::BigInt(..)
        | ExprKind::Literal(_)
        | ExprKind::Name(_)
        | ExprKind::Regex(_, _)
        | ExprKind::Update(..) => false,
        ExprKind::Group(inner)
        | ExprKind::Unary(_, inner)
        | ExprKind::Await(inner)
        | ExprKind::Spread(inner)
        | ExprKind::Assign(_, _, inner)
        | ExprKind::Destructure(_, inner)
        | ExprKind::UpdateMember(inner, _, _, _) => register_expression_reads(inner, what),
        ExprKind::Sequence(left, right)
        | ExprKind::Binary(_, left, right)
        | ExprKind::Member(left, right) => {
            register_expression_reads(left, what) || register_expression_reads(right, what)
        }
        ExprKind::SetMember(target, _, value, _) => {
            register_expression_reads(target, what) || register_expression_reads(value, what)
        }
        ExprKind::Conditional(condition, yes, no) => {
            register_expression_reads(condition, what)
                || register_expression_reads(yes, what)
                || register_expression_reads(no, what)
        }
        ExprKind::Call(callee, arguments) | ExprKind::Construct(callee, arguments) => {
            register_expression_reads(callee, what)
                || arguments
                    .iter()
                    .any(|expression| register_expression_reads(expression, what))
        }
        ExprKind::Template(_, parts) => parts
            .iter()
            .any(|(expression, _)| register_expression_reads(expression, what)),
        ExprKind::Object(properties) => properties.iter().any(|property| {
            register_expression_reads(&property.key, what)
                || register_expression_reads(&property.value, what)
        }),
        ExprKind::Array(items) => items
            .iter()
            .flatten()
            .any(|expression| register_expression_reads(expression, what)),
        ExprKind::Function(function) => function.arrow && register_body_reads(&function.body, what),
    }
}

fn register_expression_writes_names(expression: &Expr, names: &BTreeSet<String>) -> Option<bool> {
    Some(match &expression.kind {
        ExprKind::Assign(name, _, value) => {
            names.contains(name) || register_expression_writes_names(value, names)?
        }
        ExprKind::Update(name, _, _) => names.contains(name),
        ExprKind::Sequence(left, right)
        | ExprKind::Binary(_, left, right)
        | ExprKind::Member(left, right) => {
            register_expression_writes_names(left, names)?
                || register_expression_writes_names(right, names)?
        }
        // 27.7.5.3 and 13.2.4.2 evaluate their operand like any other
        // expression, as a group and a unary operator do.
        ExprKind::Group(inner)
        | ExprKind::Unary(_, inner)
        | ExprKind::Await(inner)
        | ExprKind::Spread(inner) => register_expression_writes_names(inner, names)?,
        ExprKind::Conditional(condition, yes, no) => {
            register_expression_writes_names(condition, names)?
                || register_expression_writes_names(yes, names)?
                || register_expression_writes_names(no, names)?
        }
        ExprKind::Call(callee, arguments) | ExprKind::Construct(callee, arguments) => {
            if register_expression_writes_names(callee, names)? {
                return Some(true);
            }
            for argument in arguments {
                if register_expression_writes_names(argument, names)? {
                    return Some(true);
                }
            }
            false
        }
        ExprKind::Object(properties) => {
            for property in properties {
                if register_expression_writes_names(&property.key, names)?
                    || register_expression_writes_names(&property.value, names)?
                {
                    return Some(true);
                }
            }
            false
        }
        ExprKind::Array(items) => {
            for item in items.iter().flatten() {
                if register_expression_writes_names(item, names)? {
                    return Some(true);
                }
            }
            false
        }
        ExprKind::SetMember(target, _, value, _) => {
            register_expression_writes_names(target, names)?
                || register_expression_writes_names(value, names)?
        }
        // 13.4.4.1 writes a property and not a binding, so only what names
        // the property can write one of these names.
        ExprKind::UpdateMember(target, _, _, _) => register_expression_writes_names(target, names)?,
        ExprKind::Function(function) => register_function_writes_names(function, names)?,
        // 15.7.14 is the heritage and one function for each member, so a class
        // writes what any of those writes.
        ExprKind::Class(class) => register_class_writes_names(class, names)?,
        // `this` is resolved on the Function Environment Record, so it writes
        // and names no binding of this analysis.
        // 22.2.4.1 makes an object of a pattern this unit compiled, and
        // reaches no binding of this analysis.
        // 13.3.7.3 answers `super` out of the `[[HomeObject]]` of the running
        // function, which is no binding of the body.
        ExprKind::BigInt(..)
        | ExprKind::Literal(_)
        | ExprKind::Name(_)
        | ExprKind::This
        | ExprKind::Super
        | ExprKind::DefaultSuper
        | ExprKind::NewTarget
        | ExprKind::Regex(_, _) => false,
        ExprKind::Destructure(pattern, right) => {
            register_expression_writes_names(right, names)?
                || register_assignment_pattern_writes_names(pattern, names)?
        }
        ExprKind::Template(_, parts) => {
            for (expression, _) in parts {
                if register_expression_writes_names(expression, names)? {
                    return Some(true);
                }
            }
            false
        }
    })
}

fn register_assignment_pattern_writes_names(
    pattern: &parser::AssignmentPattern,
    names: &BTreeSet<String>,
) -> Option<bool> {
    match pattern {
        parser::AssignmentPattern::Target(target) => {
            if let Some(name) = target.reference_name() {
                Some(names.contains(name))
            } else {
                register_expression_writes_names(target, names)
            }
        }
        parser::AssignmentPattern::Array(array) => {
            for element in &array.elements {
                if let parser::AssignmentArrayElement::Element {
                    target,
                    initializer,
                } = element
                {
                    if register_assignment_pattern_writes_names(target, names)? {
                        return Some(true);
                    }
                    if let Some(initializer) = initializer
                        && register_expression_writes_names(initializer, names)?
                    {
                        return Some(true);
                    }
                }
            }
            if let Some(rest) = &array.rest {
                return register_assignment_pattern_writes_names(rest, names);
            }
            Some(false)
        }
        parser::AssignmentPattern::Object(object) => {
            for property in &object.properties {
                if register_expression_writes_names(&property.key, names)?
                    || register_assignment_pattern_writes_names(&property.target, names)?
                {
                    return Some(true);
                }
                if let Some(initializer) = &property.initializer
                    && register_expression_writes_names(initializer, names)?
                {
                    return Some(true);
                }
            }
            object.rest.as_ref().map_or(Some(false), |rest| {
                if let Some(name) = rest.reference_name() {
                    Some(names.contains(name))
                } else {
                    register_expression_writes_names(rest, names)
                }
            })
        }
    }
}

/// Whether any part of a class body writes one of these names (15.7.14).
fn register_class_writes_names(class: &parser::Class, names: &BTreeSet<String>) -> Option<bool> {
    if let Some(heritage) = &class.heritage
        && register_expression_writes_names(heritage, names)?
    {
        return Some(true);
    }
    if register_function_writes_names(&class.constructor, names)? {
        return Some(true);
    }
    for (_, method) in &class.methods {
        if register_expression_writes_names(&method.key, names)?
            || register_expression_writes_names(&method.value, names)?
        {
            return Some(true);
        }
    }
    Some(false)
}

/// The names a class body reads, and the free names of the functions it holds.
fn register_class_references(
    class: &parser::Class,
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
) -> Option<()> {
    if let Some(heritage) = &class.heritage {
        register_expression_references(heritage, names, nested_free_names)?;
    }
    nested_free_names.extend(register_function_scope(&class.constructor)?.free_names);
    for (_, method) in &class.methods {
        register_expression_references(&method.key, names, nested_free_names)?;
        register_expression_references(&method.value, names, nested_free_names)?;
    }
    Some(())
}

fn register_function_writes_names(function: &Function, names: &BTreeSet<String>) -> Option<bool> {
    let local_names = register_function_local_names(function)?;
    let free_targets: BTreeSet<_> = names.difference(&local_names).cloned().collect();
    if free_targets.is_empty() {
        return Some(false);
    }
    for statement in &function.body {
        if register_statement_writes_names(statement, &free_targets)? {
            return Some(true);
        }
    }
    Some(false)
}

fn merge_optional_register_types(
    left: Option<RegisterType>,
    right: Option<RegisterType>,
) -> Option<RegisterType> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.merge(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

struct RegisterFunctionScope {
    free_names: BTreeSet<String>,
    captured_names: BTreeSet<String>,
}

/// The name the lowering gives the register a pattern parameter's argument
/// arrives in, which no identifier of a Script can be.
/// The name 10.2.10 gives an accessor: the key behind `get ` or `set `.
fn accessor_name(setter: bool, key: &[u16]) -> Vec<u16> {
    let prefix = if setter { "set " } else { "get " };
    prefix.encode_utf16().chain(key.iter().copied()).collect()
}

fn register_argument_name(index: usize) -> String {
    alloc::format!("argument {index}")
}

fn register_function_local_names(function: &Function) -> Option<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    for parameter in &function.parameters {
        let mut bound = Vec::new();
        parameter.names(&mut bound);
        names.extend(bound);
    }
    if let Some(name) = &function.name {
        names.insert(name.clone());
    }
    for name in register_body_var_names(&function.body)? {
        names.insert(name);
    }
    for statement in &function.body {
        match statement {
            Stmt::Declare(bindings) => {
                for (pattern, _, _) in bindings {
                    let mut bound = Vec::new();
                    pattern.names(&mut bound);
                    names.extend(bound);
                }
            }
            Stmt::Function(name, _) => {
                names.insert(name.clone());
            }
            _ => {}
        }
    }
    Some(names)
}

fn register_block_local_names(body: &[Stmt]) -> Option<BTreeMap<String, bool>> {
    let mut names = BTreeMap::new();
    for statement in body {
        match statement {
            Stmt::Declare(bindings) => {
                for (pattern, mutable, _) in bindings {
                    let mut bound = Vec::new();
                    pattern.names(&mut bound);
                    for name in bound {
                        if names.insert(name, *mutable).is_some() {
                            return None;
                        }
                    }
                }
            }
            // 14.2.2 makes a function declaration a binding of the Block,
            // which 14.2.3 initializes before the first statement runs.
            Stmt::Function(name, _) if names.insert(name.clone(), true).is_some() => {
                return None;
            }
            _ => {}
        }
    }
    Some(names)
}

fn register_function_scope(function: &Function) -> Option<RegisterFunctionScope> {
    let mut local_names = register_function_local_names(function)?;
    // 10.4.4 binds `arguments` in every ordinary function, so one nested in
    // another body names its own object and not the one of that body. 10.2.1.1
    // gives an arrow none, which is why only this case is local.
    if !function.arrow {
        local_names.insert(String::from(ARGUMENTS));
    }
    let mut scope = register_body_scope(&function.body, &local_names)?;
    // An Initializer of 8.6.2 runs in the frame of the call, so a name it
    // reads and the body does not is captured just the same.
    let mut direct = BTreeSet::new();
    let mut nested = BTreeSet::new();
    for parameter in &function.parameters {
        if let Some(default) = &parameter.default {
            register_expression_references(default, &mut direct, &mut nested)?;
        }
        register_binding_pattern_references(&parameter.pattern, &mut direct, &mut nested)?;
    }
    for name in direct.into_iter().chain(nested) {
        if local_names.contains(&name) {
            scope.captured_names.insert(name);
        } else {
            scope.free_names.insert(name);
        }
    }
    Some(scope)
}

fn register_body_scope(
    body: &[Stmt],
    local_names: &BTreeSet<String>,
) -> Option<RegisterFunctionScope> {
    let mut direct_references = BTreeSet::new();
    let mut nested_free_names = BTreeSet::new();
    // A binding of a Block, of a Catch or of a `for` head that a nested
    // function reads is captured just as a binding of the body is.
    let mut scoped_captures = BTreeSet::new();
    for statement in body {
        register_statement_references(
            statement,
            &mut direct_references,
            &mut nested_free_names,
            &mut scoped_captures,
        )?;
    }
    let mut free_names: BTreeSet<_> = direct_references.difference(local_names).cloned().collect();
    let mut captured_names = scoped_captures;
    for name in nested_free_names {
        if local_names.contains(&name) {
            captured_names.insert(name);
        } else {
            free_names.insert(name);
        }
    }
    Some(RegisterFunctionScope {
        free_names,
        captured_names,
    })
}

fn register_expression_references(
    expression: &Expr,
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
) -> Option<()> {
    match &expression.kind {
        ExprKind::Name(name) | ExprKind::Assign(name, _, _) | ExprKind::Update(name, _, _) => {
            names.insert(name.clone());
            if let ExprKind::Assign(_, _, value) = &expression.kind {
                register_expression_references(value, names, nested_free_names)?;
            }
        }
        ExprKind::Sequence(left, right)
        | ExprKind::Binary(_, left, right)
        | ExprKind::Member(left, right) => {
            register_expression_references(left, names, nested_free_names)?;
            register_expression_references(right, names, nested_free_names)?;
        }
        ExprKind::Group(inner)
        | ExprKind::Unary(_, inner)
        | ExprKind::Await(inner)
        | ExprKind::Spread(inner) => {
            register_expression_references(inner, names, nested_free_names)?;
        }
        ExprKind::Conditional(condition, yes, no) => {
            register_expression_references(condition, names, nested_free_names)?;
            register_expression_references(yes, names, nested_free_names)?;
            register_expression_references(no, names, nested_free_names)?;
        }
        ExprKind::Call(callee, arguments) | ExprKind::Construct(callee, arguments) => {
            register_expression_references(callee, names, nested_free_names)?;
            for argument in arguments {
                register_expression_references(argument, names, nested_free_names)?;
            }
        }
        ExprKind::Object(properties) => {
            for property in properties {
                register_expression_references(&property.key, names, nested_free_names)?;
                register_expression_references(&property.value, names, nested_free_names)?;
            }
        }
        ExprKind::Array(items) => {
            for item in items.iter().flatten() {
                register_expression_references(item, names, nested_free_names)?;
            }
        }
        ExprKind::SetMember(target, _, value, _) => {
            register_expression_references(target, names, nested_free_names)?;
            register_expression_references(value, names, nested_free_names)?;
        }
        ExprKind::UpdateMember(target, _, _, _) => {
            register_expression_references(target, names, nested_free_names)?;
        }
        ExprKind::Function(function) => {
            nested_free_names.extend(register_function_scope(function)?.free_names);
        }
        ExprKind::Class(class) => register_class_references(class, names, nested_free_names)?,
        // `this` is resolved on the Function Environment Record, so it is free
        // of every name this analysis collects.
        // 13.3.7.3 answers `super` out of the `[[HomeObject]]` of the running
        // function, which is no binding of the body.
        ExprKind::BigInt(..)
        | ExprKind::Literal(_)
        | ExprKind::This
        | ExprKind::Super
        | ExprKind::DefaultSuper
        | ExprKind::NewTarget
        | ExprKind::Regex(_, _) => {}
        ExprKind::Destructure(pattern, right) => {
            register_expression_references(right, names, nested_free_names)?;
            register_assignment_pattern_references(pattern, names, nested_free_names)?;
        }
        ExprKind::Template(_, parts) => {
            for (expression, _) in parts {
                register_expression_references(expression, names, nested_free_names)?;
            }
        }
    }
    Some(())
}

fn register_assignment_pattern_references(
    pattern: &parser::AssignmentPattern,
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
) -> Option<()> {
    match pattern {
        parser::AssignmentPattern::Target(target) => {
            if let Some(name) = target.reference_name() {
                names.insert(String::from(name));
            } else {
                register_expression_references(target, names, nested_free_names)?;
            }
        }
        parser::AssignmentPattern::Array(array) => {
            for element in &array.elements {
                if let parser::AssignmentArrayElement::Element {
                    target,
                    initializer,
                } = element
                {
                    register_assignment_pattern_references(target, names, nested_free_names)?;
                    if let Some(initializer) = initializer {
                        register_expression_references(initializer, names, nested_free_names)?;
                    }
                }
            }
            if let Some(rest) = &array.rest {
                register_assignment_pattern_references(rest, names, nested_free_names)?;
            }
        }
        parser::AssignmentPattern::Object(object) => {
            for property in &object.properties {
                register_expression_references(&property.key, names, nested_free_names)?;
                register_assignment_pattern_references(&property.target, names, nested_free_names)?;
                if let Some(initializer) = &property.initializer {
                    register_expression_references(initializer, names, nested_free_names)?;
                }
            }
            if let Some(rest) = &object.rest {
                if let Some(name) = rest.reference_name() {
                    names.insert(String::from(name));
                } else {
                    register_expression_references(rest, names, nested_free_names)?;
                }
            }
        }
    }
    Some(())
}

fn register_binding_pattern_references(
    pattern: &parser::BindingPattern,
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
) -> Option<()> {
    match pattern {
        parser::BindingPattern::Name(_) => {}
        parser::BindingPattern::Array(array) => {
            for element in &array.elements {
                if let parser::ArrayBindingElement::Element {
                    pattern,
                    initializer,
                } = element
                {
                    register_binding_pattern_references(pattern, names, nested_free_names)?;
                    if let Some(initializer) = initializer {
                        register_expression_references(initializer, names, nested_free_names)?;
                    }
                }
            }
        }
        parser::BindingPattern::Object(object) => {
            for property in &object.properties {
                register_expression_references(&property.key, names, nested_free_names)?;
                register_binding_pattern_references(&property.pattern, names, nested_free_names)?;
                if let Some(initializer) = &property.initializer {
                    register_expression_references(initializer, names, nested_free_names)?;
                }
            }
        }
    }
    Some(())
}

#[expect(
    clippy::too_many_lines,
    reason = "one function names every statement beside the names it reaches"
)]
fn register_statement_references(
    statement: &Stmt,
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
    captured_names: &mut BTreeSet<String>,
) -> Option<()> {
    match statement {
        Stmt::Expr(expression) => {
            register_expression_references(expression, names, nested_free_names)?;
        }
        Stmt::Declare(bindings) => {
            for (pattern, _, expression) in bindings {
                register_binding_pattern_references(pattern, names, nested_free_names)?;
                if let Some(expression) = expression {
                    register_expression_references(expression, names, nested_free_names)?;
                }
            }
        }
        Stmt::Var(bindings) => {
            for (pattern, expression) in bindings {
                register_binding_pattern_references(pattern, names, nested_free_names)?;
                if let Some(expression) = expression {
                    register_expression_references(expression, names, nested_free_names)?;
                }
            }
        }
        Stmt::Block(body) => {
            register_scoped_block_references(
                body,
                &BTreeSet::new(),
                names,
                nested_free_names,
                captured_names,
            )?;
        }
        Stmt::If(condition, yes, no) => {
            register_expression_references(condition, names, nested_free_names)?;
            register_statement_references(yes, names, nested_free_names, captured_names)?;
            if let Some(no) = no {
                register_statement_references(no, names, nested_free_names, captured_names)?;
            }
        }
        Stmt::While(condition, body) => {
            register_expression_references(condition, names, nested_free_names)?;
            register_statement_references(body, names, nested_free_names, captured_names)?;
        }
        Stmt::DoWhile(body, condition) => {
            register_statement_references(body, names, nested_free_names, captured_names)?;
            register_expression_references(condition, names, nested_free_names)?;
        }
        Stmt::For(initializer, condition, step, body) => {
            register_statement_references(initializer, names, nested_free_names, captured_names)?;
            if let Some(condition) = condition {
                register_expression_references(condition, names, nested_free_names)?;
            }
            if let Some(step) = step {
                register_expression_references(step, names, nested_free_names)?;
            }
            register_statement_references(body, names, nested_free_names, captured_names)?;
        }
        Stmt::Return(value) => {
            if let Some(value) = value {
                register_expression_references(value, names, nested_free_names)?;
            }
        }
        Stmt::Function(_, function) => {
            nested_free_names.extend(register_function_scope(function)?.free_names);
        }
        Stmt::Throw(value) => {
            register_expression_references(value, names, nested_free_names)?;
        }
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            // 14.15.1: the try Block, the Catch Block and the Finally Block are
            // separate scopes, and the catch parameter binds only in its Block.
            register_scoped_block_references(
                body,
                &BTreeSet::new(),
                names,
                nested_free_names,
                captured_names,
            )?;
            if let Some((parameter, body)) = catch {
                let mut bound = BTreeSet::new();
                if let Some(parameter) = parameter {
                    let mut declared = Vec::new();
                    parameter.names(&mut declared);
                    bound.extend(declared);
                }
                register_scoped_block_references(
                    body,
                    &bound,
                    names,
                    nested_free_names,
                    captured_names,
                )?;
            }
            if let Some(body) = finally {
                register_scoped_block_references(
                    body,
                    &BTreeSet::new(),
                    names,
                    nested_free_names,
                    captured_names,
                )?;
            }
        }
        Stmt::Switch(_, _) | Stmt::ForIn { .. } | Stmt::ForOf { .. } => {
            register_scoped_statement_references(
                statement,
                names,
                nested_free_names,
                captured_names,
            )?;
        }
        Stmt::Empty | Stmt::Break | Stmt::Continue => {}
    }
    Some(())
}

/// Collects the free names of the two statements whose head owns a scope.
fn register_scoped_statement_references(
    statement: &Stmt,
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
    captured_names: &mut BTreeSet<String>,
) -> Option<()> {
    match statement {
        Stmt::Switch(discriminant, clauses) => {
            // 14.12: one CaseBlock is a single Block scope over every clause.
            register_expression_references(discriminant, names, nested_free_names)?;
            for (test, _) in clauses {
                if let Some(test) = test {
                    register_expression_references(test, names, nested_free_names)?;
                }
            }
            let body: Vec<&Stmt> = clauses.iter().flat_map(|(_, body)| body).collect();
            register_scoped_clause_references(&body, names, nested_free_names, captured_names)?;
        }
        Stmt::ForIn {
            binding,
            target,
            object,
            body,
        }
        | Stmt::ForOf {
            binding,
            target,
            object,
            body,
        } => {
            // 14.7.5.4: the head's declaration binds only in the loop.
            register_expression_references(object, names, nested_free_names)?;
            if let Some(target) = target {
                match target {
                    parser::AssignmentTarget::Reference(reference) => {
                        register_expression_references(reference, names, nested_free_names)?;
                    }
                    parser::AssignmentTarget::Pattern(pattern) => {
                        let mut written = Vec::new();
                        register_assignment_pattern_names(pattern, &mut written);
                        names.extend(written.into_iter().map(String::from));
                    }
                }
            }
            let mut bound = BTreeSet::new();
            if let Some((pattern, _)) = binding {
                let mut declared = Vec::new();
                pattern.names(&mut declared);
                bound.extend(declared);
            }
            let mut direct = BTreeSet::new();
            let mut nested = BTreeSet::new();
            register_statement_references(body, &mut direct, &mut nested, captured_names)?;
            names.extend(direct.difference(&bound).cloned());
            // 14.7.5.6 gives the head binding a copy per iteration, which the
            // lowering refuses to capture.
            captured_names.extend(nested.intersection(&bound).cloned());
            nested_free_names.extend(nested.difference(&bound).cloned());
        }
        _ => return None,
    }
    Some(())
}

/// Collects the free names of one `CaseBlock`, hiding its lexical bindings.
fn register_scoped_clause_references(
    body: &[&Stmt],
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
    captured_names: &mut BTreeSet<String>,
) -> Option<()> {
    let mut local_names = BTreeSet::new();
    for statement in body {
        if let Stmt::Declare(bindings) = statement {
            for (pattern, _, _) in bindings {
                let mut bound = Vec::new();
                pattern.names(&mut bound);
                for name in bound {
                    if !local_names.insert(name) {
                        return None;
                    }
                }
            }
        }
        if matches!(statement, Stmt::Function(_, _)) {
            return None;
        }
    }
    let mut direct = BTreeSet::new();
    let mut nested = BTreeSet::new();
    for statement in body {
        register_statement_references(statement, &mut direct, &mut nested, captured_names)?;
    }
    // A binding of this scope that a nested function reads lives in the
    // context of the enclosing function, which the lowering makes.
    captured_names.extend(nested.intersection(&local_names).cloned());
    names.extend(direct.difference(&local_names).cloned());
    nested_free_names.extend(nested.difference(&local_names).cloned());
    Some(())
}

/// Collects the free names of one Block, hiding its own lexical bindings and
/// any additional `bound` names such as a catch parameter.
fn register_scoped_block_references(
    body: &[Stmt],
    bound: &BTreeSet<String>,
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
    captured_names: &mut BTreeSet<String>,
) -> Option<()> {
    let mut local_names: BTreeSet<_> = register_block_local_names(body)?.into_keys().collect();
    local_names.extend(bound.iter().cloned());
    let mut direct = BTreeSet::new();
    let mut nested = BTreeSet::new();
    for statement in body {
        register_statement_references(statement, &mut direct, &mut nested, captured_names)?;
    }
    // A binding of this scope that a nested function reads lives in the
    // context of the enclosing function, which the lowering makes.
    captured_names.extend(nested.intersection(&local_names).cloned());
    names.extend(direct.difference(&local_names).cloned());
    nested_free_names.extend(nested.difference(&local_names).cloned());
    Some(())
}

fn register_script_features(body: &[Stmt], realm: bool) -> Option<(bool, bool)> {
    let mut saw_expression = false;
    let mut saw_declaration = false;
    let mut saw_function = false;
    for statement in body {
        match statement {
            // 16.1.7 puts a lexical declaration of a Realm Script on the
            // [[DeclarativeRecord]] of the Global Environment Record, which
            // outlives the Script, so it is no binding of this lowering and
            // needs none of the checks one of a Script without a Realm needs.
            Stmt::Declare(bindings) => {
                if !realm {
                    if saw_expression {
                        return None;
                    }
                    saw_declaration = true;
                    let mut names = BTreeSet::new();
                    for (pattern, _, _) in bindings {
                        let mut bound = Vec::new();
                        pattern.names(&mut bound);
                        for name in bound {
                            if !names.insert(name) {
                                return None;
                            }
                        }
                    }
                }
            }
            Stmt::Function(_, _) if realm => {}
            Stmt::Function(_, _) => saw_function = true,
            Stmt::Expr(_)
            | Stmt::Block(_)
            | Stmt::If(_, _, _)
            | Stmt::While(_, _)
            | Stmt::DoWhile(_, _)
            | Stmt::Throw(_)
            | Stmt::Try { .. }
            | Stmt::Switch(_, _)
            | Stmt::ForIn { .. }
            | Stmt::ForOf { .. }
            | Stmt::For(_, _, _, _) => saw_expression = true,
            Stmt::Empty | Stmt::Var(_) => {}
            _ => return None,
        }
    }
    // A Script of a Realm that only declares is still a Script the lowering
    // takes: 16.1.7 is the work it does.
    (saw_expression
        || realm
        || body
            .iter()
            .any(|statement| matches!(statement, Stmt::Var(_))))
    .then_some((saw_declaration, saw_function))
}

fn prepare_register_bindings(
    lowerer: &mut RegisterLowerer,
    body: &[Stmt],
    saw_declaration: bool,
    saw_function: bool,
) -> Option<()> {
    if saw_declaration || saw_function {
        for statement in body {
            if let Stmt::Declare(bindings) = statement {
                for (pattern, mutable, _) in bindings {
                    let mut names = Vec::new();
                    pattern.names(&mut names);
                    for name in names {
                        lowerer.declare(&name, *mutable)?;
                    }
                }
            } else if let Stmt::Function(name, _) = statement
                && !lowerer.bindings.contains_key(name)
            {
                lowerer.declare(name, true)?;
            }
        }
    }
    lowerer.prepare_var_bindings(body)?;
    lowerer.infer_binding_type_hints(body);
    if saw_function {
        for statement in body {
            if let Stmt::Function(name, function) = statement {
                let value_type = lowerer.lower_function_declaration(name, function)?;
                let binding = *lowerer.bindings.get(name)?;
                lowerer.store_binding(binding);
                lowerer.bindings.get_mut(name)?.value_type = Some(value_type);
            }
        }
    }
    Some(())
}

/// An array pattern the iterator walk takes: the Binding form of 8.6.2 or the
/// Assignment form of 13.15.5.5.
enum RegisterArrayPattern<'a> {
    Binding(&'a parser::ArrayBindingPattern),
    Assignment(&'a parser::AssignmentArrayPattern),
}

impl RegisterArrayPattern<'_> {
    /// How many elements the walk steps over before the rest element.
    const fn len(&self) -> usize {
        match self {
            Self::Binding(binding) => binding.elements.len(),
            Self::Assignment(assignment) => assignment.elements.len(),
        }
    }

    const fn has_rest(&self) -> bool {
        match self {
            Self::Binding(binding) => binding.rest.is_some(),
            Self::Assignment(assignment) => assignment.rest.is_some(),
        }
    }
}

/// The name a refused expression reports.
///
/// The names are of the grammar, not of the lowering, because a Script is
/// refused for what it holds and not for how this file is written.
const fn expression_refusal(kind: &ExprKind) -> &'static str {
    match kind {
        ExprKind::Sequence(..) => "a sequence expression",
        ExprKind::Literal(_) => "a literal",
        ExprKind::BigInt(..) => "a BigInt literal",
        ExprKind::Regex(..) => "a regular-expression literal",
        ExprKind::Template(..) => "a template literal",
        ExprKind::Await(_) => "await",
        ExprKind::Name(_) => "a name",
        ExprKind::Group(_) => "a parenthesised expression",
        ExprKind::Unary(..) => "a unary operator",
        ExprKind::Binary(..) => "a binary operator",
        ExprKind::Assign(..) => "an assignment to a name",
        ExprKind::Destructure(..) => "a destructuring assignment",
        ExprKind::Update(..) => "an update of a name",
        ExprKind::Conditional(..) => "a conditional expression",
        ExprKind::Call(..) => "a call",
        ExprKind::Construct(..) => "new",
        ExprKind::Function(_) => "a function expression",
        ExprKind::Class(_) => "a class expression",
        ExprKind::Super => "super",
        ExprKind::NewTarget => "new.target",
        ExprKind::DefaultSuper => "the default constructor of a derived class",
        ExprKind::This => "this",
        ExprKind::Object(_) => "an object literal",
        ExprKind::Array(_) => "an array literal",
        ExprKind::Spread(_) => "a spread element",
        ExprKind::Member(..) => "a property read",
        ExprKind::SetMember(..) => "a property write",
        ExprKind::UpdateMember(..) => "an update of a property",
    }
}

/// The name a refused statement reports.
const fn statement_refusal(statement: &Stmt) -> &'static str {
    match statement {
        Stmt::Empty => "an empty statement",
        Stmt::Expr(_) => "an expression statement",
        Stmt::Block(_) => "a block",
        Stmt::Declare(_) => "a lexical declaration",
        Stmt::Var(_) => "a var declaration",
        Stmt::If(..) => "an if statement",
        Stmt::While(..) => "a while statement",
        Stmt::DoWhile(..) => "a do-while statement",
        Stmt::For(..) => "a for statement",
        Stmt::Switch(..) => "a switch statement",
        Stmt::ForIn { .. } => "a for-in statement",
        Stmt::ForOf { .. } => "a for-of statement",
        Stmt::Break => "break",
        Stmt::Continue => "continue",
        Stmt::Function(..) => "a function declaration",
        Stmt::Return(_) => "return",
        Stmt::Throw(_) => "throw",
        Stmt::Try { .. } => "a try statement",
    }
}

/// Lowers a Script, and names the construct it would not take when it takes
/// none, so the gap reports what is missing rather than that something is.
/// Whether the Directive Prologue of 11.2.1 makes this Script strict.
///
/// The parser has already decided what is a directive and what is an
/// expression, so a leading String literal statement is one.
fn register_directive_prologue_is_strict(body: &[Stmt]) -> bool {
    for statement in body {
        let Stmt::Expr(expression) = statement else {
            return false;
        };
        let ExprKind::Literal(crate::value::Value::String(units)) = &expression.kind else {
            return false;
        };
        if units.iter().copied().eq("use strict".encode_utf16()) {
            return true;
        }
    }
    false
}

fn lower_register_script(
    body: &[Stmt],
    realm: bool,
    entry_fuel_cost: u64,
    property_limit: usize,
) -> (
    Option<crate::engine::bytecode::BytecodeFunction>,
    Option<&'static str>,
) {
    if body
        .iter()
        .any(register_statement_has_unsupported_binding_pattern)
    {
        return (None, Some("a binding pattern"));
    }
    let Some((saw_declaration, saw_function)) = register_script_features(body, realm) else {
        return (None, Some("a declaration of the Script"));
    };
    let stack_requirement = body.iter().fold(1usize, |maximum, statement| {
        maximum.max(register_statement_stack_requirement(statement))
    });
    let mut lowerer = RegisterLowerer::new(entry_fuel_cost, stack_requirement, property_limit, 0);
    lowerer.code.strict = register_directive_prologue_is_strict(body);
    lowerer.realm = realm;
    lowerer.script_globals = realm;
    let mut code = lower_register_body(&mut lowerer, body, realm, saw_declaration, saw_function);
    if let Some(code) = code.as_mut() {
        code.realm_script = realm;
    }
    let refusal = code.is_none().then(|| {
        lowerer
            .refusal
            .unwrap_or("a Script the register lowering does not take")
    });
    (code, refusal)
}

/// The body of [`lower_register_script`], so that the refusal it recorded
/// survives the lowering that failed.
fn lower_register_body(
    lowerer: &mut RegisterLowerer,
    body: &[Stmt],
    realm: bool,
    saw_declaration: bool,
    saw_function: bool,
) -> Option<crate::engine::bytecode::BytecodeFunction> {
    if realm {
        lowerer.instantiate_global_declarations(body)?;
    }
    prepare_register_bindings(lowerer, body, saw_declaration, saw_function)?;
    let result_register = lowerer.allocate_register()?;
    lowerer
        .code
        .emit(crate::engine::bytecode::Instruction::LdaUndefined);
    lowerer
        .code
        .emit(crate::engine::bytecode::Instruction::Star(result_register));
    let mut completion_type = RegisterType::Undefined;
    let mut abrupt = false;
    for statement in body {
        if abrupt {
            // 16.1.4: statements after an abrupt completion never evaluate.
            break;
        }
        let snapshot = lowerer.snapshot();
        lowerer
            .code
            .emit(crate::engine::bytecode::Instruction::Ldar(result_register));
        match statement {
            Stmt::Function(_, _) => {
                lowerer
                    .code
                    .emit(crate::engine::bytecode::Instruction::Ldar(result_register));
            }
            Stmt::Declare(bindings) => {
                if realm {
                    // 9.1.1.4.4 gives the binding 16.1.7 created its value
                    // where the declaration stands, and a read of it before
                    // that is the ReferenceError of its temporal dead zone.
                    for (pattern, _, initializer) in bindings {
                        lowerer.initialize_global_lexical(pattern, initializer.as_ref())?;
                    }
                } else {
                    if register_lexical_dead_zone_read(bindings)? {
                        return None;
                    }
                    for (pattern, _, initializer) in bindings {
                        if let Some(initializer) = initializer {
                            lowerer.initialize_pattern(pattern, initializer)?;
                        } else {
                            lowerer.initialize(pattern.identifier()?, None)?;
                        }
                    }
                }
                lowerer
                    .code
                    .emit(crate::engine::bytecode::Instruction::Ldar(result_register));
            }
            _ => match lowerer.lower_statement(statement) {
                Some(RegisterFlow::Value(value_type)) => {
                    lowerer
                        .code
                        .emit(crate::engine::bytecode::Instruction::Star(result_register));
                    completion_type = value_type;
                }
                Some(RegisterFlow::Empty) => {}
                Some(RegisterFlow::Abrupt) => abrupt = true,
                None => {
                    lowerer.restore(snapshot);
                    return None;
                }
            },
        }
    }
    // An Object completion has no identity outside the engine. The boundary
    // refuses it there, by the name of what is missing, so the lowering does
    // not repeat the rule and report the Script as one it cannot take.
    if !completion_type.is_returnable() {
        return None;
    }
    lowerer
        .code
        .emit(crate::engine::bytecode::Instruction::Ldar(result_register));
    lowerer.release_register(result_register)?;
    lowerer
        .code
        .emit(crate::engine::bytecode::Instruction::Return);
    lowerer.code.register_count = lowerer.register_count;
    lowerer.code.binding_count = lowerer.max_binding_count;
    if lowerer.code.verify().is_err() {
        lowerer.refuse("bytecode the verifier of the engine refuses");
        return None;
    }
    Some(core::mem::replace(
        &mut lowerer.code,
        crate::engine::bytecode::BytecodeFunction::new(0, 0),
    ))
}

fn register_statement_has_unsupported_binding_pattern(statement: &Stmt) -> bool {
    match statement {
        Stmt::Declare(bindings) => bindings
            .iter()
            .any(|(pattern, _, _)| !register_binding_pattern_supported(pattern)),
        Stmt::Var(bindings) => bindings
            .iter()
            .any(|(pattern, _)| !register_binding_pattern_supported(pattern)),
        Stmt::Block(body) => body
            .iter()
            .any(register_statement_has_unsupported_binding_pattern),
        Stmt::If(_, yes, no) => {
            register_statement_has_unsupported_binding_pattern(yes)
                || no
                    .as_deref()
                    .is_some_and(register_statement_has_unsupported_binding_pattern)
        }
        Stmt::While(_, body) | Stmt::DoWhile(body, _) => {
            register_statement_has_unsupported_binding_pattern(body)
        }
        Stmt::For(initializer, _, _, body) => {
            register_statement_has_unsupported_binding_pattern(initializer)
                || register_statement_has_unsupported_binding_pattern(body)
        }
        Stmt::Switch(_, clauses) => clauses.iter().any(|(_, body)| {
            body.iter()
                .any(register_statement_has_unsupported_binding_pattern)
        }),
        Stmt::ForOf { binding, body, .. } | Stmt::ForIn { binding, body, .. } => {
            binding
                .as_ref()
                .is_some_and(|(pattern, _)| !register_binding_pattern_supported(pattern))
                || register_statement_has_unsupported_binding_pattern(body)
        }
        Stmt::Function(_, function) => function
            .body
            .iter()
            .any(register_statement_has_unsupported_binding_pattern),
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            body.iter()
                .any(register_statement_has_unsupported_binding_pattern)
                || catch.as_ref().is_some_and(|(_, body)| {
                    body.iter()
                        .any(register_statement_has_unsupported_binding_pattern)
                })
                || finally.as_ref().is_some_and(|body| {
                    body.iter()
                        .any(register_statement_has_unsupported_binding_pattern)
                })
        }
        Stmt::Empty
        | Stmt::Expr(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Return(_)
        | Stmt::Throw(_) => false,
    }
}

fn register_binding_pattern_supported(pattern: &parser::BindingPattern) -> bool {
    match pattern {
        parser::BindingPattern::Name(_) => true,
        parser::BindingPattern::Array(array) => {
            array.elements.iter().all(|element| match element {
                parser::ArrayBindingElement::Elision => true,
                parser::ArrayBindingElement::Element { pattern, .. } => {
                    register_binding_pattern_supported(pattern)
                }
            }) && array
                .rest
                .as_deref()
                .is_none_or(register_binding_pattern_supported)
        }
        parser::BindingPattern::Object(object) => {
            object.properties.iter().all(|property| {
                (!property.computed
                    && RegisterLowerer::static_property_name(&property.key).is_some()
                    || property.computed && register_computed_property_key_supported(&property.key))
                    && register_binding_pattern_supported(&property.pattern)
            }) && (object.rest.is_none()
                || object
                    .properties
                    .iter()
                    .all(|property| RegisterLowerer::binding_property_name(property).is_some()))
        }
    }
}

/// Whether the head of a `for`-`of` or `for`-`in` that declares nothing names
/// a Reference this lowering writes.
/// Whether an Initializer of a lexical declaration reads a name the
/// declaration binds and has not initialized yet.
///
/// 9.1.1.1.1 leaves such a name in its temporal dead zone, where a read is a
/// `ReferenceError`. The lowering writes the binding's register directly and
/// has no zone to check, so it does not take the declaration at all. A name a
/// nested function reads is read when that function runs, which is after the
/// declaration.
fn register_lexical_dead_zone_read(
    bindings: &[(BindingPattern, bool, Option<Expr>)],
) -> Option<bool> {
    for (at, (_, _, initializer)) in bindings.iter().enumerate() {
        let Some(initializer) = initializer else {
            continue;
        };
        let mut pending = BTreeSet::new();
        for (pattern, _, _) in bindings.get(at..)? {
            let mut names = Vec::new();
            pattern.names(&mut names);
            pending.extend(names);
        }
        let mut direct = BTreeSet::new();
        let mut nested = BTreeSet::new();
        register_expression_references(initializer, &mut direct, &mut nested)?;
        if direct.intersection(&pending).next().is_some() {
            return Some(true);
        }
    }
    Some(false)
}

/// Whether 8.5.2 gives this value the name of the key it is defined under,
/// which is what an anonymous function definition takes.
fn register_names_itself_after_its_key(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Group(inner) => register_names_itself_after_its_key(inner),
        ExprKind::Function(function) => function.name.is_none(),
        ExprKind::Class(class) => class.name.is_none(),
        _ => false,
    }
}

fn register_assignment_target_supported(target: &parser::AssignmentTarget) -> bool {
    match target {
        parser::AssignmentTarget::Reference(reference) => {
            reference.reference_name().is_some() || register_member_assignment_supported(reference)
        }
        parser::AssignmentTarget::Pattern(pattern) => {
            register_assignment_pattern_supported(pattern)
        }
    }
}

/// The names such a head writes.
fn register_assignment_target_names<'a>(
    target: &'a parser::AssignmentTarget,
    names: &mut Vec<&'a str>,
) {
    match target {
        parser::AssignmentTarget::Reference(reference) => {
            names.extend(reference.reference_name());
        }
        parser::AssignmentTarget::Pattern(pattern) => {
            register_assignment_pattern_names(pattern, names);
        }
    }
}

fn register_assignment_pattern_names<'a>(
    pattern: &'a parser::AssignmentPattern,
    names: &mut Vec<&'a str>,
) {
    match pattern {
        parser::AssignmentPattern::Target(target) => names.extend(target.reference_name()),
        parser::AssignmentPattern::Array(array) => {
            for element in &array.elements {
                if let parser::AssignmentArrayElement::Element { target, .. } = element {
                    register_assignment_pattern_names(target, names);
                }
            }
            if let Some(rest) = array.rest.as_deref() {
                register_assignment_pattern_names(rest, names);
            }
        }
        parser::AssignmentPattern::Object(object) => {
            for property in &object.properties {
                register_assignment_pattern_names(&property.target, names);
            }
            if let Some(rest) = object.rest.as_deref() {
                names.extend(rest.reference_name());
            }
        }
    }
}

fn register_assignment_pattern_supported(pattern: &parser::AssignmentPattern) -> bool {
    match pattern {
        parser::AssignmentPattern::Target(target) => {
            target.reference_name().is_some() || register_member_assignment_supported(target)
        }
        parser::AssignmentPattern::Array(array) => {
            array.elements.iter().all(|element| match element {
                parser::AssignmentArrayElement::Elision => true,
                parser::AssignmentArrayElement::Element { target, .. } => {
                    register_assignment_pattern_supported(target)
                }
            }) && array
                .rest
                .as_deref()
                .is_none_or(register_assignment_pattern_supported)
        }
        parser::AssignmentPattern::Object(object) => {
            object.properties.iter().all(|property| {
                register_computed_property_key_supported(&property.key)
                    && register_assignment_pattern_supported(&property.target)
            }) && object.rest.as_ref().is_none_or(|rest| {
                (rest.reference_name().is_some() || register_member_assignment_supported(rest))
                    && object.properties.iter().all(|property| {
                        RegisterLowerer::static_property_key_units(&property.key).is_some()
                    })
            })
        }
    }
}

/// Whether a target of 13.15.5.2 is a member expression.
///
/// The base and the key are whatever the lowering takes:
/// `prepare_member_assignment` evaluates both where the clause evaluates them
/// and refuses what it cannot name, so this asks only for the shape.
fn register_member_assignment_supported(target: &Expr) -> bool {
    target.member().is_some()
}

fn register_computed_property_key_supported(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Literal(
            Value::Number(_)
            | Value::Boolean(_)
            | Value::Null
            | Value::Undefined
            | Value::String(_),
        )
        | ExprKind::Name(_) => true,
        ExprKind::Group(inner) => register_computed_property_key_supported(inner),
        _ => false,
    }
}

fn register_expression_stack_requirement(expression: &Expr) -> usize {
    match &expression.kind {
        ExprKind::Group(inner) | ExprKind::Unary(_, inner) => {
            register_expression_stack_requirement(inner)
        }
        ExprKind::Sequence(left, right) => register_expression_stack_requirement(left)
            .max(register_expression_stack_requirement(right)),
        ExprKind::Assign(_, Some(_), right) => {
            1usize.saturating_add(register_expression_stack_requirement(right))
        }
        ExprKind::Assign(_, None, right) => register_expression_stack_requirement(right),
        ExprKind::Conditional(condition, yes, no) => {
            register_expression_stack_requirement(condition)
                .max(register_expression_stack_requirement(yes))
                .max(register_expression_stack_requirement(no))
        }
        ExprKind::Binary(_, left, right) => register_expression_stack_requirement(left)
            .max(1usize.saturating_add(register_expression_stack_requirement(right))),
        _ => 1,
    }
}

fn register_statement_stack_requirement(statement: &Stmt) -> usize {
    match statement {
        Stmt::Declare(bindings) => bindings.iter().fold(1usize, |maximum, (_, _, init)| {
            maximum.max(
                init.as_ref()
                    .map_or(1, register_expression_stack_requirement),
            )
        }),
        Stmt::Var(bindings) => bindings.iter().fold(1usize, |maximum, (_, init)| {
            maximum.max(
                init.as_ref()
                    .map_or(1, register_expression_stack_requirement),
            )
        }),
        Stmt::Expr(expression) => register_expression_stack_requirement(expression),
        Stmt::If(condition, yes, no) => register_expression_stack_requirement(condition)
            .max(register_statement_stack_requirement(yes))
            .max(
                no.as_deref()
                    .map_or(1, register_statement_stack_requirement),
            ),
        Stmt::Block(body) => body.iter().fold(1usize, |maximum, statement| {
            maximum.max(register_statement_stack_requirement(statement))
        }),
        Stmt::While(condition, body) => register_expression_stack_requirement(condition)
            .max(register_statement_stack_requirement(body)),
        Stmt::DoWhile(body, condition) => register_statement_stack_requirement(body)
            .max(register_expression_stack_requirement(condition)),
        Stmt::For(initializer, condition, step, body) => {
            register_statement_stack_requirement(initializer)
                .max(
                    condition
                        .as_ref()
                        .map_or(1, register_expression_stack_requirement),
                )
                .max(
                    step.as_ref()
                        .map_or(1, register_expression_stack_requirement),
                )
                .max(register_statement_stack_requirement(body))
        }
        Stmt::Throw(value) => register_expression_stack_requirement(value),
        Stmt::ForIn { object, body, .. } | Stmt::ForOf { object, body, .. } => {
            register_expression_stack_requirement(object)
                .max(register_statement_stack_requirement(body))
        }
        Stmt::Switch(discriminant, clauses) => clauses.iter().fold(
            register_expression_stack_requirement(discriminant),
            |maximum, (test, body)| {
                body.iter().fold(
                    maximum.max(
                        test.as_ref()
                            .map_or(1, register_expression_stack_requirement),
                    ),
                    |maximum, statement| {
                        maximum.max(register_statement_stack_requirement(statement))
                    },
                )
            },
        ),
        Stmt::Try {
            body,
            catch,
            finally,
        } => try_statements(body, catch.as_ref(), finally.as_deref())
            .fold(1usize, |maximum, statement| {
                maximum.max(register_statement_stack_requirement(statement))
            }),
        _ => 1,
    }
}

pub(crate) fn compile_dynamic(
    parameters: &str,
    body: &str,
    limits: Limits,
    async_kind: parser::AsyncKind,
) -> Result<Rc<FunctionCode>, Error> {
    let function = parser::dynamic_function(parameters, body, limits, async_kind)?;
    let mut compiler = Compiler {
        program: Program::empty(),
        scopes: alloc::vec![BTreeMap::new()],
        loops: Vec::new(),
        limits,
        outer: BTreeMap::new(),
        captures: Vec::new(),
        capture_names: BTreeMap::new(),
        enumerations: 0,
        realm: true,
    };
    compiler.function(&function, Some("anonymous"))?;
    compiler
        .program
        .functions
        .first()
        .cloned()
        .ok_or(Error::InvalidBytecode)
}

#[derive(Default)]
struct Loop {
    breaks: Vec<usize>,
    continues: Vec<usize>,
    switch: bool,
}

struct Compiler {
    program: Program,
    scopes: Vec<BTreeMap<String, usize>>,
    loops: Vec<Loop>,
    limits: Limits,
    outer: BTreeMap<String, Access>,
    captures: Vec<Access>,
    capture_names: BTreeMap<String, usize>,
    enumerations: usize,
    realm: bool,
}

impl Compiler {
    fn finish(&mut self) {
        for op in &mut self.program.code {
            if let Op::Renew(index) = op
                && self
                    .program
                    .slots
                    .get(*index)
                    .is_some_and(|slot| !slot.captured)
            {
                *op = Op::Nop;
            }
        }
    }
    fn emit(&mut self, op: Op) -> Result<usize, Error> {
        if self.program.total_instructions >= self.limits.instructions {
            return Err(Error::Limit {
                resource: "bytecode instructions",
            });
        }
        let at = self.program.code.len();
        self.program.code.push(op);
        self.program.total_instructions = self.program.total_instructions.saturating_add(1);
        Ok(at)
    }
    fn patch(&mut self, at: usize, target: usize) -> Result<(), Error> {
        match self.program.code.get_mut(at) {
            Some(
                Op::Jump(to)
                | Op::Branch(to, _)
                | Op::AbruptJump(to)
                | Op::ForInNext(_, to)
                | Op::IteratorGuard(_, to),
            ) => {
                *to = target;
                Ok(())
            }
            _ => Err(Error::InvalidBytecode),
        }
    }
    fn local(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }
    fn resolve(&mut self, name: &str) -> Option<Access> {
        if let Some(slot) = self.local(name) {
            return Some(Access::Local(slot));
        }
        if let Some(slot) = self.capture_names.get(name) {
            return Some(Access::Capture(*slot));
        }
        let from = *self.outer.get(name)?;
        let index = self.captures.len();
        self.captures.push(from);
        self.capture_names.insert(String::from(name), index);
        Some(Access::Capture(index))
    }
    fn declare(&mut self, name: &str, mutable: bool) -> Result<(), Error> {
        let scope = self.scopes.last_mut().ok_or(Error::InvalidBytecode)?;
        if scope.contains_key(name) {
            return Err(Error::Syntax {
                offset: 0,
                message: "duplicate lexical declaration",
            });
        }
        let index = self.program.slots.len();
        scope.insert(String::from(name), index);
        self.program.slots.push(Slot {
            name: String::from(name),
            mutable,
            captured: false,
        });
        self.emit(Op::Reset(index))?;
        Ok(())
    }
    fn declarations(&mut self, stmt: &Stmt) -> Result<(), Error> {
        if let Stmt::Declare(bindings) = stmt {
            for (pattern, mutable, _) in bindings {
                let mut names = Vec::new();
                pattern.names(&mut names);
                for name in names {
                    self.declare(&name, *mutable)?;
                }
            }
        }
        if let Stmt::Function(name, _) = stmt {
            self.declare(name, true)?;
        }
        Ok(())
    }
    fn block(&mut self, body: &[Stmt]) -> Result<(), Error> {
        validate_lexical_vars(body)?;
        self.scopes.push(BTreeMap::new());
        for stmt in body {
            self.declarations(stmt)?;
        }
        self.hoist(body)?;
        for stmt in body {
            self.statement(stmt)?;
        }
        self.scopes.pop();
        Ok(())
    }
    fn statement(&mut self, stmt: &Stmt) -> Result<(), Error> {
        match stmt {
            Stmt::ForOf {
                binding,
                target,
                object,
                body,
            } => self.for_of(binding.as_ref(), target.as_ref(), object, body)?,
            Stmt::Switch(value, clauses) => self.switch_statement(value, clauses)?,
            Stmt::ForIn {
                binding,
                target,
                object,
                body,
            } => self.for_in(binding.as_ref(), target.as_ref(), object, body)?,
            Stmt::Throw(value) => {
                self.expression(value)?;
                self.emit(Op::Throw)?;
            }
            Stmt::Try {
                body,
                catch,
                finally,
            } => self.try_statement(body, catch.as_ref(), finally.as_deref())?,
            Stmt::Var(bindings) => self.var_statement(bindings)?,
            Stmt::Return(value) => {
                self.return_value(value.as_ref())?;
            }
            Stmt::Empty | Stmt::Function(_, _) => {}
            Stmt::Expr(expr) => {
                self.expression(expr)?;
                self.emit(Op::Result)?;
            }
            Stmt::Block(body) => self.block(body)?,
            Stmt::Declare(bindings) => {
                self.initialize_bindings(bindings)?;
            }
            Stmt::If(cond, yes, no) => {
                self.emit(Op::Constant(Value::Undefined))?;
                self.emit(Op::Result)?;
                self.expression(cond)?;
                let branch = self.emit(Op::Branch(0, Branch::False))?;
                self.statement(yes)?;
                let jump = self.emit(Op::Jump(0))?;
                self.patch(branch, self.program.code.len())?;
                if let Some(no) = no {
                    self.statement(no)?;
                }
                self.patch(jump, self.program.code.len())?;
            }
            Stmt::While(cond, body) => self.loop_body(Some(cond), None, body, &[])?,
            Stmt::DoWhile(body, condition) => self.do_while_body(body, condition)?,
            Stmt::For(init, cond, step, body) => {
                if let Stmt::Declare(bindings) = init.as_ref() {
                    let mut names = Vec::new();
                    var_names(body, &mut names);
                    for (pattern, _, _) in bindings {
                        let mut bound = Vec::new();
                        pattern.names(&mut bound);
                        if bound.iter().any(|name| names.contains(name)) {
                            return Err(Error::Syntax {
                                offset: 0,
                                message: "loop var conflicts with lexical binding",
                            });
                        }
                    }
                }
                self.scopes.push(BTreeMap::new());
                self.declarations(init)?;
                // The initializer's expression value is not the loop completion.
                if let Stmt::Expr(expr) = init.as_ref() {
                    self.expression(expr)?;
                    self.emit(Op::Pop)?;
                } else {
                    self.statement(init)?;
                }
                let mut per_iteration = Vec::new();
                if let Stmt::Declare(bindings) = init.as_ref() {
                    for (pattern, mutable, _) in bindings {
                        if *mutable {
                            let mut names = Vec::new();
                            pattern.names(&mut names);
                            for name in names {
                                per_iteration
                                    .push(self.local(&name).ok_or(Error::InvalidBytecode)?);
                            }
                        }
                    }
                }
                self.loop_body(cond.as_ref(), step.as_ref(), body, &per_iteration)?;
                self.scopes.pop();
            }
            Stmt::Break | Stmt::Continue => {
                self.loop_control(matches!(stmt, Stmt::Break))?;
            }
        }
        Ok(())
    }

    fn var_statement(
        &mut self,
        bindings: &[(parser::BindingPattern, Option<Expr>)],
    ) -> Result<(), Error> {
        for (pattern, init) in bindings {
            if let Some(init) = init {
                if let Some(name) = pattern.identifier() {
                    self.binding_initializer(init, name)?;
                } else {
                    self.expression(init)?;
                }
                self.bind_pattern(pattern, false)?;
            }
        }
        Ok(())
    }
    fn loop_control(&mut self, is_break: bool) -> Result<(), Error> {
        let at = self.emit(Op::AbruptJump(0))?;
        if is_break {
            self.loops
                .last_mut()
                .ok_or(Error::InvalidBytecode)?
                .breaks
                .push(at);
        } else {
            self.loops
                .iter_mut()
                .rev()
                .find(|s| !s.switch)
                .ok_or(Error::InvalidBytecode)?
                .continues
                .push(at);
        }
        Ok(())
    }

    fn initialize_bindings(
        &mut self,
        bindings: &[(parser::BindingPattern, bool, Option<Expr>)],
    ) -> Result<(), Error> {
        for (pattern, _, init) in bindings {
            if let Some(init) = init {
                if let Some(name) = pattern.identifier() {
                    self.binding_initializer(init, name)?;
                } else {
                    self.expression(init)?;
                }
            } else {
                self.emit(Op::Constant(Value::Undefined))?;
            }
            self.bind_pattern(pattern, true)?;
        }
        Ok(())
    }
    fn loop_body(
        &mut self,
        cond: Option<&Expr>,
        step: Option<&Expr>,
        body: &Stmt,
        per_iteration: &[usize],
    ) -> Result<(), Error> {
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Result)?;
        for slot in per_iteration {
            self.emit(Op::Renew(*slot))?;
        }
        let head = self.program.code.len();
        let end = if let Some(cond) = cond {
            self.expression(cond)?;
            Some(self.emit(Op::Branch(0, Branch::False))?)
        } else {
            None
        };
        self.loops.push(Loop::default());
        self.statement(body)?;
        let next = self.program.code.len();
        for slot in per_iteration {
            self.emit(Op::Renew(*slot))?;
        }
        if let Some(step) = step {
            self.expression(step)?;
            self.emit(Op::Pop)?;
        }
        self.emit(Op::Jump(head))?;
        let done = self.program.code.len();
        if let Some(end) = end {
            self.patch(end, done)?;
        }
        let state = self.loops.pop().ok_or(Error::InvalidBytecode)?;
        for at in state.breaks {
            self.patch(at, done)?;
        }
        for at in state.continues {
            self.patch(at, next)?;
        }
        Ok(())
    }

    fn do_while_body(&mut self, body: &Stmt, condition: &Expr) -> Result<(), Error> {
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Result)?;
        let head = self.program.code.len();
        self.loops.push(Loop::default());
        self.statement(body)?;
        let condition_start = self.program.code.len();
        self.expression(condition)?;
        self.emit(Op::Branch(head, Branch::True))?;
        let done = self.program.code.len();
        let state = self.loops.pop().ok_or(Error::InvalidBytecode)?;
        for at in state.breaks {
            self.patch(at, done)?;
        }
        for at in state.continues {
            self.patch(at, condition_start)?;
        }
        Ok(())
    }

    fn return_value(&mut self, value: Option<&Expr>) -> Result<(), Error> {
        if let Some(value) = value {
            self.expression(value)?;
        } else {
            self.emit(Op::Constant(Value::Undefined))?;
        }
        self.emit(Op::Return)?;
        Ok(())
    }

    fn switch_statement(
        &mut self,
        value: &Expr,
        clauses: &[(Option<Expr>, Vec<Stmt>)],
    ) -> Result<(), Error> {
        self.expression(value)?;
        self.scopes.push(BTreeMap::new());
        let mut vars = Vec::new();
        for (_, body) in clauses {
            for stmt in body {
                var_names(stmt, &mut vars);
            }
        }
        for (_, body) in clauses {
            for stmt in body {
                if let Stmt::Declare(bindings) = stmt {
                    for (pattern, _, _) in bindings {
                        let mut names = Vec::new();
                        pattern.names(&mut names);
                        if names.iter().any(|name| vars.contains(name)) {
                            return Err(Error::Syntax {
                                offset: 0,
                                message: "switch var conflicts with lexical declaration",
                            });
                        }
                    }
                }
                self.declarations(stmt)?;
            }
        }
        for (_, body) in clauses {
            self.hoist(body)?;
        }
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Result)?;
        let mut branches = Vec::new();
        let mut default = None;
        for (index, (selector, _)) in clauses.iter().enumerate() {
            if let Some(selector) = selector {
                self.emit(Op::Dup)?;
                self.expression(selector)?;
                self.emit(Op::Binary(Binary::StrictEq))?;
                let miss = self.emit(Op::Branch(0, Branch::False))?;
                self.emit(Op::Pop)?;
                branches.push((index, self.emit(Op::Jump(0))?));
                self.patch(miss, self.program.code.len())?;
            } else {
                default = Some(index);
            }
        }
        self.emit(Op::Pop)?;
        let fallback = self.emit(Op::Jump(0))?;
        self.loops.push(Loop {
            switch: true,
            ..Loop::default()
        });
        let mut positions = Vec::new();
        for (_, body) in clauses {
            positions.push(self.program.code.len());
            for stmt in body {
                self.statement(stmt)?;
            }
        }
        let end = self.program.code.len();
        for (index, branch) in branches {
            self.patch(branch, *positions.get(index).ok_or(Error::InvalidBytecode)?)?;
        }
        self.patch(
            fallback,
            default
                .and_then(|i| positions.get(i))
                .copied()
                .unwrap_or(end),
        )?;
        let state = self.loops.pop().ok_or(Error::InvalidBytecode)?;
        for at in state.breaks {
            self.patch(at, end)?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn for_in(
        &mut self,
        binding: Option<&(parser::BindingPattern, Option<bool>)>,
        target: Option<&parser::AssignmentTarget>,
        object: &Expr,
        body: &Stmt,
    ) -> Result<(), Error> {
        let id = self.enumerations;
        self.enumerations = self.enumerations.saturating_add(1);
        self.scopes.push(BTreeMap::new());
        let mut binding_names = Vec::new();
        if let Some((pattern, Some(mutable))) = binding {
            pattern.names(&mut binding_names);
            let mut body_vars = Vec::new();
            var_names(body, &mut body_vars);
            for name in &binding_names {
                if body_vars.contains(name) {
                    return Err(Error::Syntax {
                        offset: 0,
                        message: "for-in lexical binding conflicts with var",
                    });
                }
                self.declare(name, *mutable)?;
            }
        }
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Result)?;
        self.expression(object)?;
        self.emit(Op::ForInInit(id))?;
        let head = self.program.code.len();
        let end = self.emit(Op::ForInNext(id, 0))?;
        if let Some((pattern, kind)) = binding {
            if kind.is_some() {
                for name in &binding_names {
                    let slot = self.local(name).ok_or(Error::InvalidBytecode)?;
                    self.emit(Op::Reset(slot))?;
                }
            }
            self.bind_pattern(pattern, kind.is_some())?;
        } else if let Some(target) = target {
            match target {
                parser::AssignmentTarget::Reference(target) => {
                    self.assign_reference_after_value(target)?;
                }
                parser::AssignmentTarget::Pattern(pattern) => self.assign_pattern(pattern)?,
            }
        }
        self.loops.push(Loop::default());
        self.statement(body)?;
        self.emit(Op::Jump(head))?;
        let done = self.program.code.len();
        self.patch(end, done)?;
        let state = self.loops.pop().ok_or(Error::InvalidBytecode)?;
        for at in state.breaks {
            self.patch(at, done)?;
        }
        for at in state.continues {
            self.patch(at, head)?;
        }
        self.emit(Op::ForInEnd(id))?;
        self.scopes.pop();
        Ok(())
    }

    fn for_of(
        &mut self,
        binding: Option<&(parser::BindingPattern, Option<bool>)>,
        target: Option<&parser::AssignmentTarget>,
        object: &Expr,
        body: &Stmt,
    ) -> Result<(), Error> {
        let id = self.enumerations;
        self.enumerations = self.enumerations.saturating_add(1);
        self.scopes.push(BTreeMap::new());
        let mut names = Vec::new();
        if let Some((pattern, Some(mutable))) = binding {
            pattern.names(&mut names);
            let mut vars = Vec::new();
            var_names(body, &mut vars);
            for name in &names {
                if vars.contains(name) {
                    return Err(Error::Syntax {
                        offset: 0,
                        message: "for-of lexical binding conflicts with var",
                    });
                }
                self.declare(name, *mutable)?;
            }
        }
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Result)?;
        self.expression(object)?;
        self.emit(Op::ForOfInit(id))?;
        let guard = self.emit(Op::IteratorGuard(id, 0))?;
        let head = self.program.code.len();
        let end = self.emit(Op::ForInNext(id, 0))?;
        for name in &names {
            self.emit(Op::Reset(self.local(name).ok_or(Error::InvalidBytecode)?))?;
        }
        if let Some((pattern, kind)) = binding {
            self.bind_pattern(pattern, kind.is_some())?;
        } else if let Some(target) = target {
            match target {
                parser::AssignmentTarget::Reference(target) => {
                    self.assign_reference_after_value(target)?;
                }
                parser::AssignmentTarget::Pattern(pattern) => self.assign_pattern(pattern)?,
            }
        }
        self.loops.push(Loop::default());
        self.statement(body)?;
        self.emit(Op::Jump(head))?;
        let done = self.program.code.len();
        self.patch(end, done)?;
        let state = self.loops.pop().ok_or(Error::InvalidBytecode)?;
        for at in state.breaks {
            self.patch(at, done)?;
        }
        for at in state.continues {
            self.patch(at, head)?;
        }
        self.emit(Op::IteratorEnd(id))?;
        self.patch(guard, done.saturating_add(1))?;
        self.scopes.pop();
        Ok(())
    }

    fn bind_pattern(
        &mut self,
        pattern: &parser::BindingPattern,
        initialize: bool,
    ) -> Result<(), Error> {
        match pattern {
            parser::BindingPattern::Name(name) => {
                if initialize {
                    if let Some(slot) = self.local(name) {
                        self.emit(Op::Init(slot))?;
                    } else if self.realm {
                        self.emit(Op::InitGlobal(name.clone()))?;
                    } else {
                        return Err(Error::InvalidBytecode);
                    }
                } else {
                    let op = if let Some(slot) = self.resolve(name) {
                        Op::Store(slot)
                    } else if self.realm {
                        Op::SetGlobal(name.clone(), false)
                    } else {
                        return Err(Error::InvalidBytecode);
                    };
                    self.emit(op)?;
                    self.emit(Op::Pop)?;
                }
            }
            parser::BindingPattern::Array(array) => {
                let id = self.enumerations;
                self.enumerations = self.enumerations.saturating_add(1);
                self.emit(Op::ForOfInit(id))?;
                let guard = self.emit(Op::IteratorGuard(id, 0))?;
                for element in &array.elements {
                    if let parser::ArrayBindingElement::Element {
                        pattern,
                        initializer,
                    } = element
                    {
                        self.emit(Op::IteratorValue(id))?;
                        if let Some(initializer) = initializer {
                            self.emit(Op::Dup)?;
                            self.emit(Op::Constant(Value::Undefined))?;
                            self.emit(Op::Binary(Binary::StrictEq))?;
                            let present = self.emit(Op::Branch(0, Branch::False))?;
                            self.emit(Op::Pop)?;
                            if let Some(name) = pattern.identifier() {
                                self.binding_initializer(initializer, name)?;
                            } else {
                                self.expression(initializer)?;
                            }
                            self.patch(present, self.program.code.len())?;
                        }
                        self.bind_pattern(pattern, initialize)?;
                    } else {
                        self.emit(Op::IteratorSkip(id))?;
                    }
                }
                if let Some(rest) = &array.rest {
                    self.emit(Op::IteratorRest(id))?;
                    self.bind_pattern(rest, initialize)?;
                }
                let end = self.program.code.len();
                self.emit(Op::IteratorEnd(id))?;
                self.patch(guard, end.saturating_add(1))?;
            }
            parser::BindingPattern::Object(object) => {
                self.emit(Op::ObjectBindingStart)?;
                for (index, property) in object.properties.iter().enumerate() {
                    self.expression(&property.key)?;
                    self.emit(Op::Key)?;
                    self.emit(Op::ObjectBindingGet(index))?;
                    if let Some(initializer) = &property.initializer {
                        self.emit_binding_default(&property.pattern, initializer)?;
                    }
                    self.bind_pattern(&property.pattern, initialize)?;
                }
                if let Some(rest) = &object.rest {
                    self.emit(Op::ObjectBindingRest(object.properties.len()))?;
                    self.bind_pattern(&parser::BindingPattern::Name(rest.clone()), initialize)?;
                } else {
                    self.emit(Op::ObjectBindingEnd(object.properties.len()))?;
                }
            }
        }
        Ok(())
    }

    fn assign_pattern(&mut self, pattern: &parser::AssignmentPattern) -> Result<(), Error> {
        match pattern {
            parser::AssignmentPattern::Target(target) => {
                self.assign_reference_after_value(target)?;
            }
            parser::AssignmentPattern::Array(array) => {
                let id = self.enumerations;
                self.enumerations = self.enumerations.saturating_add(1);
                self.emit(Op::ForOfInit(id))?;
                let guard = self.emit(Op::IteratorGuard(id, 0))?;
                for element in &array.elements {
                    if let parser::AssignmentArrayElement::Element {
                        target,
                        initializer,
                    } = element
                    {
                        let prepared = self.prepare_assignment_pattern_target(target)?;
                        self.emit(Op::IteratorValue(id))?;
                        if let Some(initializer) = initializer {
                            self.emit_assignment_default(target, initializer)?;
                        }
                        self.finish_assignment_pattern_target(target, prepared)?;
                    } else {
                        self.emit(Op::IteratorSkip(id))?;
                    }
                }
                if let Some(rest) = &array.rest {
                    let prepared = self.prepare_assignment_pattern_target(rest)?;
                    self.emit(Op::IteratorRest(id))?;
                    self.finish_assignment_pattern_target(rest, prepared)?;
                }
                let end = self.program.code.len();
                self.emit(Op::IteratorEnd(id))?;
                self.patch(guard, end.saturating_add(1))?;
            }
            parser::AssignmentPattern::Object(object) => {
                self.emit(Op::ObjectBindingStart)?;
                for (index, property) in object.properties.iter().enumerate() {
                    self.expression(&property.key)?;
                    self.emit(Op::Key)?;
                    let prepared = self.prepare_assignment_pattern_target(&property.target)?;
                    self.emit(Op::ObjectAssignmentGet {
                        excluded: index,
                        target_slots: prepared,
                    })?;
                    if let Some(initializer) = &property.initializer {
                        self.emit_assignment_default(&property.target, initializer)?;
                    }
                    self.finish_assignment_pattern_target(&property.target, prepared)?;
                }
                if let Some(rest) = &object.rest {
                    let prepared = self.prepare_assignment_reference(rest)?;
                    self.emit(Op::ObjectAssignmentRest {
                        excluded: object.properties.len(),
                        target_slots: prepared,
                    })?;
                    self.finish_assignment_reference(rest, prepared)?;
                } else {
                    self.emit(Op::ObjectBindingEnd(object.properties.len()))?;
                }
            }
        }
        Ok(())
    }

    fn prepare_assignment_pattern_target(
        &mut self,
        target: &parser::AssignmentPattern,
    ) -> Result<usize, Error> {
        if let parser::AssignmentPattern::Target(target) = target {
            self.prepare_assignment_reference(target)
        } else {
            Ok(0)
        }
    }

    fn finish_assignment_pattern_target(
        &mut self,
        target: &parser::AssignmentPattern,
        prepared: usize,
    ) -> Result<(), Error> {
        if let parser::AssignmentPattern::Target(target) = target {
            self.finish_assignment_reference(target, prepared)
        } else {
            self.assign_pattern(target)
        }
    }

    fn prepare_assignment_reference(&mut self, target: &Expr) -> Result<usize, Error> {
        if target.reference_name().is_some() {
            Ok(0)
        } else {
            let (base, key) = target.member().ok_or(Error::InvalidBytecode)?;
            self.expression(base)?;
            self.expression(key)?;
            Ok(2)
        }
    }

    fn finish_assignment_reference(&mut self, target: &Expr, prepared: usize) -> Result<(), Error> {
        if let Some(name) = target.reference_name() {
            if prepared != 0 {
                return Err(Error::InvalidBytecode);
            }
            let op = if let Some(slot) = self.resolve(name) {
                Op::Store(slot)
            } else if self.realm {
                Op::SetGlobal(String::from(name), target.strict)
            } else {
                Op::Missing(String::from(name))
            };
            self.emit(op)?;
        } else {
            if prepared != 2 {
                return Err(Error::InvalidBytecode);
            }
            self.emit(Op::KeyBelow)?;
            self.emit(Op::Set(target.strict))?;
        }
        self.emit(Op::Pop)?;
        Ok(())
    }

    fn assign_reference_after_value(&mut self, target: &Expr) -> Result<(), Error> {
        if target.reference_name().is_some() {
            self.finish_assignment_reference(target, 0)
        } else {
            let (base, key) = target.member().ok_or(Error::InvalidBytecode)?;
            self.reference(base, key)?;
            self.emit(Op::RotateKey)?;
            self.finish_assignment_reference(target, 2)
        }
    }

    fn emit_assignment_default(
        &mut self,
        target: &parser::AssignmentPattern,
        initializer: &Expr,
    ) -> Result<(), Error> {
        self.emit(Op::Dup)?;
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Binary(Binary::StrictEq))?;
        let present = self.emit(Op::Branch(0, Branch::False))?;
        self.emit(Op::Pop)?;
        if let parser::AssignmentPattern::Target(target) = target
            && let Some(name) = target.reference_name()
        {
            self.binding_initializer(initializer, name)?;
        } else {
            self.expression(initializer)?;
        }
        self.patch(present, self.program.code.len())?;
        Ok(())
    }

    fn emit_binding_default(
        &mut self,
        pattern: &parser::BindingPattern,
        initializer: &Expr,
    ) -> Result<(), Error> {
        self.emit(Op::Dup)?;
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Binary(Binary::StrictEq))?;
        let present = self.emit(Op::Branch(0, Branch::False))?;
        self.emit(Op::Pop)?;
        if let Some(name) = pattern.identifier() {
            self.binding_initializer(initializer, name)?;
        } else {
            self.expression(initializer)?;
        }
        self.patch(present, self.program.code.len())?;
        Ok(())
    }

    fn try_statement(
        &mut self,
        body: &[Stmt],
        catch: Option<&(Option<parser::BindingPattern>, Vec<Stmt>)>,
        finally: Option<&[Stmt]>,
    ) -> Result<(), Error> {
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Result)?;
        let at = self.emit(Op::Handler {
            catch: None,
            finally: None,
            end: 0,
        })?;
        self.block(body)?;
        self.emit(Op::EndTry)?;
        let catch_at = if let Some((pattern, body)) = catch {
            let entry = self.program.code.len();
            self.scopes.push(BTreeMap::new());
            if let Some(pattern) = pattern {
                let mut catch_names = Vec::new();
                pattern.names(&mut catch_names);
                let mut unique = BTreeSet::new();
                if catch_names.iter().any(|name| !unique.insert(name)) {
                    return Err(Error::Syntax {
                        offset: 0,
                        message: "duplicate catch binding",
                    });
                }
                let mut body_vars = Vec::new();
                for statement in body {
                    var_names(statement, &mut body_vars);
                }
                if pattern.identifier().is_none()
                    && catch_names.iter().any(|name| body_vars.contains(name))
                    || body.iter().any(|stmt| {
                        matches!(stmt, Stmt::Declare(bindings) if bindings.iter().any(|(pattern,_,_)| {
                            let mut names = Vec::new();
                            pattern.names(&mut names);
                            names.iter().any(|bound| catch_names.contains(bound))
                        }))
                    })
                {
                    return Err(Error::Syntax {
                        offset: 0,
                        message: "catch binding conflicts with block declaration",
                    });
                }
                for name in catch_names {
                    self.declare(&name, true)?;
                }
                self.bind_pattern(pattern, true)?;
            } else {
                self.emit(Op::Pop)?;
            }
            self.block(body)?;
            self.scopes.pop();
            self.emit(Op::EndTry)?;
            Some(entry)
        } else {
            None
        };
        let finally_at = if let Some(body) = finally {
            let entry = self.program.code.len();
            self.block(body)?;
            self.emit(Op::EndFinally)?;
            Some(entry)
        } else {
            None
        };
        let end = self.program.code.len();
        *self
            .program
            .code
            .get_mut(at)
            .ok_or(Error::InvalidBytecode)? = Op::Handler {
            catch: catch_at,
            finally: finally_at,
            end,
        };
        Ok(())
    }
    fn load(&mut self, name: &str, typeof_operand: bool) -> Result<(), Error> {
        let op = if let Some(slot) = self.resolve(name) {
            Op::Load(slot)
        } else if self.realm {
            Op::GetGlobal(String::from(name), typeof_operand)
        } else if matches!(name, "globalThis" | "self") {
            Op::Global
        } else if name == "Math" {
            Op::Math
        } else if name == "JSON" {
            Op::Json
        } else if name == "Reflect" {
            Op::Reflect
        } else if let Some(value) = global_constant(name) {
            Op::Constant(value)
        } else if let Some(builtin) = builtin(name) {
            Op::Constant(Value::Function(crate::value::FunctionValue::native(
                builtin,
            )))
        } else {
            Op::GetGlobal(String::from(name), typeof_operand)
        };
        self.emit(op)?;
        Ok(())
    }
    #[expect(
        clippy::too_many_lines,
        reason = "exhaustive expression lowering keeps operand ordering visible"
    )]
    fn expression(&mut self, expr: &Expr) -> Result<(), Error> {
        match &expr.kind {
            ExprKind::Spread(_) => {
                return Err(Error::Syntax {
                    offset: expr.offset,
                    message: "spread outside array or argument list",
                });
            }
            // 6.1.6.2 is a type of the new engine; the stack backend has no
            // value of it.
            ExprKind::BigInt(..) => {
                return Err(Error::Unsupported {
                    feature: "BigInt literals",
                });
            }
            ExprKind::Class(class) => self.class(class, None)?,
            ExprKind::NewTarget => {
                self.emit(Op::NewTarget)?;
            }
            ExprKind::Super => {
                return Err(Error::Syntax {
                    offset: expr.offset,
                    message: "bare super",
                });
            }
            ExprKind::DefaultSuper => {
                self.emit(Op::DefaultSuper)?;
            }
            ExprKind::Await(value) => {
                self.expression(value)?;
                self.emit(Op::Await)?;
            }
            ExprKind::Template(head, parts) => self.template(head, parts)?,
            ExprKind::Regex(pattern, flags) => self.regexp(pattern, flags)?,
            ExprKind::Array(items) => self.array(items)?,
            ExprKind::This => {
                self.emit(Op::This)?;
            }
            ExprKind::Object(properties) => self.object(properties)?,
            ExprKind::Member(base, key) => {
                if matches!(base.kind, ExprKind::Super) {
                    self.super_reference(key)?;
                    self.emit(Op::SuperGet(false))?;
                } else {
                    self.reference(base, key)?;
                    self.emit(Op::Get(false))?;
                }
            }
            ExprKind::SetMember(target, op, value, strict) => {
                let (base, key) = target.member().ok_or(Error::InvalidBytecode)?;
                if matches!(base.kind, ExprKind::Super) {
                    self.super_reference(key)?;
                    if op.is_some() {
                        self.emit(Op::DupPair)?;
                        self.emit(Op::SuperGet(false))?;
                    }
                    self.expression(value)?;
                    if let Some(op) = op {
                        self.emit(Op::Binary(*op))?;
                    }
                    self.emit(Op::SuperSet)?;
                    return Ok(());
                }
                self.reference(base, key)?;
                if op.is_some() {
                    self.emit(Op::DupPair)?;
                    self.emit(Op::Get(false))?;
                }
                self.expression(value)?;
                if let Some(op) = op {
                    self.emit(Op::Binary(*op))?;
                }
                self.emit(Op::Set(*strict))?;
            }
            ExprKind::UpdateMember(target, add, prefix, strict) => {
                let (base, key) = target.member().ok_or(Error::InvalidBytecode)?;
                if matches!(base.kind, ExprKind::Super) {
                    self.super_reference(key)?;
                    self.emit(Op::SuperUpdate(*add, *prefix))?;
                } else {
                    self.reference(base, key)?;
                    self.emit(Op::UpdateProperty(*add, *prefix, *strict))?;
                }
            }
            ExprKind::Literal(value) => {
                self.emit(Op::Constant(value.clone()))?;
            }
            ExprKind::Group(inner) => self.expression(inner)?,
            ExprKind::Sequence(left, right) => {
                self.expression(left)?;
                self.emit(Op::Pop)?;
                self.expression(right)?;
            }
            ExprKind::Name(name) => self.load(name, false)?,
            ExprKind::Unary(op, inner) => {
                self.unary(*op, inner, expr.strict)?;
            }
            ExprKind::Binary(op @ (Binary::And | Binary::Or | Binary::Nullish), left, right) => {
                self.expression(left)?;
                self.emit(Op::Dup)?;
                let kind = match op {
                    Binary::And => Branch::False,
                    Binary::Or => Branch::True,
                    _ => Branch::NotNullish,
                };
                let end = self.emit(Op::Branch(0, kind))?;
                self.emit(Op::Pop)?;
                self.expression(right)?;
                self.patch(end, self.program.code.len())?;
            }
            ExprKind::Binary(op, left, right) => {
                self.expression(left)?;
                self.expression(right)?;
                self.emit(Op::Binary(*op))?;
            }
            ExprKind::Assign(name, op, right) => {
                if op.is_some() {
                    self.load(name, false)?;
                }
                self.expression(right)?;
                if let Some(op) = op {
                    self.emit(Op::Binary(*op))?;
                }
                if let Some(slot) = self.resolve(name) {
                    self.emit(Op::Store(slot))?;
                } else if self.realm {
                    self.emit(Op::SetGlobal(name.clone(), expr.strict))?;
                } else {
                    self.emit(Op::Missing(name.clone()))?;
                }
            }
            ExprKind::Destructure(pattern, right) => {
                self.expression(right)?;
                self.emit(Op::Dup)?;
                self.assign_pattern(pattern)?;
            }
            ExprKind::Update(name, add, prefix) => {
                if let Some(slot) = self.resolve(name) {
                    self.emit(Op::Update(slot, *add, *prefix))?;
                } else if self.realm {
                    self.emit(Op::UpdateGlobal(name.clone(), *add, *prefix, expr.strict))?;
                } else {
                    self.emit(Op::Missing(name.clone()))?;
                }
            }
            ExprKind::Conditional(cond, yes, no) => {
                self.expression(cond)?;
                let branch = self.emit(Op::Branch(0, Branch::False))?;
                self.expression(yes)?;
                let end = self.emit(Op::Jump(0))?;
                self.patch(branch, self.program.code.len())?;
                self.expression(no)?;
                self.patch(end, self.program.code.len())?;
            }
            ExprKind::Function(function) => self.function(function, None)?,
            ExprKind::Call(callee, args) => {
                self.call(callee, args, expr.strict)?;
            }
            ExprKind::Construct(callee, args) => {
                self.construct(callee, args)?;
            }
        }
        Ok(())
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], strict: bool) -> Result<(), Error> {
        if matches!(callee.kind, ExprKind::Super) {
            self.emit(Op::PrepareSuperCall)?;
            let expanded = self.arguments(args)?;
            self.emit(if expanded {
                Op::SuperCallExpanded
            } else {
                Op::SuperCall(args.len())
            })?;
            return Ok(());
        }
        if let Some((base, key)) = callee.member() {
            if matches!(base.kind, ExprKind::Super) {
                self.super_reference(key)?;
                self.emit(Op::SuperGet(true))?;
            } else {
                self.reference(base, key)?;
                self.emit(Op::Get(true))?;
            }
        } else {
            self.expression(callee)?;
            self.emit(Op::Constant(Value::Undefined))?;
        }
        let expanded = self.arguments(args)?;
        let direct_eval = callee.reference_name() == Some("eval");
        self.emit(if direct_eval && expanded {
            Op::EvalExpanded(strict)
        } else if direct_eval {
            Op::Eval(args.len(), strict)
        } else if expanded {
            Op::CallExpanded
        } else {
            Op::Call(args.len())
        })?;
        Ok(())
    }

    fn arguments(&mut self, args: &[Expr]) -> Result<bool, Error> {
        let expanded = args.iter().any(|e| matches!(e.kind, ExprKind::Spread(_)));
        if expanded {
            self.emit(Op::Constant(Value::Number(0.0)))?;
        }
        for arg in args {
            if let ExprKind::Spread(value) = &arg.kind {
                self.expression(value)?;
                self.emit(Op::ArgumentAppend(true))?;
            } else {
                self.expression(arg)?;
                if expanded {
                    self.emit(Op::ArgumentAppend(false))?;
                }
            }
        }
        Ok(expanded)
    }

    fn array(&mut self, items: &[Option<Expr>]) -> Result<(), Error> {
        if items
            .iter()
            .flatten()
            .any(|e| matches!(e.kind, ExprKind::Spread(_)))
        {
            self.emit(Op::Array(0))?;
            for item in items {
                match item {
                    None => {
                        self.emit(Op::ArrayHole)?;
                    }
                    Some(Expr {
                        kind: ExprKind::Spread(value),
                        ..
                    }) => {
                        self.expression(value)?;
                        self.emit(Op::ArrayAppend(true))?;
                    }
                    Some(value) => {
                        self.expression(value)?;
                        self.emit(Op::ArrayAppend(false))?;
                    }
                }
            }
            return Ok(());
        }
        let length = u32::try_from(items.len()).map_err(|_| Error::Limit {
            resource: "array literal length",
        })?;
        self.emit(Op::Array(length))?;
        for (index, item) in items.iter().enumerate() {
            if let Some(item) = item {
                self.emit(Op::Constant(Value::string(&alloc::format!("{index}"))))?;
                self.expression(item)?;
                self.emit(Op::Define(false))?;
            }
        }
        Ok(())
    }

    fn object(&mut self, properties: &[parser::ObjectProperty]) -> Result<(), Error> {
        self.emit(Op::Object)?;
        for property in properties {
            self.expression(&property.key)?;
            self.emit(Op::Key)?;
            self.expression(&property.value)?;
            if matches!(&property.value.kind,ExprKind::Function(f) if !f.constructible&&!f.arrow) {
                self.emit(Op::MethodHome)?;
            }
            self.emit(if let Some(setter) = property.accessor {
                Op::Accessor(setter)
            } else {
                Op::Define(property.prototype)
            })?;
        }
        Ok(())
    }

    fn binding_initializer(&mut self, expression: &Expr, name: &str) -> Result<(), Error> {
        match &expression.kind {
            ExprKind::Group(inner) => self.binding_initializer(inner, name),
            ExprKind::Function(function) if function.name.is_none() => {
                self.function(function, Some(name))
            }
            ExprKind::Class(class) if class.name.is_none() => self.class(class, Some(name)),
            _ => self.expression(expression),
        }
    }

    fn super_reference(&mut self, key: &Expr) -> Result<(), Error> {
        self.emit(Op::SuperBase)?;
        self.expression(key)?;
        self.emit(Op::Key)?;
        Ok(())
    }
    fn class(&mut self, class: &parser::Class, inferred_name: Option<&str>) -> Result<(), Error> {
        self.scopes.push(BTreeMap::new());
        let slot = if let Some(name) = &class.name {
            self.declare(name, false)?;
            self.local(name)
        } else {
            None
        };
        if let Some(heritage) = &class.heritage {
            self.expression(heritage)?;
        } else {
            self.emit(Op::Constant(Value::Undefined))?;
        }
        self.function(&class.constructor, class.name.as_deref().or(inferred_name))?;
        self.emit(Op::Class(class.heritage.is_some()))?;
        for (is_static, method) in &class.methods {
            self.expression(&method.key)?;
            self.emit(Op::Key)?;
            self.expression(&method.value)?;
            self.emit(Op::ClassMethod(*is_static, method.accessor))?;
        }
        if let Some(slot) = slot {
            self.emit(Op::Dup)?;
            self.emit(Op::Init(slot))?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn construct(&mut self, callee: &Expr, args: &[Expr]) -> Result<(), Error> {
        self.expression(callee)?;
        self.emit(Op::Constant(Value::Undefined))?;
        let expanded = self.arguments(args)?;
        self.emit(if expanded {
            Op::ConstructExpanded
        } else {
            Op::Construct(args.len())
        })?;
        Ok(())
    }

    fn regexp(&mut self, pattern: &str, flags: &str) -> Result<(), Error> {
        self.emit(Op::Regex(Rc::new(crate::regexp::RegExp::compile(
            pattern.encode_utf16().collect(),
            flags,
        )?)))?;
        Ok(())
    }

    fn template(&mut self, head: &Value, parts: &[(Expr, Value)]) -> Result<(), Error> {
        self.emit(Op::Constant(head.clone()))?;
        for (value, tail) in parts {
            self.expression(value)?;
            self.emit(Op::ToString)?;
            self.emit(Op::Binary(Binary::Add))?;
            self.emit(Op::Constant(tail.clone()))?;
            self.emit(Op::Binary(Binary::Add))?;
        }
        Ok(())
    }

    fn unary(&mut self, op: Unary, inner: &Expr, strict: bool) -> Result<(), Error> {
        if matches!(op, Unary::Delete) {
            if let Some((base, key)) = inner.member() {
                self.reference(base, key)?;
                self.emit(Op::Delete(strict))?;
            } else if let Some(name) = inner.reference_name() {
                if self.realm && self.resolve(name).is_none() {
                    self.emit(Op::DeleteGlobal(name.into()))?;
                } else {
                    self.emit(Op::Constant(Value::Boolean(false)))?;
                }
            } else {
                self.expression(inner)?;
                self.emit(Op::Pop)?;
                self.emit(Op::Constant(Value::Boolean(true)))?;
            }
        } else {
            if let (Unary::Typeof, Some(name)) = (op, inner.reference_name()) {
                self.load(name, true)?;
            } else {
                self.expression(inner)?;
            }
            self.emit(Op::Unary(op))?;
        }
        Ok(())
    }

    fn reference(&mut self, base: &Expr, key: &Expr) -> Result<(), Error> {
        self.expression(base)?;
        self.expression(key)?;
        self.emit(Op::Key)?;
        Ok(())
    }

    fn hoist(&mut self, body: &[Stmt]) -> Result<(), Error> {
        for stmt in body {
            if let Stmt::Function(name, function) = stmt {
                self.function(function, Some(name))?;
                self.emit(Op::Init(self.local(name).ok_or(Error::InvalidBytecode)?))?;
            }
        }
        Ok(())
    }

    fn root_body(&mut self, body: &[Stmt]) -> Result<(), Error> {
        validate_lexical_vars(body)?;
        let mut variables = Vec::new();
        for stmt in body {
            var_names(stmt, &mut variables);
            if let Stmt::Function(name, _) = stmt {
                variables.push(name.clone());
            }
        }
        for name in variables {
            if !self
                .scopes
                .last()
                .is_some_and(|scope| scope.contains_key(&name))
            {
                self.declare(&name, true)?;
                self.emit(Op::Constant(Value::Undefined))?;
                self.emit(Op::Init(self.local(&name).ok_or(Error::InvalidBytecode)?))?;
            }
        }
        for stmt in body {
            if matches!(stmt, Stmt::Declare(_)) {
                self.declarations(stmt)?;
            }
        }
        self.hoist(body)?;
        for stmt in body {
            self.statement(stmt)?;
        }
        Ok(())
    }

    fn global_body(&mut self, body: &[Stmt]) -> Result<(), Error> {
        validate_lexical_vars(body)?;
        let mut decls = BTreeMap::new();
        let mut order = Vec::new();
        for stmt in body {
            let mut vars = Vec::new();
            var_names(stmt, &mut vars);
            for name in vars {
                if !decls.contains_key(&name) {
                    order.push(name.clone());
                }
                decls.entry(name).or_insert(GlobalKind::Var);
            }
            if let Stmt::Function(name, _) = stmt {
                if !decls.contains_key(name) {
                    order.push(name.clone());
                }
                decls.insert(name.clone(), GlobalKind::Function);
            }
        }
        let mut functions = Vec::new();
        for stmt in body.iter().rev() {
            if let Stmt::Function(name, function) = stmt
                && !functions.iter().any(|(n, _)| *n == name)
            {
                functions.push((name, function));
            }
        }
        functions.reverse();
        let mut ordered: Vec<_> = functions
            .iter()
            .map(|(name, _)| GlobalDecl {
                name: (*name).clone(),
                kind: GlobalKind::Function,
            })
            .collect();
        for name in order {
            if decls.get(&name) == Some(&GlobalKind::Var) {
                ordered.push(GlobalDecl {
                    name,
                    kind: GlobalKind::Var,
                });
            }
        }
        for stmt in body {
            if let Stmt::Declare(bindings) = stmt {
                for (pattern, mutable, _) in bindings {
                    let mut names = Vec::new();
                    pattern.names(&mut names);
                    for name in names {
                        if decls
                            .insert(name.clone(), GlobalKind::Lexical(*mutable))
                            .is_some()
                        {
                            return Err(Error::Syntax {
                                offset: 0,
                                message: "duplicate global declaration",
                            });
                        }
                        ordered.push(GlobalDecl {
                            name,
                            kind: GlobalKind::Lexical(*mutable),
                        });
                    }
                }
            }
        }
        self.program.globals = ordered;
        for (name, function) in functions {
            self.function(function, Some(name))?;
            self.emit(Op::InitGlobal(name.clone()))?;
        }
        for stmt in body {
            self.statement(stmt)?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "function environment and capture lowering keep binding initialization ordering together"
    )]
    fn function(&mut self, function: &Function, declared_name: Option<&str>) -> Result<(), Error> {
        validate_function(function)?;
        let mut outer = BTreeMap::new();
        // Forward already-visible enclosing names so a grandchild can capture
        // them through its immediate parent. Local slots still shadow them.
        let names: Vec<_> = self.outer.keys().cloned().collect();
        for name in names {
            if let Some(access) = self.resolve(&name) {
                outer.insert(name, access);
            }
        }
        for scope in &self.scopes {
            for (name, index) in scope {
                outer.insert(name.clone(), Access::Local(*index));
            }
        }
        let mut child = Self {
            program: Program {
                code: Vec::new(),
                slots: Vec::new(),
                functions: Vec::new(),
                total_instructions: 0,
                globals: Vec::new(),
                register_code: None,
                register_refusal: None,
                stack_refusal: None,
            },
            scopes: Vec::new(),
            loops: Vec::new(),
            limits: self.limits,
            outer,
            captures: Vec::new(),
            capture_names: BTreeMap::new(),
            enumerations: 0,
            realm: self.realm,
        };
        child.scopes.push(BTreeMap::new());
        let self_slot = if let Some(name) = &function.name {
            child.declare(name, false)?;
            child.local(name)
        } else {
            None
        };
        child.scopes.push(BTreeMap::new());
        let arguments_slot = child.arguments_binding(function)?;
        let (parameters, parameter_code) = child.parameter_init(function)?;
        child.root_body(&function.body)?;
        child.emit(Op::Constant(Value::Undefined))?;
        child.emit(Op::Return)?;
        let arguments_slot = arguments_slot.filter(|slot| child.slot_used(*slot));
        if arguments_slot.is_some() && !function.strict && !parameter_code {
            for index in &parameters {
                child
                    .program
                    .slots
                    .get_mut(*index)
                    .ok_or(Error::InvalidBytecode)?
                    .captured = true;
            }
        }
        child.finish();
        self.program.total_instructions = self
            .program
            .total_instructions
            .checked_add(child.program.total_instructions)
            .filter(|count| *count <= self.limits.instructions)
            .ok_or(Error::Limit {
                resource: "bytecode instructions",
            })?;
        let index = self.program.functions.len();
        for access in &child.captures {
            if let Access::Local(index) = access {
                self.program
                    .slots
                    .get_mut(*index)
                    .ok_or(Error::InvalidBytecode)?
                    .captured = true;
            }
        }
        self.program.functions.push(Rc::new(FunctionCode {
            source: function.source.clone(),
            program: child.program,
            captures: child.captures,
            parameters,
            self_slot,
            arrow: function.arrow,
            strict: function.strict,
            constructible: function.constructible,
            initialization: if parameter_code {
                ParameterInitialization::Bytecode
            } else {
                ParameterInitialization::Direct
            },
            length: function
                .parameters
                .iter()
                .take_while(|p| !p.rest && p.default.is_none())
                .count(),
            async_kind: function.async_kind,
            arguments_slot,
            constructor_kind: function.constructor_kind,
            name: declared_name
                .or(function.name.as_deref())
                .unwrap_or("")
                .into(),
        }));
        self.emit(Op::Closure(index))?;
        Ok(())
    }

    fn parameter_init(&mut self, function: &Function) -> Result<(Vec<usize>, bool), Error> {
        let complex = function.parameters.iter().any(|p| !p.is_simple());
        let mut slots = Vec::new();
        // Discard only the self-name reset: named function values initialize it
        // in the call setup. Complex parameters stay uninitialized until code.
        self.program.code.clear();
        self.program.total_instructions = 0;
        for parameter in &function.parameters {
            let mut names = Vec::new();
            parameter.names(&mut names);
            for name in &names {
                if self
                    .scopes
                    .last()
                    .is_some_and(|scope| scope.contains_key(name))
                {
                    if function.arrow
                        || function.strict
                        || complex
                        || function.async_kind == parser::AsyncKind::Async
                    {
                        return Err(Error::Syntax {
                            offset: 0,
                            message: "duplicate formal parameter",
                        });
                    }
                } else {
                    self.declare(name, true)?;
                }
            }
            if !complex {
                let name = parameter
                    .pattern
                    .identifier()
                    .ok_or(Error::InvalidBytecode)?;
                slots.push(self.local(name).ok_or(Error::InvalidBytecode)?);
            }
        }
        if !complex {
            self.program.code.clear();
            self.program.total_instructions = 0;
            return Ok((slots, false));
        }
        for (index, parameter) in function.parameters.iter().enumerate() {
            self.emit(if parameter.rest {
                Op::RestArguments(index)
            } else {
                Op::Argument(index)
            })?;
            if let Some(default) = &parameter.default {
                self.emit(Op::Dup)?;
                self.emit(Op::Constant(Value::Undefined))?;
                self.emit(Op::Binary(Binary::StrictEq))?;
                let jump = self.emit(Op::Branch(0, Branch::False))?;
                self.emit(Op::Pop)?;
                if let Some(name) = parameter.pattern.identifier() {
                    self.binding_initializer(default, name)?;
                } else {
                    self.expression(default)?;
                }
                self.patch(jump, self.program.code.len())?;
            }
            self.bind_pattern(&parameter.pattern, true)?;
        }
        self.emit(Op::EndParameters)?;
        if function
            .parameters
            .iter()
            .any(parser::Parameter::contains_expression)
        {
            self.parameter_body_scope(function)?;
        }
        Ok((slots, true))
    }

    fn arguments_binding(&mut self, function: &Function) -> Result<Option<usize>, Error> {
        let mut parameter_names = Vec::new();
        for parameter in &function.parameters {
            parameter.names(&mut parameter_names);
        }
        let needed = !function.arrow
            && !parameter_names.iter().any(|name| name == "arguments")
            && (function
                .parameters
                .iter()
                .any(parser::Parameter::contains_expression)
                || !function.body.iter().any(|stmt| match stmt {
                    Stmt::Function(name, _) => name == "arguments",
                    Stmt::Declare(bindings) => bindings.iter().any(|(pattern, _, _)| {
                        let mut names = Vec::new();
                        pattern.names(&mut names);
                        names.iter().any(|name| name == "arguments")
                    }),
                    _ => false,
                }));
        if needed {
            self.declare("arguments", true)?;
            Ok(self.local("arguments"))
        } else {
            Ok(None)
        }
    }

    fn slot_used(&self, slot: usize) -> bool {
        self.program.slots.get(slot).is_some_and(|s| s.captured)
            || self
                .program
                .code
                .iter()
                .any(|op| matches!(op,Op::Load(Access::Local(index)) | Op::Store(Access::Local(index)) if *index==slot))
    }

    fn parameter_body_scope(&mut self, function: &Function) -> Result<(), Error> {
        let mut names = Vec::new();
        for stmt in &function.body {
            var_names(stmt, &mut names);
        }
        self.scopes.push(BTreeMap::new());
        for parameter in &function.parameters {
            let mut parameter_names = Vec::new();
            parameter.names(&mut parameter_names);
            for name in parameter_names {
                if !names.contains(&name) {
                    continue;
                }
                let from = self.local(&name).ok_or(Error::InvalidBytecode)?;
                self.declare(&name, true)?;
                self.emit(Op::Load(Access::Local(from)))?;
                self.emit(Op::Init(self.local(&name).ok_or(Error::InvalidBytecode)?))?;
            }
        }
        Ok(())
    }
}

fn var_names(stmt: &Stmt, names: &mut Vec<String>) {
    match stmt {
        Stmt::ForOf { binding, body, .. } | Stmt::ForIn { binding, body, .. } => {
            if let Some((pattern, None)) = binding {
                pattern.names(names);
            }
            var_names(body, names);
        }
        Stmt::Switch(_, clauses) => {
            for (_, body) in clauses {
                for stmt in body {
                    var_names(stmt, names);
                }
            }
        }
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            for stmt in body {
                var_names(stmt, names);
            }
            if let Some((_, body)) = catch {
                for stmt in body {
                    var_names(stmt, names);
                }
            }
            if let Some(body) = finally {
                for stmt in body {
                    var_names(stmt, names);
                }
            }
        }
        Stmt::Var(bindings) => {
            for (pattern, _) in bindings {
                pattern.names(names);
            }
        }
        Stmt::Block(body) => {
            for stmt in body {
                var_names(stmt, names);
            }
        }
        Stmt::If(_, yes, no) => {
            var_names(yes, names);
            if let Some(no) = no {
                var_names(no, names);
            }
        }
        Stmt::While(_, body) | Stmt::DoWhile(body, _) => var_names(body, names),
        Stmt::For(init, _, _, body) => {
            var_names(init, names);
            var_names(body, names);
        }
        _ => {}
    }
}

fn validate_function(function: &Function) -> Result<(), Error> {
    let mut parameter_names = Vec::new();
    for parameter in &function.parameters {
        parameter.names(&mut parameter_names);
    }
    for stmt in &function.body {
        if let Stmt::Declare(bindings) = stmt {
            for (pattern, _, _) in bindings {
                let mut names = Vec::new();
                pattern.names(&mut names);
                if names.iter().any(|name| parameter_names.contains(name)) {
                    return Err(Error::Syntax {
                        offset: 0,
                        message: "parameter conflicts with lexical declaration",
                    });
                }
            }
        }
    }
    if function.strict
        && parameter_names
            .iter()
            .chain(function.name.iter())
            .any(|name| parser::strict_binding(name))
    {
        Err(Error::Syntax {
            offset: 0,
            message: "invalid strict function binding",
        })
    } else {
        Ok(())
    }
}

fn validate_lexical_vars(body: &[Stmt]) -> Result<(), Error> {
    let mut vars = Vec::new();
    for stmt in body {
        var_names(stmt, &mut vars);
    }
    for stmt in body {
        if let Stmt::Declare(bindings) = stmt {
            for (pattern, _, _) in bindings {
                let mut names = Vec::new();
                pattern.names(&mut names);
                if names.iter().any(|name| vars.contains(name)) {
                    return Err(Error::Syntax {
                        offset: 0,
                        message: "var conflicts with lexical declaration",
                    });
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn builtin(name: &str) -> Option<Builtin> {
    Some(match name {
        "print" => Builtin::Print,
        "Number" => Builtin::Number,
        "String" => Builtin::String,
        "Boolean" => Builtin::Boolean,
        "isNaN" => Builtin::IsNaN,
        "isFinite" => Builtin::IsFinite,
        "Object" => Builtin::Object,
        "Array" => Builtin::Array,
        "RegExp" => Builtin::RegExp,
        "Promise" => Builtin::Promise,
        "WeakMap" => Builtin::WeakMap,
        "Symbol" => Builtin::Symbol,
        "Function" => Builtin::Function,
        "Error" => Builtin::Error,
        "TypeError" => Builtin::TypeError,
        "RangeError" => Builtin::RangeError,
        "ReferenceError" => Builtin::ReferenceError,
        "SyntaxError" => Builtin::SyntaxError,
        "EvalError" => Builtin::EvalError,
        "URIError" => Builtin::URIError,
        "Proxy" => Builtin::Proxy,
        "eval" => Builtin::Eval,
        _ => return None,
    })
}

fn global_constant(name: &str) -> Option<Value> {
    match name {
        "undefined" => Some(Value::Undefined),
        "NaN" => Some(Value::Number(f64::NAN)),
        "Infinity" => Some(Value::Number(f64::INFINITY)),
        _ => None,
    }
}
