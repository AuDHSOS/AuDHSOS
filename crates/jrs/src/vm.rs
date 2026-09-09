// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Safe bytecode dispatch. Every instruction consumes fuel, including jumps.
//! Each run resets all state; programs have no ambient filesystem or network.

use crate::{
    Error, Limits, Program, Value,
    bytecode::{Access, Branch, Builtin, FunctionCode, Op, ParameterInitialization},
    heap::{Handle, Heap, Node},
    object::Object,
    parser::{Binary, Unary},
    value::{Callable, FunctionValue},
};
use alloc::{collections::VecDeque, rc::Rc, vec::Vec};

mod abort;
mod arrays;
mod async_function;
mod boxing;
mod classes;
mod conversion;
mod dom_exception;
mod dynamic_function;
mod embedding;
mod enumeration;
mod errors;
mod events;
mod exceptions;
mod functions;
mod instance;
mod iterators;
mod json;
mod numbers;
pub(crate) mod promises;
mod properties;
mod realm;
mod reflect;
mod regexp;
pub use realm::Realm;
mod spread;
mod strings;
mod symbols;
mod timers;
mod weakmap;
use enumeration::Enumeration;
use exceptions::{Completion, Handler};

struct Frame {
    code: Option<Rc<FunctionCode>>,
    pc: usize,
    locals: Vec<Binding>,
    captures: Vec<Handle>,
    base: usize,
    result: Value,
    callee: Value,
    this_value: Value,
    handlers: Vec<Handler>,
    constructing: bool,
    arguments: Vec<Value>,
    enumerations: Vec<Option<Enumeration>>,
    async_promise: Option<Value>,
    new_target: Value,
    this_cell: Option<Handle>,
}

#[derive(Clone)]
enum Binding {
    Direct(Option<Value>),
    Cell(Handle),
}

impl Frame {
    fn script(program: &Program, base: usize) -> Self {
        Self {
            code: None,
            pc: 0,
            locals: alloc::vec![Binding::Direct(None);program.slots.len()],
            captures: Vec::new(),
            base,
            result: Value::Undefined,
            callee: Value::Undefined,
            this_value: Value::Undefined,
            handlers: Vec::new(),
            constructing: false,
            arguments: Vec::new(),
            enumerations: Vec::new(),
            async_promise: None,
            new_target: Value::Undefined,
            this_cell: None,
        }
    }
    fn program<'a>(&'a self, top: &'a Program) -> &'a Program {
        self.code.as_ref().map_or(top, |code| &code.program)
    }
    fn handle(&self, access: Access) -> Result<Handle, Error> {
        match access {
            Access::Local(index) => match self.locals.get(index) {
                Some(Binding::Cell(handle)) => Some(*handle),
                _ => None,
            },
            Access::Capture(index) => self.captures.get(index).copied(),
        }
        .ok_or(Error::InvalidBytecode)
    }
}

/// Capabilities deliberately granted by the embedding.
pub trait Host {
    /// Monotonic active-time nanoseconds for explicitly installed timers.
    /// A browser host must exclude suspended/inactive time as appropriate.
    /// No wall clock is granted by this capability.
    ///
    /// # Errors
    /// Default denies clock access; clock failure poisons timer operations.
    fn timer_now(&mut self) -> Result<u128, Error> {
        Err(Error::Unsupported {
            feature: "host timer clock",
        })
    }
    /// Policy gate for dynamically compiled source, after argument conversion
    /// and before syntax analysis. Default allows compilation. This capability
    /// is distinct from script I/O and must not grant ambient filesystem access.
    ///
    /// # Errors
    /// Host/resource failure denies execution; language errors are catchable.
    fn ensure_can_compile_strings(&mut self) -> Result<(), Error> {
        Ok(())
    }
    /// Milliseconds relative to the embedding's time origin for event creation.
    /// The default has no clock and returns zero. Browser hosts must supply a
    /// monotonic, appropriately precision-reduced timestamp.
    fn event_timestamp(&mut self) -> f64 {
        0.0
    }
    /// Accepts one `print(...)` call with evaluated arguments.
    ///
    /// # Errors
    /// Returns [`Error::Host`] if output cannot be accepted.
    fn print(&mut self, arguments: &[Value]) -> Result<(), Error>;

    /// Invokes an explicitly installed host function identified by the embedding.
    /// Receiver and arguments are not coerced. Heap identities passed here are
    /// retained by the realm until explicitly released or the realm is dropped.
    /// The host may store them for later `Realm::call` but cannot reenter the
    /// borrowed realm during this call. Host work must impose its own time/memory
    /// bounds: VM fuel cannot interrupt arbitrary Rust code.
    ///
    /// # Errors
    /// The default returns [`Error::Host`]. Language errors may be caught by JS;
    /// resource/host errors terminate execution. Returned/thrown heap values must
    /// belong to the calling realm; foreign or stale identities are rejected.
    fn call(&mut self, _id: u32, _receiver: &Value, _arguments: &[Value]) -> Result<Value, Error> {
        Err(Error::Host)
    }

    /// Reports a rejection still unhandled after the job checkpoint. The default
    /// fails execution so rejected tests cannot silently look successful.
    ///
    /// # Errors
    /// The default returns the thrown reason; an embedding may record it instead.
    fn unhandled_rejection(&mut self, reason: &Value) -> Result<(), Error> {
        Err(Error::Thrown {
            value: reason.clone(),
        })
    }

    /// Reports an exception from an explicitly queued microtask, separately
    /// from rejected Promises. A browser embedding can dispatch an `ErrorEvent`;
    /// this core does not provide DOM events or cross-realm error routing.
    /// The reason is retained for later host inspection. Returning a language
    /// error fails the checkpoint after the remaining microtasks have run;
    /// returning a host/resource error stops immediately and poisons the realm.
    ///
    /// # Errors
    /// The default fails the checkpoint with the original exception value.
    fn report_exception(&mut self, reason: &Value) -> Result<(), Error> {
        Err(Error::Thrown {
            value: reason.clone(),
        })
    }
}

/// An embedding with no observable output capability.
#[derive(Default)]
pub struct SilentHost;

impl Host for SilentHost {
    fn print(&mut self, _: &[Value]) -> Result<(), Error> {
        Ok(())
    }
}

/// An isolated, fuel-limited bytecode executor. Buffers are reused between runs.
pub struct Runtime {
    limits: Limits,
    stack: Vec<Value>,
    intrinsic_code: Vec<(Builtin, Rc<FunctionCode>)>,
}

