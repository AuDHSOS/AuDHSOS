// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Register-based Bytecode Instruction Set.
//!
//! Instructions operate on a register file (r0..rn) and an explicit
//! accumulator register (acc). This mirrors modern production engines (Ignition)
//! and eliminates the stack push/pop dispatch overhead.

use super::value::Value;
use alloc::{collections::VecDeque, rc::Rc, vec::Vec};

/// Virtual register index inside a function's call frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reg(pub u16);

/// Runtime feedback category assigned statically to one bytecode site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackKind {
    /// Named or keyed property access.
    NamedAccess,
    /// Binary operator type profile.
    BinaryOp,
    /// Callable target profile.
    Call,
}

/// What an access of a private element of 6.2.13 does with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateOp {
    /// `PrivateGet` of 7.3.28, which answers the value.
    Get,
    /// `PrivateSet` of 7.3.29, which writes the value in the accumulator.
    Set,
    /// `PrivateFieldAdd` of 7.3.27, which adds the value in the accumulator.
    Add,
    /// `PrivateMethodOrAccessorAdd` of 7.3.26, which adds the method in the
    /// accumulator, and no write of the Script reaches it afterwards.
    AddMethod,
    /// The same for the getter of an accessor of 15.7.1, which stands beside
    /// the setter of the same name in one element.
    AddGetter,
    /// The same for the setter of one.
    AddSetter,
    /// The `in` of 13.10.1 with a Private Name, which answers whether the
    /// object carries the element.
    Has,
}

/// Arithmetic operation executed through a typed feedback slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    /// ECMAScript addition or string concatenation.
    Add,
    /// Numeric subtraction.
    Sub,
    /// Numeric multiplication.
    Mul,
    /// Numeric exponentiation.
    Pow,
    /// Numeric division.
    Div,
    /// Numeric remainder.
    Mod,
    /// Abstract relational less-than comparison.
    LessThan,
    /// Abstract relational less-than-or-equal comparison.
    LessThanOrEqual,
    /// Abstract relational greater-than comparison.
    GreaterThan,
    /// Abstract relational greater-than-or-equal comparison.
    GreaterThanOrEqual,
    /// Bitwise AND of 13.12.
    BitAnd,
    /// Bitwise OR of 13.12.
    BitOr,
    /// Bitwise XOR of 13.12.
    BitXor,
    /// Left shift of 13.9.1.
    ShiftLeft,
    /// Signed right shift of 13.9.2.
    ShiftRight,
    /// Unsigned right shift of 13.9.3.
    UnsignedShiftRight,
    /// Loose equality of 13.11.1, which is 7.2.14.
    Equals,
}

