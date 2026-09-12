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
    if realm {
        compiler.global_body(body)?;
    } else {
        compiler.root_body(body)?;
    }
    compiler.finish();
    compiler.program.register_code = lower_register_script(
        body,
        realm,
        u64::try_from(compiler.program.total_instructions).unwrap_or(u64::MAX),
        limits.properties,
    )
    .map(Rc::new);
    Ok(compiler.program)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegisterType {
    Array(u32),
    Function(u32),
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
            Self::Array(_) | Self::Function(_) | Self::Object(_) | Self::Unknown
        )
    }

    const fn is_object(self) -> bool {
        matches!(self, Self::Array(_) | Self::Function(_) | Self::Object(_))
    }

    const fn is_returnable(self) -> bool {
        self.is_primitive() || self.is_object()
    }

    const fn is_numeric_primitive(self) -> bool {
        matches!(self, Self::Number | Self::NumberOrUndefined)
    }

    fn accepts(self, actual: Self) -> bool {
        match self {
            Self::Primitive => actual.is_primitive(),
            _ => self == actual,
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
    return_type: Option<RegisterType>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RegisterFlow {
    Empty,
    Value(RegisterType),
    Abrupt,
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

#[derive(Clone)]
struct RegisterSnapshot {
    instructions: usize,
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
    binding_type_hints: BTreeMap<String, RegisterType>,
    return_type: Option<RegisterType>,
}

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
            completions: Vec::new(),
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
            return_type: None,
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
            let binding = *self.bindings.get(name)?;
            if binding.stable_function_identity {
                return None;
            }
            let value_type = self.lower(expression)?;
            self.store_binding(binding);
            self.bindings.get_mut(name)?.value_type = Some(value_type);
            return Some(());
        }
        let value_type = self.lower(expression)?;
        self.bind_pattern(value_type, pattern)
    }

    fn bind_pattern(
        &mut self,
        value_type: RegisterType,
        pattern: &parser::BindingPattern,
    ) -> Option<()> {
        use crate::engine::bytecode::Instruction;
        match pattern {
            parser::BindingPattern::Name(name) => {
                let binding = *self.bindings.get(name)?;
                if binding.stable_function_identity {
                    return None;
                }
                self.store_binding(binding);
                self.bindings.get_mut(name)?.value_type = Some(value_type);
            }
            parser::BindingPattern::Object(object) => {
                if !value_type.is_object() {
                    return None;
                }
                if object.rest.is_some() && !matches!(value_type, RegisterType::Object(_)) {
                    return None;
                }
                let source = self.allocate_register()?;
                self.code.emit(Instruction::Star(source));
                let mut excluded = Vec::new();
                for property in &object.properties {
                    if object.rest.is_some() {
                        excluded.push(Self::binding_property_name(property)?);
                    }
                    let mut property_type = self.lower_property_from_register(
                        source,
                        value_type,
                        &property.key,
                        property.computed,
                        true,
                    )?;
                    if let Some(initializer) = &property.initializer {
                        property_type = self.lower_binding_default(property_type, initializer)?;
                    }
                    self.bind_pattern(property_type, &property.pattern)?;
                }
                if let Some(rest) = &object.rest {
                    let (rest_type, rest_object) =
                        self.lower_object_rest_from_register(source, value_type, &excluded)?;
                    self.code.emit(Instruction::Ldar(rest_object));
                    self.bind_pattern(rest_type, &parser::BindingPattern::Name(rest.clone()))?;
                    self.release_register(rest_object)?;
                }
                self.release_register(source)?;
            }
            parser::BindingPattern::Array(array) => {
                if !matches!(value_type, RegisterType::Array(_)) {
                    return None;
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
                        element_type = self.lower_binding_default(element_type, initializer)?;
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

    fn lower_binding_default(
        &mut self,
        value_type: RegisterType,
        initializer: &Expr,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if value_type == RegisterType::Undefined {
            let bindings_without_default = self.bindings.clone();
            let layouts_without_default = self.object_layouts.clone();
            let default_type = self.lower(initializer)?;
            return (default_type.is_primitive()
                && self.object_layouts == layouts_without_default
                && register_context_bindings_unchanged(&bindings_without_default, &self.bindings))
            .then_some(default_type);
        }
        if !matches!(
            value_type,
            RegisterType::NumberOrUndefined | RegisterType::Primitive
        ) {
            return Some(value_type);
        }
        let present = self.code.emit(Instruction::JumpIfNotUndefined(0));
        let bindings_without_default = self.bindings.clone();
        let layouts_without_default = self.object_layouts.clone();
        let default_type = self.lower(initializer)?;
        if !default_type.is_primitive()
            || self.object_layouts != layouts_without_default
            || !register_context_bindings_unchanged(&bindings_without_default, &self.bindings)
        {
            return None;
        }
        self.bindings = merge_register_bindings(&bindings_without_default, &self.bindings)?;
        let end = self.code.instructions.len();
        self.patch_jump(present, end)?;
        Some(if value_type == RegisterType::Primitive {
            RegisterType::Primitive
        } else {
            RegisterType::Number.merge(default_type)
        })
    }

    fn prepare_var_bindings(&mut self, body: &[Stmt]) -> Option<()> {
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
        // Captured readers are compiled against the merged primitive type of
        // every var initializer. An arbitrary later assignment can invalidate
        // that contract after the closure bytecode has been emitted; until
        // guards and deoptimization exist, reject the complete enclosing body.
        if !captured_vars.is_empty() {
            for statement in body {
                if register_statement_writes_names(statement, &captured_vars)? {
                    return None;
                }
            }
        }
        let mut inferred = self.bindings.clone();
        infer_register_body_var_types_to_fixed_point(body, &mut inferred)?;
        for name in initialized_names {
            let hint = inferred.get(&name)?.value_type?;
            self.binding_type_hints.insert(name, hint);
        }
        Some(())
    }

    fn load_binding(&mut self, binding: RegisterBinding) {
        use crate::engine::bytecode::Instruction;
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

    fn snapshot(&self) -> RegisterSnapshot {
        RegisterSnapshot {
            instructions: self.code.instructions.len(),
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
            binding_type_hints: self.binding_type_hints.clone(),
            return_type: self.return_type,
        }
    }

    fn restore(&mut self, snapshot: RegisterSnapshot) {
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
        self.binding_type_hints = snapshot.binding_type_hints;
        self.return_type = snapshot.return_type;
    }

    #[expect(
        clippy::too_many_lines,
        reason = "expression lowering keeps type propagation beside emitted operations"
    )]
    fn lower(&mut self, expression: &Expr) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let result = match &expression.kind {
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
                        _ => return None,
                    }
                }
            }
            ExprKind::Group(inner) => self.lower(inner)?,
            ExprKind::Sequence(left, right) => {
                self.lower(left)?;
                self.lower(right)?
            }
            ExprKind::Unary(operator, inner) => {
                let inner_type = self.lower(inner)?;
                match operator {
                    Unary::Plus | Unary::Minus | Unary::BitNot if inner_type.is_primitive() => {
                        if inner_type != RegisterType::Number {
                            self.code.emit(Instruction::ToNumber);
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
                    Unary::Plus | Unary::Minus | Unary::BitNot | Unary::Delete => return None,
                }
                match operator {
                    Unary::Plus | Unary::Minus | Unary::BitNot => RegisterType::Number,
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
                self.lower_assignment(name, *operator, right)?
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
            ExprKind::Call(callee, arguments) => self.lower_call(callee, arguments)?,
            ExprKind::Member(base, key) => self.lower_member(base, key)?,
            ExprKind::SetMember(target, operator, value, _) => {
                self.lower_member_assignment(target, *operator, value)?
            }
            _ => return None,
        };
        Some(result)
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
        if !value_type.is_object() {
            return None;
        }
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
                let name = target.reference_name()?;
                self.assign_name(value_type, name)?;
            }
            parser::AssignmentPattern::Array(array) => {
                if !matches!(value_type, RegisterType::Array(_)) {
                    return None;
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
                        element_type = self.lower_binding_default(element_type, initializer)?;
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
                if !value_type.is_object()
                    || object.rest.is_some() && !matches!(value_type, RegisterType::Object(_))
                {
                    return None;
                }
                let source = self.allocate_register()?;
                self.code.emit(Instruction::Star(source));
                let mut excluded = Vec::new();
                for property in &object.properties {
                    let prepared = self.prepare_assignment_pattern_target(&property.target)?;
                    if object.rest.is_some() {
                        excluded.push(Self::static_property_key_units(&property.key)?);
                    }
                    let mut property_type = self.lower_property_from_register(
                        source,
                        value_type,
                        &property.key,
                        Self::static_property_name(&property.key).is_none(),
                        true,
                    )?;
                    if let Some(initializer) = &property.initializer {
                        property_type = self.lower_binding_default(property_type, initializer)?;
                    }
                    self.finish_assignment_pattern_target(
                        property_type,
                        &property.target,
                        prepared,
                    )?;
                }
                if let Some(rest) = &object.rest {
                    let prepared = self.prepare_assignment_reference(rest)?;
                    let (rest_type, rest_object) =
                        self.lower_object_rest_from_register(source, value_type, &excluded)?;
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
            Some(RegisterPreparedAssignment::Name)
        } else {
            self.prepare_member_assignment(target)
                .map(RegisterPreparedAssignment::Member)
        }
    }

    fn finish_assignment_reference(
        &mut self,
        value_type: RegisterType,
        target: &Expr,
        prepared: RegisterPreparedAssignment,
    ) -> Option<()> {
        match prepared {
            RegisterPreparedAssignment::Name => {
                self.assign_name(value_type, target.reference_name()?)
            }
            RegisterPreparedAssignment::Member(prepared) => {
                target.member()?;
                self.finish_member_assignment(prepared, value_type)
            }
            RegisterPreparedAssignment::NestedPattern => None,
        }
    }

    fn assign_name(&mut self, value_type: RegisterType, name: &str) -> Option<()> {
        let binding = *self.bindings.get(name)?;
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
            if property.prototype || property.accessor.is_some() {
                return None;
            }
            let key = if property.computed {
                let static_name = Self::static_property_key_units(&property.key);
                if !self.lower(&property.key)?.is_primitive() {
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
            let value_type = self.lower(&property.value)?;
            if !value_type.is_primitive() && !value_type.is_object() {
                return None;
            }
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            let property_name = match key {
                RegisterMemberKey::Named { constant, name } => {
                    self.code.emit(Instruction::SetNamed {
                        obj: object,
                        name: constant,
                        slot,
                    });
                    Some(name)
                }
                RegisterMemberKey::ObjectKeyed(register, name) => {
                    self.code.emit(Instruction::SetByValue {
                        obj: object,
                        key: register,
                        slot,
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
        self.code.emit(Instruction::CreateArray(length));
        let array = self.allocate_register()?;
        self.code.emit(Instruction::Star(array));
        for (index, item) in items.iter().enumerate() {
            let Some(item) = item else {
                continue;
            };
            if matches!(item.kind, ExprKind::Spread(_)) {
                return None;
            }
            let index = u32::try_from(index).ok()?;
            self.emit_array_index(index)?;
            let key = self.allocate_register()?;
            self.code.emit(Instruction::Star(key));
            let value_type = self.lower(item)?;
            if !value_type.is_primitive() && !value_type.is_object() {
                return None;
            }
            let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
            self.code.emit(Instruction::SetByValue {
                obj: array,
                key,
                slot,
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

    fn lower_function(&mut self, function: &Function) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        if !Self::register_function_supported(function) {
            return None;
        }
        let code_id = u32::try_from(self.code.functions.len())
            .ok()?
            .checked_add(self.function_table_base)?;
        let first_child_object_id = self.next_object_id;
        let inherited_layouts = self.object_layouts.clone();
        let scope = register_function_scope(function)?;
        let mut captures = BTreeMap::new();
        for name in &scope.free_names {
            if self.bindings.contains_key(name) {
                captures.insert(name.clone(), self.capture_binding(name)?);
            }
        }
        let (mut child, self_register) =
            self.register_function_child(function, code_id, &captures, &scope.captured_names)?;
        if !captures.is_empty() {
            child.code.outer_context_slot_counts = self.context_slot_counts()?;
        }
        let return_type = Self::lower_function_body(&mut child, &function.body)?;
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
        child.code.binding_count = child.max_binding_count;
        child.code.self_register = self_register;
        let nested_functions = core::mem::take(&mut child.code.functions);
        self.code.functions.push(child.code);
        self.code.functions.extend(nested_functions);
        self.function_returns.extend(child.function_returns);
        self.function_parameters.extend(child.function_parameters);
        self.function_capture_effects
            .extend(child.function_capture_effects);
        self.function_layout_effects
            .extend(child.function_layout_effects);
        self.function_returns.insert(code_id, return_type);
        self.function_parameters.insert(
            code_id,
            alloc::vec![RegisterType::Primitive; function.parameters.len()],
        );
        self.function_capture_effects
            .insert(code_id, capture_effects);
        self.function_layout_effects.insert(code_id, layout_effects);
        self.code.emit(Instruction::CreateClosure(code_id));
        Some(RegisterType::Function(code_id))
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
                alloc::vec![RegisterType::Primitive; function.parameters.len()],
            );
        }
        self.lower_function(function)
    }

    fn register_function_supported(function: &Function) -> bool {
        function.async_kind == parser::AsyncKind::Sync
            && function.constructor_kind == parser::ConstructorKind::Ordinary
            && function.parameters.iter().all(parser::Parameter::is_simple)
            && function.parameters.iter().all(|parameter| {
                function
                    .parameters
                    .iter()
                    .filter(|candidate| {
                        candidate.pattern.identifier() == parameter.pattern.identifier()
                    })
                    .count()
                    == 1
            })
    }

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
        child.function_returns = self.function_returns.clone();
        child.function_parameters = self.function_parameters.clone();
        child.function_capture_effects = self.function_capture_effects.clone();
        child.function_layout_effects = self.function_layout_effects.clone();
        child.next_object_id = self.next_object_id;
        child.object_layouts = self.object_layouts.clone();
        let depth_shift = u16::from(!captured_names.is_empty());
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
        for parameter in &function.parameters {
            let name = parameter.pattern.identifier()?;
            child.declare(name, true)?;
            child.bindings.get_mut(name)?.value_type = Some(RegisterType::Primitive);
        }
        let self_register = if let Some(name) = &function.name {
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
                alloc::vec![RegisterType::Primitive; function.parameters.len()],
            );
            Some(register)
        } else {
            None
        };
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
        child.infer_binding_type_hints(&function.body)?;
        for name in captured_names {
            child.capture_binding(name)?;
        }
        Some((child, self_register))
    }

    fn infer_binding_type_hints(&mut self, body: &[Stmt]) -> Option<()> {
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
                bindings.get_mut(name)?.value_type = Some(value_type);
                self.binding_type_hints
                    .insert(String::from(name), value_type);
            }
        }
        Some(())
    }

    fn context_slot_counts(&self) -> Option<Vec<u16>> {
        let mut counts = Vec::new();
        if let Some(own) = self.code.own_context_slot_count {
            counts.push(own);
        }
        counts.extend(self.code.outer_context_slot_counts.iter().copied());
        (!counts.is_empty()).then_some(counts)
    }

    fn lower_function_body(child: &mut Self, body: &[Stmt]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
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

    fn lower_call(&mut self, callee: &Expr, arguments: &[Expr]) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let RegisterType::Function(code_id) = self.lower(callee)? else {
            return None;
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
        for (index, argument) in arguments.iter().enumerate() {
            let argument_type = self.lower(argument)?;
            if !argument_type.is_primitive()
                || parameter_types
                    .get(index)
                    .is_some_and(|parameter| !parameter.accepts(argument_type))
            {
                return None;
            }
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            argument_registers.push(register);
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
        self.function_returns.get(&code_id).copied()
    }

    fn lower_member(&mut self, base: &Expr, key: &Expr) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let base_type = self.lower(base)?;
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
        let static_name = keyed
            .then(|| Self::static_property_key_units(key))
            .flatten();
        let result_type = if static_name.as_deref().is_some_and(Self::is_length) {
            RegisterType::Number
        } else if let Some(index) = static_index {
            let static_type = elements
                .get(&index)
                .copied()
                .unwrap_or(RegisterType::Undefined);
            dynamic.map_or(static_type, |dynamic| static_type.merge(dynamic))
        } else {
            elements
                .values()
                .copied()
                .chain(dynamic.iter().copied())
                .reduce(RegisterType::merge)
                .unwrap_or(RegisterType::Undefined)
                .merge(RegisterType::Number)
                .merge(RegisterType::Undefined)
        };
        if keyed {
            if !self.lower(key)?.is_primitive() {
                return None;
            }
        } else if let Some(index) = static_index {
            self.emit_array_index(index)?;
        } else if !self.lower(key)?.is_primitive() {
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

    fn lower_array_index_from_register(
        &mut self,
        object: crate::engine::bytecode::Reg,
        base_type: RegisterType,
        index: u32,
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
            Self::static_property_key_units(key)
        } else {
            Some(Self::static_property_name(key)?.to_vec())
        };
        let result_type = if let Some(name) = static_name.as_deref() {
            let known = properties.get(name).copied();
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
        };
        let slot = self.feedback_slot(crate::engine::bytecode::FeedbackKind::NamedAccess)?;
        if keyed {
            if !self.lower(key)?.is_primitive() {
                return None;
            }
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
        if operator.is_some() {
            return None;
        }
        let prepared = self.prepare_member_assignment(target)?;
        let value_type = self.lower(value)?;
        self.finish_member_assignment(prepared, value_type)?;
        Some(value_type)
    }

    fn prepare_member_assignment(&mut self, target: &Expr) -> Option<RegisterMemberAssignment> {
        use crate::engine::bytecode::Instruction;
        let (base, key) = target.member()?;
        let base_type = self.lower(base)?;
        if !base_type.is_object()
            || matches!(base_type, RegisterType::Array(_)) && Self::is_length_name(key)
        {
            return None;
        }
        let object = self.allocate_register()?;
        self.code.emit(Instruction::Star(object));
        let key = if matches!(base_type, RegisterType::Array(_)) {
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
            if !self.lower(key)?.is_primitive() {
                return None;
            }
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
        if !value_type.is_primitive() && !value_type.is_object() {
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
                });
                self.release_register(register)?;
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
                });
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
        let flow = match statement {
            Stmt::Expr(expression) => RegisterFlow::Value(self.lower(expression)?),
            Stmt::If(condition, yes, no) => self.lower_if(condition, yes, no.as_deref())?,
            Stmt::Block(body) => self.lower_block(body)?,
            Stmt::While(condition, body) => RegisterFlow::Value(self.lower_while(condition, body)?),
            Stmt::DoWhile(body, condition) => {
                RegisterFlow::Value(self.lower_do_while(body, condition)?)
            }
            Stmt::For(initializer, condition, step, body) => RegisterFlow::Value(self.lower_for(
                initializer,
                condition.as_ref(),
                step.as_ref(),
                body,
            )?),
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
                    return None;
                }
                self.return_type = Some(
                    self.return_type
                        .map_or(return_type, |current| current.merge(return_type)),
                );
                self.code.emit(crate::engine::bytecode::Instruction::Return);
                RegisterFlow::Abrupt
            }
            Stmt::Throw(value) => {
                let value_type = self.lower(value)?;
                // A value that leaves the script has to be representable at the
                // legacy boundary, which only carries primitives.
                if !value_type.is_primitive() {
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
        let name = match catch.map(|(parameter, _)| parameter) {
            Some(Some(pattern)) => Some(pattern.identifier()?),
            Some(None) | None => None,
        };
        // A `break`, `continue` or `return` inside a protected Block has to run
        // the Finally Block before it leaves, which this lowering does not do.
        if finally.is_some()
            && try_statements(body, catch, finally).any(register_statement_transfers_control)
        {
            return None;
        }
        let result_register = self.allocate_register()?;
        let exception_register = self.allocate_register()?;
        let token_register = match finally {
            Some(_) => Some(self.allocate_register()?),
            None => None,
        };
        self.code.emit(Instruction::LdaUndefined);
        self.code.emit(Instruction::Star(result_register));
        if let Some(token_register) = token_register {
            self.code.emit(Instruction::LdaSmi(0));
            self.code.emit(Instruction::Star(token_register));
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
        if self.bindings != bindings_before || self.object_layouts != layouts_before {
            return None;
        }
        let opaque = self.range_contains_call(start, end)?;
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
        let mut bindings_after_handler = bindings_before.clone();
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
            self.code.emit(Instruction::LdaUndefined);
            self.thrown.push(Vec::new());
            let flow = self.lower_block(handler);
            let handler_thrown = self.thrown.pop()?;
            if let (Some(outer), true) = (self.thrown.last_mut(), finally.is_none()) {
                // Without a Finally Block a value thrown by the Catch Block
                // leaves this statement, so an enclosing range observes it.
                outer.extend(handler_thrown);
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

        let finally_start = self.code.instructions.len();
        for exit in exits {
            self.patch_jump(exit, finally_start)?;
        }
        if let Some(finally) = finally {
            self.bindings = bindings_before.clone();
            self.code.emit(Instruction::LdaUndefined);
            // 14.15.3: a normal Finally completion is discarded and the try or
            // Catch completion is kept.
            self.lower_block(finally)?;
            if self.bindings != bindings_before || self.object_layouts != layouts_before {
                return None;
            }
            self.code.emit(Instruction::Ldar(token_register?));
            let normal = self.code.emit(Instruction::JumpIfFalse(0));
            self.code.emit(Instruction::Ldar(exception_register));
            self.code.emit(Instruction::Throw);
            let after = self.code.instructions.len();
            self.patch_jump(normal, after)?;
        }
        self.bindings = merge_register_bindings(&bindings_before, &bindings_after_handler)?;
        self.code.emit(Instruction::Ldar(result_register));
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
    fn range_contains_call(&self, start: usize, end: usize) -> Option<bool> {
        Some(
            self.code
                .instructions
                .get(start..end)?
                .iter()
                .any(|instruction| {
                    matches!(
                        instruction,
                        crate::engine::bytecode::Instruction::Call { .. }
                    )
                }),
        )
    }

    fn lower_block(&mut self, body: &[Stmt]) -> Option<RegisterFlow> {
        use crate::engine::bytecode::Instruction;
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));
        let scoped_bindings = self.enter_block_scope(body)?;
        self.completions.push(result_register);
        let mut result_type = None;
        let mut flow = RegisterFlow::Empty;
        for statement in body {
            flow = match statement {
                Stmt::Declare(bindings) => {
                    for (pattern, _, initializer) in bindings {
                        if let Some(initializer) = initializer {
                            self.initialize_pattern(pattern, initializer)?;
                        } else {
                            self.initialize(pattern.identifier()?, None)?;
                        }
                        let mut names = Vec::new();
                        pattern.names(&mut names);
                        if names.iter().any(|name| {
                            !self
                                .bindings
                                .get(name)
                                .and_then(|binding| binding.value_type)
                                .is_some_and(RegisterType::is_primitive)
                        }) {
                            return None;
                        }
                    }
                    RegisterFlow::Empty
                }
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
        if !register_bindings_fit(&self.bindings, &loop_state.bindings)
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
            // A clause is entered by a jump as well as by fallthrough, and in
            // both cases the accumulator has to hold the value accumulated so
            // far, not the discriminant the dispatch left behind.
            self.code.emit(Instruction::Ldar(result_register));
            for statement in body {
                let flow = self.lower_statement(statement)?;
                // Every clause body is an entry point of its own, so a body
                // that changes a tracked binding type has no single type at the
                // next one.
                if self.bindings != bindings_before || self.object_layouts != layouts_before {
                    return None;
                }
                match flow {
                    RegisterFlow::Value(clause_type) => {
                        value_type = value_type.merge(clause_type);
                        self.code.emit(Instruction::Star(result_register));
                    }
                    RegisterFlow::Empty => {}
                    RegisterFlow::Abrupt => break,
                }
            }
        }
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

    fn lower_while(&mut self, condition: &Expr, body: &Stmt) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        self.code.emit(Instruction::LdaUndefined);
        let result_register = self.allocate_register()?;
        self.code.emit(Instruction::Star(result_register));
        let mut bindings_at_head = self.bindings.clone();
        infer_register_var_types_to_fixed_point(body, &mut bindings_at_head)?;
        self.bindings = bindings_at_head.clone();
        let head = self.code.instructions.len();
        self.lower(condition)?;
        if self.bindings != bindings_at_head {
            return None;
        }
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
        infer_register_var_types_to_fixed_point(body, &mut bindings_at_head)?;
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
        infer_register_var_types_to_fixed_point(body, &mut bindings_at_head)?;
        self.bindings = bindings_at_head.clone();
        let head = self.code.instructions.len();
        let branch = if let Some(condition) = condition {
            self.lower(condition)?;
            if self.bindings != bindings_at_head {
                return None;
            }
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
        self.code
            .emit(crate::engine::bytecode::Instruction::LdaUndefined);
        let yes_flow = self.lower_statement(yes)?;
        let bindings_after_yes = self.bindings.clone();
        let properties_after_yes = self.object_layouts.clone();
        let jump = (yes_flow != RegisterFlow::Abrupt).then(|| self.code.emit(Instruction::Jump(0)));
        let no_start = self.code.instructions.len();
        self.bindings = bindings_before;
        self.object_layouts = properties_before;
        self.code
            .emit(crate::engine::bytecode::Instruction::LdaUndefined);
        let no_flow = no.map_or(Some(RegisterFlow::Value(RegisterType::Undefined)), |no| {
            self.lower_statement(no)
        })?;
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
            _ if properties_after_yes == self.object_layouts => {
                merge_register_bindings(&bindings_after_yes, &bindings_after_no)?
            }
            _ => return None,
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
        if self.object_layouts != object_layouts_after_left {
            return None;
        }
        self.bindings = merge_register_bindings(&bindings_after_left, &bindings_after_right)?;
        let end = self.code.instructions.len();
        self.patch_jump(branch, end)?;
        Some(left_type.merge(right_type))
    }

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
                if left_type.is_primitive() && right_type.is_primitive() =>
            {
                self.feedback_binary(operator, right_register)?
            }
            Binary::BitAnd
            | Binary::BitOr
            | Binary::BitXor
            | Binary::Shl
            | Binary::Shr
            | Binary::Ushr => {
                if !left_type.is_primitive() || !right_type.is_primitive() {
                    return None;
                }
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
            Binary::Lt => (BinaryOp::LessThan, RegisterType::Boolean),
            Binary::Le => (BinaryOp::LessThanOrEqual, RegisterType::Boolean),
            Binary::Gt => (BinaryOp::GreaterThan, RegisterType::Boolean),
            Binary::Ge => (BinaryOp::GreaterThanOrEqual, RegisterType::Boolean),
            _ => return None,
        };
        let slot = self.feedback_slot(FeedbackKind::BinaryOp)?;
        Some((Instruction::Binary { op, rhs, slot }, result_type))
    }

    fn lower_assignment(
        &mut self,
        name: &str,
        operator: Option<Binary>,
        right: &Expr,
    ) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let binding = *self.bindings.get(name)?;
        if !binding.mutable || binding.value_type.is_none() || binding.stable_function_identity {
            return None;
        }
        let result_type = if let Some(operator) = operator {
            let left_type = binding.value_type?;
            self.load_binding(binding);
            let left_register = self.allocate_register()?;
            self.code.emit(Instruction::Star(left_register));
            let right_type = self.lower(right)?;
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
                Binary::Pow
                | Binary::BitAnd
                | Binary::BitOr
                | Binary::BitXor
                | Binary::Shl
                | Binary::Shr
                | Binary::Ushr => {
                    if !left_type.is_primitive() || !right_type.is_primitive() {
                        return None;
                    }
                    RegisterType::Number
                }
                _ => return None,
            };
            let right_register = self.allocate_register()?;
            self.code.emit(Instruction::Star(right_register));
            self.code.emit(Instruction::Ldar(left_register));
            let instruction = match operator {
                Binary::Add => Instruction::Add(right_register),
                Binary::Sub => Instruction::Sub(right_register),
                Binary::Mul => Instruction::Mul(right_register),
                Binary::Pow => {
                    let slot =
                        self.feedback_slot(crate::engine::bytecode::FeedbackKind::BinaryOp)?;
                    Instruction::Binary {
                        op: crate::engine::bytecode::BinaryOp::Pow,
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
            result_type
        } else {
            self.lower(right)?
        };
        self.store_binding(binding);
        self.bindings.get_mut(name)?.value_type = Some(result_type);
        Some(result_type)
    }

    fn lower_update(&mut self, name: &str, add: bool, prefix: bool) -> Option<RegisterType> {
        use crate::engine::bytecode::Instruction;
        let binding = *self.bindings.get(name)?;
        if !binding.mutable || binding.value_type != Some(RegisterType::Number) {
            return None;
        }
        self.load_binding(binding);
        let original = if prefix {
            None
        } else {
            let register = self.allocate_register()?;
            self.code.emit(Instruction::Star(register));
            Some(register)
        };
        let one = self.allocate_register()?;
        self.code.emit(Instruction::LdaSmi(1));
        self.code.emit(Instruction::Star(one));
        self.load_binding(binding);
        self.code.emit(if add {
            Instruction::Add(one)
        } else {
            Instruction::Sub(one)
        });
        self.store_binding(binding);
        self.release_register(one)?;
        if let Some(original) = original {
            self.code.emit(Instruction::Ldar(original));
            self.release_register(original)?;
        }
        Some(RegisterType::Number)
    }

    fn allocate_register(&mut self) -> Option<crate::engine::bytecode::Reg> {
        let register = crate::engine::bytecode::Reg(self.next_register);
        self.next_register = self.next_register.checked_add(1)?;
        self.register_count = self.register_count.max(self.next_register);
        Some(register)
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
                Unary::Plus | Unary::Minus | Unary::BitNot if inner.is_primitive() => {
                    RegisterType::Number
                }
                Unary::Not => RegisterType::Boolean,
                Unary::Void => RegisterType::Undefined,
                Unary::Typeof => RegisterType::String,
                Unary::Plus | Unary::Minus | Unary::BitNot | Unary::Delete => {
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
        Stmt::Empty
        | Stmt::Expr(_)
        | Stmt::Declare(_)
        | Stmt::Function(_, _)
        | Stmt::Return(_)
        | Stmt::Throw(_)
        | Stmt::Break
        | Stmt::Continue => {}
        Stmt::ForIn { .. } | Stmt::ForOf { .. } => return None,
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

/// Whether the statement can transfer control past the Block it stands in.
///
/// A nested function body is not scanned: its `return` leaves that function.
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
        Stmt::While(_, body) | Stmt::DoWhile(body, _) | Stmt::For(_, _, _, body) => {
            register_statement_transfers_control(body)
        }
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

fn infer_register_var_types(
    statement: &Stmt,
    bindings: &mut BTreeMap<String, RegisterBinding>,
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
                let binding = bindings.get_mut(name)?;
                binding.value_type =
                    merge_optional_register_types(binding.value_type, Some(observed));
            }
        }
        Stmt::Block(body) => {
            for statement in body {
                infer_register_var_types(statement, bindings)?;
            }
        }
        Stmt::If(_, yes, no) => {
            infer_register_var_types(yes, bindings)?;
            if let Some(no) = no {
                infer_register_var_types(no, bindings)?;
            }
        }
        Stmt::While(_, body) | Stmt::DoWhile(body, _) => {
            infer_register_var_types(body, bindings)?;
        }
        Stmt::For(initializer, _, _, body) => {
            infer_register_var_types(initializer, bindings)?;
            infer_register_var_types(body, bindings)?;
        }
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            for statement in try_statements(body, catch.as_ref(), finally.as_deref()) {
                infer_register_var_types(statement, bindings)?;
            }
        }
        Stmt::Switch(_, clauses) => {
            for statement in clauses.iter().flat_map(|(_, body)| body) {
                infer_register_var_types(statement, bindings)?;
            }
        }
        Stmt::Empty
        | Stmt::Expr(_)
        | Stmt::Declare(_)
        | Stmt::Function(_, _)
        | Stmt::Return(_)
        | Stmt::Throw(_)
        | Stmt::Break
        | Stmt::Continue => {}
        Stmt::ForIn { .. } | Stmt::ForOf { .. } => return None,
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
            infer_register_var_types(statement, bindings)?;
        }
        if *bindings == before {
            return Some(());
        }
    }
}

fn infer_register_var_types_to_fixed_point(
    statement: &Stmt,
    bindings: &mut BTreeMap<String, RegisterBinding>,
) -> Option<()> {
    loop {
        let before = bindings.clone();
        infer_register_var_types(statement, bindings)?;
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
        Stmt::Empty | Stmt::Return(None) | Stmt::Break | Stmt::Continue => false,
        Stmt::ForIn { .. } | Stmt::ForOf { .. } => return None,
    })
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
        ExprKind::Group(inner) | ExprKind::Unary(_, inner) => {
            register_expression_writes_names(inner, names)?
        }
        ExprKind::Conditional(condition, yes, no) => {
            register_expression_writes_names(condition, names)?
                || register_expression_writes_names(yes, names)?
                || register_expression_writes_names(no, names)?
        }
        ExprKind::Call(callee, arguments) => {
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
        ExprKind::Function(function) => register_function_writes_names(function, names)?,
        ExprKind::Literal(_) | ExprKind::Name(_) => false,
        ExprKind::Destructure(pattern, right) => {
            register_expression_writes_names(right, names)?
                || register_assignment_pattern_writes_names(pattern, names)?
        }
        ExprKind::Regex(_, _)
        | ExprKind::Template(_, _)
        | ExprKind::Await(_)
        | ExprKind::Construct(_, _)
        | ExprKind::Class(_)
        | ExprKind::Super
        | ExprKind::NewTarget
        | ExprKind::DefaultSuper
        | ExprKind::This
        | ExprKind::Spread(_)
        | ExprKind::UpdateMember(_, _, _, _) => return None,
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
            Stmt::Function(_, _) => return None,
            _ => {}
        }
    }
    Some(names)
}

fn register_function_scope(function: &Function) -> Option<RegisterFunctionScope> {
    let local_names = register_function_local_names(function)?;
    register_body_scope(&function.body, &local_names)
}

fn register_body_scope(
    body: &[Stmt],
    local_names: &BTreeSet<String>,
) -> Option<RegisterFunctionScope> {
    let mut direct_references = BTreeSet::new();
    let mut nested_free_names = BTreeSet::new();
    for statement in body {
        register_statement_references(statement, &mut direct_references, &mut nested_free_names)?;
    }
    let mut free_names: BTreeSet<_> = direct_references.difference(local_names).cloned().collect();
    let mut captured_names = BTreeSet::new();
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
        ExprKind::Group(inner) | ExprKind::Unary(_, inner) => {
            register_expression_references(inner, names, nested_free_names)?;
        }
        ExprKind::Conditional(condition, yes, no) => {
            register_expression_references(condition, names, nested_free_names)?;
            register_expression_references(yes, names, nested_free_names)?;
            register_expression_references(no, names, nested_free_names)?;
        }
        ExprKind::Call(callee, arguments) => {
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
        ExprKind::Function(function) => {
            nested_free_names.extend(register_function_scope(function)?.free_names);
        }
        ExprKind::Literal(_) => {}
        ExprKind::Destructure(pattern, right) => {
            register_expression_references(right, names, nested_free_names)?;
            register_assignment_pattern_references(pattern, names, nested_free_names)?;
        }
        ExprKind::Regex(_, _)
        | ExprKind::Template(_, _)
        | ExprKind::Await(_)
        | ExprKind::Construct(_, _)
        | ExprKind::Class(_)
        | ExprKind::Super
        | ExprKind::NewTarget
        | ExprKind::DefaultSuper
        | ExprKind::This
        | ExprKind::Spread(_)
        | ExprKind::UpdateMember(_, _, _, _) => return None,
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

fn register_statement_references(
    statement: &Stmt,
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
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
            register_scoped_block_references(body, &BTreeSet::new(), names, nested_free_names)?;
        }
        Stmt::If(condition, yes, no) => {
            register_expression_references(condition, names, nested_free_names)?;
            register_statement_references(yes, names, nested_free_names)?;
            if let Some(no) = no {
                register_statement_references(no, names, nested_free_names)?;
            }
        }
        Stmt::While(condition, body) => {
            register_expression_references(condition, names, nested_free_names)?;
            register_statement_references(body, names, nested_free_names)?;
        }
        Stmt::DoWhile(body, condition) => {
            register_statement_references(body, names, nested_free_names)?;
            register_expression_references(condition, names, nested_free_names)?;
        }
        Stmt::For(initializer, condition, step, body) => {
            register_statement_references(initializer, names, nested_free_names)?;
            if let Some(condition) = condition {
                register_expression_references(condition, names, nested_free_names)?;
            }
            if let Some(step) = step {
                register_expression_references(step, names, nested_free_names)?;
            }
            register_statement_references(body, names, nested_free_names)?;
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
            register_scoped_block_references(body, &BTreeSet::new(), names, nested_free_names)?;
            if let Some((parameter, body)) = catch {
                let mut bound = BTreeSet::new();
                if let Some(parameter) = parameter {
                    let mut declared = Vec::new();
                    parameter.names(&mut declared);
                    bound.extend(declared);
                }
                register_scoped_block_references(body, &bound, names, nested_free_names)?;
            }
            if let Some(body) = finally {
                register_scoped_block_references(body, &BTreeSet::new(), names, nested_free_names)?;
            }
        }
        Stmt::Switch(discriminant, clauses) => {
            // 14.12: one CaseBlock is a single Block scope over every clause.
            register_expression_references(discriminant, names, nested_free_names)?;
            for (test, _) in clauses {
                if let Some(test) = test {
                    register_expression_references(test, names, nested_free_names)?;
                }
            }
            let body: Vec<&Stmt> = clauses.iter().flat_map(|(_, body)| body).collect();
            register_scoped_clause_references(&body, names, nested_free_names)?;
        }
        Stmt::Empty | Stmt::Break | Stmt::Continue => {}
        Stmt::ForIn { .. } | Stmt::ForOf { .. } => return None,
    }
    Some(())
}

/// Collects the free names of one `CaseBlock`, hiding its lexical bindings.
fn register_scoped_clause_references(
    body: &[&Stmt],
    names: &mut BTreeSet<String>,
    nested_free_names: &mut BTreeSet<String>,
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
        register_statement_references(statement, &mut direct, &mut nested)?;
    }
    if nested.iter().any(|name| local_names.contains(name)) {
        return None;
    }
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
) -> Option<()> {
    let mut local_names: BTreeSet<_> = register_block_local_names(body)?.into_keys().collect();
    local_names.extend(bound.iter().cloned());
    let mut direct = BTreeSet::new();
    let mut nested = BTreeSet::new();
    for statement in body {
        register_statement_references(statement, &mut direct, &mut nested)?;
    }
    if nested.iter().any(|name| local_names.contains(name)) {
        return None;
    }
    names.extend(direct.difference(&local_names).cloned());
    nested_free_names.extend(nested.difference(&local_names).cloned());
    Some(())
}

fn register_script_features(body: &[Stmt], realm: bool) -> Option<(bool, bool)> {
    let mut saw_expression = false;
    let mut saw_declaration = false;
    let mut saw_function = false;
    if realm && body.iter().any(register_statement_has_lexical_block) {
        return None;
    }
    for statement in body {
        match statement {
            Stmt::Declare(bindings) if !saw_expression && !realm => {
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
            Stmt::Function(_, _) if !realm => saw_function = true,
            Stmt::Var(_) if !realm => {}
            Stmt::Expr(_)
            | Stmt::Block(_)
            | Stmt::If(_, _, _)
            | Stmt::While(_, _)
            | Stmt::DoWhile(_, _)
            | Stmt::Throw(_)
            | Stmt::Try { .. }
            | Stmt::Switch(_, _)
            | Stmt::For(_, _, _, _) => saw_expression = true,
            Stmt::Empty => {}
            _ => return None,
        }
    }
    (saw_expression
        || body
            .iter()
            .any(|statement| matches!(statement, Stmt::Var(_))))
    .then_some((saw_declaration, saw_function))
}

fn register_statement_has_lexical_block(statement: &Stmt) -> bool {
    match statement {
        Stmt::Block(body) => register_body_has_lexical_block(body),
        Stmt::Switch(_, clauses) => clauses
            .iter()
            .any(|(_, body)| register_body_has_lexical_block(body)),
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            register_body_has_lexical_block(body)
                || catch
                    .as_ref()
                    .is_some_and(|(_, body)| register_body_has_lexical_block(body))
                || finally
                    .as_ref()
                    .is_some_and(|body| register_body_has_lexical_block(body))
        }
        Stmt::If(_, yes, no) => {
            register_statement_has_lexical_block(yes)
                || no
                    .as_deref()
                    .is_some_and(register_statement_has_lexical_block)
        }
        Stmt::While(_, body) | Stmt::DoWhile(body, _) | Stmt::For(_, _, _, body) => {
            register_statement_has_lexical_block(body)
        }
        Stmt::Function(_, function) => function
            .body
            .iter()
            .any(register_statement_has_lexical_block),
        Stmt::Empty
        | Stmt::Expr(_)
        | Stmt::Declare(_)
        | Stmt::Var(_)
        | Stmt::Return(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::ForIn { .. }
        | Stmt::ForOf { .. }
        | Stmt::Throw(_) => false,
    }
}

fn register_body_has_lexical_block(body: &[Stmt]) -> bool {
    body.iter()
        .any(|statement| matches!(statement, Stmt::Declare(_)))
        || body.iter().any(register_statement_has_lexical_block)
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
    lowerer.infer_binding_type_hints(body)?;
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

fn lower_register_script(
    body: &[Stmt],
    realm: bool,
    entry_fuel_cost: u64,
    property_limit: usize,
) -> Option<crate::engine::bytecode::BytecodeFunction> {
    if body
        .iter()
        .any(register_statement_has_unsupported_binding_pattern)
    {
        return None;
    }
    let (saw_declaration, saw_function) = register_script_features(body, realm)?;
    let stack_requirement = body.iter().fold(1usize, |maximum, statement| {
        maximum.max(register_statement_stack_requirement(statement))
    });
    let mut lowerer = RegisterLowerer::new(entry_fuel_cost, stack_requirement, property_limit, 0);
    prepare_register_bindings(&mut lowerer, body, saw_declaration, saw_function)?;
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
                for (pattern, _, initializer) in bindings {
                    if let Some(initializer) = initializer {
                        lowerer.initialize_pattern(pattern, initializer)?;
                    } else {
                        lowerer.initialize(pattern.identifier()?, None)?;
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
    if !completion_type.is_primitive() {
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
    lowerer.code.verify().ok()?;
    Some(lowerer.code)
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

fn register_member_assignment_supported(target: &Expr) -> bool {
    target.member().is_some_and(|(base, key)| {
        base.reference_name().is_some() && register_computed_property_key_supported(key)
    })
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