struct Execution<'host> {
    host: &'host mut dyn Host,
    limits: Limits,
    stack: Vec<Value>,
    heap: Heap,
    frames: Vec<Frame>,
    owner: Rc<()>,
    binding_slots: usize,
    object_prototype: Option<Value>,
    global: Option<Value>,
    array_proto: Option<Value>,
    intrinsic_code: Vec<(Builtin, Rc<FunctionCode>)>,
    fuel: u64,
    top_program: Option<Rc<Program>>,
    native_depth: usize,
    flatten_frames: usize,
    native_roots: Vec<Value>,
    unwind_boundary: usize,
    math: Option<Value>,
    json: Option<Value>,
    reflect: Option<Value>,
    globals_initialized: bool,
    object_enumerators: Option<(Value, Value)>,
    object_constructor_storage: Option<Value>,
    array_constructor_storage: Option<Value>,
    number_constructor_storage: Option<Value>,
    regexp_constructor_storage: Option<Value>,
    regexp_proto: Option<Value>,
    number_parsers: Option<(Value, Value)>,
    // Shared across nested native JSON calls as well as container traversal.
    json_depth: usize,
    promise_proto: Option<Value>,
    jobs: VecDeque<crate::promise::Job>,
    timers: timers::Timers,
    suspended_frames: usize,
    suspended_slots: usize,
    rejections: Vec<Value>,
    reaction_count: usize,
    error_prototypes: Vec<(Builtin, Value)>,
    function_proto: Option<Value>,
    async_function_proto: Option<Value>,
    async_function_ctor: Option<Value>,
    throw_type_error: Option<Value>,
    primitive_protos: Vec<(Builtin, Value)>,
    weakmap_proto: Option<Value>,
    weak_entries: usize,
    symbol_proto: Option<Value>,
    well_known: alloc::collections::BTreeMap<&'static str, crate::SymbolValue>,
    symbol_registry: alloc::collections::BTreeMap<Rc<[u16]>, crate::SymbolValue>,
    iterator_protos: Vec<(Builtin, Value)>,
    iterator_methods: Vec<(Builtin, Value)>,
    global_lexicals: alloc::collections::BTreeMap<alloc::string::String, Handle>,
    retained: alloc::collections::BTreeMap<(Handle, bool), Value>,
    event_proto: Option<Value>,
    custom_event_proto: Option<Value>,
    event_trusted_getter: Option<Value>,
    dom_exception_proto: Option<Value>,
    event_target_proto: Option<Value>,
    event_depth: usize,
    abort_controller_proto: Option<Value>,
    abort_signal_proto: Option<Value>,
    reported_exceptions: Vec<Error>,
}

impl Runtime {
    /// Creates an executor with explicit resource limits.
    #[must_use]
    pub const fn new(limits: Limits) -> Self {
        Self {
            limits,
            stack: Vec::new(),
            intrinsic_code: Vec::new(),
        }
    }

    /// Executes a previously compiled program in a fresh lexical environment.
    /// Returns the script's completion value. No state survives a run except
    /// reused buffer capacity and effects explicitly accepted by the host.
    ///
    /// # Errors
    /// Returns reference/type errors, resource exhaustion, or host failure.
    pub fn run(&mut self, program: &Program, host: &mut impl Host) -> Result<Value, Error> {
        let mut execution = Execution::new(host, self.limits);
        execution.stack = core::mem::take(&mut self.stack);
        execution.intrinsic_code = core::mem::take(&mut self.intrinsic_code);
        let result = execution.run(program);
        self.stack = execution.stack;
        self.intrinsic_code = execution.intrinsic_code;
        result
    }
}

impl<'host> Execution<'host> {
    fn new(host: &'host mut dyn Host, limits: Limits) -> Self {
        Self {
            host,
            limits,
            stack: Vec::new(),
            intrinsic_code: Vec::new(),
            heap: Heap::default(),
            frames: Vec::new(),
            owner: Rc::new(()),
            binding_slots: 0,
            object_prototype: None,
            global: None,
            array_proto: None,
            fuel: 0,
            top_program: None,
            native_depth: 0,
            flatten_frames: 0,
            native_roots: Vec::new(),
            unwind_boundary: 0,
            math: None,
            json: None,
            reflect: None,
            globals_initialized: false,
            object_enumerators: None,
            object_constructor_storage: None,
            array_constructor_storage: None,
            number_constructor_storage: None,
            regexp_constructor_storage: None,
            regexp_proto: None,
            number_parsers: None,
            json_depth: 0,
            promise_proto: None,
            jobs: VecDeque::new(),
            timers: timers::Timers::new(limits.jobs),
            suspended_frames: 0,
            suspended_slots: 0,
            rejections: Vec::new(),
            reaction_count: 0,
            error_prototypes: Vec::new(),
            function_proto: None,
            async_function_proto: None,
            async_function_ctor: None,
            throw_type_error: None,
            primitive_protos: Vec::new(),
            weakmap_proto: None,
            weak_entries: 0,
            symbol_proto: None,
            well_known: alloc::collections::BTreeMap::new(),
            symbol_registry: alloc::collections::BTreeMap::new(),
            iterator_protos: Vec::new(),
            iterator_methods: Vec::new(),
            global_lexicals: alloc::collections::BTreeMap::new(),
            retained: alloc::collections::BTreeMap::new(),
            event_proto: None,
            custom_event_proto: None,
            event_trusted_getter: None,
            dom_exception_proto: None,
            event_target_proto: None,
            event_depth: 0,
            abort_controller_proto: None,
            abort_signal_proto: None,
            reported_exceptions: Vec::new(),
        }
    }
}