/// What an operation needed of a value that was undefined or null.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequireKind {
    /// `RequireObjectCoercible` of 7.2.1, which 14.3.3.3 asks before it reads
    /// any property of the source.
    ObjectCoercible,
    /// `GetIterator` of 7.4.2, whose `@@iterator` must not be undefined.
    Iterable,
}

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
    /// A bytecode site uses a feedback slot declared for a different category.
    FeedbackKindMismatch {
        /// Instruction offset.
        pc: usize,
        /// Mismatched feedback slot.
        slot: u16,
        /// Kind required by the instruction.
        expected: FeedbackKind,
        /// Kind declared by the function.
        actual: FeedbackKind,
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
    /// A context access leaves the statically declared lexical context chain.
    ContextOutOfBounds {
        /// Instruction offset.
        pc: usize,
        /// Outer-context depth.
        depth: u16,
        /// Slot index at that depth.
        slot: u16,
    },
    /// A closure expects a lexical context layout unavailable at its creation site.
    ClosureContextMismatch {
        /// Instruction offset.
        pc: usize,
        /// Function whose outer context contract does not match.
        index: u32,
    },
    /// A relative branch target is outside the instruction array.
    JumpOutOfBounds {
        /// Instruction offset.
        pc: usize,
    },
    /// An exception handler names an instruction offset outside the function.
    HandlerOutOfBounds {
        /// Index of the offending handler.
        index: usize,
    },
    /// An exception handler protects an empty or inverted instruction range.
    HandlerRangeInverted {
        /// Index of the offending handler.
        index: usize,
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
    /// `acc = bigints[index]`, materialized in the current Agent's `BigInt`
    /// arena, which 12.9.3 names.
    LdaBigInt(u16),
    /// `acc = [[GlobalThisValue]]` of the Realm's Global Environment Record
    /// (9.1.1.4.11), which 9.4.2 answers for `this` where no function bound
    /// one, that is at the top level of a Script.
    LdaGlobalThis,
    /// `acc = GetBindingValue(strings[index])` on the Realm's Global
    /// Environment Record (9.1.1.4.6), which throws a `ReferenceError` for a
    /// name it does not bind.
    LdaGlobal(u16),
    /// The same read for the operand of `typeof`, which 13.5.3 answers with
    /// undefined for an unresolvable Reference instead of throwing.
    LdaGlobalForTypeOf(u16),
    /// `SetMutableBinding(strings[index], acc, strict)` of 9.1.1.4.5 on the
    /// Realm's Global Environment Record.
    StaGlobal {
        /// Index of the name in the string constants.
        name: u16,
        /// Whether the assignment is evaluated under strict mode, which
        /// 9.1.1.2.5 refuses for a name nothing binds.
        strict: bool,
    },
    /// Step 4 of 16.1.7 for one `var` name: a lexical declaration of the same
    /// name in this Realm is a `SyntaxError`.
    VerifyGlobalVar(u16),
    /// `CreateGlobalVarBinding(strings[index], false)` of 9.1.1.4.16, which
    /// step 18 of 16.1.7 performs once every name has been verified.
    DeclareGlobalVar(u16),
    /// Step 9 of 16.1.7 for one function name: a global property that cannot
    /// take the function is a `TypeError`.
    VerifyGlobalFunction(u16),
    /// `CreateGlobalFunctionBinding(strings[index], acc, false)` of 9.1.1.4.17,
    /// which step 17 of 16.1.7 performs with the function object it made.
    DeclareGlobalFunction(u16),
    /// Step 3 of 16.1.7 for one lexically declared name: a lexical
    /// declaration of the same name in this Realm, or a restricted global
    /// property, is a `SyntaxError`.
    VerifyGlobalLexical(u16),
    /// `CreateMutableBinding` or `CreateImmutableBinding` of 9.1.1.4.2 and
    /// 9.1.1.4.3, which step 16 of 16.1.7 performs on the
    /// `[[DeclarativeRecord]]`. The binding has no value until
    /// [`Instruction::InitializeGlobalLexical`] gives it one, and a read of it
    /// before that is the `ReferenceError` of its temporal dead zone.
    DeclareGlobalLexical {
        /// Name index in the heap-independent UTF-16 constant pool.
        name: u16,
        /// Whether a later assignment may write it, which `const` withholds.
        mutable: bool,
    },
    /// `InitializeBinding(strings[index], acc)` of 9.1.1.4.4, which the
    /// declaration performs where it stands.
    InitializeGlobalLexical(u16),
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
    /// `acc = ToNumber(acc)` for an already primitive operand.
    ToNumber,
    /// `acc = ToString(reg)` of 7.1.17, which 13.2.8 applies to each
    /// substitution of a template.
    ///
    /// An Object operand reaches 7.1.1 with the hint `string`, which may run a
    /// method of the Script: the primitive comes back into the register and the
    /// instruction runs again.
    ToText(Reg),
    /// A call whose arguments are the List 13.3.8 makes of an argument list
    /// that holds a spread element.
    ///
    /// No register window holds a count the run time decides, so the
    /// arguments travel in an Array and the frame takes its parameters from
    /// there.
    CallSpread {
        /// Register holding the `this` value of the call.
        receiver: Reg,
        /// Register holding the callee.
        func: Reg,
        /// Register holding the Array of arguments.
        list: Reg,
        /// Call-site feedback slot.
        slot: u16,
    },
    /// The same for `new`, whose object 10.1.13 makes as it does for a
    /// construct with registers.
    ConstructSpread {
        /// Register holding the constructor.
        func: Reg,
        /// Register the object 10.1.13 makes is written to.
        target: Reg,
        /// Register holding the Array of arguments.
        list: Reg,
        /// Call-site feedback slot.
        slot: u16,
    },
    /// 27.7.5.3: the body waits for what the accumulator holds and leaves.
    ///
    /// The frame is copied into a continuation of the heap, the value is sent
    /// through 27.2.4.7.1, and the pair of 27.7.5.3 takes the body back where
    /// it stopped. The accumulator holds the promise of the body on the way
    /// out, and the value the wait answered on the way back.
    Await,
    /// `acc = ToNumeric(reg)` of 7.1.4, which 13.4 applies to the old value of
    /// an update.
    ///
    /// An Object operand reaches 7.1.1 with the hint `number`, which may run a
    /// method of the Script: the primitive comes back into the register and the
    /// instruction runs again.
    ToNumeric(Reg),

    /// `acc = ToPropertyKey(reg)` of 7.1.19, which keeps a Symbol and sends
    /// every other value through 7.1.17.
    ToPropertyKey(Reg),

    /// `CopyDataProperties` of 7.3.25 with no excluded name: the own
    /// enumerable properties of the accumulator are defined on the object the
    /// register holds, which 13.2.5.5 does for a `...` of an Object literal.
    SpreadDataProperties(Reg),
    /// `acc = ToNumber(register)`, which 13.5.4 asks and which refuses the
    /// `BigInt` `ToNumeric` would answer.
    NumberOnly(Reg),
    /// `acc = acc + 1`, on the Number or the `BigInt` 13.4.4.1 made of it, which
    /// answers the value of its own type and not a Number beside a `BigInt`.
    Increment,
    /// `acc = acc - 1`, the same way.
    Decrement,
    /// `acc = ~ToInt32(acc)` for an already numeric primitive.
    BitNot,
    /// `acc = typeof acc`, materialized as an Agent-local String.
    TypeOf,
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
    /// `acc = current_context[depth][slot]`.
    LoadContext {
        /// Outer-context depth.
        depth: u16,
        /// Binding slot.
        slot: u16,
    },
    /// `current_context[depth][slot] = acc`.
    StoreContext {
        /// Outer-context depth.
        depth: u16,
        /// Binding slot.
        slot: u16,
    },
    /// `acc = acc + reg`
    Add(Reg),
    /// `acc = acc - reg`
    Sub(Reg),
    /// `acc = acc * reg`
    Mul(Reg),
    /// `acc = acc ** reg`
    Pow(Reg),
    /// `acc = acc / reg`
    Div(Reg),
    /// `acc = acc % reg`
    Mod(Reg),
    /// Arithmetic, concatenation or comparison with runtime type feedback.
    ///
    /// Both operands are registers rather than the accumulator and a register,
    /// because 7.1.1 may call a user method to convert one of them and the
    /// converted value is written back where the collector can see it.
    Binary {
        /// Operation to execute.
        op: BinaryOp,
        /// Left-hand operand register.
        lhs: Reg,
        /// Right-hand operand register.
        rhs: Reg,
        /// `BinaryOp` feedback slot.
        slot: u16,
    },
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
    /// `acc = acc >>> reg`
    Ushr(Reg),
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
    /// Jump if `acc` is neither `undefined` nor `null`.
    JumpIfNotNullish(i32),
    /// Jump if `acc` is not `undefined`.
    JumpIfNotUndefined(i32),
    /// Load named property: `acc = obj_reg[name]` (uses feedback slot).
    GetNamed {
        /// Object register.
        obj: Reg,
        /// Property-name index in the heap-independent UTF-16 constant pool.
        name: u16,
        /// Feedback vector slot for inline caching.
        slot: u16,
    },
    /// `acc = obj[@@symbol]`, the read 7.4.2 makes of `@@iterator` and the
    /// only way the lowering names a Symbol key.
    GetWellKnown {
        /// Object register.
        obj: Reg,
        /// Index of the Symbol in [`crate::engine::realm::WellKnownSymbol::ALL`].
        symbol: u16,
        /// Feedback vector slot for inline caching.
        slot: u16,
    },
    /// `acc = acc in reg` (13.10.2), which is `HasProperty` of 7.3.11 on the
    /// key `ToPropertyKey` makes of the accumulator.
    TestIn(Reg),
    /// `acc = acc instanceof reg` (13.10.2).
    TestInstanceOf(Reg),
    /// Builds the arguments object of this call in `reg` (10.4.4).
    ///
    /// The frame keeps the caller registers the call passed, so the object
    /// holds every argument, not only the declared parameters.
    CreateArguments(Reg),
    /// Builds the Array of 8.6.3 in `reg`, out of the arguments the call
    /// passed beyond the `skip` parameters before the rest one (10.2.11).
    CreateRest {
        /// Register the rest parameter binds.
        target: Reg,
        /// Number of parameters the rest one stands behind.
        skip: u16,
    },
    /// Constructs with a callable: `acc = new func_reg(args)` (7.3.15).
    ///
    /// `target` holds the object 10.1.13 creates, where the collector sees it
    /// while the constructor runs and where the return takes it when the
    /// constructor answers no object of its own.
    Construct {
        /// Register holding the constructor.
        func: Reg,
        /// Register the created object is kept in.
        target: Reg,
        /// First argument register.
        arg_start: Reg,
        /// Number of arguments passed.
        arg_count: u16,
        /// Feedback vector slot for call target caching.
        slot: u16,
    },
    /// Deletes a named property: `acc = delete obj_reg.name` (13.5.1.2).
    ///
    /// `strict` is the strictness of the Reference, which decides whether a
    /// `[[Delete]]` that answered false throws instead.
    DeleteNamed {
        /// Object register.
        obj: Reg,
        /// Property-name index in the heap-independent UTF-16 constant pool.
        name: u16,
        /// Strictness of the Reference the operand made.
        strict: bool,
    },
    /// Deletes a computed property: `acc = delete obj_reg[key_reg]` (13.5.1.2).
    DeleteByValue {
        /// Object register.
        obj: Reg,
        /// Key register.
        key: Reg,
        /// Strictness of the Reference the operand made.
        strict: bool,
    },
    /// Store named property: `obj_reg[name] = acc` (uses feedback slot).
    SetNamed {
        /// Object register.
        obj: Reg,
        /// Property-name index in the heap-independent UTF-16 constant pool.
        name: u16,
        /// Feedback vector slot for inline caching.
        slot: u16,
        /// Strictness of the Reference, which 10.1.9.1 reads to decide
        /// whether a write it refuses throws.
        strict: bool,
        /// Whether this defines an own property (`CreateDataPropertyOrThrow`,
        /// 13.2.5.5) instead of assigning through `[[Set]]` (13.15.2). A
        /// definition reaches no Prototype and asks nothing of one.
        define: bool,
    },
    /// Defines an accessor property of an object literal (13.2.5.1), taking
    /// the function in `acc` as one half of it.
    ///
    /// The other half is whatever the property already holds, so the two
    /// clauses of one name meet on the object.
    DefineAccessor {
        /// Object register.
        obj: Reg,
        /// Property-name index in the heap-independent UTF-16 constant pool.
        name: u16,
        /// Whether the function is the `[[Set]]` rather than the `[[Get]]`.
        setter: bool,
        /// Whether the property is enumerable, which a literal gives it and a
        /// class body does not.
        enumerable: bool,
    },
    /// [`Instruction::DefineAccessor`] under a key only the run time knows,
    /// which 7.1.19 makes of the value in `key`.
    DefineAccessorByValue {
        /// Object register.
        obj: Reg,
        /// Register holding the key.
        key: Reg,
        /// Whether the function is the `[[Set]]` rather than the `[[Get]]`.
        setter: bool,
        /// Whether the property is enumerable, which a literal gives it and a
        /// class body does not.
        enumerable: bool,
    },
    /// [`Instruction::DefineMethod`] under a key only the run time knows,
    /// which 7.1.19 makes of the value in `key`.
    DefineMethodByValue {
        /// Register of the object the method belongs to.
        obj: Reg,
        /// Register holding the key.
        key: Reg,
        /// Whether the property is enumerable, which a literal gives it and a
        /// class body does not.
        enumerable: bool,
    },
    /// Defines a method of a class body (15.7.14), taking the function in
    /// `acc` as its value.
    ///
    /// The property is writable and configurable and not enumerable, which is
    /// what `CreateMethodProperty` of 7.3.5 gives it.
    DefineMethod {
        /// Register of the object the method belongs to.
        obj: Reg,
        /// Property-name index in the heap-independent UTF-16 constant pool.
        name: u16,
    },
    /// 15.7.14 steps 6 through 8 and 14: ties the class in `acc` to the value
    /// in `heritage`.
    ///
    /// The class takes the heritage as its `[[Prototype]]` and its `prototype`
    /// takes the `prototype` of the heritage; `null` gives the class
    /// `%Function.prototype%` and its `prototype` no Prototype at all.
    DeriveClass {
        /// Register holding the value the `extends` clause produced.
        heritage: Reg,
    },
    /// `SuperCall` of 13.3.7.1: constructs the Prototype of the running
    /// function with the `[[NewTarget]]` of the call.
    ///
    /// The answer is the `this` the derived constructor binds, which the
    /// instruction that follows writes into the register of the binding.
    SuperCall {
        /// First argument register.
        arg_start: Reg,
        /// Number of arguments.
        arg_count: u16,
        /// Set for the default constructor of 15.7.14, which passes the
        /// arguments its own call was given.
        forwarded: bool,
        /// Feedback vector slot for the call.
        slot: u16,
    },
    /// `GetThisBinding` of 9.4.5 for a derived constructor, whose binding is
    /// uninitialized until 13.3.7.1 has run.
    ThisBinding {
        /// Register of the `this` binding.
        register: Reg,
    },
    /// `MakeMethod` of 10.2.11: gives the function in `acc` the object in
    /// `home` as its `[[HomeObject]]`, which 13.3.7.3 reads the Prototype of.
    ///
    /// 13.2.5.5 makes every method of a literal one; only a body that reads
    /// `super` can tell, so the lowering emits it only there.
    MakeMethod {
        /// Register of the object the method belongs to.
        home: Reg,
    },
    /// `MakeSuperPropertyReference` of 13.3.7.3: the `[[Prototype]]` of the
    /// `[[HomeObject]]` the running function carries.
    ///
    /// The method that reads `super` carries its home, so no environment is
    /// walked. A function with no home reaches no `super`, which is a
    /// `SyntaxError` the parser raises and never this instruction.
    SuperBase {
        /// Register the base is written to.
        target: Reg,
    },
    /// `super.name` of 13.3.7: the property of the base, read with `this` as
    /// the receiver, which is what a getter of the chain is called with.
    GetSuper {
        /// Register holding the base of 13.3.7.3.
        base: Reg,
        /// Property-name index in the heap-independent UTF-16 constant pool.
        name: u16,
    },
    /// [`Instruction::GetSuper`] under a key only the run time knows, which
    /// 7.1.19 makes of the value in `key`.
    GetSuperByValue {
        /// Register holding the base of 13.3.7.3.
        base: Reg,
        /// Register holding the key.
        key: Reg,
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
    /// A Private Name of 6.2.13 in the accumulator, which 15.7.14 step 12
    /// makes once per evaluation of the class body.
    CreatePrivateName(u16),
    /// 27.5.1.1: the body of a generator leaves before its first instruction
    /// runs, and the call answers the Generator the frame holds.
    GeneratorStart,
    /// `YieldExpression` of 15.5: the body leaves with the accumulator, and
    /// 27.5.1.2 takes it back here with what the call of `next` was given.
    Yield,
    /// The private element of 6.2.13 that the key register names, which
    /// 7.3.28 reads and 7.3.29 writes, and 7.3.27 adds.
    ///
    /// Every one of the three answers a `TypeError` where the object carries
    /// no such element, or already carries one for an add.
    PrivateAccess {
        /// Object register.
        obj: Reg,
        /// Register holding the Private Name.
        key: Reg,
        /// What the access does with the element.
        op: PrivateOp,
    },
    /// Store indexed element: `obj_reg[key_reg] = acc` (uses feedback slot).
    SetByValue {
        /// Object register.
        obj: Reg,
        /// Key/index register.
        key: Reg,
        /// Feedback vector slot for inline caching.
        slot: u16,
        /// Whether this defines an own property of a literal
        /// (`CreateDataPropertyOrThrow`, 13.2.5.5) instead of assigning
        /// through `[[Set]]` (13.15.2). A definition reaches no Prototype.
        define: bool,
        /// Strictness of the Reference, which 10.1.9.1 reads to decide
        /// whether a write it refuses throws.
        strict: bool,
    },
    /// Loads an Array exotic object's `length` data property.
    GetArrayLength {
        /// Array object register.
        obj: Reg,
    },
    /// Throws a `TypeError` when `acc` is undefined or null, naming what the
    /// operation that asked needed of it.
    Require(RequireKind),
    /// Creates the `RegExp` object of 22.2.4.1 for one pattern of this unit.
    CreateRegExp(u16),
    /// Creates an empty object `{}` in `acc`.
    CreateObject,
    /// `CopyDataProperties` of 7.3.25 for the rest element of an object
    /// pattern (14.3.3.3), leaving the new object in `acc`.
    ///
    /// Every own enumerable key of `source` that none of the `excluded`
    /// registers names becomes a data property of it.
    CopyDataProperties {
        /// Register holding the object the properties come from.
        source: Reg,
        /// First of the `excluded` consecutive registers.
        excluded: Reg,
        /// How many of them there are.
        count: u16,
    },
    /// Creates an empty array `[]` in `acc` with initial capacity.
    CreateArray(u32),
    /// Creates a callable closure for one entry in the shared function table.
    CreateClosure(u32),
    /// Creates the constructor of a class and the prototype it carries
    /// (15.7.14), leaving the constructor in `acc`.
    ///
    /// The `prototype` a class gives its constructor is neither writable nor
    /// configurable, which no ordinary function's is.
    CreateClass(u32),
    /// Calls a method: `acc = func(args...)` with `receiver` as the `this`
    /// value the callee sees (13.3.6.1).
    CallMethod {
        /// Register holding the `this` value.
        receiver: Reg,
        /// Callable function register.
        func: Reg,
        /// First argument register.
        arg_start: Reg,
        /// Number of arguments passed.
        arg_count: u16,
        /// Feedback vector slot for call target caching.
        slot: u16,
    },
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
    /// The same call, written as the name `eval`, which 13.3.6.1 makes a
    /// direct eval: 19.2.1.1 step 4 evaluates its text in the variable
    /// environment of the function the call stands in.
    CallDirectEval {
        /// Callable function register.
        func: Reg,
        /// First argument register.
        arg_start: Reg,
        /// Number of arguments passed.
        arg_count: u16,
        /// Feedback vector slot for call target caching.
        slot: u16,
    },
    /// Advances a for-in enumeration (14.7.5.9).
    ///
    /// `state` names the first of four consecutive registers holding the object
    /// currently enumerated, that level's own-key Array, the index reached in
    /// it, and the object recording the keys already visited. The accumulator
    /// receives the next key, or undefined when the Prototype Chain is spent.
    ForInNext {
        /// First register of the four-register enumeration window.
        state: Reg,
    },
    /// Advances an iterator by one step (7.4.8) and unpacks its result.
    ///
    /// `state` names the first of two consecutive registers holding the
    /// iterator and the value a step produced. The accumulator receives true
    /// while the iterator produced a value and false once it is done.
    IteratorNext {
        /// First register of the two-register iteration window.
        state: Reg,
    },
    /// Throws `acc` as an exception (14.14.1).
    Throw,
    /// Return `acc` to caller.
    Return,
}

/// One protected instruction range and the handler that receives control when
/// a value is thrown inside it (14.15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExceptionHandler {
    /// First protected instruction offset.
    pub start_pc: u32,
    /// One past the last protected instruction offset.
    pub end_pc: u32,
    /// Instruction offset that receives control.
    pub handler_pc: u32,
    /// Register the thrown value is written to before the handler runs.
    pub exception: Reg,
}

/// What 10.4.4.7 maps the indices of an arguments object onto.
///
/// The lowering puts the parameters of a sloppy function with a simple
/// parameter list in consecutive slots of the function's own context, so the
/// index `i` maps to `slot + i` while bit `i` of `mask` is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParameterMap {
    /// The context slot index 0 maps to.
    pub slot: u16,
    /// One bit per index 10.4.4.7 maps, index 0 in bit 0.
    pub mask: u64,
}

