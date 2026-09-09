// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Only the compiler creates bytecode. Bindings resolve to stable slot indices;
//! entering a lexical scope resets its slots to the temporal dead zone.

use crate::{
    Error, Limits, Value,
    parser::{self, Binary, Expr, ExprKind, Function, Stmt, Unary},
};
use alloc::{collections::BTreeMap, rc::Rc, string::String, vec::Vec};

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
    IteratorGuard(usize, usize),
    IteratorEnd(usize),
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
    pub const fn binding_count(&self) -> usize {
        self.slots.len()
    }
}

/// Parses and compiles a script without executing any host operation.
///
/// # Errors
/// Returns [`Error::Syntax`] for invalid or unsupported syntax and duplicate
/// declarations, or [`Error::Limit`] when a compilation budget is exceeded.
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
/// Syntax/unsupported grammar, early errors, or compilation resource exhaustion.
pub fn compile_script(source: &str, limits: Limits) -> Result<Script, Error> {
    Ok(Script {
        program: compile_realm(source, limits)?,
        limits,
    })
}
fn compile_mode(source: &str, limits: Limits, realm: bool) -> Result<Program, Error> {
    let body = parser::parse(source, limits)?;
    let mut compiler = Compiler {
        program: Program {
            code: Vec::new(),
            slots: Vec::new(),
            functions: Vec::new(),
            total_instructions: 0,
            globals: Vec::new(),
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
        compiler.global_body(&body)?;
    } else {
        compiler.root_body(&body)?;
    }
    compiler.finish();
    Ok(compiler.program)
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
            for (name, mutable, _) in bindings {
                self.declare(name, *mutable)?;
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
            Stmt::Var(bindings) => {
                for (name, init) in bindings {
                    if let Some(init) = init {
                        self.expression(init)?;
                        let op = if let Some(access) = self.resolve(name) {
                            Op::Store(access)
                        } else if self.realm {
                            Op::SetGlobal(name.clone(), init.strict)
                        } else {
                            return Err(Error::InvalidBytecode);
                        };
                        self.emit(op)?;
                        self.emit(Op::Pop)?;
                    }
                }
            }
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
            Stmt::For(init, cond, step, body) => {
                if let Stmt::Declare(bindings) = init.as_ref() {
                    let mut names = Vec::new();
                    var_names(body, &mut names);
                    if bindings.iter().any(|(name, _, _)| names.contains(name)) {
                        return Err(Error::Syntax {
                            offset: 0,
                            message: "loop var conflicts with lexical binding",
                        });
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
                    for (name, mutable, _) in bindings {
                        if *mutable {
                            per_iteration.push(self.local(name).ok_or(Error::InvalidBytecode)?);
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
        bindings: &[(String, bool, Option<Expr>)],
    ) -> Result<(), Error> {
        for (name, _, init) in bindings {
            if let Some(init) = init {
                self.expression(init)?;
            } else {
                self.emit(Op::Constant(Value::Undefined))?;
            }
            let op = if let Some(slot) = self.local(name) {
                Op::Init(slot)
            } else if self.realm {
                Op::InitGlobal(name.clone())
            } else {
                return Err(Error::InvalidBytecode);
            };
            self.emit(op)?;
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
                if let Stmt::Declare(bindings) = stmt
                    && bindings.iter().any(|(name, _, _)| vars.contains(name))
                {
                    return Err(Error::Syntax {
                        offset: 0,
                        message: "switch var conflicts with lexical declaration",
                    });
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
        binding: Option<&(String, Option<bool>)>,
        target: Option<&Expr>,
        object: &Expr,
        body: &Stmt,
    ) -> Result<(), Error> {
        let id = self.enumerations;
        self.enumerations = self.enumerations.saturating_add(1);
        self.scopes.push(BTreeMap::new());
        if let Some((name, Some(mutable))) = binding {
            let mut names = Vec::new();
            var_names(body, &mut names);
            if names.contains(name) {
                return Err(Error::Syntax {
                    offset: 0,
                    message: "for-in lexical binding conflicts with var",
                });
            }
            self.declare(name, *mutable)?;
        }
        self.emit(Op::Constant(Value::Undefined))?;
        self.emit(Op::Result)?;
        self.expression(object)?;
        self.emit(Op::ForInInit(id))?;
        let head = self.program.code.len();
        let end = self.emit(Op::ForInNext(id, 0))?;
        if let Some((name, kind)) = binding {
            if kind.is_some() {
                let slot = self.local(name).ok_or(Error::InvalidBytecode)?;
                self.emit(Op::Reset(slot))?;
                self.emit(Op::Init(slot))?;
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
        } else if let Some(target) = target {
            if let Some(name) = target.reference_name() {
                let op = if let Some(slot) = self.resolve(name) {
                    Op::Store(slot)
                } else if self.realm {
                    Op::SetGlobal(String::from(name), target.strict)
                } else {
                    Op::Missing(String::from(name))
                };
                self.emit(op)?;
                self.emit(Op::Pop)?;
            } else {
                let (base, key) = target.member().ok_or(Error::InvalidBytecode)?;
                self.reference(base, key)?;
                self.emit(Op::RotateKey)?;
                self.emit(Op::Set(target.strict))?;
                self.emit(Op::Pop)?;
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
        target: Option<&Expr>,
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
            if let Some(name) = target.reference_name() {
                let op = if let Some(slot) = self.resolve(name) {
                    Op::Store(slot)
                } else if self.realm {
                    Op::SetGlobal(String::from(name), target.strict)
                } else {
                    Op::Missing(String::from(name))
                };
                self.emit(op)?;
                self.emit(Op::Pop)?;
            } else {
                let (base, key) = target.member().ok_or(Error::InvalidBytecode)?;
                self.reference(base, key)?;
                self.emit(Op::RotateKey)?;
                self.emit(Op::Set(target.strict))?;
                self.emit(Op::Pop)?;
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
                    let slot = self.local(name).ok_or(Error::InvalidBytecode)?;
                    self.emit(Op::Init(slot))?;
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
            parser::BindingPattern::Array(items) => {
                let id = self.enumerations;
                self.enumerations = self.enumerations.saturating_add(1);
                self.emit(Op::ForOfInit(id))?;
                let guard = self.emit(Op::IteratorGuard(id, 0))?;
                for item in items {
                    if let Some(item) = item {
                        self.emit(Op::IteratorValue(id))?;
                        self.bind_pattern(item, initialize)?;
                    } else {
                        self.emit(Op::IteratorSkip(id))?;
                    }
                }
                let end = self.program.code.len();
                self.emit(Op::IteratorEnd(id))?;
                self.patch(guard, end.saturating_add(1))?;
            }
        }
        Ok(())
    }

    fn try_statement(
        &mut self,
        body: &[Stmt],
        catch: Option<&(Option<String>, Vec<Stmt>)>,
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
        let catch_at = if let Some((name, body)) = catch {
            let entry = self.program.code.len();
            self.scopes.push(BTreeMap::new());
            if let Some(name) = name {
                if body.iter().any(|stmt| matches!(stmt, Stmt::Declare(bindings) if bindings.iter().any(|(bound,_,_)| bound == name))) {
                    return Err(Error::Syntax { offset: 0, message: "catch binding conflicts with lexical declaration" });
                }
                self.declare(name, true)?;
                self.emit(Op::Init(self.local(name).ok_or(Error::InvalidBytecode)?))?;
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
            ExprKind::Class(class) => self.class(class)?,
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
                self.call(callee, args)?;
            }
            ExprKind::Construct(callee, args) => {
                self.construct(callee, args)?;
            }
        }
        Ok(())
    }

    fn call(&mut self, callee: &Expr, args: &[Expr]) -> Result<(), Error> {
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
        self.emit(if expanded {
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

    fn super_reference(&mut self, key: &Expr) -> Result<(), Error> {
        self.emit(Op::SuperBase)?;
        self.expression(key)?;
        self.emit(Op::Key)?;
        Ok(())
    }
    fn class(&mut self, class: &parser::Class) -> Result<(), Error> {
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
        self.function(&class.constructor, class.name.as_deref())?;
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
                for (name, mutable, _) in bindings {
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
                        name: name.clone(),
                        kind: GlobalKind::Lexical(*mutable),
                    });
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
        let complex = function
            .parameters
            .iter()
            .any(|p| p.rest || p.default.is_some());
        let mut slots = Vec::new();
        // Discard only the self-name reset: named function values initialize it
        // in the call setup. Complex parameters stay uninitialized until code.
        self.program.code.clear();
        self.program.total_instructions = 0;
        for parameter in &function.parameters {
            let name = &parameter.name;
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
            slots.push(self.local(name).ok_or(Error::InvalidBytecode)?);
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
                self.expression(default)?;
                self.patch(jump, self.program.code.len())?;
            }
            self.emit(Op::Init(*slots.get(index).ok_or(Error::InvalidBytecode)?))?;
        }
        self.emit(Op::EndParameters)?;
        if function.parameters.iter().any(|p| p.default.is_some()) {
            self.parameter_body_scope(function)?;
        }
        Ok((slots, true))
    }

    fn arguments_binding(&mut self, function: &Function) -> Result<Option<usize>, Error> {
        let needed = !function.arrow
            && !function.parameters.iter().any(|p| p.name == "arguments")
            && (function.parameters.iter().any(|p| p.default.is_some())
                || !function.body.iter().any(|stmt| match stmt {
                    Stmt::Function(name, _) => name == "arguments",
                    Stmt::Declare(bindings) => {
                        bindings.iter().any(|(name, _, _)| name == "arguments")
                    }
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
                .any(|op| matches!(op,Op::Load(Access::Local(index)) if *index==slot))
    }

    fn parameter_body_scope(&mut self, function: &Function) -> Result<(), Error> {
        let mut names = Vec::new();
        for stmt in &function.body {
            var_names(stmt, &mut names);
        }
        self.scopes.push(BTreeMap::new());
        for parameter in &function.parameters {
            if names.contains(&parameter.name) {
                let from = self.local(&parameter.name).ok_or(Error::InvalidBytecode)?;
                self.declare(&parameter.name, true)?;
                self.emit(Op::Load(Access::Local(from)))?;
                self.emit(Op::Init(
                    self.local(&parameter.name).ok_or(Error::InvalidBytecode)?,
                ))?;
            }
        }
        Ok(())
    }
}

fn var_names(stmt: &Stmt, names: &mut Vec<String>) {
    match stmt {
        Stmt::ForOf { binding, body, .. } => {
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
        Stmt::ForIn { binding, body, .. } => {
            if let Some((name, None)) = binding {
                names.push(name.clone());
            }
            var_names(body, names);
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
        Stmt::Var(bindings) => names.extend(bindings.iter().map(|(name, _)| name.clone())),
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
        Stmt::While(_, body) => var_names(body, names),
        Stmt::For(init, _, _, body) => {
            var_names(init, names);
            var_names(body, names);
        }
        _ => {}
    }
}

fn validate_function(function: &Function) -> Result<(), Error> {
    for stmt in &function.body {
        if let Stmt::Declare(bindings) = stmt
            && bindings
                .iter()
                .any(|(name, _, _)| function.parameters.iter().any(|p| p.name == *name))
        {
            return Err(Error::Syntax {
                offset: 0,
                message: "parameter conflicts with lexical declaration",
            });
        }
    }
    if function.strict
        && function
            .parameters
            .iter()
            .map(|p| &p.name)
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
            for (name, _, _) in bindings {
                if vars.contains(name) {
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