impl Execution<'_> {
    fn run(&mut self, program: &Program) -> Result<Value, Error> {
        self.stack.clear();
        self.frames.clear();
        self.heap = Heap::default();
        self.object_prototype = None;
        self.global = None;
        self.array_proto = None;
        self.fuel = self.limits.fuel;
        self.owner = Rc::new(());
        self.binding_slots = program.slots.len();
        if self.binding_slots > self.limits.binding_slots {
            return Err(Error::Limit {
                resource: "binding slots",
            });
        }
        if program.instruction_count() > self.limits.instructions {
            return Err(Error::Limit {
                resource: "bytecode instructions",
            });
        }
        self.frames.push(Frame {
            code: None,
            pc: 0,
            locals: alloc::vec![Binding::Direct(None); program.slots.len()],
            captures: Vec::new(),
            base: 0,
            result: Value::Undefined,
            callee: Value::Undefined,
            this_value: Value::Undefined,
            handlers: Vec::new(),
            constructing: false,
            arguments: Vec::new(),
            enumerations: Vec::new(),
            async_promise: None,
            new_target: Value::Undefined,
            this_cell: None,
        });
        self.top_program = Some(Rc::new(program.clone()));
        let result = self.execute(program, 0).and_then(|value| {
            self.native_roots.push(value.clone());
            self.drain_jobs()?;
            Ok(value)
        });
        // Release values on both success and failure, while retaining capacity.
        self.stack.clear();
        self.frames.clear();
        self.heap = Heap::default();
        self.binding_slots = 0;
        self.object_prototype = None;
        self.global = None;
        self.array_proto = None;
        self.top_program = None;
        self.native_depth = 0;
        self.native_roots.clear();
        result
    }

    fn push(&mut self, value: Value) -> Result<(), Error> {
        if self.stack.len().saturating_add(self.suspended_slots) >= self.limits.stack {
            return Err(Error::Limit {
                resource: "operand stack",
            });
        }
        if let Value::String(units) = &value
            && units.len() > self.limits.string_units
        {
            return Err(Error::Limit {
                resource: "string units",
            });
        }
        self.stack.push(value);
        Ok(())
    }
    fn pop(&mut self) -> Result<Value, Error> {
        self.stack.pop().ok_or(Error::InvalidBytecode)
    }
    fn load(&self, access: Access, program: &Program) -> Result<Value, Error> {
        let frame = self.frames.last().ok_or(Error::InvalidBytecode)?;
        if let Access::Local(index) = access
            && let Some(Binding::Direct(value)) = frame.locals.get(index)
        {
            return value.clone().ok_or_else(|| {
                frame
                    .program(program)
                    .slots
                    .get(index)
                    .map_or(Error::InvalidBytecode, |slot| Error::Reference {
                        name: slot.name.clone(),
                    })
            });
        }
        let handle = self
            .frames
            .last()
            .ok_or(Error::InvalidBytecode)?
            .handle(access)?;
        match self.heap.get(handle)? {
            Node::Cell { value, slot } => value.clone().ok_or_else(|| Error::Reference {
                name: slot.name.clone(),
            }),
            Node::Symbol
            | Node::HostFunction { .. }
            | Node::Function { .. }
            | Node::Object(_)
            | Node::Resolving { .. }
            | Node::Suspended(_)
            | Node::Bound { .. } => Err(Error::InvalidBytecode),
        }
    }
    fn store(
        &mut self,
        access: Access,
        value: Value,
        initialize: bool,
        program: &Program,
    ) -> Result<(), Error> {
        let frame = self.frames.last_mut().ok_or(Error::InvalidBytecode)?;
        if let Access::Local(index) = access
            && matches!(frame.locals.get(index), Some(Binding::Direct(_)))
        {
            let slot = frame
                .program(program)
                .slots
                .get(index)
                .ok_or(Error::InvalidBytecode)?;
            if !initialize {
                if matches!(frame.locals.get(index), Some(Binding::Direct(None))) {
                    return Err(Error::Reference {
                        name: slot.name.clone(),
                    });
                }
                if !slot.mutable {
                    return Err(Error::Type {
                        message: "assignment to a constant binding",
                    });
                }
            }
            *frame.locals.get_mut(index).ok_or(Error::InvalidBytecode)? =
                Binding::Direct(Some(value));
            return Ok(());
        }
        let handle = self
            .frames
            .last()
            .ok_or(Error::InvalidBytecode)?
            .handle(access)?;
        match self.heap.get_mut(handle)? {
            Node::Cell {
                slot,
                value: current,
            } => {
                if !initialize {
                    if current.is_none() {
                        return Err(Error::Reference {
                            name: slot.name.clone(),
                        });
                    }
                    if !slot.mutable {
                        return Err(Error::Type {
                            message: "assignment to a constant binding",
                        });
                    }
                }
                *current = Some(value);
                Ok(())
            }
            Node::Symbol
            | Node::HostFunction { .. }
            | Node::Function { .. }
            | Node::Object(_)
            | Node::Resolving { .. }
            | Node::Suspended(_)
            | Node::Bound { .. } => Err(Error::InvalidBytecode),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive hot dispatch loop keeps bytecode borrowing and frame transitions explicit"
    )]
    fn execute(&mut self, program: &Program, boundary: usize) -> Result<Value, Error> {
        let mut active = self.frames.last().and_then(|frame| frame.code.clone());
        let mut pc = self.frames.last().map_or(0, |frame| frame.pc);
        loop {
            if boundary > 0 && self.frames.len() <= boundary {
                return self.pop();
            }
            let code = active
                .as_ref()
                .map_or(program, |function| &function.program);
            let Some(op) = code.code.get(pc) else {
                if self.frames.len() != boundary.saturating_add(1) || active.is_some() {
                    return Err(Error::InvalidBytecode);
                }
                if self.stack.len() != self.frames.last().ok_or(Error::InvalidBytecode)?.base {
                    return Err(Error::InvalidBytecode);
                }
                return Ok(self
                    .frames
                    .last()
                    .ok_or(Error::InvalidBytecode)?
                    .result
                    .clone());
            };
            self.fuel = self.fuel.checked_sub(1).ok_or(Error::Limit {
                resource: "execution fuel",
            })?;
            pc = pc.saturating_add(1);
            let mut frame_changed = false;
            let step = (|| -> Result<(), Error> {
                match op {
                    Op::ArgumentAppend(spread) => self.append_arguments(*spread)?,
                    Op::ArrayAppend(spread) => self.append_array(*spread)?,
                    Op::ArrayHole => self.array_hole()?,
                    Op::Class(_)
                    | Op::ClassMethod(_, _)
                    | Op::MethodHome
                    | Op::NewTarget
                    | Op::SuperBase
                    | Op::SuperGet(_)
                    | Op::SuperSet
                    | Op::SuperUpdate(_, _)
                    | Op::SuperCall(_)
                    | Op::PrepareSuperCall
                    | Op::SuperCallExpanded
                    | Op::DefaultSuper => self.class_op(op)?,
                    Op::IteratorGuard(id, end) => {
                        let mut handler = Handler::new(pc, None, None, *end, self.stack.len());
                        handler.iterator = Some(*id);
                        self.frames
                            .last_mut()
                            .ok_or(Error::InvalidBytecode)?
                            .handlers
                            .push(handler);
                    }
                    Op::IteratorEnd(id) => {
                        let handler = self
                            .frames
                            .last_mut()
                            .ok_or(Error::InvalidBytecode)?
                            .handlers
                            .pop()
                            .ok_or(Error::InvalidBytecode)?;
                        if handler.iterator != Some(*id) {
                            return Err(Error::InvalidBytecode);
                        }
                        self.close_iteration(*id, false)?;
                    }
                    Op::Await => {
                        let value = self.pop()?;
                        self.suspend_await(value, pc)?;
                        pc = self.frames.last().ok_or(Error::InvalidBytecode)?.pc;
                        frame_changed = true;
                    }
                    Op::Handler {
                        catch,
                        finally,
                        end,
                    } => {
                        self.frames
                            .last_mut()
                            .ok_or(Error::InvalidBytecode)?
                            .handlers
                            .push(Handler::new(pc, *catch, *finally, *end, self.stack.len()));
                    }
                    Op::EndTry => {
                        pc = self.end_try()?;
                    }
                    Op::EndFinally => {
                        self.end_finally(&mut pc)?;
                        frame_changed = true;
                    }
                    Op::Throw => {
                        let value = self.pop()?;
                        return Err(Error::Thrown { value });
                    }
                    Op::AbruptJump(target) => {
                        self.abrupt(Completion::Jump(*target), &mut pc)?;
                    }
                    Op::Object
                    | Op::Array(_)
                    | Op::Define(_)
                    | Op::Accessor(_)
                    | Op::Key
                    | Op::Get(_)
                    | Op::Set(_)
                    | Op::Delete(_)
                    | Op::UpdateProperty(_, _, _)
                    | Op::DupPair
                    | Op::This
                    | Op::Global => self.property_op(op)?,
                    Op::Math => {
                        let value = self.math_object()?;
                        self.push(value)?;
                    }
                    Op::Json => {
                        let value = self.json_object()?;
                        self.push(value)?;
                    }
                    Op::Reflect => {
                        let value = self.reflect_object()?;
                        self.push(value)?;
                    }
                    Op::Nop => {}
                    Op::Constant(value) => self.push(value.clone())?,
                    Op::ForInInit(id) => {
                        let value = self.pop()?;
                        self.enumeration_init(*id, value, false)?;
                    }
                    Op::ForOfInit(id) => {
                        let value = self.pop()?;
                        self.enumeration_init(*id, value, true)?;
                    }
                    Op::IteratorValue(id) => {
                        let value = self.enumeration_next(*id)?.unwrap_or(Value::Undefined);
                        self.push(value)?;
                    }
                    Op::IteratorSkip(id) => {
                        self.enumeration_step_value(*id, false)?;
                    }
                    Op::ForInNext(id, end) => {
                        if let Some(key) = self.enumeration_next(*id)? {
                            self.push(key)?;
                        } else {
                            pc = *end;
                        }
                    }
                    Op::ForInEnd(id) => {
                        *self
                            .frames
                            .last_mut()
                            .and_then(|f| f.enumerations.get_mut(*id))
                            .ok_or(Error::InvalidBytecode)? = None;
                    }
                    Op::RotateKey => {
                        let key = self.pop()?;
                        let base = self.pop()?;
                        let value = self.pop()?;
                        self.push(base)?;
                        self.push(key)?;
                        self.push(value)?;
                    }
                    Op::Argument(index) => {
                        let value = self
                            .frames
                            .last()
                            .and_then(|f| f.arguments.get(*index))
                            .cloned()
                            .unwrap_or(Value::Undefined);
                        self.push(value)?;
                    }
                    Op::RestArguments(index) => {
                        let values = self
                            .frames
                            .last()
                            .ok_or(Error::InvalidBytecode)?
                            .arguments
                            .get(*index..)
                            .unwrap_or_default()
                            .to_vec();
                        let array = self.array_from_values(&values)?;
                        self.push(array)?;
                    }
                    Op::EndParameters => self
                        .frames
                        .last_mut()
                        .ok_or(Error::InvalidBytecode)?
                        .arguments
                        .clear(),
                    Op::ToString => {
                        let value = self.pop()?;
                        let text = self.string_units(&value)?;
                        self.push(Value::String(text))?;
                    }
                    Op::Regex(regex) => {
                        let value = self.new_regexp(regex.clone())?;
                        self.push(value)?;
                    }
                    Op::Load(index) => self.push(self.load(*index, program)?)?,
                    Op::Store(index) => {
                        let value = self.stack.last().ok_or(Error::InvalidBytecode)?.clone();
                        self.store(*index, value, false, program)?;
                    }
                    Op::Init(index) => {
                        let value = self.pop()?;
                        self.store(Access::Local(*index), value, true, program)?;
                    }
                    Op::Reset(index) => {
                        self.cell(*index, None, program)?;
                    }
                    Op::Renew(index) => {
                        let value = self.load(Access::Local(*index), program)?;
                        self.cell(*index, Some(value), program)?;
                    }
                    Op::Missing(name) => return Err(Error::Reference { name: name.clone() }),
                    Op::GetGlobal(name, typeof_operand) => {
                        let value = self.global_get(name, *typeof_operand)?;
                        self.push(value)?;
                    }
                    Op::DeleteGlobal(name) => {
                        let result = self.global_delete(name)?;
                        self.push(Value::Boolean(result))?;
                    }
                    Op::SetGlobal(name, strict) => {
                        let value = self.stack.last().cloned().ok_or(Error::InvalidBytecode)?;
                        self.global_set(name, value, *strict, false)?;
                    }
                    Op::InitGlobal(name) => {
                        let value = self.pop()?;
                        self.global_set(name, value, true, true)?;
                    }
                    Op::UpdateGlobal(name, add, prefix, strict) => {
                        let old = self.global_get(name, false)?;
                        let number = self.numeric(&old)?;
                        let next = if *add { number + 1.0 } else { number - 1.0 };
                        self.global_set(name, Value::Number(next), *strict, false)?;
                        self.push(Value::Number(if *prefix { next } else { number }))?;
                    }
                    Op::Unary(op) => {
                        let value = self.pop()?;
                        let value = self.convert_unary(*op, value)?;
                        self.push(value)?;
                    }
                    Op::Binary(op) => {
                        if !self.numeric_binary_in_place(*op)? {
                            let right = self.pop()?;
                            let left = self.pop()?;
                            let value = self.convert_binary(*op, &left, &right)?;
                            self.push(value)?;
                        }
                    }
                    Op::Dup => {
                        self.push(self.stack.last().ok_or(Error::InvalidBytecode)?.clone())?;
                    }
                    Op::Pop => {
                        self.pop()?;
                    }
                    Op::Result => {
                        let value = self.pop()?;
                        self.frames.last_mut().ok_or(Error::InvalidBytecode)?.result = value;
                    }
                    Op::Jump(target) => {
                        pc = *target;
                    }
                    Op::Branch(target, kind) => {
                        let value = self.pop()?;
                        if match kind {
                            Branch::False => !value.to_boolean(),
                            Branch::True => value.to_boolean(),
                            Branch::NotNullish => !matches!(value, Value::Null | Value::Undefined),
                        } {
                            pc = *target;
                        }
                    }
                    Op::Construct(_) | Op::ConstructExpanded => {
                        let count = if let Op::Construct(count) = op {
                            *count
                        } else {
                            self.expanded_count()?
                        };
                        self.frames.last_mut().ok_or(Error::InvalidBytecode)?.pc = pc;
                        self.construct_dispatch(count, program)?;
                        pc = self.frames.last().ok_or(Error::InvalidBytecode)?.pc;
                        frame_changed = true;
                    }
                    Op::Call(_) | Op::CallExpanded => {
                        let count = if let Op::Call(count) = op {
                            *count
                        } else {
                            self.expanded_count()?
                        };
                        self.frames.last_mut().ok_or(Error::InvalidBytecode)?.pc = pc;
                        self.invoke(count, program)?;
                        let frame = self.frames.last().ok_or(Error::InvalidBytecode)?;
                        pc = frame.pc;
                        frame_changed = true;
                    }
                    Op::Closure(index) => self.closure(*index, program)?,
                    Op::Return => {
                        let value = self.pop()?;
                        self.abrupt(Completion::Return(value), &mut pc)?;
                        frame_changed = true;
                    }
                    Op::Update(index, add, prefix) => {
                        let old_value = self.load(*index, program)?;
                        let old = self.numeric(&old_value)?;
                        let value =
                            arithmetic(if *add { Binary::Add } else { Binary::Sub }, old, 1.0)?;
                        self.store(*index, Value::Number(value), false, program)?;
                        self.push(Value::Number(if *prefix { value } else { old }))?;
                    }
                }
                Ok(())
            })();
            if frame_changed {
                active.clone_from(&self.frames.last().ok_or(Error::InvalidBytecode)?.code);
            }
            if let Err(error) = step {
                if !self.catchable(&error) {
                    return Err(error);
                }
                self.abrupt_until(Completion::Throw(error), &mut pc, boundary)?;
                active.clone_from(&self.frames.last().ok_or(Error::InvalidBytecode)?.code);
            }
            if pc
                > active
                    .as_ref()
                    .map_or(program, |function| &function.program)
                    .code
                    .len()
            {
                return Err(Error::InvalidBytecode);
            }
        }
    }

    /// Pure Number/Number operators do not allocate, invoke hooks or grow the
    /// stack. Replace the left operand in place without cloning either Value.
    #[expect(
        clippy::float_cmp,
        reason = "JavaScript Number comparisons require exact IEEE equality"
    )]
    #[inline]
    fn numeric_binary_in_place(&mut self, op: Binary) -> Result<bool, Error> {
        let len = self.stack.len();
        let start = len.checked_sub(2).ok_or(Error::InvalidBytecode)?;
        let Some([Value::Number(a), Value::Number(b)]) = self.stack.get(start..) else {
            return Ok(false);
        };
        let value = match op {
            Binary::Add => Value::Number(a + b),
            Binary::Sub => Value::Number(a - b),
            Binary::Mul => Value::Number(a * b),
            Binary::Div => Value::Number(a / b),
            Binary::Rem => Value::Number(a % b),
            Binary::Lt => Value::Boolean(a < b),
            Binary::Le => Value::Boolean(a <= b),
            Binary::Gt => Value::Boolean(a > b),
            Binary::Ge => Value::Boolean(a >= b),
            Binary::Eq | Binary::StrictEq => Value::Boolean(a == b),
            Binary::Ne | Binary::StrictNe => Value::Boolean(a != b),
            _ => return Ok(false),
        };
        *self.stack.get_mut(start).ok_or(Error::InvalidBytecode)? = value;
        self.stack.pop();
        Ok(true)
    }
    fn reserve(&mut self) -> Result<(), Error> {
        if !self.heap.full(self.limits.heap_entries) {
            return Ok(());
        }
        self.collect_garbage()
    }

    fn collect_garbage(&mut self) -> Result<(), Error> {
        self.mark_realm_roots();
        self.mark_execution_roots();
        self.finish_collection()
    }
    fn mark_realm_roots(&mut self) {
        for (_, value) in &self.iterator_methods {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.array_constructor_storage {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.regexp_constructor_storage {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.regexp_proto {
            self.heap.mark_value(value);
        }
        for timer in self.timers.queue.values() {
            self.heap.mark_value(&timer.handler);
            for argument in &timer.arguments {
                self.heap.mark_value(argument);
            }
        }
        if let Some(value) = &self.timers.timeout_function {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.object_constructor_storage {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.number_constructor_storage {
            self.heap.mark_value(value);
        }
        if let Some((integer, float)) = &self.number_parsers {
            self.heap.mark_value(integer);
            self.heap.mark_value(float);
        }
        if let Some((entries, values)) = &self.object_enumerators {
            self.heap.mark_value(entries);
            self.heap.mark_value(values);
        }
        if let Some(value) = &self.async_function_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.async_function_ctor {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.reflect {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.json {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.dom_exception_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.custom_event_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.event_trusted_getter {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.event_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.event_target_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.abort_controller_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.abort_signal_proto {
            self.heap.mark_value(value);
        }
        for error in &self.reported_exceptions {
            if let Error::Thrown { value } = error {
                self.heap.mark_value(value);
            }
        }
        for handle in self.global_lexicals.values() {
            self.heap.mark(*handle);
        }
        for value in self.retained.values() {
            self.heap.mark_value(value);
        }
    }
    fn mark_execution_roots(&mut self) {
        for value in &self.stack {
            self.heap.mark_value(value);
        }
        for value in &self.native_roots {
            self.heap.mark_value(value);
        }
        for value in &self.rejections {
            self.heap.mark_value(value);
        }
        for (_, value) in &self.error_prototypes {
            self.heap.mark_value(value);
        }
        for (_, value) in &self.primitive_protos {
            self.heap.mark_value(value);
        }
        for (_, value) in &self.iterator_protos {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.function_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.throw_type_error {
            self.heap.mark_value(value);
        }
        for frame in &self.frames {
            for binding in &frame.locals {
                match binding {
                    Binding::Cell(handle) => self.heap.mark(*handle),
                    Binding::Direct(Some(value)) => self.heap.mark_value(value),
                    Binding::Direct(None) => {}
                }
            }
            for handle in &frame.captures {
                self.heap.mark(*handle);
            }
            self.heap.mark_value(&frame.result);
            self.heap.mark_value(&frame.callee);
            for value in &frame.arguments {
                self.heap.mark_value(value);
            }
            for enumeration in frame.enumerations.iter().flatten() {
                self.heap.mark_value(&enumeration.object);
                if let Some(record) = &enumeration.iterator {
                    self.heap.mark_value(&record.object);
                    self.heap.mark_value(&record.next);
                }
            }
            self.heap.mark_value(&frame.this_value);
            self.heap.mark_value(&frame.new_target);
            if let Some(handle) = frame.this_cell {
                self.heap.mark(handle);
            }
            if let Some(value) = &frame.async_promise {
                self.heap.mark_value(value);
            }
            for handler in &frame.handlers {
                handler.trace(&mut self.heap);
            }
        }
        if let Some(value) = &self.object_prototype {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.global {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.array_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.math {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.promise_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.weakmap_proto {
            self.heap.mark_value(value);
        }
        if let Some(value) = &self.symbol_proto {
            self.heap.mark_value(value);
        }
        for symbol in self
            .well_known
            .values()
            .chain(self.symbol_registry.values())
        {
            self.heap.mark(symbol.handle);
        }
        for job in &self.jobs {
            for value in job.roots() {
                self.heap.mark_value(value);
            }
            if let crate::promise::Job::Reaction { reaction, .. } = job
                && let Some(handle) = reaction.resume
            {
                self.heap.mark(handle);
            }
        }
    }
    fn finish_collection(&mut self) -> Result<(), Error> {
        self.heap.collect(&mut self.fuel)?;
        self.weak_entries = self.heap.weak_entries();
        let (frames, slots, operands, reactions) = self.heap.async_usage();
        self.suspended_frames = frames;
        self.suspended_slots = operands;
        self.reaction_count = reactions;
        self.binding_slots = self
            .frames
            .iter()
            .fold(slots, |n, frame| n.saturating_add(frame.locals.len()));
        Ok(())
    }

    fn cell(&mut self, index: usize, value: Option<Value>, program: &Program) -> Result<(), Error> {
        let frame = self.frames.last_mut().ok_or(Error::InvalidBytecode)?;
        if !frame
            .program(program)
            .slots
            .get(index)
            .ok_or(Error::InvalidBytecode)?
            .captured
        {
            *frame.locals.get_mut(index).ok_or(Error::InvalidBytecode)? = Binding::Direct(value);
            return Ok(());
        }
        // Old binding remains rooted through collection, including a loop's
        // cloned value. The replacement is published before another allocation.
        self.reserve()?;
        let frame = self.frames.last_mut().ok_or(Error::InvalidBytecode)?;
        let slot = frame
            .program(program)
            .slots
            .get(index)
            .ok_or(Error::InvalidBytecode)?
            .clone();
        let handle = self
            .heap
            .allocate(Node::Cell { slot, value }, self.limits.heap_entries)?;
        *frame.locals.get_mut(index).ok_or(Error::InvalidBytecode)? = Binding::Cell(handle);
        Ok(())
    }

    fn closure(&mut self, index: usize, program: &Program) -> Result<(), Error> {
        if self.frames.last().is_some_and(|frame| frame.code.is_none())
            && self
                .frames
                .last()
                .and_then(|f| f.program(program).functions.get(index))
                .is_some_and(|f| f.arrow)
        {
            let value = self.global_object()?;
            self.frames
                .last_mut()
                .ok_or(Error::InvalidBytecode)?
                .this_value = value;
        }
        self.reserve()?;
        let frame = self.frames.last().ok_or(Error::InvalidBytecode)?;
        let code = frame
            .program(program)
            .functions
            .get(index)
            .ok_or(Error::InvalidBytecode)?
            .clone();
        let captures = code
            .captures
            .iter()
            .map(|access| frame.handle(*access))
            .collect::<Result<Vec<_>, _>>()?;
        let lexical_this = if code.arrow {
            Some(frame.this_value.clone())
        } else {
            None
        };
        let handle = self.heap.allocate(
            Node::Function {
                code,
                captures,
                lexical_this,
                object: Object::new(Value::Null),
            },
            self.limits.heap_entries,
        )?;
        let function = Value::Function(FunctionValue(Callable::Script {
            handle,
            owner: self.owner.clone(),
        }));
        self.push(function.clone())?;
        if self
            .frames
            .last()
            .and_then(|frame| frame.program(program).functions.get(index))
            .is_some_and(|code| code.arrow)
        {
            let frame = self.frames.last().ok_or(Error::InvalidBytecode)?;
            let cell = frame.this_cell;
            let new_target = frame.new_target.clone();
            let callee = frame.callee.clone();
            let home = self.object_ref(&callee).ok().and_then(|o| o.home.clone());
            let super_callee = self
                .object_ref(&callee)
                .ok()
                .and_then(|o| o.lexical_super.clone())
                .unwrap_or(callee);
            let object = self.object_mut(&function)?;
            object.lexical_this_cell = cell;
            object.lexical_new_target = new_target;
            object.home = home;
            object.lexical_super = Some(super_callee);
        }
        let name = self
            .frames
            .last()
            .and_then(|frame| {
                frame
                    .program(program)
                    .functions
                    .get(index)
                    .map(|code| code.name.clone())
            })
            .ok_or(Error::InvalidBytecode)?;
        self.define(
            &function,
            Value::string("name").units(),
            crate::object::Property {
                value: Value::string(&name),
                writable: false,
                enumerable: false,
                configurable: true,
                accessor: None,
            },
        )?;
        let Node::Function { code, .. } = self.heap.get(handle)? else {
            return Err(Error::InvalidBytecode);
        };
        let kind = code.async_kind;
        let proto = self.callable_prototype(kind)?;
        self.object_mut(&function)?.prototype = proto;
        self.ensure_function_prototype(&function)
    }

    fn invoke(&mut self, count: usize, program: &Program) -> Result<(), Error> {
        self.invoke_kind(count, program, None)
    }
    #[expect(
        clippy::too_many_lines,
        reason = "single call dispatch handles forwarding and frame initialization without recursive forwarding"
    )]
    fn invoke_kind(
        &mut self,
        mut count: usize,
        program: &Program,
        mut construction: Option<(Value, Option<Handle>)>,
    ) -> Result<(), Error> {
        loop {
            self.charge(1)?;
            let start = self
                .stack
                .len()
                .checked_sub(count.saturating_add(2))
                .ok_or(Error::InvalidBytecode)?;
            let callee = self.stack.get(start).ok_or(Error::InvalidBytecode)?.clone();
            let Value::Function(function) = &callee else {
                return Err(Error::Type {
                    message: "value is not callable",
                });
            };
            let receiver = self.argument_value(start.saturating_add(1));
            let args_start = start.saturating_add(2);
            match &function.0 {
                Callable::Host { handle, .. } => {
                    let args = self.arguments_from(args_start)?;
                    // GC-owned intrinsics have mutable metadata, but callback
                    // algorithms still forward to VM frames without Rust recursion.
                    if let Node::HostFunction {
                        behavior: crate::heap::HostBehavior::Intrinsic(builtin),
                        ..
                    } = self.heap.get(*handle)?
                        && let Some(new_count) =
                            self.prepare_array_callback(*builtin, &receiver, &args, start)?
                    {
                        count = new_count;
                        continue;
                    }
                    let result = self.invoke_host(*handle, &receiver, &args)?;
                    self.stack.truncate(start);
                    return self.push(result);
                }
                Callable::Bound { handle, .. } => {
                    count = self.forward_bound(*handle, start, args_start)?;
                }
                Callable::Resolver { handle, reject, .. } => {
                    let value = self.argument_value(args_start);
                    self.resolve_once(*handle, value, *reject)?;
                    self.stack.truncate(start);
                    return self.push(Value::Undefined);
                }
                Callable::Native(builtin) => {
                    if *builtin == Builtin::InternalCall {
                        self.forward_internal(start, args_start, count)?;
                        count = count.saturating_sub(2);
                        continue;
                    }
                    if *builtin == Builtin::FunctionCall {
                        self.forward_call(start, args_start, count, receiver)?;
                        count = count.saturating_sub(1);
                        // Every forwarding step removes one argument; at zero a
                        // second call cannot have a callable receiver.
                        continue;
                    }
                    if *builtin == Builtin::FunctionApply {
                        count = self.forward_apply(receiver, start, args_start)?;
                        continue;
                    }
                    let args = self.arguments_from(args_start)?;
                    if let Some(new_count) =
                        self.prepare_array_callback(*builtin, &receiver, &args, start)?
                    {
                        count = new_count;
                        continue;
                    }
                    let result = self.native_call(*builtin, &receiver, &args)?;
                    self.stack.truncate(start);
                    return self.push(result);
                }
                Callable::Script { handle, owner } => {
                    if !Rc::ptr_eq(owner, &self.owner) {
                        return Err(Error::Type {
                            message: "function belongs to another execution",
                        });
                    }
                    self.check_frame_limit()?;
                    let Node::Function {
                        code,
                        captures,
                        lexical_this,
                        ..
                    } = self.heap.get(*handle)?
                    else {
                        return Err(Error::InvalidBytecode);
                    };
                    let code = code.clone();
                    if code.constructor_kind != crate::parser::ConstructorKind::Ordinary
                        && construction.is_none()
                    {
                        return Err(Error::Type {
                            message: "class constructor requires new",
                        });
                    }
                    let captures = captures.clone();
                    let lexical_this = lexical_this.clone();
                    let async_promise = self.async_result(&code)?;
                    let this_value = self.call_this(&code, lexical_this, receiver)?;
                    let object = self.object_ref(&callee)?;
                    let constructing = construction.is_some();
                    let (new_target, this_cell) = construction
                        .take()
                        .unwrap_or((object.lexical_new_target.clone(), object.lexical_this_cell));
                    self.reserve_bindings(code.program.slots.len())?;
                    self.frames.push(Frame {
                        code: Some(code.clone()),
                        pc: 0,
                        locals: alloc::vec![Binding::Direct(None); code.program.slots.len()],
                        captures,
                        base: start,
                        result: Value::Undefined,
                        callee: callee.clone(),
                        this_value,
                        handlers: Vec::new(),
                        constructing,
                        arguments: self.saved_arguments(&code, args_start)?,
                        enumerations: Vec::new(),
                        async_promise,
                        new_target,
                        this_cell,
                    });
                    self.initialize_call(&code, callee, args_start, count, program)?;
                    self.stack.truncate(start);
                    if code.async_kind == crate::parser::AsyncKind::Async {
                        self.native_roots.pop();
                    }
                    return Ok(());
                }
            }
        }
    }

    fn forward_call(
        &mut self,
        start: usize,
        args_start: usize,
        count: usize,
        receiver: Value,
    ) -> Result<(), Error> {
        let this = self
            .stack
            .get(args_start)
            .cloned()
            .unwrap_or(Value::Undefined);
        let args = self
            .stack
            .get(args_start.saturating_add(usize::from(count > 0))..)
            .ok_or(Error::InvalidBytecode)?
            .to_vec();
        self.stack.truncate(start);
        self.push(receiver)?;
        self.push(this)?;
        for arg in args {
            self.push(arg)?;
        }
        Ok(())
    }

    fn check_frame_limit(&mut self) -> Result<(), Error> {
        if self
            .frames
            .len()
            .saturating_sub(1)
            .saturating_add(self.suspended_frames)
            >= self.limits.call_frames
        {
            self.collect_garbage()?;
        }
        if self
            .frames
            .len()
            .saturating_sub(1)
            .saturating_add(self.suspended_frames)
            >= self.limits.call_frames
        {
            Err(Error::Limit {
                resource: "call frames",
            })
        } else {
            Ok(())
        }
    }

    fn async_result(&mut self, code: &FunctionCode) -> Result<Option<Value>, Error> {
        if code.async_kind == crate::parser::AsyncKind::Async {
            let value = self.new_promise()?;
            self.native_roots.push(value.clone());
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
    fn call_this(
        &mut self,
        code: &FunctionCode,
        lexical: Option<Value>,
        receiver: Value,
    ) -> Result<Value, Error> {
        if let Some(value) = lexical {
            Ok(value)
        } else if !code.strict && matches!(receiver, Value::Null | Value::Undefined) {
            self.global_object()
        } else if !code.strict {
            self.box_value(&receiver)
        } else {
            Ok(receiver)
        }
    }
    fn reserve_bindings(&mut self, count: usize) -> Result<(), Error> {
        self.binding_slots = self
            .binding_slots
            .checked_add(count)
            .filter(|n| n.saturating_add(self.global_lexicals.len()) <= self.limits.binding_slots)
            .ok_or(Error::Limit {
                resource: "binding slots",
            })?;
        Ok(())
    }

    fn initialize_call(
        &mut self,
        code: &FunctionCode,
        callee: Value,
        start: usize,
        count: usize,
        program: &Program,
    ) -> Result<(), Error> {
        if let Some(index) = code.self_slot {
            self.cell(index, Some(callee.clone()), program)?;
        }
        if code.initialization == ParameterInitialization::Direct {
            self.parameters(code, start, count, program)?;
        }
        if let Some(index) = code.arguments_slot {
            let object = self.arguments_object(code, callee, start, count)?;
            self.cell(index, Some(object), program)?;
        }
        Ok(())
    }

    fn saved_arguments(&self, code: &FunctionCode, start: usize) -> Result<Vec<Value>, Error> {
        if code.initialization == ParameterInitialization::Bytecode
            || code
                .program
                .code
                .iter()
                .any(|op| matches!(op, Op::DefaultSuper))
        {
            self.arguments_from(start)
        } else {
            Ok(Vec::new())
        }
    }

    fn native_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        if let Some(value) = self.timer_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if matches!(builtin, Builtin::ReflectApply | Builtin::ReflectConstruct) {
            return self.reflect_call(builtin, args);
        }
        if builtin == Builtin::FunctionHasInstance {
            return self
                .has_instance(receiver, args.first().unwrap_or(&Value::Undefined), false)
                .map(Value::Boolean);
        }
        if let Some(value) = self.json_call(builtin, args)? {
            return Ok(value);
        }
        if let Some(value) = self.event_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if let Some(value) = self.abort_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if let Some(value) = self.iterator_call(builtin, receiver)? {
            return Ok(value);
        }
        if let Some(value) = self.symbol_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if let Some(value) = self.weakmap_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if let Some(value) = self.boxing_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if let Some(value) = self.error_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if builtin == Builtin::FunctionBind {
            return self.bind_function(receiver, args);
        }
        if builtin == Builtin::Function {
            return self.dynamic_function(
                args,
                &Value::Function(FunctionValue::native(Builtin::Function)),
            );
        }
        if let Some(value) = self.string_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if let Some(value) = self.promise_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if let Some(result) = self.regexp_call(builtin, receiver, args)? {
            return Ok(result);
        }
        let first = args.first().cloned().unwrap_or(Value::Undefined);
        match builtin {
            Builtin::DomException => {
                return Err(Error::Type {
                    message: "DOMException requires new",
                });
            }
            Builtin::ArraySpecies => return Ok(receiver.clone()),
            Builtin::Print => return self.host_print(args),
            Builtin::ThrowTypeError => {
                return Err(Error::Type {
                    message: "restricted function property",
                });
            }
            Builtin::MathMax | Builtin::MathMin => return self.math_extreme(builtin, args),
            Builtin::MathPow => return self.math_pow(args),
            Builtin::String if !args.is_empty() => {
                return Ok(Value::String(self.string_units(&first)?));
            }
            Builtin::Number if !args.is_empty() => return Ok(Value::Number(self.numeric(&first)?)),
            Builtin::IsNaN => return Ok(Value::Boolean(self.numeric(&first)?.is_nan())),
            Builtin::ParseInt => {
                return self.parse_integer(&first, args.get(1).unwrap_or(&Value::Undefined));
            }
            Builtin::ParseFloat => return self.parse_float(&first),
            Builtin::NumberIsFinite
            | Builtin::NumberIsInteger
            | Builtin::NumberIsNaN
            | Builtin::NumberIsSafeInteger => {
                return Ok(numbers::predicate(builtin, &first));
            }
            Builtin::IsFinite => return Ok(Value::Boolean(self.numeric(&first)?.is_finite())),
            _ => {}
        }
        if let Some(value) = self.array_call(builtin, receiver, args)? {
            return Ok(value);
        }
        if let Some(value) = self.object_call(builtin, receiver, args)? {
            return Ok(value);
        }
        call(builtin, args, self.host)
    }

    fn arguments_from(&self, start: usize) -> Result<Vec<Value>, Error> {
        Ok(self
            .stack
            .get(start..)
            .ok_or(Error::InvalidBytecode)?
            .to_vec())
    }

    fn argument_value(&self, index: usize) -> Value {
        self.stack.get(index).cloned().unwrap_or(Value::Undefined)
    }

    fn forward_internal(
        &mut self,
        start: usize,
        args_start: usize,
        count: usize,
    ) -> Result<(), Error> {
        if count < 2 {
            return Err(Error::InvalidBytecode);
        }
        let args = self
            .stack
            .get(args_start..)
            .ok_or(Error::InvalidBytecode)?
            .to_vec();
        self.stack.truncate(start);
        for value in args {
            self.push(value)?;
        }
        Ok(())
    }

    fn charge(&mut self, amount: u64) -> Result<(), Error> {
        self.fuel = self.fuel.checked_sub(amount).ok_or(Error::Limit {
            resource: "execution fuel",
        })?;
        Ok(())
    }

    fn call_sync(
        &mut self,
        callee: Value,
        receiver: Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        if self.native_depth >= 12 {
            return Err(Error::Limit {
                resource: "native reentry",
            });
        }
        let program = self.top_program.clone().ok_or(Error::InvalidBytecode)?;
        let frames = self.frames.len();
        let stack = self.stack.len();
        self.native_depth = self.native_depth.saturating_add(1);
        let boundary = self.unwind_boundary;
        self.unwind_boundary = frames;
        let result = (|| {
            self.push(callee)?;
            self.push(receiver)?;
            for value in args {
                self.push(value.clone())?;
            }
            self.invoke(args.len(), &program)?;
            if self.frames.len() > frames {
                self.execute(&program, frames)
            } else {
                self.pop()
            }
        })();
        self.native_depth = self.native_depth.saturating_sub(1);
        self.unwind_boundary = boundary;
        while self.frames.len() > frames {
            let frame = self.frames.pop().ok_or(Error::InvalidBytecode)?;
            self.binding_slots = self.binding_slots.saturating_sub(frame.locals.len());
        }
        self.stack.truncate(stack);
        result
    }

    fn parameters(
        &mut self,
        code: &FunctionCode,
        start: usize,
        count: usize,
        program: &Program,
    ) -> Result<(), Error> {
        for (position, index) in code.parameters.iter().enumerate() {
            let value = self
                .stack
                .get(start.saturating_add(position))
                .filter(|_| position < count)
                .cloned()
                .unwrap_or(Value::Undefined);
            if self
                .frames
                .last()
                .and_then(|frame| frame.locals.get(*index))
                .is_some_and(|binding| !matches!(binding, Binding::Direct(None)))
            {
                self.store(Access::Local(*index), value, true, program)?;
            } else {
                self.cell(*index, Some(value), program)?;
            }
        }
        Ok(())
    }
}

fn unary(op: Unary, value: &Value) -> Value {
    match op {
        Unary::Plus => Value::Number(value.to_number()),
        Unary::Minus => Value::Number(-value.to_number()),
        Unary::Not => Value::Boolean(!value.to_boolean()),
        Unary::Void => Value::Undefined,
        Unary::Typeof => Value::string(value.type_name()),
        Unary::BitNot => Value::Number(f64::from(!value.to_int32())),
        Unary::Delete => Value::Boolean(true),
    }
}

fn binary(op: Binary, left: &Value, right: &Value, limits: Limits) -> Result<Value, Error> {
    if (matches!(left, Value::Symbol(_)) || matches!(right, Value::Symbol(_)))
        && !matches!(
            op,
            Binary::Eq | Binary::Ne | Binary::StrictEq | Binary::StrictNe
        )
    {
        return Err(Error::Type {
            message: "cannot convert Symbol operand",
        });
    }
    let value = match op {
        Binary::Add if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) => {
            let a = left.units();
            let b = right.units();
            let size = a
                .len()
                .checked_add(b.len())
                .filter(|n| *n <= limits.string_units)
                .ok_or(Error::Limit {
                    resource: "string units",
                })?;
            let mut units = Vec::with_capacity(size);
            units.extend_from_slice(&a);
            units.extend_from_slice(&b);
            Value::String(units.into())
        }
        Binary::StrictEq => Value::Boolean(left.strictly_equals(right)),
        Binary::StrictNe => Value::Boolean(!left.strictly_equals(right)),
        Binary::Eq => Value::Boolean(left.loosely_equals(right)),
        Binary::Ne => Value::Boolean(!left.loosely_equals(right)),
        Binary::BitAnd => Value::Number(f64::from(left.to_int32() & right.to_int32())),
        Binary::BitOr => Value::Number(f64::from(left.to_int32() | right.to_int32())),
        Binary::BitXor => Value::Number(f64::from(left.to_int32() ^ right.to_int32())),
        Binary::Shl => Value::Number(f64::from(
            left.to_int32().wrapping_shl(right.to_uint32() & 31),
        )),
        Binary::Shr => Value::Number(f64::from(
            left.to_int32().wrapping_shr(right.to_uint32() & 31),
        )),
        Binary::Ushr => Value::Number(f64::from(
            left.to_uint32().wrapping_shr(right.to_uint32() & 31),
        )),
        Binary::Lt | Binary::Le | Binary::Gt | Binary::Ge => {
            let order = if let (Value::String(a), Value::String(b)) = (left, right) {
                a.partial_cmp(b)
            } else {
                left.to_number().partial_cmp(&right.to_number())
            };
            Value::Boolean(order.is_some_and(|order| match op {
                Binary::Lt => order.is_lt(),
                Binary::Le => order.is_le(),
                Binary::Gt => order.is_gt(),
                _ => order.is_ge(),
            }))
        }
        _ => Value::Number(arithmetic(op, left.to_number(), right.to_number())?),
    };
    Ok(value)
}

fn arithmetic(op: Binary, a: f64, b: f64) -> Result<f64, Error> {
    Ok(match op {
        Binary::Add => a + b,
        Binary::Sub => a - b,
        Binary::Mul => a * b,
        Binary::Pow => audhsos_math::pow(a, b),
        Binary::Div => a / b,
        Binary::Rem => a % b,
        _ => return Err(Error::InvalidBytecode),
    })
}

fn call(builtin: Builtin, args: &[Value], host: &mut dyn Host) -> Result<Value, Error> {
    let first = args.first().unwrap_or(&Value::Undefined);
    Ok(match builtin {
        Builtin::Print => {
            host.print(args)?;
            Value::Undefined
        }
        Builtin::Number => Value::Number(if args.is_empty() {
            0.0
        } else {
            first.to_number()
        }),
        Builtin::String => {
            if args.is_empty() {
                Value::string("")
            } else {
                Value::String(first.units())
            }
        }
        Builtin::Boolean => Value::Boolean(first.to_boolean()),
        Builtin::IsNaN => Value::Boolean(first.to_number().is_nan()),
        Builtin::IsFinite => Value::Boolean(first.to_number().is_finite()),
        _ => return Err(Error::InvalidBytecode),
    })
}

#[cfg(test)]
#[path = "tests/numeric_fast_path.rs"]
mod numeric_fast_path_tests;