/// Compiled bytecode unit for a function or top-level script.
#[derive(Clone, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is one property clause 10.2 gives a function"
)]
pub struct BytecodeFunction {
    /// Sequence of bytecode instructions.
    pub instructions: Vec<Instruction>,
    /// Constant pool referenced by `LdaConstant`.
    pub constants: Vec<Value>,
    /// Heap-independent UTF-16 constants referenced by `LdaString`.
    pub string_constants: Vec<Vec<u16>>,
    /// Heap-independent `BigInt` constants referenced by `LdaBigInt`.
    pub bigint_constants: Vec<super::bigint::BigIntValue>,
    /// The patterns 22.2.4.1 compiled once, addressed by `CreateRegExp`.
    ///
    /// A pattern is compiled when the Script is, so a literal makes an object
    /// of an automaton that already exists.
    pub regex_constants: Vec<Rc<crate::regexp::RegExp>>,
    /// Heap-independent nested function code addressed by `CreateClosure`.
    pub functions: Vec<BytecodeFunction>,
    /// Number of local registers required in the stack frame.
    pub register_count: u16,
    /// Number of formal parameters expected.
    pub parameter_count: u16,
    /// `ExpectedArgumentCount` of 15.1.5, which 10.2.9 gives the function as
    /// its `length`: how many parameters stand before the first one with an
    /// Initializer and before a rest parameter.
    pub expected_arguments: u16,
    /// Number of parameter/local binding registers charged to the active binding budget.
    pub binding_count: u16,
    /// Register initialized with the currently called Function object.
    pub self_register: Option<Reg>,
    /// Register initialized with the `this` value of the call (9.4.5).
    pub this_register: Option<Reg>,
    /// Register initialized with the `[[HomeObject]]` of the called function
    /// (10.2), which 13.3.7.3 reads the Prototype of.
    pub home_register: Option<Reg>,
    /// Register initialized with the `[[NewTarget]]` of the call (9.4.3),
    /// which is undefined for every call `new` did not make.
    pub new_target_register: Option<Reg>,
    /// Register the arguments object of the call is built in (10.4.4), when
    /// the body reads it.
    pub arguments_register: Option<Reg>,
    /// The `[[ParameterMap]]` of 10.4.4.7, for a sloppy function with a simple
    /// parameter list: the context slot index 0 maps to, and one bit per index
    /// that 10.4.4.7 maps at all.
    pub arguments_map: Option<ParameterMap>,
    /// Whether this function is the constructor of a class with a heritage,
    /// which 10.2.2 gives no `this` of its own and whose return answers the
    /// binding 13.3.7.1 made.
    pub derived: bool,
    /// Whether this function has a `[[Construct]]` method, which 10.2.5 gives an
    /// ordinary function and withholds from a method and an arrow.
    pub constructible: bool,
    /// Whether this function is strict, which 10.2.1.2 reads to decide what a
    /// call without a receiver binds `this` to.
    pub strict: bool,
    /// Whether this function is the constructor of a class, which 15.7.14
    /// gives a `[[Call]]` that throws.
    pub class_constructor: bool,
    /// Whether this body is the one of an async function, which 27.7.5.2
    /// answers a promise for and 27.7.5.3 may leave before it ends.
    pub asynchronous: bool,
    /// The register the capability of 27.7.5.2 is entered in, which the body
    /// resolves when it ends and answers when it waits.
    pub promise_register: Option<Reg>,
    /// The register a generator body of 27.5 keeps its own Generator in, which
    /// the frame writes before the body runs.
    pub generator_register: Option<Reg>,
    /// Whether this unit is the body of a generator of 27.5, whose function
    /// object 27.3.3 gives a prototype of its own.
    pub generator: bool,
    /// The `name` 10.2.10 gives the function, as an index into this unit's
    /// own string constants. A function 8.5.2 gives no name has none.
    pub name: Option<u16>,
    /// The source text 20.2.3.5 answers, as an index into this unit's own
    /// string constants. A unit no grammar node of a Script produced has
    /// none.
    pub source: Option<u16>,
    /// Own heap-context slot count, when this frame creates a lexical context.
    pub own_context_slot_count: Option<u16>,
    /// Slot counts expected in each captured outer lexical context.
    pub outer_context_slot_counts: Vec<u16>,
    /// Static category of every feedback vector slot.
    pub feedback_slots: Vec<FeedbackKind>,
    /// Fuel charged once in the function prologue.
    pub entry_fuel_cost: u64,
    /// Legacy operand-stack capacity required by the selected source program.
    pub entry_stack_requirement: usize,
    /// Protected instruction ranges, searched innermost first on a throw.
    pub handlers: Vec<ExceptionHandler>,
    /// Whether this unit is the root of a Script whose top-level `var` and
    /// function declarations are bindings of the Global Environment Record
    /// (16.1.7), which is what a direct `eval` of 19.2.1 writes into.
    pub realm_script: bool,
}

impl BytecodeFunction {
    /// Creates a new empty bytecode function.
    #[must_use]
    pub const fn new(register_count: u16, parameter_count: u16) -> Self {
        Self {
            instructions: Vec::new(),
            constants: Vec::new(),
            string_constants: Vec::new(),
            bigint_constants: Vec::new(),
            regex_constants: Vec::new(),
            functions: Vec::new(),
            register_count,
            parameter_count,
            expected_arguments: parameter_count,
            binding_count: parameter_count,
            self_register: None,
            this_register: None,
            home_register: None,
            new_target_register: None,
            arguments_register: None,
            arguments_map: None,
            derived: false,
            constructible: false,
            strict: false,
            class_constructor: false,
            asynchronous: false,
            promise_register: None,
            generator_register: None,
            generator: false,
            name: None,
            source: None,
            realm_script: false,
            own_context_slot_count: None,
            outer_context_slot_counts: Vec::new(),
            feedback_slots: Vec::new(),
            entry_fuel_cost: 1,
            entry_stack_requirement: 0,
            handlers: Vec::new(),
        }
    }

    /// Returns the innermost handler protecting `pc`, if any.
    ///
    /// Ranges of nested `try` statements are nested, so the narrowest range
    /// containing `pc` is the innermost one.
    #[must_use]
    pub fn find_handler(&self, pc: usize) -> Option<ExceptionHandler> {
        let pc = u32::try_from(pc).ok()?;
        self.handlers
            .iter()
            .filter(|handler| handler.start_pc <= pc && pc < handler.end_pc)
            .min_by_key(|handler| handler.end_pc.saturating_sub(handler.start_pc))
            .copied()
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

    /// Allocates a statically typed runtime feedback slot.
    pub fn allocate_feedback_slot(&mut self, kind: FeedbackKind) -> u16 {
        let slot = u16::try_from(self.feedback_slots.len()).unwrap_or(u16::MAX);
        if slot != u16::MAX {
            self.feedback_slots.push(kind);
        }
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
        for register in [
            self.self_register,
            self.this_register,
            self.home_register,
            self.new_target_register,
        ]
        .into_iter()
        .flatten()
        {
            self.verify_register(0, register)?;
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

        let count = self.instructions.len();
        for (index, handler) in self.handlers.iter().enumerate() {
            if handler.start_pc >= handler.end_pc {
                return Err(VerificationError::HandlerRangeInverted { index });
            }
            let out_of_bounds = usize::try_from(handler.end_pc).is_err()
                || usize::try_from(handler.handler_pc).is_ok_and(|pc| pc >= count)
                || usize::try_from(handler.end_pc).is_ok_and(|pc| pc > count);
            if out_of_bounds {
                return Err(VerificationError::HandlerOutOfBounds { index });
            }
            self.verify_register(0, handler.exception)?;
        }

        let mut seen = alloc::vec![false; count];
        let mut work: VecDeque<usize> = VecDeque::from([0usize]);
        for handler in &self.handlers {
            work.push_back(usize::try_from(handler.handler_pc).unwrap_or(usize::MAX));
        }
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
                Instruction::Return | Instruction::Throw => {}
                Instruction::Jump(offset) => {
                    work.push_back(self.jump_target(pc, offset)?);
                }
                Instruction::JumpIfTrue(offset)
                | Instruction::JumpIfFalse(offset)
                | Instruction::JumpIfNotNullish(offset)
                | Instruction::JumpIfNotUndefined(offset) => {
                    work.push_back(self.jump_target(pc, offset)?);
                    work.push_back(self.fallthrough(pc)?);
                }
                _ => work.push_back(self.fallthrough(pc)?),
            }
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive instruction verifier keeps every operand contract visible"
    )]
    fn verify_instruction(
        &self,
        pc: usize,
        instruction: Instruction,
        functions: &[Self],
    ) -> Result<(), VerificationError> {
        let register = match instruction {
            Instruction::Ldar(register)
            | Instruction::Star(register)
            | Instruction::ToText(register)
            | Instruction::ToNumeric(register)
            | Instruction::ToPropertyKey(register)
            | Instruction::SpreadDataProperties(register)
            | Instruction::NumberOnly(register)
            | Instruction::Add(register)
            | Instruction::Sub(register)
            | Instruction::Mul(register)
            | Instruction::Pow(register)
            | Instruction::Div(register)
            | Instruction::Mod(register)
            | Instruction::BitAnd(register)
            | Instruction::BitOr(register)
            | Instruction::BitXor(register)
            | Instruction::Shl(register)
            | Instruction::Shr(register)
            | Instruction::Ushr(register)
            | Instruction::TestEqual(register)
            | Instruction::TestStrictEqual(register)
            | Instruction::TestLessThan(register)
            | Instruction::TestLessThanOrEqual(register)
            | Instruction::TestGreaterThan(register)
            | Instruction::TestGreaterThanOrEqual(register)
            | Instruction::TestInstanceOf(register)
            | Instruction::TestIn(register)
            | Instruction::CreateArguments(register)
            | Instruction::CreateRest {
                target: register, ..
            }
            // 9.4.5 reads the register the binding lives in.
            | Instruction::ThisBinding { register } => Some(register),
            Instruction::Mov { src, dst } => {
                self.verify_register(pc, src)?;
                Some(dst)
            }
            Instruction::LoadContext { depth, slot }
            | Instruction::StoreContext { depth, slot } => {
                self.verify_context_access(pc, depth, slot)?;
                None
            }
            Instruction::Binary { lhs, rhs, slot, .. } => {
                self.verify_feedback(pc, slot, FeedbackKind::BinaryOp)?;
                self.verify_register(pc, lhs)?;
                Some(rhs)
            }
            Instruction::GetWellKnown { obj, symbol, slot } => {
                if usize::from(symbol) >= crate::engine::realm::WellKnownSymbol::ALL.len() {
                    return Err(VerificationError::ConstantOutOfBounds { pc, index: symbol });
                }
                self.verify_feedback(pc, slot, FeedbackKind::NamedAccess)?;
                Some(obj)
            }
            Instruction::GetNamed { obj, name, slot }
            | Instruction::SetNamed {
                obj, name, slot, ..
            } => {
                self.verify_string_constant(pc, name)?;
                self.verify_feedback(pc, slot, FeedbackKind::NamedAccess)?;
                Some(obj)
            }
            Instruction::GetByValue { obj, key, slot, .. }
            | Instruction::SetByValue { obj, key, slot, .. } => {
                self.verify_register(pc, obj)?;
                self.verify_feedback(pc, slot, FeedbackKind::NamedAccess)?;
                Some(key)
            }
            Instruction::DeleteNamed { obj, name, .. }
            | Instruction::DefineMethod { obj, name }
            | Instruction::DefineAccessor { obj, name, .. } => {
                self.verify_string_constant(pc, name)?;
                Some(obj)
            }
            Instruction::DeleteByValue { obj, key, .. }
            | Instruction::PrivateAccess { obj, key, .. }
            | Instruction::DefineAccessorByValue { obj, key, .. }
            | Instruction::DefineMethodByValue { obj, key, .. } => {
                self.verify_register(pc, obj)?;
                Some(key)
            }
            Instruction::CallSpread {
                receiver,
                func,
                list,
                slot,
            } => {
                self.verify_register(pc, receiver)?;
                self.verify_register(pc, func)?;
                self.verify_feedback(pc, slot, FeedbackKind::Call)?;
                Some(list)
            }
            Instruction::ConstructSpread {
                func,
                target,
                list,
                slot,
            } => {
                self.verify_register(pc, func)?;
                self.verify_register(pc, target)?;
                self.verify_feedback(pc, slot, FeedbackKind::Call)?;
                Some(list)
            }
            Instruction::GetArrayLength { obj } => Some(obj),
            Instruction::SuperBase { target } => Some(target),
            Instruction::MakeMethod { home } => Some(home),
            Instruction::DeriveClass { heritage } => Some(heritage),
            Instruction::SuperCall {
                arg_start,
                arg_count,
                forwarded,
                slot,
            } => {
                self.verify_feedback(pc, slot, FeedbackKind::Call)?;
                if forwarded {
                    None
                } else {
                    let end = u32::from(arg_start.0).saturating_add(u32::from(arg_count));
                    if end > u32::from(self.register_count) {
                        return Err(VerificationError::CallArgumentsOutOfBounds { pc });
                    }
                    Some(arg_start)
                }
            }
            Instruction::GetSuper { base, name } => {
                self.verify_string_constant(pc, name)?;
                Some(base)
            }
            Instruction::GetSuperByValue { base, key } => {
                self.verify_register(pc, base)?;
                Some(key)
            }
            Instruction::Call {
                func,
                arg_start,
                arg_count,
                slot,
            }
            | Instruction::CallDirectEval {
                func,
                arg_start,
                arg_count,
                slot,
            } => {
                self.verify_call(pc, func, arg_start, arg_count, slot)?;
                None
            }
            Instruction::CallMethod {
                receiver,
                func,
                arg_start,
                arg_count,
                slot,
            } => {
                self.verify_register(pc, receiver)?;
                self.verify_call(pc, func, arg_start, arg_count, slot)?;
                None
            }
            Instruction::Construct {
                func,
                target,
                arg_start,
                arg_count,
                slot,
            } => {
                self.verify_register(pc, target)?;
                self.verify_call(pc, func, arg_start, arg_count, slot)?;
                None
            }
            Instruction::LdaConstant(index) => {
                if usize::from(index) >= self.constants.len() {
                    return Err(VerificationError::ConstantOutOfBounds { pc, index });
                }
                None
            }
            Instruction::LdaBigInt(index) => {
                if usize::from(index) >= self.bigint_constants.len() {
                    return Err(VerificationError::ConstantOutOfBounds { pc, index });
                }
                None
            }
            Instruction::StaGlobal { name: index, .. }
            | Instruction::CreatePrivateName(index)
            | Instruction::LdaString(index)
            | Instruction::LdaGlobal(index)
            | Instruction::LdaGlobalForTypeOf(index)
            | Instruction::VerifyGlobalVar(index)
            | Instruction::DeclareGlobalVar(index)
            | Instruction::VerifyGlobalFunction(index)
            | Instruction::VerifyGlobalLexical(index)
            | Instruction::InitializeGlobalLexical(index)
            | Instruction::DeclareGlobalFunction(index)
            | Instruction::DeclareGlobalLexical { name: index, .. } => {
                if usize::from(index) >= self.string_constants.len() {
                    return Err(VerificationError::StringConstantOutOfBounds { pc, index });
                }
                None
            }
            Instruction::CreateClosure(index) | Instruction::CreateClass(index) => {
                let target = usize::try_from(index)
                    .ok()
                    .and_then(|index| functions.get(index))
                    .ok_or(VerificationError::FunctionOutOfBounds { pc, index })?;
                if !target.outer_context_matches(self) {
                    return Err(VerificationError::ClosureContextMismatch { pc, index });
                }
                None
            }
            Instruction::Jump(offset)
            | Instruction::JumpIfTrue(offset)
            | Instruction::JumpIfFalse(offset)
            | Instruction::JumpIfNotNullish(offset)
            | Instruction::JumpIfNotUndefined(offset) => {
                self.jump_target(pc, offset)?;
                None
            }
            Instruction::LdaSmi(_)
            | Instruction::Negate
            | Instruction::LogicalNot
            | Instruction::ToUndefined
            | Instruction::ToNumber
            | Instruction::BitNot
            | Instruction::TypeOf
            | Instruction::Increment
            | Instruction::Decrement
            | Instruction::LdaGlobalThis
            | Instruction::LdaUndefined
            | Instruction::LdaNull
            | Instruction::LdaTrue
            | Instruction::LdaFalse
            | Instruction::Require(_)
            | Instruction::CreateObject
            | Instruction::CreateRegExp(_)
            | Instruction::CreateArray(_)
            | Instruction::Throw
            | Instruction::Await
            | Instruction::GeneratorStart
            | Instruction::Yield
            | Instruction::Return => None,
            Instruction::CopyDataProperties {
                source,
                excluded,
                count,
            } => {
                self.verify_register(pc, source)?;
                for offset in 0..count {
                    let register = excluded.0.checked_add(offset).ok_or(
                        VerificationError::RegisterOutOfBounds {
                            pc,
                            register: excluded,
                        },
                    )?;
                    self.verify_register(pc, Reg(register))?;
                }
                None
            }
            Instruction::IteratorNext { state } => {
                for offset in 0..2 {
                    let register = state.0.checked_add(offset).ok_or(
                        VerificationError::RegisterOutOfBounds {
                            pc,
                            register: state,
                        },
                    )?;
                    self.verify_register(pc, Reg(register))?;
                }
                None
            }
            Instruction::ForInNext { state } => {
                for offset in 0..4 {
                    let register = state.0.checked_add(offset).ok_or(
                        VerificationError::RegisterOutOfBounds {
                            pc,
                            register: state,
                        },
                    )?;
                    self.verify_register(pc, Reg(register))?;
                }
                None
            }
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

    fn verify_feedback(
        &self,
        pc: usize,
        slot: u16,
        expected: FeedbackKind,
    ) -> Result<(), VerificationError> {
        let actual = *self
            .feedback_slots
            .get(usize::from(slot))
            .ok_or(VerificationError::FeedbackOutOfBounds { pc, slot })?;
        if actual == expected {
            Ok(())
        } else {
            Err(VerificationError::FeedbackKindMismatch {
                pc,
                slot,
                expected,
                actual,
            })
        }
    }

    fn verify_string_constant(&self, pc: usize, index: u16) -> Result<(), VerificationError> {
        if usize::from(index) >= self.string_constants.len() {
            Err(VerificationError::StringConstantOutOfBounds { pc, index })
        } else {
            Ok(())
        }
    }

    fn context_slot_count(&self, depth: u16) -> Option<u16> {
        let depth = usize::from(depth);
        if let Some(own) = self.own_context_slot_count {
            if depth == 0 {
                Some(own)
            } else {
                self.outer_context_slot_counts
                    .get(depth.checked_sub(1)?)
                    .copied()
            }
        } else {
            self.outer_context_slot_counts.get(depth).copied()
        }
    }

    fn verify_context_access(
        &self,
        pc: usize,
        depth: u16,
        slot: u16,
    ) -> Result<(), VerificationError> {
        if self
            .context_slot_count(depth)
            .is_some_and(|count| slot < count)
        {
            Ok(())
        } else {
            Err(VerificationError::ContextOutOfBounds { pc, depth, slot })
        }
    }

    fn verify_call(
        &self,
        pc: usize,
        function: Reg,
        argument_start: Reg,
        argument_count: u16,
        slot: u16,
    ) -> Result<(), VerificationError> {
        self.verify_register(pc, function)?;
        self.verify_register(pc, argument_start)?;
        self.verify_feedback(pc, slot, FeedbackKind::Call)?;
        let end = u32::from(argument_start.0).saturating_add(u32::from(argument_count));
        if end > u32::from(self.register_count) {
            Err(VerificationError::CallArgumentsOutOfBounds { pc })
        } else {
            Ok(())
        }
    }

    fn outer_context_matches(&self, creator: &Self) -> bool {
        self.outer_context_slot_counts
            .iter()
            .enumerate()
            .all(|(depth, count)| {
                u16::try_from(depth)
                    .ok()
                    .and_then(|depth| creator.context_slot_count(depth))
                    == Some(*count)
            })
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

        let mut default = BytecodeFunction::new(0, 0);
        default.emit(Instruction::LdaUndefined);
        default.emit(Instruction::JumpIfNotUndefined(1));
        default.emit(Instruction::LdaSmi(42));
        default.emit(Instruction::Return);
        assert_eq!(default.verify(), Ok(()));
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

        let mut self_register = BytecodeFunction::new(1, 0);
        self_register.self_register = Some(Reg(1));
        self_register.emit(Instruction::Return);
        assert_eq!(
            self_register.verify(),
            Err(VerificationError::RegisterOutOfBounds {
                pc: 0,
                register: Reg(1)
            })
        );

        let mut self_binding = BytecodeFunction::new(1, 0);
        self_binding.self_register = Some(Reg(0));
        self_binding.binding_count = 1;
        self_binding.emit(Instruction::Return);
        assert_eq!(self_binding.verify(), Ok(()));

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

        feedback.feedback_slots.push(FeedbackKind::Call);
        assert_eq!(
            feedback.verify(),
            Err(VerificationError::FeedbackKindMismatch {
                pc: 0,
                slot: 0,
                expected: FeedbackKind::NamedAccess,
                actual: FeedbackKind::Call,
            })
        );

        let mut call = BytecodeFunction::new(2, 0);
        call.feedback_slots.push(FeedbackKind::Call);
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

        let mut undefined_jump = BytecodeFunction::new(0, 0);
        undefined_jump.emit(Instruction::JumpIfNotUndefined(1));
        undefined_jump.emit(Instruction::Return);
        assert_eq!(
            undefined_jump.verify(),
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

        let mut context = BytecodeFunction::new(0, 0);
        context.own_context_slot_count = Some(1);
        context.emit(Instruction::LoadContext { depth: 0, slot: 1 });
        context.emit(Instruction::Return);
        assert_eq!(
            context.verify(),
            Err(VerificationError::ContextOutOfBounds {
                pc: 0,
                depth: 0,
                slot: 1,
            })
        );

        let mut root = BytecodeFunction::new(0, 0);
        root.emit(Instruction::CreateClosure(0));
        root.emit(Instruction::Return);
        let mut child = BytecodeFunction::new(0, 0);
        child.outer_context_slot_counts.push(1);
        child.emit(Instruction::LoadContext { depth: 0, slot: 0 });
        child.emit(Instruction::Return);
        root.functions.push(child);
        assert_eq!(
            root.verify(),
            Err(VerificationError::ClosureContextMismatch { pc: 0, index: 0 })
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
