# jrs — JavaScript Rust

An initial JavaScript bytecode runtime core, written in safe Rust without
external dependencies. The library uses the internal `audhsos-regex` and
`audhsos-event-target` crates
and is `no_std` plus the Rust toolchain's `alloc`;
the host executable is a separate crate. It is **not yet an ECMA-262
conforming implementation** and does not implement a browser environment.

```rust
use jrs::{compile, Limits, Runtime, SilentHost, Value};
let limits = Limits::default();
let program = compile("let sum = 0; for (let i = 0; i < 10; i++) { sum += i; } sum", limits)?;
assert_eq!(Runtime::new(limits).run(&program, &mut SilentHost)?, Value::Number(45.0));
# Ok::<(), jrs::Error>(())
```

## Execution

`sh tools/xtask.sh jrs -e 'print("hello from jrs"); 6 * 7'` builds the host
CLI in release mode and runs a script. A file path can replace `-e SOURCE`.
`--fuel N` bounds executed instructions. `--stats` reports separate compile
and execute timings; `--bench N` compiles once and repeats isolated runs.

`sh tools/xtask.sh jrs --realm FIRST.js SECOND.js` executes independent scripts
in one persistent realm, with a Promise job checkpoint after each script.
The embedding equivalent is `Realm::new(limits, host)?.evaluate(source)`.
Unlike `Runtime::run`, this mode retains globals, lexical bindings, closures,
intrinsics, symbol identities and pending async continuations between scripts.
Global declarations are validated before publication: let/const/class live in
the declarative environment, var/functions in the global object. Lookup by name
observes both; global object writes and var reads stay synchronized. Function
hoisting never crosses script boundaries. Sloppy global assignment/deletion and
cross-script conflicts are supported in this mode. Fuel and heap quotas are
cumulative. Syntax/runtime language errors preserve earlier state; embedding
resource/host/invariant failures poison the realm. Returned/thrown identities
are explicitly retained with a bounded quota until release or Realm drop.

## Host embedding

`Realm::host_function(id, name, length)` creates a distinct, non-constructible
function that dispatches to the embedding's `Host::call`. It has ordinary mutable
properties, a Function prototype, readonly/configurable name/length and no
constructor prototype. The id selects only a capability the host explicitly
implements; no filesystem, network or clock access is added implicitly.

`Realm::set_global`, `get_global`, `object`, `get`, `set` and `define_property`
let Rust supply object data and accessor functions without generating source.
`Realm::call` invokes a saved JavaScript function with the supplied receiver
and arguments as a later host turn, then drains Promise jobs. Conversion helpers
`to_string`/`to_number` use the fallible VM algorithms, including user hooks.

Host calls receive uncoerced values. Their receiver/arguments, print arguments,
unhandled-rejection reasons and API results are retained so the host can keep a
callback and invoke it later. `Realm::release` removes that value's implicit
retention; Rust Value clones do not have separate root counts. After release,
do not reuse old copies without obtaining a newly retained value. Retention is
bounded by the properties quota and indexed by handle for O(log n) lookup.
All incoming values are checked for realm ownership, generation, kind and string
limits before entering the heap. Foreign/stale API inputs return `TypeError`;
invalid host return/throw values become a fatal Host error. Language exceptions
remain catchable, while embedding errors poison the realm.

The host cannot synchronously reenter the borrowed realm from `Host::call`;
store callback identities and schedule a later explicit `Realm::call` instead.
Rust host code must bound its own work and allocations: VM fuel cannot interrupt
an arbitrary host function. These mechanisms support future event/timer hosts;
they do not themselves implement browser events, tasks or origins.

`Realm::install_queue_microtask()` explicitly installs the HTML queueMicrotask
function. It is absent from the default ECMAScript globals. Callbacks enter the
same FIFO queue as Promise reactions, receive no arguments and undefined this,
and their return values are ignored. Throws are sent to `Host::report_exception`,
not converted into Promise rejections. The default host reports failure after
draining remaining microtasks; a custom host may record errors and continue.
Host/resource failures abort immediately. `Realm::queue_microtask` lets Rust
enqueue without executing; `Realm::checkpoint` drains the queue. Queue entries
root their callbacks even after host release, with the existing jobs/fuel quotas.
DOM `ErrorEvent` dispatch, incumbent settings and cross-realm exception routing are absent.

`Realm::install_events()` installs standalone Event, `CustomEvent`, `EventTarget`,
`DOMException`, `AbortController` and `AbortSignal` interfaces. The separate `crates/event-target` core owns bounded
listener IDs/snapshots and event flags; the adapter performs callback/dictionary
conversion, GC tracing and exception reporting. Targets have no parent path.
Capture runs before the non-capture phase at the target. Once listeners are
removed before callbacks; removed/re-added IDs cannot reappear in an old
snapshot. Passive cancellation, stop/stopImmediatePropagation, composedPath,
initEvent/initCustomEvent, handleEvent objects and shared isTrusted getters are
implemented. Synthetic events are untrusted. Listener exceptions are reported
to Host, not thrown from dispatchEvent; the default host fails the enclosing
turn after continuing the dispatch. Fatal host/resource errors abort promptly.
Event timestamps come from `Host::event_timestamp` (default zero, no clock).
The WPT host supplies elapsed monotonic milliseconds. `DOMException` has branded
name/message/code accessors, Error ancestry and legacy constants.

This is not a DOM tree or Window implementation: tree/shadow propagation,
activation, global event-handler attributes,
`ErrorEvent` routing and cross-realm callback contexts remain
unimplemented. Listener storage and dispatch work are quota/fuel bounded.

`AbortController` stores a stable branded signal; abort/throwIfAborted preserve
the exact reason (including null) and default to an `AbortError` `DOMException`.
AbortSignal.abort creates an already-aborted signal without dispatch.
AbortSignal.any converts the complete Web IDL sequence before flattening weak
source/dependent links. Reasons are set on all affected signals before any
abort callback; listener-removal algorithms run before each trusted abort event.
`onabort` is an HTML event-handler registration whose position survives callback
replacement. Public dispatchEvent and initEvent clear event trust. Registry IDs
prevent stale abort steps from removing a later registration; once/removal detach
obsolete algorithms. GC traces reasons, handlers and live removal algorithms;
source links stay weak, with observed pending dependents retained by live sources.
Signal/algorithm/sequence storage and scans obey property/fuel quotas.
`Realm::install_timers` explicitly enables setTimeout/setInterval, cancellation
and AbortSignal.timeout. `Host::timer_now` supplies monotonic active-time
nanoseconds, with a default-denied capability. `Realm::timer_wait` exposes the
next delay; `run_timer` executes at most one due task and its microtask checkpoint.
Neither script evaluation nor the core waits or advances time. The host must
exclude suspension/inactive time for a browser embedding; the shell is always
active. Missing/backwards clocks and quota failures are fatal.

`crates/timer-queue` is the independent bounded stable deadline queue, with two
B-tree indexes, O(log n) insertion/cancellation/extraction and no tombstones.
The adapter applies Web IDL signed-long delay conversion, the 4 ms nesting clamp,
global callback receivers, argument retention, interval rescheduling and shared
timeout/interval cancellation. String handlers run as separate global Scripts
after the host compilation-policy check; errors use `Host::report_exception`.
Trusted Types, CSP metadata, real Window/Worker brands and cross-realm task
selection remain unimplemented. The embedding must not treat this shell adapter
as full browser timer conformance. Pending tasks are bounded by `Limits.jobs`,
retained timer arguments by `Limits.stack`; IDs are positive and never reused.
IDs/deadline/token exhaustion return resource errors. Abort timeouts enforce the
Web IDL 53-bit range, allocate `TimeoutError` only when fired, and retain their
signal until expiry (including unobserved signals); cancellation by GC is not
implemented. No timer is turned into an immediate synchronous abort.

```rust
use jrs::{Error, Host, Limits, Realm, Value};
struct Echo;
impl Host for Echo {
    fn print(&mut self, _: &[Value]) -> Result<(), Error> { Ok(()) }
    fn call(&mut self, _: u32, _: &Value, args: &[Value]) -> Result<Value, Error> {
        Ok(args.first().cloned().unwrap_or(Value::Undefined))
    }
}
let mut host = Echo;
let mut realm = Realm::new(Limits::default(), &mut host)?;
let echo = realm.host_function(1, "echo", 1)?;
realm.set_global("echo", &echo)?;
assert_eq!(realm.evaluate("echo(42)")?, Value::Number(42.0));
let callback = realm.evaluate("(x => x + 1)")?;
assert_eq!(realm.call(&callback, &Value::Undefined, &[Value::Number(41.0)])?, Value::Number(42.0));
realm.release(&callback)?;
# Ok::<(), Error>(())
```

The pipeline is a bounded lexer, bounded Pratt parser, lexical slot
resolution, immutable bytecode, and a fuel-metered stack VM. Loops do not
reparse source, walk an AST, or look up binding names. Runtime buffers and
compiled programs are reusable. Immutable UTF-16 strings share allocations.
Ordinary and arrow functions compile to separate bytecode bodies. Calls use
an explicit VM frame stack, never the Rust stack. Uncaptured locals reside
directly in frames; captured bindings use generation-checked heap handles.
A non-moving mark/sweep collector traces binding/function cycles. The caller
controls both the live heap-entry limit and the call-frame limit.
There is no JIT and no claim of competitive performance without benchmarks.

## Implemented surface

- Undefined, Null, Boolean, binary64 Number and UTF-16 String primitives;
  unpaired surrogates are preserved internally.
- Symbol identities, UTF-16 descriptions, Symbol.for/keyFor with a bounded
  registry, well-known symbols, wrappers and branded prototype methods.
  Symbol properties are separate from string properties, support accessors,
  descriptors, deletion and integrity operations, and retain creation order.
  Object.getOwnPropertySymbols exposes them; string-key enumeration omits them.
  Symbol.toPrimitive honors string/number/default hints, and Symbol.toStringTag
  is an inherited, overridable property used by Object.prototype.toString.
- Boolean/Number/String wrappers, their distinct prototypes, constructors,
  branded valueOf/toString, Object boxing and primitive prototype lookup.
  Sloppy calls box `this` once; strict accessor receivers preserve the primitive.
  String wrappers expose virtual, immutable UTF-16 character properties without
  allocating a property per code unit. Key enumeration has its own quota.
  Math's eight numeric constants have readonly attributes.
- Decimal, binary, octal and hexadecimal numeric literals, numeric
  separators, string escapes, comments, ASCII identifiers and basic ASI.
- `let`, `const`, `var`, blocks, lexical shadowing and temporal dead zones;
  duplicate declarations are rejected before execution.
- Arithmetic, primitive loose/strict equality and relational comparisons,
  Number bitwise operators and signed/unsigned shifts,
  logical operators, nullish coalescing, conditional expressions, assignment,
  compound arithmetic assignment, prefix/postfix increment/decrement.
- Array and object destructuring assignments support nested patterns, defaults,
  rest targets, computed property keys and property-reference targets. Reference
  evaluation, property reads, iterator closing and the assignment expression's
  result follow their specified observable order.
- `if`, `while`, three-part `for`, unlabelled `break` and `continue`.
- Untagged template literals with nested substitutions, cooked escapes, line
  normalization and string-hint conversion in evaluation order. Tagged
  templates and their raw/cooked template-object identity are not implemented.
- `instanceof` with ordinary prototype-chain semantics (without Symbol hooks),
  `switch` with strict selectors, fallthrough and shared lexical case scope,
  `for…in` key enumeration, and synchronous iterable `for…of`. Declarations and
  for-of bindings support nested array/object binding patterns, elisions,
  defaults and array rest. Object rest copies own enumerable String and Symbol
  properties while preserving observable key/getter order. Symbol.iterator is called with the original receiver; the
  returned iterator's next method is cached and done is read before value.
  `IteratorClose` runs on early exits and binding-pattern completion, in lexical
  order with finally handlers. Next/done/value failures mark the record done;
  elisions do not access value. `Object.entries` and `Object.values` return arrays.
- Array keys/values/entries, Array.prototype[Symbol.iterator] (the same function
  as values), String code-point iterators and iterable arguments objects. Builtin
  iterators have shared prototypes, live array-like lengths, branded next calls
  and permanent completion. Their sources and cached user next functions are
  traced through active and suspended async frames. User next/return functions
  run under the same host, fuel and native-reentry quotas as other calls.
- Function declarations and expressions, named self-recursion, arrow
  functions with simple, default, rest and destructured parameters, `return`,
  hoisting, first-class calls and closures over shared mutable bindings.
  `for (let ...)` creates
  per-iteration bindings; `for (var ...)` retains one shared binding.
  Non-simple parameters initialize through bytecode with a parameter TDZ;
  default-expression closures do not see body `var`/function declarations.
  Function `length` stops at the first default/rest parameter.
- Spread arguments in calls, constructors and explicit super calls, plus array
  literal spread and mixed elisions. Expansion consumes the actual Symbol.iterator
  protocol before evaluating the next argument/element; iterated holes become
  undefined values while literal elisions remain holes. Calls without spread
  retain their fixed-arity bytecode path. Expanded arguments live directly on the
  bounded operand stack, with a private count preserved across await/suspension.
  Array elements use data-property creation, not push/setter/species hooks.
  The super constructor is captured before argument evaluation, including
  iterator side effects. Object spread remains unimplemented.
- Async function declarations/expressions, async arrows/methods and `await`.
  Await stores the frame, operand stack, lexical bindings, loop state and
  exception handlers in the tracing heap and resumes only through a FIFO job.
- Promise constructor, `resolve`, `reject`, `withResolvers`, `then`, `catch`,
  `finally`, and iterable-input `all`, `allSettled`, `race`. Executors run
  synchronously; reactions and thenable adoption are jobs. Resolve/reject pairs
  share a one-shot latch. The host checkpoint drains jobs under the script's
  fuel and reports unhandled rejections via `Host::unhandled_rejection`.
- Promise capabilities use the selected constructor's resolve/reject functions,
  including generic non-Promise constructors. Then/finally select Symbol.species;
  resolve preserves identity only for a matching constructor. Combinators read
  the constructor's resolve once, use its receiver and close abrupt iterations.
  Finally uses the abstract `PromiseResolve` operation, not an overridable public
  resolve property. Queued reactions trace the captured capability functions;
  exceptions thrown by custom resolvers remain observable host job failures.
- `WeakMap` constructor for iterable entries, get/set/has/delete, getOrInsert and
  getOrInsertComputed. Keys use identity and O(log n) B-tree lookup, including
  script/bound/native functions and distinct resolve/reject identities. A global
  `Limits.weak_entries` quota triggers collection before rejecting an insert.
  Entries are ephemerons: neither a dead map nor an otherwise dead key keeps a
  value alive. The collector uses a key-indexed worklist, not repeated full-heap
  scans; cycles and long chains are tested against a fixed-point reference model.
  Non-registered Symbol keys and `Symbol.toStringTag` are implemented. Registered
  symbols are rejected as weak keys. Constructor entry/adder errors close the
  iterator while preserving the original throw; next failures do not close it.
- First-class native calls to `Number`, `String`, `Boolean`, `isNaN`,
  `isFinite`, and the explicit host capability `print`.
- Ordinary object literals (including shorthand, computed keys, methods and
  the literal `__proto__` setter), dot/bracket references, property assignment,
  updates, `delete`, `in`, prototype inheritance and data-property attributes.
  Objects, property values, prototypes and lexical `this` are traced by GC.
- Method-call receivers, strict function `this`, lexical arrow `this`,
  `Function.prototype.call` dispatch, and the `globalThis` object. Ordinary
  calls with null/undefined receivers bind to the global object in sloppy mode.
- `Object`, `Object.create` with descriptors, `defineProperties`, `getPrototypeOf`,
  `setPrototypeOf`, `defineProperty`/`getOwnPropertyDescriptor`,
  `hasOwn`, `hasOwnProperty`, `is`, `preventExtensions`, `isExtensible`,
  `freeze`, `seal`, `isFrozen`, `isSealed`, and basic object `toString`.
- `throw`, `try`/`catch`/`finally`, catch binding scopes and exact thrown-value
  identity. Explicit completion unwinding runs finally on return and loop
  exits, allows finally to replace a completion, and traces pending values.
  Type/reference errors become objects with `name` and `message` when caught;
  Error/TypeError/RangeError/ReferenceError/SyntaxError/EvalError/URIError have
  distinct prototypes, constructors, message/cause, and Error toString/isError.
  Fuel, heap, binding and host failures remain non-catchable embedding errors.
- Array literals with holes, `Array`/`Array.of`/`Array.from`/`Array.isArray`, the exotic
  length/index invariant and shrinking past non-configurable elements.
  `push`, `pop`, `join`, `slice`, `indexOf`, `includes`, `toString`, `forEach`,
  `map`, `filter`, `some`, `every` and `reduce`; object key/name enumeration
  now returns arrays. Callback algorithms compile once into private bytecode
  and use the same VM frames, fuel and exception machinery as script calls.
  Stable `sort` collects present elements first, uses bounded O(n log n)
  merge passes, then writes elements and deletes holes. Comparator exceptions,
  mutation, GC, undefined ordering and UTF-16 default comparison are covered.
- Function `apply` and bound calls with length/name/prototype handling. Bound
  construction is not implemented. Arguments objects use shared cells for
  simple sloppy parameters; strict/non-simple arguments are unmapped, with
  restricted callee access. Deletion, descriptors and freeze detach mappings.
- String split with string/RegExp separators, captures and empty-match handling;
  replace with string/RegExp searches
  and callbacks or substitution templates, and String.fromCharCode. Lone
  surrogates remain intact. Match/search/split/replace Symbol hooks and split species
  are implemented; matchAll and other regex operations remain open.
- `new Array`, `new Object` and ordinary function constructors, including
  their prototype/constructor links and object-versus-primitive return rules.
  Arrows and method definitions are not constructors.
- Class declarations/expressions with a lexical class-name TDZ, strict methods,
  static methods, computed keys, getters/setters, extends and default or explicit
  constructors. Class calls without new fail. Derived constructors initialize a
  shared this cell exactly once through super; earlier arrow closures see its
  later initialization. New.target propagates through inheritance and default
  parameters. Super method/accessor lookup uses the home object's prototype
  with the actual receiver, including assignment/update. Direct construction
  uses explicit VM frames; synchronous super/native construction has the existing
  bounded native-reentry bridge. Array/Promise/WeakMap/primitive/Error subclasses
  use the derived prototype. Promise capabilities/species support custom constructors.
- Accessor literals and descriptors, inherited getters/setters with the original
  receiver, descriptor field evaluation order and frozen-accessor behaviour.
  Ordinary object-to-primitive conversion calls `valueOf`/`toString` in hint
  order for arithmetic, comparisons, String/Number and property keys.
  `Math.max`/`Math.min` convert all arguments in order and preserve signed zero.
  Native reentry shares the real host, fuel and VM frames. Native intermediate
  values are explicit GC roots. Reentry is capped at 12 to bound Rust stack use.
- `RegExp` literals and constructor, `exec`/`test`/`toString`, UTF-16 captures,
  `d` indices, `g`/`y` lastIndex handling, `m`/`s`, and string match/search.
  Every match is executed by `audhsos-regex` in `crates/regex/`, never by
  another engine; its measured work consumes VM fuel.

Missing language features include complete function properties,
remaining array methods, complete completion records outside exception handlers, Unicode
identifiers, `BigInt`, full `RegExp`, modules, class fields/private elements/static
blocks, iterator helpers, generators,
remaining Promise combinators, typed arrays, and the standard object library. Number-to-string
uses the toolchain's shortest decimal conversion with ECMAScript's notation
window and exponent sign; non-decimal number formatting is absent.
Legacy literals are rejected. Ordinary
script sloppy-mode global assignment is implemented in Realm mode, not the
isolated compile/`Runtime::run` mode. No unsupported
feature is a conformance pass.
The public Value-only conversion helpers cannot run object hooks; the VM's
fallible conversions do. The public Value-only `ToNumber` helper returns NaN for
Symbols because it cannot throw; the VM correctly throws on numeric/implicit
string Symbol conversion. Explicit String(symbol) uses `SymbolDescriptiveString`.
Array `join` handles nested arrays and user-defined
element conversion, but cyclic joins currently hit the reentry limit.
Array `concat` supports holes, inherited elements, generic receivers,
`Symbol.isConcatSpreadable` and species result construction (including ordinary
object results). It creates data properties and sets the final length. The
concat path uses the full 53-bit array-like range, and loops consume fuel.
Map/filter/slice also implement same-realm `Symbol.species`, including ordinary
object and aliased results. Proxies and cross-realm species remain incomplete.
The generic reverse/lastIndexOf/fill/copyWithin/at/splice, push/pop/join,
indexOf/includes and callback paths use the full `ToLength` 53-bit range.
Sort now also uses 53-bit array-like length/index arithmetic. Selected internal
JSON array consumers retain a u32 helper; actual Array lengths remain u32.
Array, Object, Boolean, Number, String, `RegExp`, Promise, Errors and ordinary
user functions currently support `new`.
Async iteration and async generators
are not implemented. Pending promises do not keep the CLI alive without
runnable jobs. Suspension/frame/operand quotas include suspended state and
are recomputed after collection. The job/reaction quota is explicit. The
default unhandled-rejection host policy fails the run rather than hiding a
rejected test.
Intrinsic native function properties are currently a fixed lookup. Explicitly
installed host functions have ordinary mutable function objects.
The Object, Number, Array and `RegExp` constructors keep realm-local, traced property storage while their
callable identities remain immediate: static properties can be assigned, deleted,
redefined, sealed/frozen and given Symbol keys without being recreated by lookup.
Other immediate native constructors retain their existing mutability gaps.
`globalThis` mirrors var declarations in Realm mode; the
isolated `Runtime::run` compiler retains its earlier local-slot global model.
Object keys are UTF-16; lookup uses a B-tree and tracks creation order separately.
Strict directives are recognized (including inherited strictness) for binding
and parameter validation, but full strict/sloppy semantics remain incomplete.
Block functions currently have lexical semantics, not Annex B sloppy-mode
extensions. Function-to-string preserves the original grammar-node source for
implemented functions, arrows, methods/accessors and classes; native/bound
functions return native-function syntax. Returned Realm
functions can be called from Rust through `Realm::call`. Values returned from
isolated `Runtime::run` do not keep that execution heap alive and cannot be imported
into another Realm.

Limits bound source, tokens, recursion, bytecode, operand stack, string length
and executed instructions, active call frames, total active binding slots
and live heap entries. Own-property counts have a separate per-object quota.
Weak associations have a global entry quota; GC state/edge visits, ephemeron
activation and sweeping consume shared fuel and cannot be caught by scripts.
Symbols are generation-checked heap entries; property keys, wrappers and
registry/well-known roots are traced. Symbol keys in `WeakMap` instances stay weak.
Registry and well-known symbol identity persist across scripts in one Realm;
`Runtime::run` remains fresh per run. Multiple communicating realms/agents and
HTML incumbent-realm tracking are not implemented.
Symbol-keyed properties on script functions work; mutable native-function
property objects remain incomplete. Well-known symbol values alone do not
implement all array/RegExp species or `RegExp` dispatch protocols. Concat,
Promise
species and the synchronous
iterator protocol is implemented; async iteration and Iterator helpers are not.
The recursive frontend has an internal depth cap of 48 after the object AST
increased its debug-build stack usage; deep inputs return a limit error.
Allocation failure is **not** recoverable: an
embedding must supply an allocator and a memory quota. The current OS
userland has no `alloc` adapter, so this core is host-executable and
cross-compilable, not installed as an `AuDHSOS` user process yet. No filesystem,
network or clock is exposed to scripts.

## Standards and acceptance

The implementation reads the repository's immutable
[ECMA-262 draft](../../docs/ecma/ecma262.html), in particular clauses 2,
4.1–4.2, 6.1, 6.1.6.1.2–20, 7.1.2, 7.1.4–9, 7.2.13–14, 12.2–12.4,
9.1.1.1, 10.2.1, 10.2.3, 10.2.11, 11.2.1–2, 12.7, 12.9.3–4, 12.10,
10.1, 13.2.5, 13.3.2, 13.7–13.8, 14.3.1–2, 14.7.4.4, 14.10, 14.14–15, 15.2, 15.3
and 20.1.2. Array clauses consulted are 10.4.2, 13.2.4, 23.1.1–2 and the
implemented methods of 23.1.3; constructor clauses are 10.2.2, 10.2.5 and
13.3.5. Algorithms are independently implemented; no source code from another
runtime is included.

`RegExp` uses the separate [regex core](../regex/README.md),
with a Thompson NFA/DFA and no backtracking fallback or external dependency.
General backreferences conflict with this automaton restriction and remain
an explicit compatibility gap, not a silently approximated feature.
The separately authorized `crates/regex-bt` now provides a bounded backtracking
core. jrs has not switched to it and has no automatic fallback; runtime integration
and full `RegExp` syntax remain separate work.
The current regex subset supports single-literal/class lookahead as a zero-width
NFA predicate; it rejects general lookaround, named groups, nullable
quantified expressions, Unicode/code-point flags and ignore-case mode.
`RegExp` instances inherit methods/accessors from the intrinsic prototype and
own lastIndex. The lexer uses contextual expression
tracking for slash tokens; the complete ECMAScript lexical-goal grammar is
not implemented. A single NFA search is linear in input length for a fixed
pattern; global match/replace and split repeatedly call it and are fuel-bounded,
but their total work is not yet guaranteed linear across all returned matches.
Regex split observes species construction and dynamic exec, includes captures
and skips boundary empty matches. A guard permits one Thompson search across
failed starts only for an initialized sticky instance with the intrinsic prototype
and unchanged builtin exec data property. It is rechecked after custom code;
generic splitters expose each Set/exec/Get step. Normally the original lastIndex
is unchanged, but custom species may deliberately return the original object.
`matchAll` and other intrinsic/pattern functionality remain incomplete.

WPT describes itself as a cross-browser Web-platform test suite and its
`wpt run` command as running tests in a browser. ECMA-262 4.1–4.2 likewise
distinguishes the language from browser host facilities. Full WPT acceptance
therefore remains open: the language core alone cannot provide DOM, HTML,
CSS/layout, navigation, network services, workers or browser automation.
The language specification also links Test262 as its language conformance
suite. Neither suite is replaced by the crate's own regression tests.

On 2026-09-08, direct execution probes against WPT revision
`7926f3ca1cd9f7f1df88db0bb0cee14f278a559b` failed: `resources/testharness.js`
was rejected by the lexer at byte 22427, and `js/builtins/Math.maxmin.js`
by the parser at byte 0. After function/var support was added, the latter
advanced to byte 51, the first object literal. Object support moved that to
byte 204 (`throw`). With throw/catch/finally support the entire support file
loads successfully; its test function has **not** run, and this is no WPT pass.
These are harness/support-script probes, **not a
WPT suite run**; no WPT pass count is claimed. The original files were
streamed into the CLI unchanged, not vendored as runtime dependencies.
After regex integration, the unchanged harness probe advances from byte 22427
to byte 54850, a template literal. The first harness regex now has a matching
regression test. This remains a failed harness probe, not a WPT suite pass.
Template support allows the entire source to tokenize. Subsequent parser
gaps at `instanceof` (20423), default parameters (46466), for-in (54708),
switch (56676) and for-of pair binding (105879) have been implemented. The
async-function gap at byte 194062 is now implemented. The unchanged harness
parses further and compilation reaches a `RegExp` lookaround. The one-character
negative class assertion needed there now compiles directly into the NFA.
The unchanged harness loads and executes in the shell host.

On 2026-09-09, `jrs --wpt ROOT FILE...` executed the original harness and
scripts from that pinned revision: all twelve selected files pass completely,
with 106 passing subtests.
Passing files: Array.DefineOwnProperty, Array.prototype.join-order, Math.max, Math.min,
Object.prototype.freeze, getOwnPropertyNames, hasOwnProperty-order,
hasOwnProperty-prototype-chain, preventExtensions and seal (all `.html`).
Array.prototype.join-order now passes all 51 subtests with primitive boxing.
Promise-subclassing passes all eight constructor/species subtests.
WeakMap.prototype-properties
passes all 20 subtests, including the actual inherited Symbol.toStringTag property.
This is a selected WPT shell run, **not**
the complete WPT suite or `wpt run` browser integration. The shell runner uses
the original testharness callbacks and fails on missing completion/results.
It extracts classic inline/external HTML scripts or JS META dependencies;
there is no DOM, URL variant support, module loading or browser scheduler.
The runner now evaluates each script independently in a shared Realm, including
separate compilation, declaration instantiation and microtask checkpoints. The
twelve-file result was rerun successfully with these boundaries intact. This
does not supply browser scheduling, navigation or HTML realm/agent integration.

The probe was then expanded to the three Promise-incumbent-global `.sub.html`
files in the same directory (including the two support-frame documents). Those
still fail: the top-level file registers onload but receives no browser load or
message events, so its completion callback is absent; the frame documents need document.
They require an HTML/window/frame host and incumbent-realm tracking, not just
Promise language semantics. The twelve passing files are therefore still a
selected shell-compatible excerpt, not evidence of full WPT acceptance.

The WPT runner now enables queueMicrotask and additionally passes all five
subtests of `html/webappapis/microtask-queuing/queue-microtask.any.js`, for a
combined selected run of 13 files / 112 passing subtests. This is a shell run,
not both window and worker variants of that any-test. The other three microtask
files remain failed: queue-microtask-exceptions requires addEventListener and
`ErrorEvent` dispatch; queue-microtask.window requires MutationObserver/document; the
cross-realm callback-reporting file requires window/frames and callback realms.
These failures are not masked or replaced by local regression fixtures.

Standalone event support additionally passes the unchanged DOM files
EventTarget-add-remove-listener.any.js (1), EventTarget-addEventListener.any.js
(1), AddEventListenerOptions-once.any.js (4), Event-constructors.any.js (14) and
Event-isTrusted.any.js (1). With call spread, EventTarget-constructible.any.js
also passes all three subtests, including subclass helper methods. The combined
selected run now passes 19 files / 135 subtests.
With JSON support, AddEventListenerOptions-passive.any.js passes all five subtests.
The combined selected run is now 20 files / 141 subtests.
Abort support additionally passes AddEventListenerOptions-signal.any.js (11) and
dom/abort/event.any.js (16). The combined selection is 22 files / 167 subtests.
The 2026-09-09 search-method rerun confirmed every file. Summing the individual
result counts corrected an earlier documentation error (168 instead of 167);
no test was removed or newly failed.
With the optional timer host, AbortSignal.any.js (2), abort-signal-any.any.js (14)
and timeout.any.js (3) also pass completely: the selection is now 25 files / 186
subtests. The runner waits on real monotonic time, runs one task per checkpoint,
and still requires actual completion/results, with a 30-second timer wall limit.
The nine JavaScript files in html/webappapis/timers add twelve subtests (34 files /
198 subtests combined). Files were retrieved at the same pinned revision and
verified against Git blob SHA-1s from the directory listing. Four additional HTML
timer files remain failures due to absent Window/DOM/performance/cross-realm APIs.
The runner's start callback observes the original harness properties and does not
call done automatically for `single_test` or `explicit_done` tests. Transport tests
reject missing or late failed completion, rather than completing asynchronous
single-file tests before their timer runs. Timer task nesting is cleared before
the microtask checkpoint; timers scheduled by Promise jobs or queueMicrotask
do not inherit the preceding task's nesting clamp.
The old Date dependency was the harness's fallback when setTimeout was absent;
the actual timer host needs no Date shim. Date itself remains unimplemented.
The reason-constructor
requires iframe realms. EventTarget-removeEventListener still needs global listeners.
No file with failed or unexecuted subtests is counted as a pass.

The complete Test262 checkout at `docs/test-ext/test262`, revision
`419d3e0a2273ba01a3bfcbec423f2801425b8e93`, is an additional acceptance requirement.
Its INTERPRETING.md requires isolated realms, strict/non-strict variants, ordered
original harness includes, negative phase/type checking, async completion,
module resolution and the `$262` host API. `jrs --test262 ROOT --all --summary`
now inventories the complete checkout, including staging and Intl, and executes
supported Script variants through original cached harness code in fresh realms.
Selections use `test/...` paths; `_FIXTURE` files are not standalone tests.
The command returns failure if any variant fails or is unsupported. Metadata is
parsed by a bounded Test262-specific parser, not an external YAML dependency.
Unknown execution metadata fails closed; source bytes and line endings stay intact
except the prescribed strict directive. CR metadata and raw modules are recognized.

`compile_script` returns an immutable global `Script`; `evaluate_compiled` keeps
compile errors distinct from declaration-instantiation/runtime errors. Exact
matching compile/realm limits prevent cross-limit execution. Test print applies
`ToString` to only its first argument; async tests require exactly one completion
marker after the job checkpoint, and failure markers always fail. `$262.global`
and `$262.gc` are real capabilities. `$262.evalScript` now evaluates an independent
global Script in the same realm, preserving global lexical/var bindings without
accessing the caller's local environment or inheriting caller strictness.
Its dedicated Script frame leaves outer operands, locals and pending completions
GC-visible; errors unwind to the native boundary exactly once and Promise jobs
wait for the outer host-turn checkpoint. Compilation work consumes shared fuel;
source/instruction/frame/binding limits and native reentry (12) bound recursive
use. `Realm::eval_script_function()` exposes the same optional capability to
embedders. It is not the ECMAScript direct/indirect eval builtin, which is still
absent. Non-scalar UTF-16 Script source is explicitly unsupported by the current
UTF-8 compiler instead of silently replacing lone surrogates.
createRealm/detachArrayBuffer,
`AbstractModuleSource` and agents are explicit fatal unsupported host callbacks;
modules are counted as unsupported. No browser events are installed for Test262.
These host gaps mean this is still a diagnostic runner, not complete Test262 support.
Parse-negative tests pass only for an explicitly classified `SyntaxError`.
Recognized unavailable features and unverified parser rejections remain
unsupported. Runtime-negative errors must
occur at runtime with the expected constructor name; harness/resource failures
cannot satisfy them. Regex implementation restrictions now use fatal
`Error::Unsupported`, not catchable `SyntaxError`, so they cannot produce false
negative-test passes. Other parser/builtin completeness gaps remain open.

### Current Test262 result

The latest measurements were run on 2026-09-14, 2026-09-15, 2026-09-16 and
2026-09-17 against Test262
revision
`419d3e0a2273ba01a3bfcbec423f2801425b8e93`; the rows carry the implementation
commit each one measured, and the compound-assignment rows and the full runs
beside them are of the second day. The checkout was obtained with
`sh tools/xtask.sh test-ext` and was clean at that pinned revision, which
`sh tools/xtask.sh test-ext --status` reported back. Every run used the
original Test262 harness, a fresh realm per test, no expected-failure masks
and no feature exclusions, and a limit of 1,000,000 fuel units per realm.
Resource exhaustion, harness failure, an unexecuted test and `unsupported`
are not passes, so all commands correctly returned failure. Host: macOS on
aarch64 with the pinned nightly-2026-08-25 toolchain, release profile.

| Scope | Run | Implementation commit | Command | Files | Variants | Passed | Failed | Unsupported |
|---|---|---|---|---:|---:|---:|---:|---:|
| Throw statements (focused) | focused | `e5402166a49fcab99d5075ef2c076e3a49572d46` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/throw --summary` | 14 | 28 | 28 (100.00%) | 0 (0.00%) | 0 (0.00%) |
| Try statements (focused) | focused | `e5402166a49fcab99d5075ef2c076e3a49572d46` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/try --summary` | 201 | 388 | 334 (86.08%) | 5 (1.29%) | 49 (12.63%) |
| Switch statements (focused) | focused | `e5402166a49fcab99d5075ef2c076e3a49572d46` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/switch --summary` | 111 | 216 | 135 (62.50%) | 11 (5.09%) | 70 (32.41%) |
| For-in and for-of statements (focused) | focused | `401c4c1d875c9e5a2107510217b2ea4994ce4088` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/for-of test/language/statements/for-in --summary` | 870 | 1,648 | 1,230 (74.64%) | 86 (5.22%) | 332 (20.15%) |
| Array search methods (focused) | focused | `449f79faf5c7336b6014249523d00b6c817c6609` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/at test/built-ins/Array/prototype/includes test/built-ins/Array/prototype/indexOf test/built-ins/Array/prototype/lastIndexOf --summary` | 442 | 882 | 850 (96.37%) | 32 (3.63%) | 0 (0.00%) |
| Array join (focused) | focused | `7d4fe9ec875244e04043c08414f9878469504051` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/join --summary` | 23 | 46 | 40 (86.96%) | 6 (13.04%) | 0 (0.00%) |
| Array push and pop (focused) | focused | `ab8aae8eb7e1b2368dabe5a49d79f3a9e402a395` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/push test/built-ins/Array/prototype/pop --summary` | 47 | 94 | 94 (100.00%) | 0 (0.00%) | 0 (0.00%) |
| Array reverse (focused) | focused | `4b859f4feca764020ca4bd385f168e4e213dd767` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/reverse --summary` | 18 | 36 | 32 (88.89%) | 4 (11.11%) | 0 (0.00%) |
| Array slice (focused) | focused | `42d23a2033eb26f2dc996747e92adb157c6ef1d1` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/slice --summary` | 71 | 142 | 126 (88.73%) | 12 (8.45%) | 4 (2.82%) |
| Object.prototype methods (focused) | focused | `e5402166a49fcab99d5075ef2c076e3a49572d46` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/prototype --summary` | 248 | 494 | 284 (57.49%) | 202 (40.89%) | 8 (1.62%) |
| Property accessors (focused) | focused | `e5402166a49fcab99d5075ef2c076e3a49572d46` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/property-accessors --summary` | 21 | 42 | 32 (76.19%) | 10 (23.81%) | 0 (0.00%) |
| String.prototype methods (focused) | focused | `e5402166a49fcab99d5075ef2c076e3a49572d46` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 1,708 (79.66%) | 382 (17.82%) | 54 (2.52%) |
| Functions and `this` (focused) | focused | `4fd3513` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/this test/language/statements/function --summary` | 457 | 794 | 688 (86.65%) | 4 (0.50%) | 102 (12.85%) |
| Functions and `this` on the register engine (focused) | focused | `4fd3513` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/this test/language/statements/function --summary` | 457 | 794 | 189 (23.80%) | 24 (3.02%) | 581 (73.17%) |
| Loop statements (focused) | focused | `24d6a88` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/for test/language/statements/while test/language/statements/do-while --summary` | 459 | 900 | 713 (79.22%) | 11 (1.22%) | 176 (19.56%) |
| Loop statements on the register engine (focused) | focused | `24d6a88` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/for test/language/statements/while test/language/statements/do-while --summary` | 459 | 900 | 107 (11.89%) | 619 (68.78%) | 174 (19.33%) |
| Property reads over the Prototype Chain (focused) | focused | `a240620` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/property-accessors test/built-ins/Object/prototype test/built-ins/Array/prototype --summary` | 3,080 | 6,119 | 5,060 (82.69%) | 1,021 (16.69%) | 38 (0.62%) |
| Property reads over the Prototype Chain on the register engine (focused) | focused | `a240620` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/property-accessors test/built-ins/Object/prototype test/built-ins/Array/prototype --summary` | 3,080 | 6,119 | 2 (0.03%) | 6,099 (99.67%) | 18 (0.29%) |
| Property reads, writes and `Function.prototype` (focused) | focused | `30b9dc3` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/property-accessors test/language/expressions/assignment test/built-ins/Function/prototype --summary` | 815 | 1,494 | 1,077 (72.09%) | 73 (4.89%) | 344 (23.03%) |
| Property reads, writes and `Function.prototype` on the register engine (focused) | focused | `30b9dc3` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/property-accessors test/language/expressions/assignment test/built-ins/Function/prototype --summary` | 815 | 1,494 | 60 (4.02%) | 1,100 (73.63%) | 334 (22.36%) |
| `instanceof`, `new` and `throw` (focused) | focused | `cae1518` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/instanceof test/language/expressions/new test/language/statements/throw --summary` | 116 | 231 | 187 (80.95%) | 0 (0.00%) | 44 (19.05%) |
| `instanceof`, `new` and `throw` on the register engine (focused) | focused | `cae1518` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/instanceof test/language/expressions/new test/language/statements/throw --summary` | 116 | 231 | 0 (0.00%) | 189 (81.82%) | 42 (18.18%) |
| `delete` (focused) | focused | `0989fc9` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/delete --summary` | 69 | 103 | 91 (88.35%) | 10 (9.71%) | 2 (1.94%) |
| `delete` on the register engine (focused) | focused | `0989fc9` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/delete --summary` | 69 | 103 | 31 (30.10%) | 10 (9.71%) | 62 (60.19%) |
| Assignment and property accessors (focused) | focused | `7047dcf` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/assignment test/language/expressions/property-accessors --summary` | 506 | 892 | 570 (63.90%) | 28 (3.14%) | 294 (32.96%) |
| Assignment and property accessors on the register engine (focused) | focused | `7047dcf` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/assignment test/language/expressions/property-accessors --summary` | 506 | 892 | 86 (9.64%) | 65 (7.29%) | 741 (83.07%) |
| `if` and object literals (focused) | focused | `6438218` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/if test/language/expressions/object --summary` | 1,239 | 2,377 | 793 (33.36%) | 58 (2.44%) | 1,526 (64.20%) |
| `if` and object literals on the register engine (focused) | focused | `6438218` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/if test/language/expressions/object --summary` | 1,239 | 2,377 | 200 (8.41%) | 115 (4.84%) | 2,062 (86.75%) |
| `new` and function declarations (focused) | focused | `1ed956e` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/new test/language/statements/function --summary` | 510 | 901 | 751 (83.35%) | 4 (0.44%) | 146 (16.20%) |
| `new` and function declarations on the register engine (focused) | focused | `1ed956e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/new test/language/statements/function --summary` | 510 | 901 | 202 (22.42%) | 26 (2.89%) | 673 (74.69%) |
| `%Array%` and its methods (focused) | focused | `877447f` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array --summary` | 3,082 | 6,117 | 5,066 (82.82%) | 969 (15.84%) | 82 (1.34%) |
| `%Array%` and its methods on the register engine (focused) | focused | `877447f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array --summary` | 3,082 | 6,117 | 338 (5.52%) | 642 (10.50%) | 5,137 (83.98%) |
| `%Object%` and its methods (focused) | focused | `399ba5a` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object --summary` | 3,411 | 6,802 | 5,916 (86.98%) | 862 (12.67%) | 24 (0.35%) |
| `%Object%` and its methods on the register engine (focused) | focused | `399ba5a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object --summary` | 3,411 | 6,802 | 286 (4.20%) | 1,838 (27.02%) | 4,678 (68.77%) |
| Compound assignment (focused) | focused | `69b8a6d` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/compound-assignment --summary` | 454 | 786 | 591 (75.19%) | 55 (7.00%) | 140 (17.81%) |
| Compound assignment on the register engine (focused) | focused | `69b8a6d` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/compound-assignment --summary` | 454 | 786 | 167 (21.25%) | 0 (0.00%) | 619 (78.75%) |
| `let` and `const` statements (focused) | focused | `c14dd24` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/let test/language/statements/const --summary` | 281 | 558 | 479 (85.84%) | 0 (0.00%) | 79 (14.16%) |
| `let` and `const` statements on the register engine (focused) | focused | `c14dd24` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/let test/language/statements/const --summary` | 281 | 558 | 105 (18.82%) | 28 (5.02%) | 425 (76.16%) |
| `%Function%` and its methods (focused) | focused | `1516e27` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Function --summary` | 509 | 893 | 753 (84.32%) | 68 (7.61%) | 72 (8.06%) |
| `%Function%` and its methods on the register engine (focused) | focused | `1516e27` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Function --summary` | 509 | 893 | 61 (6.83%) | 226 (25.31%) | 606 (67.86%) |
| `%Math%` (focused, outdated) | focused | `65001ed` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Math --summary` | 327 | 654 | 306 (46.79%) | 344 (52.60%) | 4 (0.61%) |
| `%Math%` on the register engine (focused, outdated) | focused | `65001ed` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Math --summary` | 327 | 654 | 2 (0.31%) | 10 (1.53%) | 642 (98.17%) |
| The error constructors (focused) | focused | `a36aa89` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Error test/built-ins/NativeErrors --summary` | 187 | 374 | 226 (60.43%) | 128 (34.22%) | 20 (5.35%) |
| The error constructors on the register engine (focused) | focused | `a36aa89` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Error test/built-ins/NativeErrors --summary` | 187 | 374 | 54 (14.44%) | 90 (24.06%) | 230 (61.50%) |
| `%String%` and its methods (focused) | focused | `86ba2e2` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String --summary` | 1,223 | 2,443 | 1,928 (78.92%) | 448 (18.34%) | 67 (2.74%) |
| `%String%` and its methods on the register engine (focused) | focused | `86ba2e2` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String --summary` | 1,223 | 2,443 | 566 (23.17%) | 8 (0.33%) | 1,869 (76.50%) |
| `%Object%` and its methods, after 20.1.2 (focused) | focused | `f123a15` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object --summary` | 3,411 | 6,802 | 5,916 (86.97%) | 862 (12.67%) | 24 (0.35%) |
| `%Object%` and its methods, after 20.1.2, on the register engine (focused) | focused | `f123a15` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object --summary` | 3,411 | 6,802 | 1,142 (16.79%) | 18 (0.26%) | 5,642 (82.95%) |
| `%Array%` and its methods, after 23.1.3 (focused) | focused | `14a8333` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array --summary` | 3,082 | 6,117 | 5,066 (82.82%) | 969 (15.84%) | 82 (1.34%) |
| `%Array%` and its methods, after 23.1.3, on the register engine (focused) | focused | `14a8333` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array --summary` | 3,082 | 6,117 | 750 (12.26%) | 358 (5.85%) | 5,009 (81.89%) |
| `%Math%`, after 21.3 (focused) | focused | `6fe4b67` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Math --summary` | 327 | 654 | 306 (46.79%) | 344 (52.60%) | 4 (0.61%) |
| `%Math%`, after 21.3, on the register engine (focused) | focused | `6fe4b67` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Math --summary` | 327 | 654 | 118 (18.04%) | 0 (0.00%) | 536 (81.96%) |
| `%Array%` and its methods, after 23.1.3 complete but `sort` (focused) | focused | `7596776` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array --summary` | 3,082 | 6,117 | 5,066 (82.82%) | 969 (15.84%) | 82 (1.34%) |
| `%Array%` and its methods, after 23.1.3 complete but `sort`, on the register engine (focused) | focused | `7596776` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array --summary` | 3,082 | 6,117 | 1,922 (31.42%) | 350 (5.72%) | 3,845 (62.86%) |
| `for`-`of` statements (focused) | focused | `f1314b2` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/for-of --summary` | 751 | 1,442 | 1,062 (73.65%) | 79 (5.48%) | 301 (20.87%) |
| `for`-`of` statements on the register engine (focused) | focused | `f1314b2` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/for-of --summary` | 751 | 1,442 | 101 (7.00%) | 21 (1.46%) | 1,320 (91.54%) |
| `%Object%` and its methods, after the integrity levels (focused) | focused | `16f268e` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object --summary` | 3,411 | 6,802 | 5,916 (86.97%) | 862 (12.67%) | 24 (0.35%) |
| `%Object%` and its methods, after the integrity levels, on the register engine (focused) | focused | `16f268e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object --summary` | 3,411 | 6,802 | 1,530 (22.49%) | 18 (0.26%) | 5,254 (77.24%) |
| `%Number%` (focused) | focused | `a011601` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Number --summary` | 340 | 680 | 572 (84.12%) | 102 (15.00%) | 6 (0.88%) |
| `%Number%` on the register engine (focused) | focused | `a011601` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Number --summary` | 340 | 680 | 226 (33.24%) | 2 (0.29%) | 452 (66.47%) |
| `%BigInt%` (focused) | focused | `9ba0eb1` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/BigInt --summary` | 77 | 154 | 88 (57.14%) | 14 (9.09%) | 52 (33.77%) |
| The same, on the stack backend, which has no value of the type (focused) | focused | `9ba0eb1` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/BigInt --summary` | 77 | 154 | 0 (0.00%) | 68 (44.16%) | 86 (55.84%) |
| `%SharedArrayBuffer%` and `%Atomics%` (focused) | focused | `29de1d6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Atomics test/built-ins/SharedArrayBuffer --summary` | 493 | 986 | 298 (30.22%) | 350 (35.50%) | 338 (34.28%) |
| The same, on the stack backend, which carries neither clause (focused) | focused | `29de1d6` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Atomics test/built-ins/SharedArrayBuffer --summary` | 493 | 986 | 0 (0.00%) | 744 (75.46%) | 242 (24.54%) |
| `%TypedArray%` and the twelve of table 71 (focused) | focused | `004e277` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/TypedArray test/built-ins/TypedArrayConstructors --summary` | 2,184 | 4,322 | 2,249 (52.04%) | 1,167 (27.00%) | 906 (20.96%) |
| The same, before the arguments a clause converts (outdated) | focused | `9c0bb62` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/TypedArray test/built-ins/TypedArrayConstructors --summary` | 2,184 | 4,322 | 2,133 (49.35%) | 1,111 (25.71%) | 1,078 (24.94%) |
| The same, before the Nursery a Safe Point leaves the natives (outdated) | focused | `141a2fd` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/TypedArray test/built-ins/TypedArrayConstructors --summary` | 2,184 | 4,322 | 1,900 (43.96%) | 1,344 (31.10%) | 1,078 (24.94%) |
| The same, before the methods of 23.2.3 that call back (outdated) | focused | `01288ab` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/TypedArray test/built-ins/TypedArrayConstructors --summary` | 2,184 | 4,322 | 1,590 (36.79%) | 1,224 (28.32%) | 1,508 (34.89%) |
| The same, before the methods of 23.2.3 (outdated) | focused | `4a15802` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/TypedArray test/built-ins/TypedArrayConstructors --summary` | 2,184 | 4,322 | 1,090 (25.22%) | 1,120 (25.91%) | 2,112 (48.87%) |
| The same, before the three list forms of 23.2.5.1 (outdated) | focused | `3eebb13` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/TypedArray test/built-ins/TypedArrayConstructors --summary` | 2,184 | 4,322 | 824 (19.07%) | 1,014 (23.46%) | 2,484 (57.47%) |
| The same, on the stack backend, which carries no clause 23.2 (focused) | focused | `3eebb13` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/TypedArray test/built-ins/TypedArrayConstructors --summary` | 2,184 | 4,322 | 0 (0.00%) | 4,262 (98.61%) | 60 (1.39%) |
| The same, before the three further rows of table 71 (outdated) | focused | `9d12e52` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/TypedArray test/built-ins/TypedArrayConstructors --summary` | 2,184 | 4,322 | 144 (3.33%) | 3,326 (76.96%) | 852 (19.71%) |
| Class elements (focused) | focused | `e3bab40` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/class/elements test/language/expressions/class/elements --summary` | 2,962 | 5,897 | 543 (9.21%) | 104 (1.76%) | 5,250 (89.03%) |
| The same, on the stack backend, which has no field of a class (focused) | focused | `e3bab40` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/class/elements test/language/expressions/class/elements --summary` | 2,962 | 5,897 | 350 (5.94%) | 132 (2.24%) | 5,415 (91.83%) |
| Object literals (focused) | focused | `fc9e271` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/object --summary` | 1,170 | 2,252 | 652 (28.95%) | 33 (1.47%) | 1,567 (69.58%) |
| Labelled statements, break and continue (focused) | focused | `425421f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/labeled test/language/statements/break test/language/statements/continue --summary` | 68 | 125 | 88 (70.40%) | 4 (3.20%) | 33 (26.40%) |
| The same, on the stack backend, which has no target for a label (focused) | focused | `425421f` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/labeled test/language/statements/break test/language/statements/continue --summary` | 68 | 125 | 57 (45.60%) | 7 (5.60%) | 61 (48.80%) |
| Logical assignment (focused) | focused | `6deae2d` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/logical-assignment --summary` | 78 | 132 | 84 (63.64%) | 0 (0.00%) | 48 (36.36%) |
| The same, before the strict write to a property with no setter (outdated) | focused | `d9ee5b0` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/logical-assignment --summary` | 78 | 132 | 81 (61.36%) | 3 (2.27%) | 48 (36.36%) |
| The same, on the stack backend, which has no write that happens on one path alone (focused) | focused | `6deae2d` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/logical-assignment --summary` | 78 | 132 | 18 (13.64%) | 0 (0.00%) | 114 (86.36%) |
| `%Iterator%` (focused) | focused | `bee51e3` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Iterator --summary` | 654 | 1,308 | 170 (13.00%) | 642 (49.08%) | 496 (37.92%) |
| The same, before the five helpers of 27.1.3.3 that answer at once (outdated) | focused | `e6fa517` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Iterator --summary` | 654 | 1,308 | 122 (9.33%) | 692 (52.91%) | 494 (37.77%) |
| The same, before the `toArray` and the `forEach` of 27.1.3.3 (outdated) | focused | `69e48f0` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Iterator --summary` | 654 | 1,308 | 102 (7.80%) | 712 (54.43%) | 494 (37.77%) |
| `%DataView%` (focused) | focused | `0f39a55` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/DataView --summary` | 561 | 1,122 | 676 (60.25%) | 184 (16.40%) | 262 (23.35%) |
| The same, on the stack backend, which carries no clause 25.3 (focused) | focused | `0f39a55` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/DataView --summary` | 561 | 1,122 | 0 (0.00%) | 1,062 (94.65%) | 60 (5.35%) |
| The same, before the host object of the conformance suite (outdated) | focused | `0a8e566` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/DataView --summary` | 561 | 1,122 | 422 (37.61%) | 276 (24.60%) | 424 (37.79%) |
| `%ArrayBuffer%` (focused) | focused | `4e18bd9` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/ArrayBuffer --summary` | 221 | 442 | 144 (32.58%) | 130 (29.41%) | 168 (38.01%) |
| The same, on the stack backend, which carries no clause 25.1 (focused) | focused | `4e18bd9` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/ArrayBuffer --summary` | 221 | 442 | 0 (0.00%) | 442 (100.00%) | 0 (0.00%) |
| `%Date%`, with the Annex B three (focused) | focused | `2aa83f5` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Date test/annexB/built-ins/Date --summary` | 618 | 1,236 | 822 (66.50%) | 54 (4.37%) | 360 (29.13%) |
| The same, before the texts of 21.4.4.41 (outdated) | focused | `f947e5c` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Date --summary` | 594 | 1,188 | 732 (61.62%) | 104 (8.75%) | 352 (29.63%) |
| The same, before the setters of 21.4.4 (outdated) | focused | `eab8154` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Date --summary` | 594 | 1,188 | 516 (43.43%) | 272 (22.90%) | 400 (33.67%) |
| The same, on the stack backend, which carries no clause 21.4 (focused) | focused | `eab8154` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Date --summary` | 594 | 1,188 | 0 (0.00%) | 1,188 (100.00%) | 0 (0.00%) |
| `escape` and `unescape` of B.2.1 (focused) | focused | `31aa8eb` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/annexB/built-ins/escape test/annexB/built-ins/unescape --summary` | 35 | 70 | 66 (94.29%) | 0 (0.00%) | 4 (5.71%) |
| The four URI functions of 19.2.6 (focused) | focused | `32d54c2` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/encodeURI test/built-ins/decodeURI test/built-ins/encodeURIComponent test/built-ins/decodeURIComponent --summary` | 173 | 346 | 310 (89.60%) | 6 (1.73%) | 30 (8.67%) |
| `%JSON%` (focused) | focused | `bd95535` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/JSON --summary` | 165 | 330 | 282 (85.45%) | 32 (9.70%) | 16 (4.85%) |
| `%JSON%` on the register engine (focused) | focused | `bd95535` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/JSON --summary` | 165 | 330 | 98 (29.70%) | 12 (3.64%) | 220 (66.67%) |
| `%Array.prototype%` element accessors (focused) | focused | `6948012` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,744 (84.97%) | 809 (14.49%) | 30 (0.54%) |
| `%Array.prototype%` element accessors on the register engine (focused) | focused | `6948012` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 3,181 (56.98%) | 657 (11.77%) | 1,745 (31.26%) |
| Destructuring declarations, after the rest element (focused) | focused | `9c9533a` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/variable/dstr --summary` | 97 | 194 | 158 (81.44%) | 0 (0.00%) | 36 (18.56%) |
| Destructuring declarations, after the rest element, on the register engine (focused) | focused | `9c9533a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/variable/dstr --summary` | 97 | 194 | 144 (74.23%) | 2 (1.03%) | 48 (24.74%) |
| Destructuring assignment, after the value with no layout (focused) | focused | `2dd0728` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/assignment/dstr --summary` | 368 | 640 | 446 (69.69%) | 0 (0.00%) | 194 (30.31%) |
| Destructuring assignment, after the value with no layout, on the register engine (focused) | focused | `2dd0728` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/assignment/dstr --summary` | 368 | 640 | 287 (44.84%) | 10 (1.56%) | 343 (53.59%) |
| `String.prototype.split` (focused) | focused | `594df30` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype/split --summary` | 120 | 240 | 236 (98.33%) | 0 (0.00%) | 4 (1.67%) |
| `String.prototype.split` on the register engine (focused) | focused | `594df30` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/prototype/split --summary` | 120 | 240 | 34 (14.17%) | 27 (11.25%) | 179 (74.58%) |
| `String.prototype.match` and `search` (focused) | focused | `86b09f8` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype/match test/built-ins/String/prototype/search --summary` | 94 | 188 | 172 (91.49%) | 0 (0.00%) | 16 (8.51%) |
| `String.prototype.match` and `search` on the register engine (focused) | focused | `86b09f8` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/prototype/match test/built-ins/String/prototype/search --summary` | 94 | 188 | 48 (25.53%) | 6 (3.19%) | 134 (71.28%) |
| `%String.prototype%`, after the argument conversions (focused) | focused | `3825637` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 1,708 (79.66%) | 382 (17.82%) | 54 (2.52%) |
| `%String.prototype%`, after the argument conversions, on the register engine (focused) | focused | `3825637` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 724 (33.77%) | 87 (4.06%) | 1,333 (62.17%) |
| `try` statements, after the catch pattern (focused) | focused | `bce813f` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/try --summary` | 201 | 388 | 334 (86.08%) | 5 (1.29%) | 49 (12.63%) |
| `try` statements, after the catch pattern, on the register engine (focused) | focused | `bce813f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/try --summary` | 201 | 388 | 187 (48.20%) | 3 (0.77%) | 198 (51.03%) |
| The global Number functions (focused) | focused | `91aba35` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/isNaN test/built-ins/isFinite test/built-ins/parseInt test/built-ins/parseFloat --summary` | 139 | 278 | 270 (97.12%) | 8 (2.88%) | 0 (0.00%) |
| The global Number functions on the register engine (focused) | focused | `91aba35` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/isNaN test/built-ins/isFinite test/built-ins/parseInt test/built-ins/parseFloat --summary` | 139 | 278 | 230 (82.73%) | 8 (2.88%) | 40 (14.39%) |
| Template literals (focused) | focused | `5cdf8b3` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/template-literal --summary` | 57 | 114 | 80 (70.18%) | 0 (0.00%) | 34 (29.82%) |
| Template literals on the register engine (focused) | focused | `5cdf8b3` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/template-literal --summary` | 57 | 114 | 78 (68.42%) | 0 (0.00%) | 36 (31.58%) |
| Assignments and declarations, after the owned name (focused) | focused | `fe340f5` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/assignment test/language/statements/variable --summary` | 663 | 1,159 | 802 (69.20%) | 18 (1.55%) | 339 (29.25%) |
| Assignments and declarations, after the owned name, on the register engine (focused) | focused | `fe340f5` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/assignment test/language/statements/variable --summary` | 663 | 1,159 | 583 (50.30%) | 20 (1.73%) | 556 (47.97%) |
| The arguments object, after the empty mapping (focused) | focused | `0f5cc0f` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/arguments-object --summary` | 263 | 460 | 188 (40.87%) | 2 (0.43%) | 270 (58.70%) |
| The arguments object, after the empty mapping, on the register engine (focused) | focused | `0f5cc0f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/arguments-object --summary` | 263 | 460 | 120 (26.09%) | 0 (0.00%) | 340 (73.91%) |
| `for`-`of` statements, after the assignment head (focused) | focused | `459d78f` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/for-of --summary` | 751 | 1,442 | 1,062 (73.65%) | 79 (5.48%) | 301 (20.87%) |
| `for`-`of` statements, after the assignment head, on the register engine (focused) | focused | `459d78f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/for-of --summary` | 751 | 1,442 | 686 (47.57%) | 41 (2.84%) | 715 (49.58%) |
| `%Number.prototype%`, after the primitive receiver (focused) | focused | `e97b8c6` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Number/prototype --summary` | 168 | 336 | 232 (69.05%) | 102 (30.36%) | 2 (0.60%) |
| `%Number.prototype%`, after the primitive receiver, on the register engine (focused) | focused | `e97b8c6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Number/prototype --summary` | 168 | 336 | 126 (37.50%) | 96 (28.57%) | 114 (33.93%) |
| `%Symbol%` (focused) | focused | `b19277f` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Symbol --summary` | 98 | 192 | 94 (48.96%) | 64 (33.33%) | 34 (17.71%) |
| `%Symbol%` on the register engine (focused) | focused | `b19277f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Symbol --summary` | 98 | 192 | 86 (44.79%) | 82 (42.71%) | 24 (12.50%) |
| `%String.prototype%`, after the receiver conversion (focused) | focused | `7b37b0e` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 1,708 (79.66%) | 382 (17.82%) | 54 (2.52%) |
| `%String.prototype%`, after the receiver conversion, on the register engine (focused) | focused | `7b37b0e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 1,084 (50.56%) | 106 (4.94%) | 954 (44.50%) |
| Computed property names (focused) | focused | `23fa2a0` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/computed-property-names --summary` | 48 | 96 | 80 (83.33%) | 6 (6.25%) | 10 (10.42%) |
| Computed property names on the register engine (focused) | focused | `23fa2a0` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/computed-property-names --summary` | 48 | 96 | 56 (58.33%) | 8 (8.33%) | 32 (33.33%) |
| `%Array.prototype%`, after the find clauses (focused) | focused | `b8b6da1` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,744 (84.97%) | 809 (14.49%) | 30 (0.54%) |
| `%Array.prototype%`, after the find clauses, on the register engine (focused) | focused | `b8b6da1` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 3,480 (62.33%) | 683 (12.23%) | 1,420 (25.43%) |
| `%Function.prototype%`, after `apply` (focused) | focused | `f2e46b9` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Function/prototype --summary` | 309 | 602 | 507 (84.22%) | 45 (7.48%) | 50 (8.31%) |
| `%Function.prototype%`, after `apply`, on the register engine (focused) | focused | `f2e46b9` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Function/prototype --summary` | 309 | 602 | 184 (30.56%) | 188 (31.23%) | 230 (38.21%) |
| `%Array.prototype%`, after the length getter (focused) | focused | `8898794` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,744 (84.97%) | 809 (14.49%) | 30 (0.54%) |
| `%Array.prototype%`, after the length getter, on the register engine (focused) | focused | `8898794` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 3,680 (65.91%) | 687 (12.31%) | 1,216 (21.78%) |
| The scan clauses of 23.1.3 (focused) | focused | `603d094` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/indexOf test/built-ins/Array/prototype/lastIndexOf test/built-ins/Array/prototype/includes --summary` | 429 | 856 | 828 (96.73%) | 28 (3.27%) | 0 (0.00%) |
| The scan clauses of 23.1.3 on the register engine (focused) | focused | `603d094` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype/indexOf test/built-ins/Array/prototype/lastIndexOf test/built-ins/Array/prototype/includes --summary` | 429 | 856 | 694 (81.07%) | 66 (7.71%) | 96 (11.21%) |
| The Array length descriptor (focused) | focused | `ea2f97c` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/length test/built-ins/Object/defineProperty --summary` | 1,161 | 2,310 | 2,240 (96.97%) | 66 (2.86%) | 4 (0.17%) |
| The Array length descriptor on the register engine (focused) | focused | `ea2f97c` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/length test/built-ins/Object/defineProperty --summary` | 1,161 | 2,310 | 1,955 (84.63%) | 120 (5.19%) | 235 (10.17%) |
| Global lexical declarations (focused) | focused | `dfe431b` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/const test/language/statements/let --summary` | 281 | 558 | 479 (85.84%) | 0 (0.00%) | 79 (14.16%) |
| Global lexical declarations on the register engine (focused) | focused | `dfe431b` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/const test/language/statements/let --summary` | 281 | 558 | 379 (67.92%) | 12 (2.15%) | 167 (29.93%) |
| `setPrototypeOf` and `%Reflect%` (focused) | focused | `cd6fe50` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/setPrototypeOf test/built-ins/Reflect --summary` | 165 | 330 | 276 (83.64%) | 52 (15.76%) | 2 (0.61%) |
| `setPrototypeOf` and `%Reflect%` on the register engine (focused) | focused | `cd6fe50` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/setPrototypeOf test/built-ins/Reflect --summary` | 165 | 330 | 196 (59.39%) | 6 (1.82%) | 128 (38.79%) |
| `Function.prototype.toString` (focused) | focused | `2107883` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Function/prototype/toString --summary` | 80 | 160 | 106 (66.25%) | 14 (8.75%) | 40 (25.00%) |
| `Function.prototype.toString` on the register engine (focused) | focused | `2107883` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Function/prototype/toString --summary` | 80 | 160 | 18 (11.25%) | 96 (60.00%) | 46 (28.75%) |
| Block scopes, after the Object binding (focused) | focused | `b93d713` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/block-scope --summary` | 145 | 287 | 169 (58.89%) | 26 (9.06%) | 92 (32.06%) |
| Block scopes, after the Object binding, on the register engine (focused) | focused | `b93d713` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/block-scope --summary` | 145 | 287 | 147 (51.22%) | 26 (9.06%) | 114 (39.72%) |
| Object patterns, after the rest element (focused) | focused | `22fdd9f` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/object/dstr test/language/statements/variable/dstr --summary` | 658 | 1,316 | 474 (36.02%) | 6 (0.46%) | 836 (63.53%) |
| Object patterns, after the rest element, on the register engine (focused) | focused | `22fdd9f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/object/dstr test/language/statements/variable/dstr --summary` | 658 | 1,316 | 456 (34.65%) | 2 (0.15%) | 858 (65.20%) |
| `for`-`of`, after the iterator close (focused) | focused | `f151cfc` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/for-of --summary` | 751 | 1,442 | 1,062 (73.65%) | 79 (5.48%) | 301 (20.87%) |
| `for`-`of`, after the iterator close, on the register engine (focused) | focused | `f151cfc` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/for-of --summary` | 751 | 1,442 | 742 (51.46%) | 45 (3.12%) | 655 (45.42%) |
| `%Object.prototype%`, after `valueOf` (focused) | focused | `c1b270e` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/prototype --summary` | 248 | 494 | 284 (57.49%) | 202 (40.89%) | 8 (1.62%) |
| `%Object.prototype%`, after `valueOf`, on the register engine (focused) | focused | `c1b270e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/prototype --summary` | 248 | 494 | 246 (49.80%) | 12 (2.43%) | 236 (47.77%) |
| `%Function.prototype%`, `instanceof` and `concat`, after the four answers (focused) | focused | `83ca8c2` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Function/prototype test/language/expressions/instanceof test/built-ins/Array/prototype/concat --summary` | 421 | 824 | 713 (86.53%) | 57 (6.92%) | 54 (6.55%) |
| The same three, after the four answers, on the register engine (focused) | focused | `83ca8c2` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Function/prototype test/language/expressions/instanceof test/built-ins/Array/prototype/concat --summary` | 421 | 824 | 293 (35.56%) | 240 (29.13%) | 291 (35.32%) |
| `%Array.prototype%`, after the String receiver (focused) | focused | `9e8eb35` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,744 (84.97%) | 809 (14.49%) | 30 (0.54%) |
| `%Array.prototype%`, after the String receiver, on the register engine (focused) | focused | `9e8eb35` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 3,974 (71.18%) | 586 (10.50%) | 1,023 (18.32%) |
| `%Error%`, after `Error.prototype.toString` (focused) | focused | `a21f844` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Error --summary` | 93 | 186 | 86 (46.24%) | 92 (49.46%) | 8 (4.30%) |
| `%Error%`, after `Error.prototype.toString`, on the register engine (focused) | focused | `a21f844` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Error --summary` | 93 | 186 | 74 (39.78%) | 86 (46.24%) | 26 (13.98%) |
| `%Reflect%`, after `construct` (focused) | focused | `0d60fa0` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Reflect --summary` | 153 | 306 | 260 (84.97%) | 46 (15.03%) | 0 (0.00%) |
| `%Reflect%`, after `construct`, on the register engine (focused) | focused | `0d60fa0` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Reflect --summary` | 153 | 306 | 214 (69.93%) | 8 (2.61%) | 84 (27.45%) |
| `sort`, `toSorted`, `toSpliced` and `flat` (focused) | focused | `58401ba` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/sort test/built-ins/Array/prototype/toSorted test/built-ins/Array/prototype/toSpliced test/built-ins/Array/prototype/flat --summary` | 124 | 247 | 233 (94.33%) | 10 (4.05%) | 4 (1.62%) |
| The same four, on the register engine (focused) | focused | `58401ba` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype/sort test/built-ins/Array/prototype/toSorted test/built-ins/Array/prototype/toSpliced test/built-ins/Array/prototype/flat --summary` | 124 | 247 | 114 (46.15%) | 22 (8.91%) | 111 (44.94%) |
| `%Array.prototype%`, after the converted `length` (focused) | focused | `41b7a16` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,744 (84.97%) | 809 (14.49%) | 30 (0.54%) |
| `%Array.prototype%`, after the converted `length`, on the register engine (focused) | focused | `41b7a16` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,358 (78.06%) | 610 (10.93%) | 615 (11.02%) |
| The three scans of 23.1.3, after the converted index (focused) | focused | `a97ff3b` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/indexOf test/built-ins/Array/prototype/lastIndexOf test/built-ins/Array/prototype/includes --summary` | 429 | 856 | 828 (96.73%) | 28 (3.27%) | 0 (0.00%) |
| The same three, on the register engine (focused) | focused | `a97ff3b` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype/indexOf test/built-ins/Array/prototype/lastIndexOf test/built-ins/Array/prototype/includes --summary` | 429 | 856 | 768 (89.72%) | 46 (5.37%) | 42 (4.91%) |
| `%RegExp%`, after the constructor (focused) | focused | `d684811` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/RegExp --summary` | 1,879 | 3,756 | 1,462 (38.92%) | 96 (2.56%) | 2,198 (58.52%) |
| `%RegExp%`, after the constructor, on the register engine (focused) | focused | `d684811` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/RegExp --summary` | 1,879 | 3,756 | 1,026 (27.32%) | 626 (16.67%) | 2,104 (56.02%) |
| `@@species` of `%Array%` and `%RegExp%` (focused) | focused | `4036ed2` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/Symbol.species test/built-ins/RegExp/Symbol.species --summary` | 8 | 16 | 16 (100.00%) | 0 (0.00%) | 0 (0.00%) |
| The same two, on the register engine (focused) | focused | `4036ed2` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/Symbol.species test/built-ins/RegExp/Symbol.species --summary` | 8 | 16 | 12 (75.00%) | 4 (25.00%) | 0 (0.00%) |
| `Function.prototype.bind`, after the bound arguments (focused) | focused | `dfbad4f` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Function/prototype/bind --summary` | 100 | 200 | 190 (95.00%) | 4 (2.00%) | 6 (3.00%) |
| The same, on the register engine (focused) | focused | `dfbad4f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Function/prototype/bind --summary` | 100 | 200 | 136 (68.00%) | 24 (12.00%) | 40 (20.00%) |
| `String.prototype.match` and `search`, after the made `RegExp` (focused) | focused | `248f5f0` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype/match test/built-ins/String/prototype/search --summary` | 94 | 188 | 172 (91.49%) | 0 (0.00%) | 16 (8.51%) |
| The same two, on the register engine (focused) | focused | `248f5f0` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/prototype/match test/built-ins/String/prototype/search --summary` | 94 | 188 | 130 (69.15%) | 8 (4.26%) | 50 (26.60%) |
| `copyWithin` and `slice`, after the intrinsic base (focused) | focused | `710ba34` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/copyWithin test/built-ins/Array/prototype/slice --summary` | 110 | 220 | 198 (90.00%) | 18 (8.18%) | 4 (1.82%) |
| The same two, on the register engine (focused) | focused | `710ba34` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype/copyWithin test/built-ins/Array/prototype/slice --summary` | 110 | 220 | 144 (65.45%) | 38 (17.27%) | 38 (17.27%) |
| `replace` and `@@replace` (focused) | focused | `7c92bf7` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype/replace test/built-ins/RegExp/prototype/Symbol.replace --summary` | 125 | 246 | 228 (92.68%) | 0 (0.00%) | 18 (7.32%) |
| The same two, on the register engine (focused) | focused | `7c92bf7` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/prototype/replace test/built-ins/RegExp/prototype/Symbol.replace --summary` | 125 | 246 | 94 (38.21%) | 18 (7.32%) | 134 (54.47%) |
| `join` and the property accessors, after `ToString` of an Object (focused) | focused | `88370c5` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/join test/language/expressions/property-accessors --summary` | 44 | 88 | 72 (81.82%) | 16 (18.18%) | 0 (0.00%) |
| The same two, on the register engine (focused) | focused | `88370c5` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype/join test/language/expressions/property-accessors --summary` | 44 | 88 | 54 (61.36%) | 6 (6.82%) | 28 (31.82%) |
| `%Array.prototype%`, after the answered length (focused) | focused | `64f7a69` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,744 (84.97%) | 809 (14.49%) | 30 (0.54%) |
| `%Array.prototype%`, after the answered length, on the register engine (focused) | focused | `64f7a69` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,509 (80.76%) | 600 (10.75%) | 474 (8.49%) |
| `@@unscopables` of `%Array.prototype%` (focused) | focused | `eaaca5d` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/Symbol.unscopables --summary` | 5 | 10 | 10 (100.00%) | 0 (0.00%) | 0 (0.00%) |
| The same, on the register engine (focused) | focused | `eaaca5d` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype/Symbol.unscopables --summary` | 5 | 10 | 8 (80.00%) | 2 (20.00%) | 0 (0.00%) |
| `Array.from` and `Array.of` (focused) | focused | `7eb647c` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/from test/built-ins/Array/of --summary` | 63 | 122 | 110 (90.16%) | 6 (4.92%) | 6 (4.92%) |
| The same two, on the register engine (focused) | focused | `7eb647c` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/from test/built-ins/Array/of --summary` | 63 | 122 | 40 (32.79%) | 22 (18.03%) | 60 (49.18%) |
| `Array.from` with a mapper (focused) | focused | `92ee3db` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/from test/built-ins/Array/of --summary` | 63 | 122 | 110 (90.16%) | 6 (4.92%) | 6 (4.92%) |
| The same two, with the mapper, on the register engine (focused) | focused | `92ee3db` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/from test/built-ins/Array/of --summary` | 63 | 122 | 40 (32.79%) | 24 (19.67%) | 58 (47.54%) |
| The five copying clauses, after 23.1.3.4 (focused) | focused | `b84ccb9` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/map test/built-ins/Array/prototype/filter test/built-ins/Array/prototype/slice test/built-ins/Array/prototype/splice test/built-ins/Array/prototype/concat --summary` | 679 | 1,350 | 1,262 (93.48%) | 64 (4.74%) | 24 (1.78%) |
| The same five, on the register engine (focused) | focused | `b84ccb9` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype/map test/built-ins/Array/prototype/filter test/built-ins/Array/prototype/slice test/built-ins/Array/prototype/splice test/built-ins/Array/prototype/concat --summary` | 679 | 1,350 | 1,034 (76.59%) | 141 (10.44%) | 175 (12.96%) |
| `%Function%`, after the dynamic body (focused) | focused | `7e8a7bb` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Function --summary` | 509 | 893 | 753 (84.32%) | 68 (7.61%) | 72 (8.06%) |
| `%Function%`, after the dynamic body, on the register engine (focused) | focused | `7e8a7bb` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Function --summary` | 509 | 893 | 542 (60.69%) | 209 (23.40%) | 142 (15.90%) |
| `eval` code (focused) | focused | `7cd7ebb` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/eval-code test/annexB/language/eval-code --summary` | 816 | 924 | 273 (29.55%) | 542 (58.66%) | 109 (11.80%) |
| `eval` code, on the register engine (focused) | focused | `7cd7ebb` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/eval-code test/annexB/language/eval-code --summary` | 816 | 924 | 85 (9.20%) | 369 (39.94%) | 470 (50.87%) |
| `Object.prototype.toString` and `%Symbol%` (focused) | focused | `174f264` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/prototype/toString test/built-ins/Symbol --summary` | 139 | 274 | 142 (51.82%) | 90 (32.85%) | 42 (15.33%) |
| The same two, on the register engine (focused) | focused | `174f264` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/prototype/toString test/built-ins/Symbol --summary` | 139 | 274 | 150 (54.74%) | 64 (23.36%) | 60 (21.90%) |
| `preventExtensions`, `seal` and `freeze` (focused) | focused | `03d1292` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/preventExtensions test/built-ins/Object/seal test/built-ins/Object/freeze --summary` | 187 | 368 | 278 (75.54%) | 86 (23.37%) | 4 (1.09%) |
| The same three, on the register engine (focused) | focused | `03d1292` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/preventExtensions test/built-ins/Object/seal test/built-ins/Object/freeze --summary` | 187 | 368 | 270 (73.37%) | 6 (1.63%) | 92 (25.00%) |
| `bind`, the restricted properties and the two wrapper prototypes (focused) | focused | `3f8b9e8` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Function/prototype/bind test/built-ins/Function/prototype/caller-arguments test/built-ins/Number/prototype test/built-ins/Boolean/prototype --summary` | 295 | 589 | 471 (79.97%) | 110 (18.68%) | 8 (1.36%) |
| The same four, on the register engine (focused) | focused | `3f8b9e8` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Function/prototype/bind test/built-ins/Function/prototype/caller-arguments test/built-ins/Number/prototype test/built-ins/Boolean/prototype --summary` | 295 | 589 | 415 (70.46%) | 20 (3.40%) | 154 (26.15%) |
| `%RegExp.prototype%` (focused) | focused | `81433dd` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/RegExp/prototype --summary` | 487 | 972 | 770 (79.22%) | 52 (5.35%) | 150 (15.43%) |
| The same, on the register engine (focused) | focused | `d603a40` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/RegExp/prototype --summary` | 487 | 972 | 526 (54.12%) | 66 (6.79%) | 380 (39.09%) |
| The same, before the Symbol-keyed methods of 22.2.6 (outdated) | focused | `81433dd` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/RegExp/prototype --summary` | 487 | 972 | 446 (45.88%) | 292 (30.04%) | 234 (24.07%) |
| `getOwnPropertyDescriptor` and `%Function.prototype%` (focused) | focused | `c9b1598` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/getOwnPropertyDescriptor test/built-ins/Function/prototype --summary` | 619 | 1,222 | 961 (78.64%) | 211 (17.27%) | 50 (4.09%) |
| The same two, on the register engine (focused) | focused | `c9b1598` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/getOwnPropertyDescriptor test/built-ins/Function/prototype --summary` | 619 | 1,222 | 818 (66.94%) | 140 (11.46%) | 264 (21.60%) |
| `for`-`of` statements (focused) | focused | `f9d7537` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/for-of --summary` | 751 | 1,442 | 1,062 (73.65%) | 79 (5.48%) | 301 (20.87%) |
| The same, on the register engine (focused) | focused | `f9d7537` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/for-of --summary` | 751 | 1,442 | 891 (61.79%) | 73 (5.06%) | 478 (33.15%) |
| The clauses that convert a position (focused) | focused | `ded2cac` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype/copyWithin test/built-ins/Array/prototype/fill test/built-ins/Array/prototype/splice test/built-ins/String/prototype/split --summary` | 262 | 524 | 498 (95.04%) | 18 (3.44%) | 8 (1.53%) |
| The same four, on the register engine (focused) | focused | `ded2cac` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype/copyWithin test/built-ins/Array/prototype/fill test/built-ins/Array/prototype/splice test/built-ins/String/prototype/split --summary` | 262 | 524 | 396 (75.57%) | 50 (9.54%) | 78 (14.89%) |
| `super` and the method definitions of 13.2.5 (focused) | focused | `80c7268` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/super test/language/expressions/object/method-definition --summary` | 397 | 741 | 262 (35.36%) | 27 (3.64%) | 452 (60.99%) |
| The same two, on the register engine (focused) | focused | `80c7268` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/super test/language/expressions/object/method-definition --summary` | 397 | 741 | 108 (14.57%) | 9 (1.21%) | 624 (84.21%) |
| `new.target` (focused) | focused | `9e0a0fb` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/new.target --summary` | 14 | 28 | 22 (78.57%) | 0 (0.00%) | 6 (21.43%) |
| The same, on the register engine (focused) | focused | `9e0a0fb` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/new.target --summary` | 14 | 28 | 16 (57.14%) | 0 (0.00%) | 12 (42.86%) |
| Subclasses and `super` (focused) | focused | `756642a` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/class/subclass test/language/statements/class/super --summary` | 117 | 233 | 182 (78.11%) | 36 (15.45%) | 15 (6.44%) |
| The same two, on the register engine (focused) | focused | `756642a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/class/subclass test/language/statements/class/super --summary` | 117 | 233 | 74 (31.76%) | 14 (6.01%) | 145 (62.23%) |
| Subclasses of a class and of a builtin (focused) | focused | `3d10524` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/class/subclass test/language/expressions/class/subclass-builtins --summary` | 145 | 289 | 198 (68.51%) | 76 (26.30%) | 15 (5.19%) |
| The same two, on the register engine (focused) | focused | `3d10524` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/class/subclass test/language/expressions/class/subclass-builtins --summary` | 145 | 289 | 146 (50.52%) | 14 (4.84%) | 129 (44.64%) |
| `%String.prototype%` (focused) | focused | `6952e99` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 1,708 (79.66%) | 382 (17.82%) | 54 (2.52%) |
| The same, on the register engine (focused) | focused | `6952e99` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 1,512 (70.52%) | 38 (1.77%) | 594 (27.71%) |
| `%String.prototype%`, before the four of 22.1.3 and B.2.2 (focused, outdated) | focused | `c1c4f04` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 1,708 (79.66%) | 382 (17.82%) | 54 (2.52%) |
| The same, on the register engine (focused, outdated) | focused | `c1c4f04` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/prototype --summary` | 1,073 | 2,144 | 1,456 (67.91%) | 38 (1.77%) | 650 (30.32%) |
| The integrity levels of 20.1.2 (focused) | focused | `a1f9e31` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/freeze test/built-ins/Object/seal test/built-ins/Object/isFrozen test/built-ins/Object/isSealed --summary` | 239 | 474 | 376 (79.32%) | 94 (19.83%) | 4 (0.84%) |
| The same four, on the register engine (focused) | focused | `a1f9e31` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/freeze test/built-ins/Object/seal test/built-ins/Object/isFrozen test/built-ins/Object/isSealed --summary` | 239 | 474 | 378 (79.75%) | 6 (1.27%) | 90 (18.99%) |
| `defineProperty` of 20.1.2.4 and 28.1.3 (focused, outdated) | focused | `a3e49ee` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/defineProperty test/built-ins/Reflect/defineProperty --summary` | 1,143 | 2,274 | 2,204 (96.92%) | 68 (2.99%) | 2 (0.09%) |
| The same two, on the register engine (focused, outdated) | focused | `a3e49ee` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/defineProperty test/built-ins/Reflect/defineProperty --summary` | 1,143 | 2,274 | 2,077 (91.34%) | 74 (3.25%) | 123 (5.41%) |
| `Object.create` and `defineProperties` (focused) | focused | `9b84330` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/create test/built-ins/Object/defineProperties --summary` | 952 | 1,904 | 1,840 (96.64%) | 60 (3.15%) | 4 (0.21%) |
| The same two, on the register engine (focused) | focused | `9b84330` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/create test/built-ins/Object/defineProperties --summary` | 952 | 1,904 | 1,737 (91.23%) | 46 (2.42%) | 121 (6.35%) |
| The three functions of 22.1.2 (focused) | focused | `bcc2ce5` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/String/raw test/built-ins/String/fromCharCode test/built-ins/String/fromCodePoint --summary` | 58 | 116 | 40 (34.48%) | 66 (56.90%) | 10 (8.62%) |
| The same three, on the register engine (focused) | focused | `bcc2ce5` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/String/raw test/built-ins/String/fromCharCode test/built-ins/String/fromCodePoint --summary` | 58 | 116 | 76 (65.52%) | 0 (0.00%) | 40 (34.48%) |
| `%Array.prototype%` (focused) | focused | `1702923` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,744 (84.97%) | 809 (14.49%) | 30 (0.54%) |
| The same, on the register engine (focused) | focused | `e8c1e53` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,737 (84.84%) | 340 (6.09%) | 506 (9.06%) |
| The same, before 23.1.3.13 (outdated) | focused | `1702923` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,712 (84.40%) | 340 (6.09%) | 531 (9.51%) |
| The same, before 23.1.3.1.1 (outdated) | focused | `04f3eb6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,666 (83.58%) | 401 (7.18%) | 516 (9.24%) |
| `defineProperty` and `defineProperties` (focused) | focused | `ea3ad4a` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object/defineProperty test/built-ins/Object/defineProperties --summary` | 1,763 | 3,514 | 3,414 (97.15%) | 96 (2.73%) | 4 (0.11%) |
| The same two, on the register engine (focused) | focused | `ea3ad4a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/defineProperty test/built-ins/Object/defineProperties --summary` | 1,763 | 3,514 | 3,275 (93.20%) | 25 (0.71%) | 214 (6.09%) |
| `%Promise%` (focused) | focused | `3a014d6` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Promise --summary` | 732 | 1,458 | 846 (58.02%) | 598 (41.01%) | 14 (0.96%) |
| The same, on the register engine (focused) | focused | `3a014d6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Promise --summary` | 732 | 1,458 | 532 (36.49%) | 170 (11.66%) | 756 (51.85%) |
| The same, before the combinators of 27.2.4 (outdated) | focused | `deca8e1` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Promise --summary` | 732 | 1,458 | 308 (21.12%) | 164 (11.25%) | 986 (67.63%) |
| The update operators of 13.4 (focused) | focused | `90f4774` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/postfix-increment test/language/expressions/postfix-decrement test/language/expressions/prefix-increment test/language/expressions/prefix-decrement --summary` | 142 | 246 | 182 (73.98%) | 16 (6.50%) | 48 (19.51%) |
| The same four, on the register engine (focused) | focused | `90f4774` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/postfix-increment test/language/expressions/postfix-decrement test/language/expressions/prefix-increment test/language/expressions/prefix-decrement --summary` | 142 | 246 | 170 (69.11%) | 0 (0.00%) | 76 (30.89%) |
| `%Math%` (focused) | focused | `3b287c4` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Math --summary` | 327 | 654 | 306 (46.79%) | 344 (52.60%) | 4 (0.61%) |
| The same, on the register engine (focused) | focused | `3b287c4` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Math --summary` | 327 | 654 | 302 (46.18%) | 4 (0.61%) | 348 (53.21%) |
| Switch statements (focused) | focused | `4ecac3b` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/switch --summary` | 111 | 216 | 135 (62.50%) | 11 (5.09%) | 70 (32.41%) |
| The same, on the register engine (focused) | focused | `4ecac3b` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/switch --summary` | 111 | 216 | 121 (56.02%) | 11 (5.09%) | 84 (38.89%) |
| Class declarations and expressions (focused) | focused | `fc20906` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/class test/language/expressions/class --summary` | 8,426 | 16,689 | 2,636 (15.79%) | 246 (1.47%) | 13,807 (82.73%) |
| The same two, on the register engine (focused) | focused | `fc20906` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/class test/language/expressions/class --summary` | 8,426 | 16,689 | 2,300 (13.78%) | 207 (1.24%) | 14,182 (84.98%) |
| Async functions and `await` (focused) | focused | `45dec1e` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/async-function test/language/statements/async-function test/language/expressions/await --summary` | 189 | 338 | 272 (80.47%) | 18 (5.33%) | 48 (14.20%) |
| The same three, on the register engine (focused) | focused | `45dec1e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/async-function test/language/statements/async-function test/language/expressions/await --summary` | 189 | 338 | 203 (60.06%) | 26 (7.69%) | 109 (32.25%) |
| Array literals (focused) | focused | `dde873b` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/array --summary` | 52 | 104 | 62 (59.62%) | 0 (0.00%) | 42 (40.38%) |
| The same, on the register engine (focused) | focused | `dde873b` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/array --summary` | 52 | 104 | 38 (36.54%) | 0 (0.00%) | 66 (63.46%) |
| `%String.prototype%` of Annex B (focused) | focused | `0972eb7` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/annexB/built-ins/String --summary` | 111 | 222 | 16 (7.21%) | 206 (92.79%) | 0 (0.00%) |
| The same, on the register engine (focused) | focused | `0972eb7` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/annexB/built-ins/String --summary` | 111 | 222 | 204 (91.89%) | 8 (3.60%) | 10 (4.50%) |
| `%Object%` and `%Reflect%` (focused) | focused | `72df6bb` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Object test/built-ins/Reflect --summary` | 3,564 | 7,108 | 6,176 (86.89%) | 908 (12.77%) | 24 (0.34%) |
| The same two, on the register engine (focused) | focused | `72df6bb` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object test/built-ins/Reflect --summary` | 3,564 | 7,108 | 6,115 (86.03%) | 79 (1.11%) | 914 (12.86%) |
| `%Math%` (focused) | focused | `13199af` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Math --summary` | 327 | 654 | 306 (46.79%) | 344 (52.60%) | 4 (0.61%) |
| The same, on the register engine (focused) | focused | `13199af` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Math --summary` | 327 | 654 | 620 (94.80%) | 4 (0.61%) | 30 (4.59%) |
| `%arguments%` (focused) | focused | `25e7f86` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/arguments-object --summary` | 263 | 460 | 188 (40.87%) | 2 (0.43%) | 270 (58.70%) |
| The same, on the register engine (focused) | focused | `25e7f86` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/arguments-object --summary` | 263 | 460 | 180 (39.13%) | 2 (0.43%) | 278 (60.43%) |
| `%Array.prototype%` (focused) | focused | `35a1038` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/prototype --summary` | 2,811 | 5,583 | 4,878 (87.37%) | 310 (5.55%) | 395 (7.08%) |
| `%JSON%` (focused) | focused | `3aacb65` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/JSON --summary` | 165 | 330 | 140 (42.42%) | 14 (4.24%) | 176 (53.33%) |
| `for`-`of` and destructuring assignment (focused) | focused | `0583f49` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/for-of test/language/expressions/assignment/dstr --summary` | 1,119 | 2,082 | 1,342 (64.46%) | 75 (3.60%) | 665 (31.94%) |
| `eval` and `%Function%` (focused) | focused | `56facc9` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/eval-code test/built-ins/Function --summary` | 856 | 1,347 | 816 (60.58%) | 142 (10.54%) | 389 (28.88%) |
| `eval` and calls (focused) | focused | `a6c244e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/eval-code test/language/expressions/call --summary` | 439 | 625 | 222 (35.52%) | 75 (12.00%) | 328 (52.48%) |
| `%arguments%` and arrow functions (focused) | focused | `68861bf` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/arguments-object test/language/expressions/arrow-function --summary` | 606 | 1,103 | 645 (58.48%) | 4 (0.36%) | 454 (41.16%) |
| Block scopes (focused) | focused | `ca12925` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/block-scope --summary` | 145 | 287 | 155 (54.01%) | 26 (9.06%) | 106 (36.93%) |
| Blocks, `try` and arrow functions (focused) | focused | `ca12925` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/block test/language/statements/try test/language/expressions/arrow-function --summary` | 565 | 1,071 | 709 (66.20%) | 19 (1.77%) | 343 (32.03%) |
| Blocks and function declarations (focused) | focused | `fe424c6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/statements/block test/language/statements/function --summary` | 472 | 823 | 640 (77.76%) | 13 (1.58%) | 170 (20.66%) |
| Destructuring assignment (focused) | focused | `ef2878e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/expressions/assignment/dstr test/language/statements/for-of/dstr --summary` | 937 | 1,735 | 1,214 (69.97%) | 36 (2.07%) | 485 (27.95%) |
| `Function.prototype.bind` (focused) | focused | `61b960f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Function/prototype/bind --summary` | 100 | 200 | 178 (89.00%) | 18 (9.00%) | 4 (2.00%) |
| `Object.fromEntries` (focused) | focused | `c30245e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Object/fromEntries --summary` | 25 | 50 | 34 (68.00%) | 4 (8.00%) | 12 (24.00%) |
| `Array.from` (focused) | focused | `815ce52` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/from --summary` | 47 | 90 | 60 (66.67%) | 20 (22.22%) | 10 (11.11%) |
| Rest parameters and function definitions (focused) | focused | `011b62f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/language/rest-parameters test/language/expressions/function test/language/statements/function --summary` | 726 | 1,289 | 1,019 (79.05%) | 15 (1.16%) | 255 (19.78%) |
| `length` of 10.4.2.4 and `defineProperty` (focused) | focused | `947214a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Array/length test/built-ins/Object/defineProperty --summary` | 1,161 | 2,310 | 2,200 (95.24%) | 28 (1.21%) | 82 (3.55%) |
| Clause 24 entire: `%Map%`, `%Set%`, `%WeakMap%` and `%WeakSet%` (focused) | focused | `ca8e672` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Map test/built-ins/Set test/built-ins/WeakMap test/built-ins/WeakSet --summary` | 813 | 1,620 | 1,268 (78.27%) | 80 (4.94%) | 272 (16.79%) |
| The same four, before the walk of 7.4.2 in their constructors (outdated) | focused | `458d2e5` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Map test/built-ins/Set test/built-ins/WeakMap test/built-ins/WeakSet --summary` | 813 | 1,620 | 1,252 (77.28%) | 75 (4.63%) | 293 (18.09%) |
| `%Map%` and `%Set%`, before the getOrInsert of 24.1.3 (outdated) | focused | `c7a73fe` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Map test/built-ins/Set --summary` | 587 | 1,169 | 766 (65.53%) | 114 (9.75%) | 289 (24.72%) |
| `%WeakMap%` and `%WeakSet%`, before the getOrInsert of 24.3.3 (outdated) | focused | `7dbd658` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/WeakMap test/built-ins/WeakSet --summary` | 226 | 451 | 328 (72.73%) | 20 (4.43%) | 103 (22.84%) |
| The same two, on the stack backend, which carries no `WeakSet` (focused) | focused | `7dbd658` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/WeakMap test/built-ins/WeakSet --summary` | 226 | 451 | 227 (50.33%) | 187 (41.46%) | 37 (8.20%) |
| The same, before the `forEach` of 24.1.3.5 and 24.2.3.7 (outdated) | focused | `5029dd6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Map test/built-ins/Set --summary` | 587 | 1,169 | 666 (56.97%) | 130 (11.12%) | 373 (31.91%) |
| The same, before the set operations of 24.2.3 (outdated) | focused | `1d1a48f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Map test/built-ins/Set --summary` | 587 | 1,169 | 518 (44.31%) | 198 (16.94%) | 453 (38.75%) |
| The same, before the iterable of 24.1.1.1 (outdated) | focused | `48b1816` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Map test/built-ins/Set --summary` | 587 | 1,169 | 458 (39.18%) | 192 (16.42%) | 519 (44.40%) |
| The same, before the iterators of 24.1.5 and 24.2.5 (outdated) | focused | `29cc550` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 test/built-ins/Map test/built-ins/Set --summary` | 587 | 1,169 | 336 (28.74%) | 240 (20.53%) | 593 (50.73%) |
| The same, on the stack backend (focused) | focused | `29cc550` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/built-ins/Map test/built-ins/Set --summary` | 587 | 1,169 | 0 (0.00%) | 1,051 (89.91%) | 118 (10.09%) |
| Complete pinned suite, including staging and Intl | full | `bee51e3` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 35,574 (34.56%) | 32,083 (31.17%) | 35,268 (34.26%) |
| Complete pinned suite on the register engine | full | `e3bab40` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 41,565 (40.38%) | 19,756 (19.19%) | 41,604 (40.42%) |
| Complete pinned suite on the register engine, before the field of a class of 15.7.1 (outdated) | full | `fc9e271` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 41,204 (40.03%) | 19,754 (19.19%) | 41,967 (40.77%) |
| Complete pinned suite on the register engine, before the spread property of 13.2.5 (outdated) | full | `425421f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 41,084 (39.92%) | 19,586 (19.03%) | 42,255 (41.05%) |
| Complete pinned suite on the register engine, before the labelled statement of 14.13 (outdated) | full | `6deae2d` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 40,900 (39.74%) | 19,615 (19.06%) | 42,410 (41.20%) |
| Complete pinned suite on the register engine, before the strict write to a property with no setter (outdated) | full | `d9ee5b0` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 40,878 (39.72%) | 19,637 (19.08%) | 42,410 (41.21%) |
| Complete pinned suite on the register engine, before the logical assignment of 13.15.2 (outdated) | full | `bee51e3` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 40,721 (39.56%) | 19,630 (19.07%) | 42,574 (41.36%) |
| Complete pinned suite on the register engine, before the five helpers of 27.1.3.3 that answer at once (outdated) | full | `e6fa517` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 40,613 (39.46%) | 19,740 (19.18%) | 42,572 (41.36%) |
| Complete pinned suite on the register engine, before the `toArray` and the `forEach` of 27.1.3.3 (outdated) | full | `69e48f0` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 40,571 (39.42%) | 19,782 (19.22%) | 42,572 (41.36%) |
| Complete pinned suite on the register engine, before the `Iterator` of 27.1.3 (outdated) | full | `0f39a55` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 40,457 (39.31%) | 19,018 (18.48%) | 43,450 (42.22%) |
| Complete pinned suite on the register engine, before the host object of the conformance suite (outdated) | full | `004e277` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 40,193 (39.05%) | 19,836 (19.27%) | 42,896 (41.68%) |
| Complete pinned suite on the register engine, before the arguments a clause converts (outdated) | full | `9c0bb62` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 39,819 (38.69%) | 19,696 (19.14%) | 43,410 (42.18%) |
| Complete pinned suite on the register engine, before the Nursery a Safe Point leaves the natives (outdated) | full | `13e7c3a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 39,585 (38.46%) | 19,930 (19.36%) | 43,410 (42.18%) |
| Complete pinned suite on the register engine, before the array index of 10.4.2.1 (outdated) | full | `141a2fd` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 39,569 (38.44%) | 19,946 (19.38%) | 43,410 (42.18%) |
| Complete pinned suite on the register engine, before the methods of 23.2.3 that call back (outdated) | full | `01288ab` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 39,259 (38.14%) | 19,826 (19.26%) | 43,840 (42.59%) |
| Complete pinned suite on the register engine, before the methods of 23.2.3 (outdated) | full | `4a15802` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 38,693 (37.59%) | 19,764 (19.20%) | 44,468 (43.20%) |
| Complete pinned suite on the register engine, before the three list forms of 23.2.5.1 (outdated) | full | `3eebb13` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 38,379 (37.29%) | 19,554 (19.00%) | 44,992 (43.71%) |
| Complete pinned suite on the register engine, before the three further rows of table 71 (outdated) | full | `9ba0eb1` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 37,635 (36.57%) | 22,922 (22.27%) | 42,368 (41.16%) |
| Complete pinned suite on the register engine, before the `BigInt` of 6.1.6.2 (outdated) | full | `29de1d6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 37,193 (36.14%) | 19,978 (19.41%) | 45,754 (44.45%) |
| Complete pinned suite on the register engine, before the `SharedArrayBuffer` of 25.2 and the `Atomics` of 25.4 (outdated) | full | `6c6df11` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 36,861 (35.81%) | 19,860 (19.30%) | 46,204 (44.89%) |
| Complete pinned suite on the register engine, before the captures and short circuits (outdated) | full | `9d12e52` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 36,858 (35.81%) | 19,858 (19.29%) | 46,209 (44.90%) |
| Complete pinned suite on the register engine, before the `TypedArray` of 23.2 (outdated) | full | `0a8e566` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 36,616 (35.58%) | 19,699 (19.14%) | 46,610 (45.29%) |
| Complete pinned suite on the register engine, before the `DataView` of 25.3 (outdated) | full | `4e18bd9` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 36,182 (35.15%) | 19,493 (18.94%) | 47,250 (45.91%) |
| Complete pinned suite on the register engine, before the `ArrayBuffer` of 25.1 (outdated) | full | `2aa83f5` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 36,026 (35.00%) | 19,291 (18.74%) | 47,608 (46.26%) |
| Complete pinned suite on the register engine, before the texts of 21.4.4.41 (outdated) | full | `f947e5c` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 35,922 (34.90%) | 19,377 (18.83%) | 47,626 (46.27%) |
| Complete pinned suite on the register engine, before the setters of 21.4.4 (outdated) | full | `eab8154` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 35,656 (34.64%) | 19,583 (19.03%) | 47,686 (46.33%) |
| Complete pinned suite on the register engine, before the time value of 21.4 (outdated) | full | `31aa8eb` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,910 (33.92%) | 19,197 (18.65%) | 48,818 (47.43%) |
| Complete pinned suite on the register engine, before the `escape` of B.2.1 (outdated) | full | `32d54c2` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,844 (33.85%) | 19,263 (18.72%) | 48,818 (47.43%) |
| Complete pinned suite on the register engine, before the URI functions of 19.2.6 (outdated) | full | `3aacb65` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,526 (33.54%) | 19,253 (18.71%) | 49,146 (47.75%) |
| Complete pinned suite on the register engine, before the rawJSON of the proposal (outdated) | full | `0583f49` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,500 (33.52%) | 19,253 (18.71%) | 49,172 (47.77%) |
| Complete pinned suite on the register engine, before the accessor element of 23.1.5.2.1 (outdated) | full | `56facc9` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,498 (33.52%) | 19,250 (18.70%) | 49,177 (47.78%) |
| Complete pinned suite on the register engine, before the lowering refusal left the `SyntaxError` of 19.2.1.1 (outdated) | full | `a6c244e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,498 (33.52%) | 19,447 (18.90%) | 48,980 (47.59%) |
| Complete pinned suite on the register engine, before the indirect eval of 19.2.1.1 (outdated) | full | `68861bf` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,458 (33.48%) | 19,422 (18.87%) | 49,045 (47.65%) |
| Complete pinned suite on the register engine, before the `arguments` an arrow reads (outdated) | full | `ca12925` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,441 (33.46%) | 19,421 (18.87%) | 49,063 (47.67%) |
| Complete pinned suite on the register engine, before the Block binding a nested function reads (outdated) | full | `fe424c6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,441 (33.46%) | 19,531 (18.98%) | 48,953 (47.56%) |
| Complete pinned suite on the register engine, before the function declaration of 14.2.2 in a Block (outdated) | full | `ef2878e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,434 (33.46%) | 19,534 (18.98%) | 48,957 (47.57%) |
| Complete pinned suite on the register engine, before the member target of 13.15.5.2 (outdated) | full | `61b960f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,408 (33.43%) | 19,518 (18.96%) | 48,999 (47.61%) |
| Complete pinned suite on the register engine, before the construct of 10.4.1.2 (outdated) | full | `c30245e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,380 (33.40%) | 19,512 (18.96%) | 49,033 (47.64%) |
| Complete pinned suite on the register engine, before the walk of 7.4.2 in 20.1.2.7 (outdated) | full | `ca8e672` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,372 (33.39%) | 19,508 (18.95%) | 49,045 (47.65%) |
| Complete pinned suite on the register engine, before the walk of 7.4.2 in the constructors of clause 24 (outdated) | full | `815ce52` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,364 (33.39%) | 19,502 (18.95%) | 49,059 (47.66%) |
| Complete pinned suite on the register engine, before the walk of 7.4.2 (outdated) | full | `011b62f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,332 (33.36%) | 19,486 (18.93%) | 49,107 (47.71%) |
| Complete pinned suite on the register engine, before the rest parameter of 8.6.3 (outdated) | full | `947214a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,298 (33.32%) | 19,483 (18.93%) | 49,144 (47.75%) |
| Complete pinned suite on the register engine, before the descending delete of 10.4.2.4 (outdated) | full | `458d2e5` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,231 (33.26%) | 19,481 (18.93%) | 49,213 (47.82%) |
| Complete pinned suite on the register engine, before the getOrInsert of 24.1.3 and 24.3.3 (outdated) | full | `7dbd658` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 34,109 (33.14%) | 19,540 (18.98%) | 49,276 (47.88%) |
| Complete pinned suite on the register engine, before 24.3 and 24.4 (outdated) | full | `c7a73fe` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 33,735 (32.78%) | 19,502 (18.95%) | 49,688 (48.28%) |
| Complete pinned suite on the register engine, before the `forEach` of 24.1.3.5 and 24.2.3.7 (outdated) | full | `5029dd6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 33,633 (32.68%) | 19,518 (18.96%) | 49,774 (48.36%) |
| Complete pinned suite on the register engine, before the set operations of 24.2.3 (outdated) | full | `1d1a48f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 33,485 (32.53%) | 19,586 (19.03%) | 49,854 (48.44%) |
| Complete pinned suite on the register engine, before the iterable of 24.1.1.1 (outdated) | full | `48b1816` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 33,417 (32.47%) | 19,562 (19.01%) | 49,946 (48.53%) |
| Complete pinned suite on the register engine, before 24.1.5 and 24.2.5 (outdated) | full | `29cc550` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 33,247 (32.30%) | 19,654 (19.09%) | 50,024 (48.60%) |
| Complete pinned suite on the register engine, before 24.1 and 24.2 (outdated) | full | `38e7004` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,905 (31.97%) | 19,358 (18.81%) | 50,662 (49.22%) |
| Complete pinned suite on the register engine, before the `this` of an arrow (outdated) | full | `b8311ff` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,901 (31.97%) | 19,358 (18.81%) | 50,666 (49.23%) |
| Complete pinned suite on the register engine, before the Initializer of 8.6.2 (outdated) | full | `943f49b` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,867 (31.93%) | 19,392 (18.84%) | 50,666 (49.23%) |
| Complete pinned suite on the register engine, before 10.2.4.1 (outdated) | full | `9487fdf` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,857 (31.92%) | 19,402 (18.85%) | 50,666 (49.23%) |
| Complete pinned suite on the register engine, before 27.7.5.2 step 4 (outdated) | full | `35a1038` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,831 (31.90%) | 19,428 (18.88%) | 50,666 (49.23%) |
| Complete pinned suite on the register engine, before the array-like of 23.1.3 (outdated) | full | `b5bdb68` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,737 (31.81%) | 19,432 (18.88%) | 50,756 (49.31%) |
| Complete pinned suite on the register engine, before the return of 14.15.3 (outdated) | full | `4d58622` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,695 (31.77%) | 19,418 (18.87%) | 50,812 (49.37%) |
| Complete pinned suite on the register engine, before the primitive of 13.15.5.2 (outdated) | full | `8285466` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,671 (31.74%) | 19,418 (18.87%) | 50,836 (49.39%) |
| Complete pinned suite on the register engine, before the scan of a spread element (outdated) | full | `1ea19f4` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,597 (31.67%) | 19,358 (18.81%) | 50,970 (49.52%) |
| Complete pinned suite on the register engine, before the escaped loop head (outdated) | full | `769d763` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,589 (31.66%) | 19,350 (18.80%) | 50,986 (49.54%) |
| Complete pinned suite on the register engine, before 7.1.17 step 2 (outdated) | full | `25e7f86` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,535 (31.61%) | 19,342 (18.79%) | 51,048 (49.60%) |
| Complete pinned suite on the register engine, before 10.4.4.7 (outdated) | full | `5f6a3bd` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,367 (31.45%) | 19,329 (18.78%) | 51,229 (49.77%) |
| Complete pinned suite on the register engine, before the inherited index (outdated) | full | `13199af` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 32,359 (31.44%) | 19,361 (18.81%) | 51,205 (49.75%) |
| Complete pinned suite on the register engine, before 21.3.2 (outdated) | full | `e8c1e53` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,995 (31.09%) | 19,359 (18.81%) | 51,571 (50.11%) |
| Complete pinned suite on the register engine, before 23.1.3.13 and 20.1.2.7 (outdated) | full | `72df6bb` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,944 (31.04%) | 19,359 (18.81%) | 51,622 (50.15%) |
| Complete pinned suite on the register engine, before 20.1.2.1 and 28.1.13 (outdated) | full | `0972eb7` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,836 (30.93%) | 19,353 (18.80%) | 51,736 (50.27%) |
| Complete pinned suite on the register engine, before B.2.2 (outdated) | full | `dde873b` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,656 (30.76%) | 19,353 (18.80%) | 51,916 (50.44%) |
| Complete pinned suite on the register engine, before the spread element (outdated) | full | `45dec1e` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,578 (30.68%) | 19,338 (18.79%) | 52,009 (50.53%) |
| Complete pinned suite on the register engine, before async functions (outdated) | full | `1702923` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,233 (30.35%) | 19,280 (18.73%) | 52,412 (50.92%) |
| Complete pinned suite on the register engine, before 23.1.3.1.1 (outdated) | full | `fc20906` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,207 (30.32%) | 19,321 (18.77%) | 52,397 (50.91%) |
| Complete pinned suite on the register engine, before the binding of the class name (outdated) | full | `4ecac3b` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,201 (30.31%) | 19,331 (18.78%) | 52,393 (50.90%) |
| Complete pinned suite on the register engine, before the jump and the Case Block (outdated) | full | `3b287c4` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 31,133 (30.25%) | 19,414 (18.86%) | 52,378 (50.89%) |
| Complete pinned suite on the register engine, before the `var` of a loop body (outdated) | full | `5993a59` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,994 (30.11%) | 19,170 (18.63%) | 52,761 (51.26%) |
| Complete pinned suite on the register engine, before the computed key of 7.1.19 (outdated) | full | `90f4774` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,928 (30.05%) | 19,164 (18.62%) | 52,833 (51.33%) |
| Complete pinned suite on the register engine, before the `ToNumeric` of 7.1.4 (outdated) | full | `c1efb4f` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,816 (29.94%) | 19,162 (18.62%) | 52,947 (51.44%) |
| Complete pinned suite on the register engine, before 13.15.5.2 (outdated) | full | `ea3ad4a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,768 (29.89%) | 19,210 (18.66%) | 52,947 (51.44%) |
| Complete pinned suite on the register engine, before the own `length` of an Array (outdated) | full | `4a63d0a` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,659 (29.79%) | 19,319 (18.77%) | 52,947 (51.44%) |
| Complete pinned suite on the register engine, before 14.3.1.2 step 4 (outdated) | full | `d603a40` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,643 (29.77%) | 19,335 (18.79%) | 52,947 (51.44%) |
| Complete pinned suite on the register engine, before the Symbol-keyed methods of 22.2.6 (outdated) | full | `04f3eb6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,569 (29.70%) | 19,595 (19.04%) | 52,761 (51.26%) |
| Complete pinned suite on the register engine, before the length of 23.1.3 (outdated) | full | `2ded0f0` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,524 (29.66%) | 19,640 (19.08%) | 52,761 (51.26%) |
| Complete pinned suite on the register engine, before the Symbol-keyed write (outdated) | full | `3a014d6` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,487 (29.62%) | 19,677 (19.12%) | 52,761 (51.26%) |
| Complete pinned suite on the register engine, before the combinators of 27.2.4 (outdated) | full | `deca8e1` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 30,262 (29.40%) | 19,671 (19.11%) | 52,992 (51.49%) |
| Complete pinned suite on the register engine, before 27.2 (outdated) | full | `6952e99` | `sh tools/xtask.sh jrs --fuel 1000000 --engine --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 29,946 (29.09%) | 19,509 (18.96%) | 53,470 (51.95%) |

The two full rows measure the two execution paths against the same suite, as do
the two function-declaration rows. Every other row is the stack backend, which
`Realm::evaluate` uses by default. The `--engine` rows are the register engine
under `crates/jrs/src/engine/`, which `jrs --engine` selects for the whole
realm: a Script its lowering does not take is refused as `Unsupported` rather
than run on the stack path, because the two paths hold separate object models.
That refusal is why its unsupported count is high; it is the honest measurement
of the migration, not a defect of the suite. The gap between each pair of rows
is what the milestone group in
[docs/jrs-architecture.md](../../docs/jrs-architecture.md) has to close. Code
identity belongs to the Realm, so a function declared in one Script is callable
from the next, and a call of a global name is lowered where it is returned and
inside the body of a loop. `this` is the receiver of the call, a method call
reaches a function of the Script and not only an intrinsic, and a property of a
value the lowering could not name is read and written, `new` constructs from
the `prototype` 10.2.5 gives a function, `instanceof` walks the chain of the
value, and 10.2.1.2 binds `this` for a call that has no receiver. A parameter is
any value (10.2.11), so `harness/assert.js`, `harness/sta.js` and
`harness/compareArray.js` all run on the engine and the suite reaches it, and a
`for`-`in` takes any head and the binding its declaration made. A conversion
that would have to call a `valueOf` of the Script names that as a gap instead of
answering, which is most of what the engine now reports as unsupported.
A body reads its own `arguments`: 10.2.11 binds the name and 10.4.4 makes the
unmapped object 10.4.4.7 describes, with its index properties, `length` and
`callee`. A body that also writes a parameter could observe the mapping this
engine does not build and is not lowered.
A `delete` takes a property off an object: 13.5.1.2 sends the base through
`ToObject` and the name through `[[Delete]]`, 10.1.10.1 keeps a property that is
not configurable, and an Array answers its indices and its `length` from where
10.4.2 puts them. A `delete` of a free name stays a gap, because the name
belongs to the global object.
A property is written under a key only the run time knows: 13.15.2 writes
through `PutValue`, and the store names the gap where it happens when the name
is one a Prototype this Realm has not built owns — `__proto__` on every object,
`length` and `name` on an Array or a function. A computed key of a literal
defines instead of assigning (13.2.5.5), reaches no Prototype, and asks that
nothing.
The join of an `if` takes the Objects its branches made (14.6.2): one only a
branch made keeps what its layout says, one the branches shaped differently
gives that up, and so does every value whose layout the join could not keep.
Two references the constructor path lost to a collection are fixed with it,
both of them the same mistake — a reference read before an allocation that may
scavenge (10.2.5 and 10.1.13).
With that, `harness/propertyHelper.js` lowers completely, and what stops it is
no longer the lowering but the constructors clause 19 gives the global object.
`%Array%` and `%Object%` are the first two it builds: 23.1.1.1 makes an Array of
one length or of many elements and 20.1.1.1 makes an ordinary object of
undefined and of null, `new` reaches the same function in both because a native
constructor answers an object of its own, 23.1.2.3 answers `IsArray`, and 17
ties each constructor and its prototype together, which gives
`%Object.prototype%` its own `constructor`. A read of a name 23.1.2 or 20.1.2
gives a constructor and this Realm has not built, `Array.from` and
`Object.create` among them, is a gap and not the undefined of a constructor
without it, as is `ToObject` of a primitive.
`%Object%` carries the three functions of 20.1.2 a Script uses to ask what a
property is and to say what it should be:
`getOwnPropertyNames` answers the own String keys in the order 10.1.11 gives
them, `getOwnPropertyDescriptor` answers the object 6.2.6.4 makes of an own
property, and `defineProperty` defines the one 6.2.6.5 reads. A field a
descriptor does not carry is absent, which 6.2.6.6 reads as false, so a
property defined from `{value: 5}` is enumerated by no `for`-`in`. An accessor
in a descriptor is a gap, because this engine has no accessor property. The
harness now asks for `%Function%`; that gap still holds 946 variants which
count as a failed harness today.

A compound assignment reaches a property and a name of the Global Environment
Record (13.15.2): the Reference is evaluated once, read through, and written
back with the operator of 13.15.3, which is the operator the expression form
applies. `o.a += 2` and, inside a Realm, `x *= 2` were Scripts the lowering
refused before. What is left of that family is no longer the lowering but the
constructors it asks for, `%Boolean%` and `%String%` first.

A `let` and a `const` of a Script bind on the `[[DeclarativeRecord]]` of the
Global Environment Record (16.1.7), where they outlive the Script that made
them and every later Script of the Realm reads them, with the `ReferenceError`
of a temporal dead zone before the declaration and the `TypeError` of 9.1.1.4.5
for an assignment to a `const`. The lowering refused every Script that held one
before, which is most of what is written today, and that is where the engine's
count of refusals falls by 2,153 while its failures rise by 2,109: a Script
that is taken now runs until it reaches a gap its harness needs, which the
runner counts as a failed harness. Neither a failure nor a refusal is a pass,
and the two full runs of the stack backend on either side of this step are
identical variant for variant, so nothing that answered before answers
differently.

`%Function%` is on the global object and `%Function.prototype%` carries the
first of the methods 20.2.3 gives every function. 20.2.3.3 does not answer a
value, it calls: the function it was reached through becomes the callee, the
first argument becomes the `this` value, and the rest shift down by one, so it
is resolved where a call resolves its callee. 20.2.1.1 compiles a body at run
time and names that as a gap. 20.2.3.2 answers the bound function exotic
object of 10.4.1, which 10.4.1.1 calls with the `this` value the bind fixed,
whatever the call site passes; a bound argument is named as a gap, because it
would have to go in front of arguments that lie in the registers of the caller.
`%Math%` is an ordinary object
that 19.1 gives the global object, with the `pow` of 21.3.2.26; a name 21.3
gives it and this Realm has not built is a gap, because answering undefined
would claim the namespace does not have it.
With that, `harness/propertyHelper.js` runs on the engine from its first line
to its last. It was the harness that stood between the engine and 946 variants,
and they now reach what they test: the engine's failures fall by 5,301 in this
step, because a variant that stopped at a harness which would not load now
stops at a gap it names. Neither is a pass, and the stack backend is unchanged
variant for variant across the step.

`%String%` is next: 22.1.1.1 answers the String `ToString` makes and the empty
String for no argument, 17 ties it to `%String.prototype%`, so `"".constructor`
is `String`, and `new` names the String exotic object of 10.4.3 that this
engine has not built rather than answering the primitive a call answers. A
property read on a String was refused by the lowering for every name
`%String.prototype%` owns, which put the gap one step too early: the
instruction already walks the Prototype, so `"".charAt` and `'abc'['slice']`
answer the function the Realm installed.

Taking those Scripts reached four wrong answers, which are gaps now. `ToString`
of an Object answered a `TypeError` the embedding reports as invalid bytecode;
it needs the `ToPrimitive` of 7.1.1 and says so. 22.1.3 begins every
`%String.prototype%` method with `RequireObjectCoercible` and `ToString`, so
`String.prototype.charAt.call(42, 0)` is `"4"` and only null and undefined are a
`TypeError`. A name an intrinsic Prototype owns and this Realm has not built was
undefined when it was read on the Prototype itself, which claims the Prototype
does not have it. And a property key a Script made was never interned, while a
Shape holds its names by reference, so every lookup by such a key missed:
`Object.getOwnPropertyDescriptor(String, "prototype")` was undefined while
`String.prototype` read fine. B.2.2 gives `trimLeft` and `trimRight` and 21.3
gives `sumPrecise`, which no list carried, so those answered undefined too.
The engine gains 644 passes in this step and loses 462 failures; no variant
that passed before stopped passing, and the stack backend is unchanged.

The error constructors follow, which is the largest step of this migration so
far: 574 more variants pass. The Realm built the prototype of every error and
none of the constructors, so an error the engine threw could not be told apart
from another by the Script that caught it. 20.5.1.1 and 20.5.6.1.1 now answer
an error under the Prototype of the constructor that was called, `new` reaches
the same function, and the errors this engine throws are made by that same
operation, so `e instanceof TypeError` answers what it should and
`assert.throws` does its work.

Every Script the lowering refused reported one sentence, which says that
something is missing but never what, and 19,800 variants stood on it. The
lowering now records the innermost construct it stopped at, by the name of the
grammar, and the two places that refuse a Script report it. Nothing runs
differently for it: the full run answers the same result for all 102,925
variants, variant for variant. The number reads as 4,619 a function expression,
2,939 a class expression, 1,515 a declaration of the Script, 1,462 a for-of
statement, 1,308 a call, 1,212 a regular-expression literal and a long tail;
1,533 are refused by a pass that runs before the lowering and still have no
name.

The first of them is `this` outside a function. 9.4.2 resolves it on the
Environment Record that has one, and every one the lowering took was bound by
10.2.1.2 into a register. At the top level of a Script that Record is the
Global Environment Record, whose `[[GlobalThisValue]]` (9.1.1.4.11) is the
global object, which is how the harness reaches
`verifyProperty(this, name, ...)`. Two wrong answers behind it are gaps now:
19.1 gives the global object properties this Realm holds in the Global
Environment Record, so reading one off the object is a gap and not the
undefined of an object without it, and `new Function(...)` names 20.2.1.1 as
the gap a call of it already named instead of answering that `%Function%` is
not a constructor. That costs one pass: `15.3.5.4_2-59gs` asserted that
`new Function("return f();")()` throws a `TypeError`, and the `TypeError` it
was given was the refusal of `new` rather than the one it describes. A pass
that rests on the wrong error is not one.

A read that reached a Prototype this Realm has not finished building said so
and no more, and 6,230 variants stood on that one sentence. Each gap now names
the object that owes the property, which is what decides what to build next:
2,671 `%Object%`, 1,706 `%Array.prototype%`, 648 `%Math%`, 504
`%String.prototype%`, 175 `%Function.prototype%`, 164 `%Object.prototype%`,
120 `%String%`, 95 the global object, 72 `%Array%`, 71
`%ArrayIteratorPrototype%` and 4 `%Error.prototype%`. Naming them changed
nothing that runs: the full run answers the same result for all 102,925
variants, variant for variant.

`%Object%` owed the most, and six of its functions are built. 20.1.2.2 makes
an ordinary object under the Prototype it is given, with the properties of
20.1.2.3 when a second argument names any; 20.1.2.3.2 reads every descriptor
before it defines one, so a source whose second entry is no descriptor leaves
the object untouched. 20.1.2.12 answers the `[[Prototype]]` of what `ToObject`
made, 20.1.2.19 the own enumerable String keys in the order 10.1.11 gives
them, 20.1.2.14 `SameValue` of 7.2.11, which tells the two zeroes apart where
strict equality does not, and 20.1.2.13 `HasOwnProperty` without the Prototype
Chain. That is 514 more variants, and `Object.keys.length` is 1 as 17 counts
the heading of 20.1.2.19, where the stack backend answers zero as it does for
`call`. The 40 variants that moved from a gap to a failure all read a global
no Realm here builds, which the stack backend fails as well.

`%Array.prototype%` owed the next most, and the eight methods of 23.1.3 that
need no callback are built: 23.1.3.27 and 23.1.3.37 move every element by one,
23.1.3.31 answers the removed elements and closes the distance its arguments
leave, 23.1.3.7 fills a range and 23.1.3.4 copies one range of the Array over
another, and 23.1.3.2, 23.1.3.39 and 23.1.3.33 copy into an Array of their own.
A hole stays a hole where the clause reads through `HasProperty`, and becomes
undefined where it reads every index. `shift` and `unshift` the stack backend
does not have at all, so the engine answers more than it there. Two internal
errors a forwarded call could reach are gaps now: this engine moves elements in
the store 10.4.2 gives an Array, and a receiver without one reported a broken
frame instead of the thing it cannot do, while its `length` is written the way
7.3.4 writes any property. That is 152 more variants and six fewer failures,
with no variant moving from a gap to a failure.

What `%Array.prototype%` still owes is the family that calls back into the
Script — `map`, `filter`, `forEach`, `every`, `some`, `reduce`, `reduceRight`
and `sort` — which needs the engine to enter a function of the Script from
inside a native and resume where it left off, as 7.1.1 already does for a
`valueOf`.

`%Math%` follows. 21.3.1 gives it eight values, each the binary64 nearest the
number the clause names, and 21.3.2 gives it functions of which this Realm
builds those that need no library for a transcendental: the magnitude, the
three roundings, the sign, the two extrema and the three that work on 32-bit
integers, with `sin` answering through the one the math crate already had.
6.1.6.1 tells the two zeroes apart and each clause says which it answers, so
`Math.ceil(-0.5)` and `Math.min(0,-0)` are −0 where `Math.abs(-0)` is +0; the
printed form does not show that, so 20.1.2.14 is what asks. Over
`test/built-ins/Math` the engine passes 118 of 654 variants and fails none,
where the stack backend fails 344 of them.

The nine methods of 23.1.3 that ask the Script about each element follow, and
they are what the engine was missing rather than one more list of functions.
Each element is a call, so the engine leaves the method to make it and comes
back through the frame that call opened, as 7.1.1 already did for a `valueOf`.
What the walk has reached outlives those frames, so it lives in an object of
the heap that the collector traces, named by a root the resume record carries;
the callback takes its arguments from there too, written into the window of the
callee after `bind_this` has run, because the operation has no frame whose
registers could hold them and a value read before `bind_this` would name an
object it moved.

Seven wrong answers the Scripts then reached are right or named now. Note 1 of
23.1.3.24 gives its callback four arguments and it was given three. 10.2.9
gives a function its `length`, which no function object had, and a `length` the
Realm has not built read as zero, which says an array-like is empty where the
truth is that the engine cannot tell. A miss on `%Boolean.prototype%`,
`%Number.prototype%`, `%Error.prototype%` and `%ArrayIteratorPrototype%` was a
gap whatever the name was, and 20.3.3 gives `%Boolean.prototype%` no `length`,
so an Array method called on a boolean reads none and walks nothing, which is
an answer. An indexed write to a receiver with no Elements store, and running
out of heap or string references, each reported a broken frame where one is a
gap and the other a resource. And 10.4.2.2 step 1 refuses a length past
2^32-1, which 23.1.3.21 reaches through 7.1.20.

That is 1,210 more variants. Over `test/built-ins/Array` the engine passes
1,922 of 6,117, up from 750. What 23.1.3 still owes is `sort`, whose comparator
is a callback of a different shape.

A parameter takes the Initializer of 8.6.2 where the call passed undefined,
and 15.1.5 stops counting the `length` of 10.2.9 at the first of them. An
Initializer runs in the frame of the call, so a name it reads is captured like
one the body reads, which the scan of free names did not do. The refusal of a
parameter list now says which of the three things 10.2.11 would have to do:
of 1,634 variants they were 62 a rest parameter, 756 an Initializer and 816 a
binding pattern.

A Realm Script that held a `let` or a `const` anywhere inside a block was
refused whole. The lowering has always bound a block in registers of the frame;
what it does not take is named where it stands now, and the Script around it is
taken. A closure over a block binding, a `const` assigned to and an object in a
block still name their own gap.

The differential fuzz then found what the suite could not: an input that ran
for over twenty minutes. Every clause of 23.1.3 walks to the `length` 7.1.20
gave it, and a Script can make one 2^32-1 while holding a single element, so
`new Array(4294967294).fill(1)` did four billion writes and answered after 27
seconds. Nothing charged for it, because a hole opens no frame and fuel is
charged when a frame opens. A walk is charged before it starts now, at the rate
a batch of indices costs; the three clauses that search stop at what they find,
so those are charged where they look. The suite answers the same result for all
102,925 variants as before the charge, and the fuzz run ends clean with its
throughput up from 11,445 to 17,801 executions a second.

A `for`-`of` takes a `var` head as a `for`-`in` always did: 14.7.5 makes one
binding per iteration for a lexical head and writes the one the declaration
made for a `var` head, which is the same question for both.

What it owed after that was the protocol itself. A `for`-`of` stepped an Array
with an instruction and refused every other iterable, because those resolve
`@@iterator` to a method the lowering had no way to name. It can name one now,
and the rest of 7.4 is calls and property reads it already emits: 7.4.2 reads
`@@iterator` and calls it, 7.4.6 calls `next` and asks whether the result is
`done`, and 7.4.7 reads `value` only where it is not. 27.1.2.1 gives
`%IteratorPrototype%` an `@@iterator` that answers `this`, so an iterator is
itself iterable. A Symbol a Prototype owns and this Realm has not built was
undefined, which 7.4.2 turns into "not iterable" — a different statement from
"the String iterator of 22.1.3.34 is not built" — and it names that instead.
A body that leaves by `break` or `return` is still refused and says so, because
7.4.9 would have to close the iterator. The refusals fall from 2,275 to 2,207.

The runner itself was the other cost. A full run took about thirteen minutes
and a step is measured two or three times, so the wait was most of the work.
Every file already gets a realm of its own, so files share nothing and are now
spread over as many threads as the machine has, merged back in the order they
were discovered. The full engine run takes 44 seconds and the stack run 39,
and both write byte-for-byte what the sequential runner wrote. `--jobs 1` is
that sequential run.

A Property Descriptor changes what a property is and not only what it holds.
A Shape carries the attributes together with the names, and defining a property
the Shape already had reused its slot and kept its old attributes, so
`Object.defineProperty(o, 'x', {enumerable: false})` left it enumerable and
`Object.keys` still answered it. The Shape an object should have is built by
walking the one it has, in the order the properties were added. Two more of
10.1.6.3 came with it: a field the descriptor does not name leaves the property
as it was, and 6.2.6.6 fills one in as false only for a property that did not
exist.

That is what the integrity levels needed. An object carried `[[Extensible]]`
and nothing read or wrote it, so 20.1.2.20 and 20.1.2.16 had nothing to answer
with; 20.1.2.22 and 20.1.2.6 now set the level of 7.3.14 by giving every own
property the attributes the level asks for, and 20.1.2.18 and 20.1.2.17 test it
with 7.3.15. Step 1 of each clause answers a value that is not an Object,
because there is nothing on it to configure. An object that holds indices names
a gap instead: 7.3.14 speaks of every own property, and an index lives in a
store that carries no attributes of its own. 20.1.2.24 and 20.1.2.5 answer the
enumerable values and the pairs.

Over `test/built-ins/Object` the engine passes 1,530 of 6,802 variants and
fails 18, where the stack backend fails 862.

An update takes `ToNumeric` of what the binding held (13.4.4.1), so
`var x = '1'; x++` is no longer refused for want of a Number, `Add` does not
concatenate, and the answer a postfix update gives is the Number `ToNumeric`
made.

`%Number%` follows the pattern `%String%` and `%Math%` set. 21.1.1.1 answers
+0 for no argument and the Number `ToNumber` makes of every other, `new` names
the Number exotic object of 21.1.3, and 21.1.2 gives eight values and four
questions, none of which coerces: step 1 of each answers false for anything
that is not a Number, where the `isFinite` and `isNaN` of 19.2 take `ToNumber`
first. Over `test/built-ins/Number` the engine passes 226 of 680 variants and
fails 2, where the stack backend fails 102.

An uncaught throw of an Object the embedding cannot hold was reported as an
unsupported feature, and it is not one: the Script ran to a `throw`, which is a
completion of the language. `assert.throws` builds a `Test262Error`, an
ordinary object and not a native error, so every assertion that failed inside
one left the engine claiming a missing feature. The boundary now answers that
the Script threw and not what it threw, which moves 1,314 variants from
unsupported to failed — where they belonged. The engine passes the same number
as before; what changed is that the measurement stopped calling its own
failures gaps.

`%Boolean%` is 20.3.1.1, `ToBoolean` of the argument, with `new` naming the
Boolean exotic object of 20.3.3 that this engine has not built.

`%Reflect%` is 19.4.4, an ordinary object like `%Math%`, and 28.1 gives it the
operations of clause 20.1.2 without their coercion: step 1 of each refuses a
target that is not an Object, and the answer says whether the operation worked
where 20.1.2 throws. Nine are built, and 28.1.1, 28.1.2, 28.1.12 and 28.1.13
are named as gaps.

Naming that last gap costs 206 passes, and they were not passes. The harness
`isConstructor` asks `try { Reflect.construct(...) } catch { return false }`.
With `Reflect.construct` absent the call threw a `TypeError`, the harness
swallowed it, and every `not-a-constructor.js` concluded what it wanted to
conclude. A test that passes because a feature is missing is what the
procedure forbids, so those variants are unsupported now and say why.

Accessor properties are 6.1.7.1: a property that holds a getter and a setter
instead of a value. The Shape already said which of its properties are
accessors and nothing ever set it, so the engine had none. The slot of one now
holds the pair, and 10.1.8.1 and 10.1.9.2 call it where a data property is read
or written, leaving the instruction the way a conversion of 7.1.1 does. A
getter answers into the accumulator, which is where every read leaves its
value; a setter answers nothing and 13.15.2 answers the value assigned, so that
value waits in a root until the setter returns. Neither is cached, because an
inline cache holds a slot and this is a call.

10.1.6.3 is now the operation the specification writes rather than a merge of
attributes. It answers whether the descriptor can be applied, which 7.3.8 turns
into a `TypeError` and 28.1.3 answers as it is, and it rejects what a
non-configurable property does not allow. Over the descriptor clauses the
engine passes 1,650 of 5,080 variants and fails 16, where it passed 1,004 and
failed 434 before; the stack backend passes 4,738 and fails 336, with both
raising the same errors under the same names.

Three gaps are named rather than answered. A native operation that finds an
accessor cannot call it, because it has no frame to call from. A setter written
in Rust would take its argument from registers of the caller that hold
something else. 10.4.2.1 defines an index against the element store, and the store
holds a value and nothing else: an index that comes out as an ordinary data
property is written there, and every other one leaves the store and becomes a
property of the Shape, which says for itself what it is. A read passes a hole
rather than answering undefined at it, which is what 10.1.8.1 says anyway, and
a write looks at the Shape first. The Array `length` stays a gap, because
10.4.2.4 sets it by deleting what is above the new one.

7.1.17 of an Object is a call of a method of the object, and a native
operation has no frame to make one from. A native that converts an argument
before it does anything else now leaves the way a conversion of 7.1.1 leaves
an instruction: the caller's registers still hold the arguments, so the native
runs again from the beginning once the register holds a primitive, and only
the `this` value travels, in a root. `%String%` (22.1.1.1) and the seven error
constructors (20.5.1.1, 20.5.6.1.1) ask this way. 7.1.1 also takes its hint for
the first time, so 7.1.17 asks `toString` before `valueOf` where 7.1.3 keeps
the order it had. 842 variants move to passed and none away from it.

21.1.3 and 20.3.3 wrap one primitive in an object, and the engine already had
the kinds that hold the data: `new Number(1)` and `new Boolean(1)` were gaps
for want of the four methods that read it back. Those are built, each refusing
a receiver of another kind, and a radix other than 10 stays a gap. 7.1.18 now
gives each wrapper the Prototype of its own constructor rather than
%Object.prototype%, and 20.1.1.1 of a primitive is that same wrapper. 537
variants move to passed and none away from it.

22.1.4 wraps a String in an object, and 10.4.3 gives that object its indices
and its `length` out of the `[[StringData]]` rather than out of a Shape. A read
answers them before it looks at the Shape, 10.4.3.3 lists the indices first, a
write has nowhere to go and a delete answers false. 22.1.3.32 and 22.1.3.28 are
built the way the other two `this`-value methods were. A method of 22.1.3
called on a wrapper stays a gap: 7.1.17 of the receiver is a call, and the
conversion a native leaves for names an argument register, which a receiver is
not. 334 variants move to passed and none away from it.

13.12, 13.9 and 13.11.1 read their operands through 6.1.6.1.2 and 7.2.14, which
for an Object is a call. They now take the same instruction the arithmetic and
relational operators take, in an expression and in a compound assignment alike,
and 7.2.14 keeps what it does not convert: two values of one type are compared
as they are, and an Object against null or undefined is false without asking
the object anything. A type the lowering does not know may be an Object, so
`Unknown` no longer counts as a primitive when the typed form is chosen — that
was why `new Number(1) | true` still reached the gap after the wrapper objects
were built. 426 variants move to passed and none away from it.

13.2.5.1 gives a property of a literal a getter or a setter instead of a value,
which the object model took an earlier step. One instruction defines it, taking
the function in the accumulator as one half and leaving the other as it is, so
the `get` and the `set` of one name meet on the object. A computed accessor
name stays a gap, because the two halves have to reach the same property and
the key is only known at run time. 71 variants move to passed and none away
from it.

15.7.14 makes a constructor and the object it carries, and puts every method the
body defines on one of the two. The lowering refused the whole class, and two of
its scope analyses refused any Script that held one, which is why the refusal
named the Script rather than the class. The body is now lowered: one instruction
makes the constructor together with a `prototype` whose attributes no ordinary
function's match, each method takes the attributes 7.3.5 gives one, and the
constructor's `[[Call]]` throws, so the body runs only under `new`. A class that
extends another and a computed name in a class body stay named gaps. 270
variants move to passed and none away from it.

14.3.3.3 reads each property a pattern names out of its source, which the
lowering could only do from a layout it tracked. A source it cannot name is now
read at run time, with 7.2.1 checked before the first read rather than left to
the read itself. A default of such a property is applied at run time too: it was
dropped before, which is a wrong answer and not a gap. A pattern that reads no
property and a rest element stay named gaps. 60 variants move to passed and none
away from it.

10.2.11 binds the argument and 8.6.2 then binds the names a pattern names out of
it. A parameter is a register the call fills and a pattern had no name for that
register; it gets one no identifier of a Script can be, so the parameter prefix
stays what the call window is, and the body starts by reading that register and
running the same 14.3.3.3 an untracked source runs. An array pattern stays a
named gap, because 8.6.2 takes its elements from the iterator of the argument.
230 variants move to passed and none away from it.

8.6.2 opens the iterator of the value (7.4.2), takes one step of 7.4.6 for each
element, and closes what it did not exhaust (7.4.9). The lowering could only read
an array pattern out of a layout it tracked; the elements are now emitted as that
sequence, with the record's `[[Done]]` in a register, and a parameter that is an
array pattern reaches it too. 7.4.2 also refuses a value whose `@@iterator` is
undefined before it calls anything, so both backends raise the same error for the
same reason. A rest element stays a named gap. 452 variants move to passed and
none away from it.

A captured reader is compiled against the type its binding carries, and an
assignment anywhere can make that type wrong after the closure's bytecode has
been emitted; the lowering rejected the whole enclosing body for it. A captured
`var` a write reaches now carries the type the lowering cannot name, so every
read of it takes the generic path and there is nothing to invalidate. 6 more
variants move to passed.

8.6.2 and 14.3.3.3 run an Initializer only where the value is undefined, and the
lowering asked every one of them to answer a primitive and to leave its layouts
untouched. A value that is known undefined always takes the Initializer, so what
it made keeps its layout; a value that may be defined takes it on one path only,
so a layout it made is dropped and the answer is the type the lowering cannot
name. A layout an Initializer *changed* is still a refusal. 722 more variants
move to passed.

10.2.10 gives a function a `name` and 8.5.2 gives an anonymous one the name of
whatever it is being given to; the engine gave none, so every read of `f.name`
reached %Function.prototype% and named a gap. The code unit carries the name as
one of its own string constants, and the lowering fills it in where the
specification does. Five of those are answers the stack backend does not give —
it leaves the name of a method, of an accessor and of a property definition's
function empty, where 13.2.5.5 and 10.2.10 name them — so their tests check the
engine against the specification rather than against the other backend. 224 more
variants move to passed.

10.4.4 binds `arguments` in every ordinary function, and the lowering read it
only as the base of a property access, because the mapping of 10.4.4.7 would
show in the object otherwise. A strict function has no mapping, so there it is a
value like any other: 10.2.4.1 is built as an intrinsic that stands on no object
and throws whenever it is called, `callee` is that accessor on both halves, and
the object carries the iterator of 23.1.3.33. That iterator was answering
nothing for an object that is no Array — 23.1.5.2.1 reads the length of the
array-like again at every step, and the engine read an element store instead, so
an array-like iterated to zero elements silently. 305 more variants move to
passed.

13.15.2 writes through `[[Set]]`, which needs no layout, and the lowering asked
for one, so a write to a value it could not name was refused; it is written at
run time now. 10.4.2.4 sets an Array's own length and deletes every index at or
above it, which the engine had no operation for — a write of `length` would have
put a property in the Shape beside the length a read answers — and it is built.
An index the Shape took over in 10.4.2.1 stays a named gap there. 14 more
variants move to passed.

13.4.4.1 reads a Reference, takes `ToNumeric` of what it held and writes the sum
back, and the lowering did that only for a binding of its own frame: a name the
Global Environment Record binds — in a Realm, every top-level `var` — was
refused, and so was every update of a property. Both are lowered now, the
property Reference evaluated once so the read and the write reach the same
property. 531 more variants move to passed.

16.1.7 makes a `var` of a Realm Script a binding of the Global Environment
Record, and the head of a `for`-`in` or a `for`-`of` looked for it in the frame:
in a Realm that is every such loop a Script writes at the top level. The head
knows that shape now — the loop keeps the key or the element in a register of
its own and writes the binding where 14.7.5.6 says it lives. 542 more variants
move to passed.

13.10.2 answers `HasProperty` of 7.3.11 on the key 7.1.19 makes, and the engine
had no instruction for it, so every Script that used `in` was refused. 60 more
variants move to passed, and 1,067 move the other way across the line between
failed and unsupported: those Scripts used to be refused whole and now run until
they reach the feature they actually need, which is `async`, `Promise` or a
template literal.

20.4 is built as the constructor it is, with the thirteen Symbols of table 1
given to it by 20.4.2; making a Symbol of its own stays a named gap, because
20.4.1.1 needs a place for the description and 20.4.2.2 a registry shared
between Realms. 7.1.19 keeps a Symbol as the key it is, which the three
computed-property instructions did not — they sent every key through `ToString`,
where a Symbol has no text. 328 more variants move to passed.

22.2.4.1 makes an object of a pattern, and the engine had none, so every Script
holding a literal was refused. The pattern is compiled where the Script is and
lives in the code unit beside its string constants; the object names the unit
and the index the way a closure names its function, and 22.2.7 gives the
instance its own ordinary `lastIndex`. 22.2.7.2 runs on the automaton the stack
backend already uses, charging the work it reports to the fuel of the call. The
constructor, which compiles at run time, and the accessors of 22.2.6 stay named
gaps. 708 more variants move to passed.

10.1.9.1 refuses a write to a property that is not writable, wherever on the
Prototype Chain it sits, and the engine wrote the slot regardless, so the
lowering guarded the three names it knew of by refusing every write of them.
The two store instructions carry the strictness of the Reference now and consult
the property before they write; they also carry whether the write defines rather
than assigns, because 13.2.5.5 defines the properties of a literal and reaches
no Prototype. 949 more variants move to passed.

14.7.5 lets the head of a `for`-`in` or a `for`-`of` be a binding pattern, and
the lowering asked every head for one name — the whole `dstr` subtree of the
`for`-`of` family rested on that. The loop keeps the step in a register of its
own, the body starts by binding the names 8.6.2 names out of it, and a lexical
head declares those names and gives back the registers it took in the order the
allocator wants them. 370 more variants move to passed.

25.5 is an ordinary object like %Math%, and the engine had neither it nor its
two functions. 25.5.1 parses with the arena the stack backend already uses: the
arena is in postorder, so each value is built after everything it holds and each
stays in a root of its own while the next is allocated. 25.5.2 answers the text
of a value, taking the own enumerable String keys in the order 10.1.11 gives
them. The reviver, the replacer, the space and `toJSON` are each a call a native
has no frame to make, and each is a named gap. 240 more variants move to
passed.

The complete run also identified 294 `_FIXTURE` files which were correctly not
executed as standalone tests. These numbers are a migration measurement, not a
conformance claim. Failed and unsupported variants of both the focused and the
full runs remain open work.

A read of a name no object of the Prototype Chain has used to answer undefined
on the engine where the chain reached a Prototype this Realm has not finished
building. It now names the gap, which is why the engine row of the property-read
family is almost entirely refusals. The full run is unchanged against the run
before it, so no pass depended on the wrong answer.

The focused families are what the register backend gained in this migration
step. The function, call, `this`, loop, property, `new` and `instanceof` rows are pairs measuring the two paths against
each other rather than a family the engine has taken over; the rest are families
it answers itself: the five statements, the methods of %Object.prototype% it now answers
itself, the property accessors, which reach a String's own "length" and indices
on the new engine, the methods of %String.prototype%, and the four search
methods, `join`, `push`, `pop`, `reverse` and `slice` of %Array.prototype%. What the engine
passes in the paired families are the files that need no harness. Each produces
the counts the legacy stack backend produces for the same family, which is what
a backend migration has to show: the full-suite counts are unchanged against the
same suite measured before it, variant for variant. The Array search run was measured at tree
`da4011ac65a23328693080a67da8efbe902c02f3`, the join run at tree
`75bc66931663a104b8f4c973fae1a7878aa53ca2`, the push and pop run at tree
`2f35debf666654ca39a9ef91cdf75097a7125288`, the reverse run at tree
`19054962f1f93e409f7b0d5dad173b74d5eb1d88`, the iteration run at tree
`769107adb8c6f8b0f87d7dfe0f348df4531d0ce0`, the slice run at tree
`e873e530ad100efc4492e5a0a4ce68c824503cae`, and the property-read runs at tree
`e946d0f5655a3f90d2abe1986bcff7cdc9765072`. The function, call and `this` runs at tree
`94023436def77fc0433c7d67f6bc9b70c2455b5d`, the property runs at tree
`c1f2a4fab7808f3b5c8b0824f8a8ed3eaf11dc59`, the `instanceof` runs at tree
`504da841ece9a2a58acc73ef0a5968b53daa252c`, and the `this` runs at tree
`3d20693df017258270e78d197497481761dee88b`, and the `delete` runs at tree
`db32abdb924343cb345a45286c3df0a2b8f3eb9d`, and the assignment runs at tree
`e2e3473e55f557f7f7a887963d18a60c857fb433`. The `new` runs and both full runs
were measured at tree `d98662e20789888cb180d99642144c95e6596f04`. The
`%Array%` runs at tree `4dcc52a89757181705ea5687778e857ae9d07408`. The
`%Object%` runs at tree `595f01d820090f035e3b992c1db9990181d2ae95`. The
compound-assignment runs at tree `8183751cf4043881206e2a64a11ef3f5f0ad0aae`.
The lexical-declaration runs were measured at tree
`ea89126ff888b6994a0b25a69baf8b0845fa25ff`. The `%Function%` runs were measured at tree
`94248adff3e44e88c2a52a671d1c38649b3e135e`. The `%Math%` runs were measured at tree
`ef4cd61875fba838eea55c5c337a3b2859ed4da4`. The error-constructor runs and both
full runs were measured at tree `0d3fee5ef3592d9e2e77a9f9bc5c146b996afa81`,
which is the tree of `a36aa89`. The `%String%` runs and both full runs beside
them were measured at tree
`54864a06604bd2b5f1d9f81f232e8e5d64939692`, which is the tree of `86ba2e2`.
Both full runs beside the named refusals and the top-level `this` were
measured at tree `950e1f858de5af48170d7517db5171da2957ccdb`, which is the tree
of `b367caf`. The `%Object%` runs and both full runs beside them were measured
at tree `32a160f59e6759ed3b30746a8eb7f024d49194a4`, which is the tree of
`f123a15`. The `%Array%` runs and both full runs beside them were measured at
tree `b01866f6404a7fdcb87da506795dcaca929665c5`, which is the tree of
`14a8333`. The `%Math%` runs and both full runs beside them were measured at
tree `6a1ceed16ca5526f8fbcb1c08c6ff841d525978d`, which is the tree of
`6fe4b67`. The `%Array%` runs and both full runs beside them were measured at
tree `6e308d3bfd1883631583168a7311c83665bb1489`, which is the tree of
`7596776`. Both full runs beside the parameter Initializers, the blocks of a
Realm Script and the bound on every Array scan were measured at tree
`f59c300f1ac9f52e3a71f09af2cfc94ba9f11c56`, which is the tree of `d7c48b2`. The `for`-`of` runs and both full runs beside them were measured at
tree `a6e6f7c4ceb2326595e511ff0ad4887d85b14b0e`, which is the tree of
`f1314b2`. The `%Object%` runs and both full runs beside them were measured at
tree `0d446537428a6d1d20eb3a6ad1ebae9a2240b8f3`, which is the tree of
`16f268e`. The `%Number%` runs and both full runs beside them were measured at
tree `5e2ec2f08ae7a18f55f1b4c0fe079e7de807efdd`, which is the tree of
`a011601`. Both full runs beside the throw that reached its end were measured
at tree `53bfb8150e68ec98d035eece875f5ec1ed4d2351`, which is the tree of
`aff55ba`. The `%Reflect%` runs and both full runs beside them were measured at
tree `23f63f95c8ebb94a88f32d50d6a089da885a06da`, which is the tree of `7342613`.
The descriptor runs and both full runs beside them were measured at tree
`91cbf2c4a87389bfa6621938c5f05a31127e21f9`, which is the tree of `602329f`.
The descriptor runs and both full runs beside the indexed descriptors were
measured at tree `f83b656d82cfc1b7a4cba114facdc6e329ff69f6`, which is the tree of `e378c5a`.
The `%String%` runs and both full runs beside the argument a native converts
were measured at tree `9aa17e0b5ae6295479ba787755d90d849690a7b1`, which is the tree of `d8a42a6`.
The `%Number%` and `%Boolean%` runs and both full runs beside the wrapper
objects were measured at tree `2d685ff819853b5e1481e5499c88caaf6ab176ee`, which is the tree of `22b642a`.
The `%String%` runs and both full runs beside the String exotic object were
measured at tree `e938d192c95e51d5340fa2d275c3c4c1765f69dd`, which is the tree of `7f92bfc`.
The operator runs and both full runs beside the integer operators were measured
at tree `c6b06778209374850329d3bf80f9e85707afcd94`, which is the tree of `5375042`.
The object-literal runs and both full runs beside the accessors of a literal
were measured at tree `897db95e9541cd60c08a909f3c43fdaa056c2280`, which is the tree of `8ab84e6`.
The class runs and both full runs beside the class body were measured at tree
`fc35d879c4d3183de699e2446724f8ad211d502b`, which is the tree of `d05f499`.
The declaration runs and both full runs beside the object pattern were measured
at tree `67426c2ce138305f53eba9d1beaac6082913a393`, which is the tree of `502784d`.
The function runs and both full runs beside the pattern parameters were measured
at tree `4c551f2b918796cf23f17d6665e79fc5f5ecd6a5`, which is the tree of `eded9bc`.
The destructuring runs and both full runs beside the array pattern were measured
at tree `3a9b434d676073eb8148c6e5865cf8ef0c75ebb7`, which is the tree of `f85a432`.
Both full runs beside the captured var were measured at tree `f81ca2e9a54ae3a0d186b1efade5eb88714e88c1`, which is
the tree of `ab6b416`. The destructuring runs and both full runs beside the
Initializer that makes an object were measured at tree `b81882bae18c737903ae0ea37c3e49ebf0f5102d`, which is the
tree of `3f46a6f`. The `%Function%` runs and both full runs beside the name a
function is given were measured at tree `81c5b8eb6bba2d95d8a62fde4ee817bc5b190f05`, which is the tree of
`915f61b`. The arguments-object runs and both full runs beside it were measured
at tree `c426eda58af35211855c91d965cc318fd88778b1`, which is the tree of `9e4966b`.
The `%Array%` runs and both full runs beside the Array length were measured at
tree `2967dd64190cdfd228e7aec38c932c7aefe40099`, which is the tree of `78ba3e1`.
The update-operator runs and both full runs beside them were measured at tree
`acd36f25f993be92bb6a73befb4917d1fae00a3f`, which is the tree of `0eec6d3`. The
`for`-`in` and `for`-`of` runs and both full runs beside the var head were
measured at tree `5171fcbc56893b889cbf004ecb90d08d50f25a2a`, which is the tree of `296962a`.
The `in` runs and both full runs beside it were measured at tree `7566b356e8ed2ed32878328b146d71676841ba39`,
which is the tree of `9161a72`. The `%Symbol%` runs and both full runs beside
the well-known Symbols were measured at tree `063dd5f1f751852170a46382cf38652a70be0fa7`, which is the tree of
`0f9693b`. The `%RegExp%` runs and both full runs beside the literal were
measured at tree `33b4f21bcf4ad4e3dd7ce0f89f6659253ffe3f6b`, which is the tree of `f3ee748`.
The assignment runs and both full runs beside the write a property refuses were
measured at tree `f4adc2a7be55c9ac1e585664ce125151c2d6d4fc`, which is the tree of `9e13fd0`.
The `for`-`in` and `for`-`of` runs and both full runs beside the pattern head
were measured at tree `adc01b94d068cb856613588cf190fbd7b8c858d3`, which is the tree of `16d29ab`.
The `%JSON%` runs and both full runs beside it were measured at tree `74f4f240409c2bbb0c582ec3e149f48701f7706f`,
which is the tree of `bd95535`. The `%Array.prototype%` runs and both full runs
beside the element accessors were measured at tree `3255af6bc5f76844b50cb85379e442ff127fa5b8`,
which is the tree of `6948012`. The destructuring runs and both full runs
beside the rest element were measured at tree `5293484ca0ebc0c2c38ab39515215735e24eb0f6`,
which is the tree of `9c9533a`. The destructuring runs and both full runs
beside the value with no layout were measured at tree `365cada8e608b5c44c290628407ef7c96a2e845d`,
which is the tree of `2dd0728`. The `String.prototype.split` runs and both
full runs beside it were measured at tree `20cbb30e9ac96a2331c8bd98b5d5bd53cc3013d3`,
which is the tree of `594df30`. The `String.prototype.match` and `search` runs
and both full runs beside them were measured at tree `eecfcb9f5d6d4db5131289faf86400a5af33f1c2`,
which is the tree of `86b09f8`. The `%String.prototype%` runs and both full
runs beside the argument conversions were measured at tree `f01b187ddc47cc3b04c7f363163d804d4c5f942b`,
which is the tree of `3825637`. The `try` runs and both full runs beside the
catch pattern were measured at tree `b45adcde7aacc737c6920083cc31f7831c5f9523`,
which is the tree of `bce813f`. The global Number function runs and both full
runs beside them were measured at tree `8091ce89534b99963ab6a867abd6f3efefcb01fa`,
which is the tree of `91aba35`. The template-literal runs and both full runs
beside them were measured at tree `cb9dc9fe7f409501a68a9afa010a219e9d858e90`,
which is the tree of `5cdf8b3`. The assignment and declaration runs and both
full runs beside the owned name were measured at tree `b1c6ab8b0854ef198bbbc1ee8ba0636bdfcb0e47`,
which is the tree of `fe340f5`. The arguments-object runs and both full runs
beside the empty mapping were measured at tree `ae2ca38251143cf7066f601d2f589bb2a5158035`,
which is the tree of `0f5cc0f`. The `for`-`of` runs and both full runs beside
the assignment head were measured at tree `1771c85b224a9bc29c7efd40e03b620072a2adef`,
which is the tree of `459d78f`. The `%Number.prototype%` runs and both full
runs beside the primitive receiver were measured at tree `d951188706de27f5d79677fee5f89f3ce12f9284`,
which is the tree of `e97b8c6`. The `%Symbol%` runs and both full runs beside
it were measured at tree `1950a0001bb73932a642838afd34359af15a076e`, which is
the tree of `b19277f`. The `%String.prototype%` runs and both full runs beside
the receiver conversion were measured at tree `2e1bff21ea2bd050d97c5ed88a8ca4c19a7a7a6a`,
which is the tree of `7b37b0e`. The computed-property-name runs and both full
runs beside them were measured at tree `7be09357adf77ef8086f4dcb39be9cad03db3473`,
which is the tree of `23fa2a0`. The `%Array.prototype%` runs and both full runs
beside the find clauses were measured at tree `723fbdc1a1091b6b165429c1195d918c650a3221`,
which is the tree of `b8b6da1`. The `%Function.prototype%` runs and both full
runs beside `apply` were measured at tree `a546adbd5b3d3a51ace8937430df6f99dc8ceb92`,
which is the tree of `f2e46b9`. The `%Array.prototype%` runs and both full runs
beside the length getter were measured at tree `18c4e4bd705494935cfcf8bdce97aabd978d71ae`,
which is the tree of `8898794`. The scan-clause runs and both full runs beside
them were measured at tree `c61ad0f49d46d4f0d7eb14d43cb474eb3994e981`, which is
the tree of `603d094`. The Array-length runs and both full runs beside the
descriptor were measured at tree `6b1dffeaf42fedb8971eaaaaa017176e29637d48`,
which is the tree of `ea2f97c`. The global lexical runs and both full runs
beside the pattern were measured at tree `1786b9d12553359e89b328626aa3fc896f397a3b`,
which is the tree of `dfe431b`. The `setPrototypeOf` runs and both full runs
beside them were measured at tree `845a30ea143711f1d72c489dd293ff61de9a2672`,
which is the tree of `cd6fe50`. The `Function.prototype.toString` runs and both
full runs beside it were measured at tree `72fe3b28cc25e058c5929e0dc204e456f158a561`,
which is the tree of `2107883`. The block-scope runs and both full runs beside
the Object binding were measured at tree `a194a281a88c32175c5cb02c2efa50dcd08ec212`,
which is the tree of `b93d713`. The object-pattern runs and both full runs
beside the rest element were measured at tree `3e4e680be64f17dfdbbd915f2cf5bdf339f2c60f`,
which is the tree of `22fdd9f`. The `for`-`of` runs and both full runs beside
the iterator close were measured at tree `22fb2237d4e46bd487f8db4e43822baef9d45cfa`,
which is the tree of `f151cfc`. The `%Object.prototype%` runs and both full
runs beside `valueOf` were measured at tree
`832a7f0dba084b4500812b8e0ed6cfdaabf5d987`, which is the tree of `c1b270e`. The three focused runs and both full runs
beside the four answers were measured at tree
`d2bff07c4c68c06757ee22646a378334210f4236`, which is the tree of `83ca8c2`. The `%Array.prototype%` runs and both full
runs beside the String receiver were measured at tree
`5457a7bac07ee697dee898c6c3230fcd0d03aef9`, which is the tree of `9e8eb35`;
that step gained 154 variants and lost none. The `%Error%` runs and both full
runs beside 20.5.3.4 were measured at tree
`f0d2589a1620728ee27a2f7e9a35572eba2a2c97`, which is the tree of `a21f844`;
that step gained 34 variants, lost none, and left the stack backend's own
result lines unchanged. The `%Reflect%` runs and both full runs beside
28.1.2 were measured at tree `ae42d67b0b48e681b72cdd51e9963eb6a25d50ca`, which
is the tree of `0d60fa0`; that step gained 296 variants and lost none, most of
them through the `isConstructor.js` harness. The four `%Array.prototype%`
runs and both full runs beside them were measured at tree
`2eed04779d98021977e1cf223b80718f3a8d31c7`, which is the tree of `58401ba`;
that step gained 125 variants and lost none. A `sort` with a comparator of the
Script is still a named gap, which is most of what the focused engine run
counts as unsupported. The `%Array.prototype%` runs and both full runs beside
the converted `length` were measured at tree
`846db9695435d17726f87823056051f269162ebb`, which is the tree of `41b7a16`;
that step gained 206 variants and lost none. The three scan runs and both full
runs beside the converted index were measured at tree
`fe22c1c11344ea6f6a7ccb6c5c0978e2fdce292e`, which is the tree of `a97ff3b`;
that step gained 16 variants and lost none. The `%RegExp%` runs and both full
runs beside the constructor were measured at tree
`073f4ebe05c7ed196ccb1bbb90f0fafb89b40b5a`, which is the tree of `d684811`;
that step gained 521 variants and lost none. The `@@species` runs and both
full runs beside the getter were measured at tree
`787dfb9c356d4c4b24f80070b484ad49f3a67512`, which is the tree of `4036ed2`;
that step gained 14 variants and lost none. The `bind` runs and both full runs
beside the bound arguments were measured at tree
`75de2993e1aac9d35561588351d57bbd75eae703`, which is the tree of `dfbad4f`;
that step gained 36 variants and lost none. The `match` and `search` runs and
both full runs beside the made `RegExp` were measured at tree
`1f8b830e2f66b00ffee0e939a845914aa1403472`, which is the tree of `248f5f0`;
that step gained 56 variants and lost none. The `copyWithin` and `slice` runs
and both full runs beside the intrinsic base were measured at tree
`0bf8201e3fcd983cc0fccbd1a81990dad0301443`, which is the tree of `710ba34`;
that step gained 76 variants and lost none. The `replace` runs and both full
runs beside it were measured at tree
`5696041bf3e4dec958ee1047c5df944e81b652c0`, which is the tree of `7c92bf7`;
that step gained 110 variants and lost 2, both of them tests that used to pass
because `@@replace` was absent and now reach the `exec` of the Script that
22.2.7.1 would call. The `join` runs and both full runs beside `ToString` of
an Object were measured at tree `cfee7fb52e9d04033205033eb049875bf8a218c8`,
which is the tree of `88370c5`; that step gained 13 variants and lost none. The
`%Array.prototype%` runs and both full runs beside the answered length were
measured at tree `d497e660c1f0346e56cff9825ed0a25c8e83f9ef`, which is the tree
of `64f7a69`; that step gained 26 variants and lost none. The `@@unscopables`
runs and both full runs beside it were measured at tree
`423f41aac930d5cba2c9c25abf7e7dffb9862a5e`, which is the tree of `eaaca5d`;
that step gained 8 variants and lost none. The `Array.from` and `Array.of`
runs and both full runs beside them were measured at tree
`4722dc97bf87361f29915e3897960f8e417e27d0`, which is the tree of `7eb647c`;
that step gained 42 variants and lost none. The same two runs and both full
runs beside the mapper were measured at tree
`ea18c72f3cd3694e94be2c23318ddd3300fde006`, which is the tree of `92ee3db`;
that step gained 2 variants and lost 2, the two being `Array.from` of a String,
which passed by accident while the clause walked indices and is now the named
gap the string iterator of 22.1.3.36 leaves. The five copying runs and both
full runs beside 23.1.3.4 were measured at tree
`4c01434b471bed114aaf7969dfee52f611360b2c`, which is the tree of `b84ccb9`;
that step gained 22 variants and lost 14, all fourteen tests of a species
constructor of the Script that the engine used to pass while ignoring
`@@species` altogether. Ninety-eight variants moved from failed to
unsupported in the same step, which is that wrong answer becoming a named
gap. The `%Function%` runs and both full runs beside the dynamic body were
measured at tree `072aa8b7f5ddea45133998ad65dbe07412dea52a`, which is the tree
of `7e8a7bb`; that step gained 324 variants and lost none. The `eval` runs and
both full runs beside 19.2.1 were measured at tree
`0f96e64ab3105aafb802a650cd4e1ebb5335e113`, which is the tree of `7cd7ebb`;
that step gained 595 variants and lost none. The `Object.prototype.toString`
runs and both full runs beside 20.1.3.6 were measured at tree
`9603dd114fe7e4d75465681c6d3d1e424c6df860`, which is the tree of `174f264`;
that step gained 92 variants and lost none, and the register engine passes
eight variants more than the stack backend on that focused pair. The
extensibility runs and both full runs beside 10.1.6.3 were measured at tree
`305e8ac720871d450eab387e090b5a95c3585293`, which is the tree of `03d1292`;
that step gained 121 variants and lost none. The `bind` runs and both full runs
beside 20.2.3 were measured at tree
`b94db3c158f13c697bcd7371825d6b616ce77c4c`, which is the tree of `3f8b9e8`;
that step gained 177 variants and lost 24. Twenty-two of the 24 read the legacy
own `caller` of a sloppy function, which no clause of the specification gives
it and which `%Function.prototype%.caller` now refuses; the other two reach
`%String.prototype%[@@iterator]`, which is a named gap of an unbuilt
Prototype. The `%RegExp.prototype%` runs and both full runs beside 22.2.6 were
measured at tree `c78edeb26ca9e93b1fc63b46b0c6a7ab4b26edbd`, which is the tree
of `81433dd`; that step gained 194 variants and lost none. The
`getOwnPropertyDescriptor` runs and both full runs beside 10.1.5 were measured
at tree `881bc1573c8579661c411a239ba324b83c3dacab`, which is the tree of
`c9b1598`; that step gained 79 variants and lost none. The `for`-`of` runs and
both full runs beside 14.7.5.6 were measured at tree
`37417685097fb03d4cd4e069f5c218dbf3cc8fe9`, which is the tree of `f9d7537`;
that step gained 128 variants and lost none. The `var` head of a nested
iteration landed in `b56e60f` after it and moved no variant of the suite, which
has no standalone variant of that shape; the engine output of `b56e60f` is
byte-identical to that of `f9d7537`. The conversion runs and both full runs
beside the positions each clause converts were measured at tree
`f3de73daea7fe93ba9f33a8b2541bc1ca7d67f81`, which is the tree of `ded2cac`;
that step gained 112 variants and lost none. The `super` runs and both full
runs beside 13.3.7 were measured at tree
`17305e626ef5862abaae8dae38698b6197c62b6d`, which is the tree of `80c7268`;
that step gained 40 variants and lost none. The `new.target` runs and both full
runs beside 9.4.3 were measured at tree
`eae44881c9c0a5f0ac537f0ccd1b94244064dcc2`, which is the tree of `9e0a0fb`;
that step gained 20 variants and lost none. A lowering that never ended was
fixed in `88c10fa` after it: a function that captures itself was followed again
on every pass of the escape walk, so `compile` never returned and no limit of
the embedding could stop it. That step moved no variant of the suite, which has
no variant of the shape, and the engine output of `88c10fa` is byte-identical
to that of `9e0a0fb`. The subclass runs and both full runs beside 15.7.14 were
measured at tree `ca30bd03d37896e9193884cee843d4fd99987461`, which is the tree
of `756642a`; that step gained 186 variants and lost none. The subclass runs
and both full runs beside the List of 7.3.15 were measured at tree
`d3e69e9aa564fe6a159eb5272a8dcd8b31d1ecad`, which is the tree of `3d10524`;
that step gained 122 variants and lost none. The `%String.prototype%` runs and
both full runs beside the receiver of 22.1.3 were measured at tree
`ace0e39c359cc0bd439c353df1290870a627ea9c`, which is the tree of `c1c4f04`;
that step gained 104 variants and lost none. The integrity-level runs and both
full runs beside 7.3.15 were measured at tree
`90443aa5d1de991eeaf06ee05961001c79e704fc`, which is the tree of `a1f9e31`;
that step gained 40 variants and lost none, and the register engine passes two
variants more than the stack backend on that focused set. The `defineProperty`
runs and both full runs beside 6.2.6.5 were measured at tree
`2f9c0bc92aed95599932eeb048aa00cf98d787a3`, which is the tree of `a3e49ee`;
that step gained 86 variants and lost none. The `Object.create` runs and both
full runs beside 7.3.25 were measured at tree
`96b8794fcaea0dfaf53a0013166a84cfce8892e3`, which is the tree of `9b84330`;
that step gained 236 variants and lost none. The `%String%` runs and both full
runs beside 22.1.2 were measured at tree
`bd25da145f8a2d680080647509b682b8acdcca8f`, which is the tree of `bcc2ce5`;
that step gained 120 variants and lost none, and the register engine passes 36
variants more than the stack backend on that focused set, which built only
22.1.2.1. The `var` pattern head of an iteration landed in `fc9a0b1` after it,
measured at tree `9597ac60049dabb11fb71f7de257764e09f9761b`; that step gained 2
variants and lost none, because the suite reaches the shape through harnesses
that need other features. The `%String.prototype%` runs and both full runs
beside 22.1.3.9, 22.1.3.12, 22.1.3.29 and B.2.2.1 were measured at tree
`8cd027ac5c29e9541f3980f874159db63c1da509`, which is the tree of `6952e99`;
that step gained 78 variants and lost none. The `%Iterator%` runs and both
full runs beside the five helpers of 27.1.3.3 that answer at once were measured
at tree `a1cabb8b96cc4d5833124056da004d5261023560`, which is the tree of
`bee51e3`; that step gained 108 variants, lost none, and moved the two
variants of `argument-effect-order.js` from a wrong failure to the named gap
of an accessor. Its full stack run classified every one of the 102,925
variants as the run beside `e6fa517` did, and its failed and unsupported
columns carry the split that run measured. The logical-assignment runs and
the full run beside 13.15.2 were measured at tree
`ad6accfb5028405655b817651c3499b3f9d12be6`, which is the tree of `d9ee5b0`;
that step gained 157 variants and lost none. The full run beside the strict
write to a property with no setter was measured at tree
`e811cbd58d69cc4c7df95a4b6865c7024847bf85`, which is the tree of `6deae2d`;
that step gained 22 variants and lost none. The label runs and the full run
beside 14.13 were measured at tree
`08e50dff7458060b0f49cba68bfcb35bc16f34c9`, which is the tree of `425421f`;
that step gained 184 variants and lost none. Eight variants it admits fail on
a gap the refusal of a label used to cover: a regular expression after a
Block, the ASI of a `let` before a Block, one answer of `decodeURI` and the
call depth of a tail call. The object-literal runs and the full run beside
13.2.5 were measured at tree `5550617b559fd3ab13da1662103edfe78f9c5b4b`, which
is the tree of `fc9e271`; that step gained 120 variants and lost none, and
moved 168 variants from unsupported to failed: a test whose own gap the spread
closed loads its harness, and the harness of Temporal and of Intl names a gap
of its own, which the runner counts as a failure of the test. The
class-element runs and the full run beside 15.7.1 were measured at tree
`6a9bb27835dfb9fa3b3645c7dae8736acfe03d6e`, which is the tree of `e3bab40`;
that step gained 361 variants and lost none, and left two variants failing on
a property name that is a `BigInt` literal.
That step gained 167 variants and lost 4: `concat` now keeps an object
element the receiver used to drop, and a `join` of one is still a gap. That step moved 868 variants from unsupported
to failed: a Script whose harness the lowering used to refuse now runs and
fails on the feature it actually needs. `RegExp.prototype[@@split]` landed in `fe5797c`
between them and moved no variant of the suite.

### Historical Test262 baseline

The preceding measurement was taken on 2026-09-12 with the same Test262
revision. It is superseded by the table above and is not the current status:

| Scope | Implementation commit | Command | Files | Variants | Passed | Failed | Unsupported |
|---|---|---|---:|---:|---:|---:|---:|
| Equality operators (focused) | `fccd5e6571261014c37fc5dc94321c8fbba8c780` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/equals test/language/expressions/does-not-equals test/language/expressions/strict-equals test/language/expressions/strict-does-not-equals --summary` | 145 | 286 | 222 (77.62%) | 0 (0.00%) | 64 (22.38%) |
| Return statements (focused) | `fab47fdb0adf3b8a7b5e9e575a4a3487e468b345` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/statements/return --summary` | 16 | 31 | 26 (83.87%) | 1 (3.23%) | 4 (12.90%) |
| Object literals (focused) | `caeb6559961fd79dd275ed4cf4f3123afc2a5a01` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/object --summary` | 1,170 | 2,252 | 684 (30.37%) | 54 (2.40%) | 1,514 (67.23%) |
| Destructuring assignment (focused) | `1f7f4879f91186dffda86812e366151a6446bcc7` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 test/language/expressions/assignment/dstr --summary` | 368 | 640 | 446 (69.69%) | 0 (0.00%) | 194 (30.31%) |
| Complete pinned suite, including staging and Intl | `fab47fdb0adf3b8a7b5e9e575a4a3487e468b345` | `sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 --all --summary` | 53,582 | 102,925 | 35,399 (34.39%) | 30,724 (29.85%) | 36,802 (35.76%) |

The full-suite baseline before that was measured on 2026-09-11 with the same
Test262 revision and command. It contained 53,582 standalone files and 102,925
executed variants: 27,965 passed (27.17%), 64,007 failed (62.19%), and 10,953
were unsupported (10.64%); 294 fixture files were not standalone tests. These
figures are retained only for historical comparison and are not the current
implementation status.

Object.prototype.propertyIsEnumerable and isPrototypeOf use the ordinary property
and prototype operations, with Symbol keys, primitive boxing and required error
ordering. Both are GC-owned mutable native Function objects. Immediate native
constructors now follow their actual prototype for inherited property lookup.
Global declaration collisions are validated before function/var definability
checks, and top-level arrows no longer incorrectly permit new.target.

The full run uses `--fuel 1000000` per realm as a bounded diagnostic baseline;
resource exhaustion is a failure, not a pass. A historical focused JSON run
covered 165 original files / 330 strict and non-strict variants: 272 passed,
54 failed and 4 were unsupported. It predates the current full-suite result
above. No expected-failure masks or feature exclusions were used.

Math.pow, exponentiation (`**`, `**=`) and the comma operator are implemented.
`crates/math` owns the allocation-free `no_std` binary64 power kernel, with exact
ECMAScript special-value classification and implementation-approximated finite
results. It uses bounded range reduction and two-component series arithmetic;
small integer powers have a fast repeated-squaring path. No platform libm or
external dependency is used. Operand expressions precede numeric conversion,
left conversion finishes before right hooks, and unary bases require parentheses.
The comma operator executes both `GetValue` operations and returns a value, not a
reference (including method-call receiver loss). Original Math.pow tests pass all
28 files / 56 variants. `BigInt` exponentiation, proper tail calls and full eval
remain unimplemented; rejected parse negatives still are not counted as passes.

The normal Function constructor now supports dynamic parameters/body compilation,
global-environment closures, construction and Function subclasses. Parameter and
body grammars are parsed independently before combined early-error validation;
comments/delimiters cannot escape a fragment into another. The synthetic
`function anonymous(...)` source is retained for Function.prototype.toString,
without introducing a lexical self-name binding. Source/copy/compile work shares
fuel and storage quotas; dynamic reentry is bounded. The embedding's
`Host::ensure_can_compile_strings` policy hook runs after string conversions and
before parsing. Lone-surrogate dynamic source and generator/async-generator constructors
remain unsupported. Direct/indirect eval remains
incomplete. Newly created dynamic functions can be called repeatedly without
recompilation; there is no source-to-bytecode cache yet.

Reflect.apply and Reflect.construct use real VM Call/Construct with array-like
argument lists, validating targets before reading length/elements. They do not
consume Symbol.iterator; getters, newTarget/prototype selection and quotas are
tested. Other Reflect methods and Proxy remain gaps.
Function.prototype now has name/length metadata and shared caller/arguments
restricted accessors using the realm's non-extensible `ThrowTypeError` function;
strict arguments.callee uses the same identity. This follows local ECMA-262
10.2.4 even where the development Node version exposes different accessors.
Initialized function properties are never recreated after deletion.

Async functions now inherit from the realm's ordinary, non-callable
`AsyncFunction.prototype`. Its constructor is a GC-owned, extensible native
`AsyncFunction` object inheriting from Function, with standard name/length and
prototype descriptors; it is not an ambient global binding. Async declarations,
expressions, arrows and methods share these intrinsics. Dynamic async construction
uses the same independently validated parameter/body compiler and policy gate as
Function, with async body grammar, Promise-returning execution and retained
`async function anonymous(...)` source. Returned functions have no Construct or
own prototype property. Subclass, Reflect.construct and bound-constructor paths
honor newTarget, while default-parameter/body errors reject the returned Promise.
Await, GC and job deferral reuse the existing VM. Native Function ancestor
enumeration now handles Function's metadata and continues along its prototype.
Cross-realm fallback and other missing language constructs remain unsupported.

Exact source text uses one shared `Rc` string per parsed input and a UTF-8 range
per `FunctionCode`, rather than copied nested source substrings. Lexer end offsets
exclude trailing trivia while preserving internal comments, CR/LF/LS/PS and
Unicode content. Classes keep the complete class text for explicit or implicit
constructors; static methods exclude the static modifier. Nested functions in
dynamic Function parameters and bodies keep their fragment source. Source text
survives code reuse, closures, async suspension and realm turns. toString output
conversion is fuel/string-budget bounded; a retained small closure can keep its
whole source input alive, bounded per compilation by `source_bytes` (not by a
realm-wide compiled-source byte quota). Function.prototype.toString is itself
an ordinary mutable native function. Not-yet-supported syntax is not claimed
through a native placeholder; its compilation still fails explicitly.

Bound functions now carry their target's immutable constructibility, support
new/Reflect.construct/super construction, ignore bound this during construction,
and substitute new.target only for the exact current bound function. Bound
argument groups are traversed iteratively and copied once from inner to outer,
for O(chain length + total arguments) forwarding work; totals are checked before
allocation and charged to fuel. Script constructors still use explicit VM frames
even when bound, so repeated new calls do not build a recursive Rust chain.
Prototype selection bypasses a bound wrapper's own prototype for ordinary new,
but honors an explicit distinct Reflect newTarget. bind captures the target's
prototype before reading length/name hooks, preserves GC roots and is itself an
ordinary mutable native Function object. Bound toString uses native syntax.

Function.prototype[Symbol.hasInstance] implements `OrdinaryHasInstance` with the
required immutable property descriptor. instanceof invokes arbitrary inherited
or own Symbol.hasInstance handlers on objects and functions, preserving receiver,
throw values and `ToBoolean` result conversion. Default bound chains are iterative;
custom recursive handlers use the existing native reentry budget. Prototype walks
and bound scans consume shared fuel; Proxy/cross-realm behavior remains open.
Math.pow also allows the original Test262 propertyHelper.js to load without
rewriting its Math.pow(2,32) calculation.

String.prototype now includes at, charAt, charCodeAt, codePointAt, concat,
indexOf/lastIndexOf/includes/startsWith/endsWith, slice/substring, repeat,
padStart/padEnd, trim/trimStart/trimEnd (with identity-preserving trim aliases),
isWellFormed and toWellFormed. These generic methods preserve UTF-16 units,
run conversions in specification order and expose ordinary mutable Function
properties. `RegExp` rejection uses Symbol.match via `IsRegExp`, including overrides.
`crates/utf16` supplies bounded KMP search and surrogate traversal with independent
tests/fuzzing; string split/replace also use the search core. No per-offset
pattern rescan is used on these string paths. Padding/repetition copy bounded
buffers by doubling. Search, scans and output copy work consume shared fuel;
allocation sizes respect string/pattern quotas. No normalization, locale case
mapping or locale comparison is implied by these methods. Arguments objects now
retain their intrinsic Object.prototype.toString brand even after mapping removal.

Array.prototype.reverse and lastIndexOf preserve holes/inherited properties and
observable get/set/delete ordering. Reverse swaps pairs without reading the middle
element or setting length; failures retain earlier observable writes. lastIndexOf
distinguishes missing fromIndex from explicit undefined, uses strict equality and
skips absent elements. Both operate over generic lengths clamped to 2^53-1 and
consume fuel per scanned pair/index; huge sparse scans are bounded, not skipped
in ways that would bypass getters. Object.values/entries snapshot key order then
recheck each descriptor and getter, protect intermediate values across GC, and
have mutable native Function metadata. Empty indexOf/includes return before
coercing fromIndex. Number now exposes the eight standard binary64 constants,
including `MAX_SAFE_INTEGER`, `MIN_SAFE_INTEGER` and `MIN_VALUE`, with readonly metadata.

Array.prototype.fill and copyWithin use the same 53-bit array-like indexing
helpers as reverse/lastIndexOf. Both snapshot length before index conversions,
clamp relative indices, return the original boxed receiver, and use strict
property writes without reading species or writing length directly. copyWithin
chooses the direction required by overlapping ranges; missing source properties
delete targets, while inherited properties and getters remain observable.
Exceptions preserve earlier writes. Work is charged per visited index, including
holes; huge generic lengths fail under fuel limits rather than overflowing.
Array.prototype[Symbol.unscopables] is a configurable, non-writable property with
a mutable null-prototype record and the names required by the loaded ECMA draft.
This metadata does not implement the absent methods or with-statement semantics.
Proxy and TypedArray/resizable-buffer integration remain open. The focused
fill/copyWithin/unscopables run passes 122 of 132 variants; all ten failures
require those missing Proxy/TypedArray facilities.

Array.prototype.at converts length before the relative index, including on empty
receivers, and performs no property access for out-of-range indices. The generic
find/findIndex/findLast/findLastIndex methods snapshot length and use ordinary
Get for every visited index, including holes and inherited getters. Predicates
receive the fetched value, numeric index and boxed receiver; their truthiness
selects the original fetched value or index, with no second Get after callbacks.
These methods have mutable GC-owned Function metadata and are nonconstructible.
Their private bytecode bodies share VM fuel, frames, roots and exception handling,
including through bound calls and Reflect.apply. They do not allocate index lists
or recurse on the Rust stack for nested callback searches. The focused five-method
Test262 run initially passed 162 of 206 variants. With splice, eight additional
mutation tests pass; the remaining 36 variants require TypedArray/resizable-buffer
functionality. No tests are excluded. Proxy integration is also still absent.

Array.prototype.splice supports insertion, deletion and replacement over 53-bit
array-like lengths. It converts length/start/deleteCount in order, distinguishes
omitted arguments from explicit undefined, checks the resulting 53-bit length
before species lookup, and shares same-realm `ArraySpeciesCreate` with concat.
Species receives the deletion count and may return a non-array object or even
the source itself. Result elements are created as own data properties, preserving
holes; result length is set before shifting. Shrinking moves forwards and deletes
the tail backwards; growing moves backwards. Every visited index consumes fuel,
with no speculative holes shortcut, and strict writes preserve prior mutations
when later operations throw. Ordinary arrays still reject lengths above 2^32-1;
the generic receiver itself is not limited to that range. Mutable Function
metadata, GC roots, model tests and a dedicated source fuzz seed cover the path.
The original splice directory passes 146 of 162 variants: ten require Proxy,
two require the absent global parseInt, and four require another realm.
With numeric parsers installed, the two parseInt-dependent variants also pass;
Proxy and cross-realm cases remain open.

Map/filter validate callbacks before species lookup, then retain the result across
the private bytecode callback body and GC. They do not set output length at the end;
slice creates result properties in ascending order then explicitly sets length.
All preserve holes and observable getter/species mutations. Push checks the full
resulting length before the first write, while pop retains the returned element
across the final length setter. Ordinary Array length remains at most 2^32-1;
generic receivers and species-created ordinary objects do not share that cap.
Map/filter/slice plus push/pop/join/concat/forEach/some/every/reduce/indexOf/includes
have mutable Function metadata. Math exposes the required Symbol.toStringTag.
The focused 13-directory Test262 selection passes 3355 of 3549 variants, with
178 failures and 16 unsupported. Missing Proxy/TypedArray/Date/eval and other
constructor/grammar limitations remain counted. Huge join/concat getter probes
are validated against the local spec, not V8's earlier implementation length caps.

Global parseInt/parseFloat and Number.parseInt/parseFloat share GC-owned mutable
Function identities. They perform ordinary `ToString` hooks (rejecting Symbols),
and parseInt converts radix after the string even if the string is empty. Prefix
recognition uses ECMAScript whitespace, preserves signed zero and accepts only
the longest valid digit/decimal prefix. Work is charged against the full bounded
input before scanning. Decimal conversion uses Rust's binary64 parser after
lexical recognition; no repeated prefix reparsing is performed. `crates/math`
supplies fixed-storage exact integer accumulation and one final rounding for
parseInt, Number's prefixed strings and nondecimal source literals. This fixes
previous repeated-rounding errors such as 0x200000000000011. Number now has traced
mutable constructor property storage, retaining readonly constants/prototype.
All 222 original parser variants pass with 20,000,000 fuel per realm; four long
Unicode-scan variants exceed the unchanged 1,000,000-fuel diagnostic baseline.

Number.isFinite/isInteger/isNaN/isSafeInteger are noncoercing mutable intrinsic
functions. Only primitive Number arguments can return true; wrappers and even
objects with throwing conversion hooks return false without running those hooks.
Integer classification inspects binary64 exponent/fraction bits, accepts signed
zero and huge integral Numbers, and rejects subnormals, fractions and infinities.
Safe integers are additionally bounded by 2^53-1 in magnitude. The four original
Test262 directories pass all 34 files / 68 variants at the baseline fuel limit.
Tests compare 20,000 generated bit patterns to an arithmetic oracle and cover
metadata, deletion, inheritance, binding and GC. Performance comparisons and
their limits are recorded in `docs/jrs-performance.md`.

General `RegExp` backreferences in that suite remain failures in the current
Thompson-only adapter. The new explicit authorization for `crates/regex-bt`
permits a separate bounded backtracking engine, but does not silently switch the
adapter or turn unsupported tests into passes.

The `RegExp` constructor now has traced mutable static-property storage and a
shared ordinary prototype with no matcher slots of its own. Only instances own
lastIndex; exec/test/toString and flag/source accessors are inherited mutable
intrinsic functions. Constructor call/new paths implement `IsRegExp` hooks,
same-constructor identity return, real-instance slot copying, regex-like property
reads and newTarget prototype selection before pattern/flag conversion. Subclasses
and instanceof therefore use actual prototype chains. Original flag order is kept
internally while the generic flags getter returns canonical order. Unsupported
automaton flags still fail explicitly after duplicate/invalid/conflicting flag
validation; neither the automaton nor engine-selection policy was changed.

RegExp.prototype.test respects overridden exec and validates object/null results;
toString reads and converts source before flags. Exec converts lastIndex even for
non-global/non-sticky instances but uses index zero for matching in that case.
Source rendering is quota-bounded and roundtrips slashes and line terminators.
Object.prototype.toString recognizes matcher slots, not prototype inheritance.
Generic flags reads hasIndices/global/ignoreCase/multiline/dotAll/unicode/
unicodeSets/sticky in the loaded draft's order. Node 26 reads the last two in the
opposite order, so that getter-order test is checked against the local standard
instead of being counted as a differential match. Other finite comparisons
agree with Node, including 250 generated regular-subset cases.
The full focused `RegExp` directory still has 1110 passing, 870 failing and 1776
unsupported variants; Unicode/case folding, named groups, symbol dispatch/species,
compile/escape, legacy `RegExp` statics and cross-realm semantics remain incomplete.

RegExp.prototype[Symbol.match] and [Symbol.search] now implement observable
`RegExpExec` dispatch, shared with test. String match/search consult object hooks
before converting the receiver and preserve its original value in custom calls.
Absent hooks use `RegExpCreate` (not the public constructor's `IsRegExp`/copy path),
then invoke the newly created object's Symbol method. Primitive arguments do not
consult prototype Symbol hooks, as required by the loaded ECMA draft.
Global match snapshots the flags string and retains the result Array across
callbacks; each exec is looked up anew, returned values must be Object/null,
and empty matches advance UTF-16 indices (including surrogate pairs for u/v).
This supports custom Unicode exec hooks without claiming Unicode matcher support.
Search snapshots lastIndex without coercing it, resets using `SameValue` (including
signed zero), restores only after normal completion and then reads result.index.
Throws preserve preceding observable mutations. Callback/result/previous-index
roots survive GC and per-iteration temporaries are released; infinite custom exec
loops remain fuel/output bounded. String and Symbol methods have mutable metadata.
The focused four-directory run passes 314 of 340 variants; six fail for absent
eval/BigInt and twenty need unsupported regex flags/named groups. `matchAll`,
Unicode matching and cross-realm integration remain open.

String.prototype.split now dispatches an object's Symbol.split before any input
or limit coercion. The `RegExp` Symbol.split method converts input, resolves species,
reads flags, constructs its splitter (adding sticky when absent), allocates the
result and only then converts limit. Custom splitters may be ordinary objects,
non-sticky regexes or the original receiver; result captures are read by the
53-bit array-like length and copied without stringification, including missing
captures as own undefined properties. Live results/captures are GC-rooted and
per-iteration temporaries are released. Limits can terminate between captures
without reading further getters. The source scan, captures and output remain
quota/fuel bounded. A repeated-prefix failing native split has a linear-search
regression; custom exec is never silently replaced by the native fast path.
The two focused directories pass 310 of 328 variants; Date/eval/BigInt account
for eight failures and regex flags/backreferences/cross-realm for ten unsupported.
459 finite cases agree with Node, including 400 generated cases. A custom v-flag
splitter uses code-point advancement per the loaded spec; Node 26's code-unit
behavior there is not used as the oracle. No Unicode pattern engine is claimed.

String.prototype.replace now dispatches Symbol.replace before receiver/search/
replacement conversion and has mutable Function metadata. Without a hook it uses
the bounded string-search kernel, not implicit `RegExp` detection. The intrinsic
`RegExp` Symbol.replace collects actual exec result identities before reading
length/captures/groups or invoking replacement callbacks. Global match strings
are read once during collection for empty-match advancement and again during
replacement, preserving mutations and duplicate result identities. Out-of-order
matches still run hooks, but their output is ignored. Results and named groups
are rooted across GC; match/capture/argument/string quotas and fuel bound work.
The shared `GetSubstitution` adapter supports numbered, unmatched and named custom
captures, literal dollars, prefixes and suffixes, with bounded output appends.
Missing closing delimiters are scanned only once, avoiding quadratic suffix scans.
Custom string-valued groups are boxed per the local spec (Node 26 differs on their
indexed properties); that case is a specification test, not a differential pass.
Named pattern groups and Unicode pattern flags remain unsupported by the engine.
The focused two-directory run passes 226 of 246 variants: two fail on the absent
`BigInt` syntax, two on mutating Array constructor properties before replace runs,
and sixteen require unsupported regex features. 566 finite cases agree with Node,
including 500 generated templates/patterns/custom results. No matcher was changed.

Array constructor properties now have mutable traced storage, including the real
configurable Symbol.species accessor and ordinary Function objects for isArray,
of and from. Deletion/redefinition remains visible to species consumers and
subclasses; literals/direct construction and static factories do not consult
species. Array.of calls a constructor receiver once with the length, or creates
an ordinary Array for nonconstructors, defines own elements without invoking
inherited setters and sets length last. Partial changes survive later exceptions.
Array.from validates the mapper before reading the iterator method once. Iterable
construction receives no length argument; array-like construction receives the
53-bit `ToLength`. The former caches next and observes live iteration; the latter
snapshots length and materializes holes through Get. Mapper and property-definition
errors close an acquired iterator while preserving the original thrown value;
step/done/value and final-length failures do not close it. Native temporaries and
pending thrown values remain rooted during hooks and iterator return. Fatal
resource/host errors are not catchable JavaScript completions and do not close.
Both branches consume fuel and ordinary heap/property/frame limits. The combined
focused factory/isArray/species/replace regression selection passes 172 of 190
variants; `ArrayBuffer`, Date, Math.cos, Proxy, unsupported bindings and cross-realm
features account for the remaining 14 failures and 4 unsupported. Array.fromAsync
remains a separate gap. 362 finite factory cases
agree with Node, including 250 generated sparse iterable/array-like cases.

Array iterator next-index slots now cover the full 53-bit `ToLength` range, with
live length conversion on each next call and no eager read during creation.
Length errors leave the index unchanged; element getters run after increment.
Reentrant next calls use the index snapshot from before the length getter,
following the explicit slot algorithm in the loaded draft, not a generator
reentry lock. Completion permanently clears the iterated source. String iterators
retain code-point traversal, while borrowed Array values traverses code units.
Iterator methods and next functions are cached mutable intrinsic Function objects;
the Array values/@@iterator identity also supplies arguments-object iteration,
even if the public aliases are subsequently replaced or deleted. GC roots include
those intrinsic identities and the still-active source. Internal slot boundary
tests exercise 2^32 and 2^53-1 without exposing a fast-forward API or unbounded scans.
The focused six-directory Test262 run passes 106 of 144 variants; all 38 failures
require TypedArrays/resizable buffers. 331 finite cases agree with Node, including
300 generated mutation sequences. `TypedArray` validation, Iterator helpers and
general generator/async-iterator machinery remain incomplete.

Array.prototype.toSorted shares the stable bounded merge-sort core with sort.
Comparator validation precedes receiver boxing and length access for both methods.
Sort collects only present properties before comparison, writes in ascending order
and deletes trailing holes, preserving observable partial effects on failure.
`toSorted` reads every index (including holes), creates an ordinary Array before
indexed reads, materializes undefined holes and never reads constructor/species or
invokes inherited setters on the result. Neither method writes source length.
Input snapshots and sort temporaries are GC-rooted across comparator/default-string
hooks; scans, comparisons and writeback consume fuel. Large generic sort receivers
follow the same observable Get/Has sequence under fuel bounds; copying lengths
above 2^32-1 fail at `ArrayCreate` before element access. Method metadata is mutable.
The two focused Test262 directories pass 137 of 149 variants; twelve fail because
of absent BigInt/TypedArrays. 435 finite reference cases agree with Node, including
400 generated sparse/stable-key cases. A poisoned inherited result setter remains
a local specification test because Node 26 invokes it for toSorted despite the
loaded draft's `CreateDataPropertyOrThrow` step. These results do not imply full
`TypedArray` or cross-realm support. See `docs/jrs-performance.md` for the paired
before/after timing protocol and remaining performance uncertainty.

The remaining copying Array methods toReversed/with/toSpliced are implemented.
They snapshot the 53-bit array-like length and create ordinary Arrays with own
data properties, not species results. toReversed reads source indices descending;
with validates a relative index then omits Get at the replaced index; toSpliced
converts start/skipCount before allocating and never visits removed indices.
Only retained holes become own undefined entries. No method writes the source or
its length, invokes iterator/species hooks or inherited result setters. Large
toSpliced inputs can produce small results by skipping a huge removed range;
result lengths above 2^32-1 still throw `RangeError` before any element reads, and
length arithmetic above 2^53-1 throws `TypeError`. Loops and output storage retain
the normal fuel/property/heap bounds and GC roots. Source mutations in conversion
or getters remain observable in the specified order. Mutable metadata and the
existing unscopables record are covered (with is intentionally not unscopable).
All 136 original variants in these three Test262 directories pass. A sparse
vector model and 453 finite reference cases (400 generated) verify output values,
presence, source preservation and boundary/error behavior. Browser/cross-realm
and `TypedArray` variants remain distinct unimplemented requirements.

Array.prototype.flat/flatMap share an iterative depth-first flattening path.
Per-source frames hold a length snapshot, index and remaining depth; each source
is rooted until its frame is popped. Only actual Arrays are flattened, without
iterator or isConcatSpreadable lookup. Missing properties are skipped (including
inherited checks); flatMap maps only present top-level entries before flattening
one level. Depth/mapper validation occurs before same-realm species construction,
and output uses own data properties without a final length Set. Custom targets,
aliasing, getter mutations and partial writes on exceptions stay observable.
The explicit frame count shares Limits.nesting across reentrant flatten calls;
cyclic/infinite-depth traversal fails under frame/fuel limits instead of consuming
unbounded Rust stack. Pending outputs and children survive callback GC, and
completed sibling temporaries are released. Input/output indices use the 53-bit
range while actual Array storage retains its standard length bound. The focused
two-directory Test262 run passes 77 of 85 variants; all eight failures require
Proxy or `TypedArrays`. 472 finite reference cases agree with Node (400 generated
nested sparse inputs), alongside an independent sparse-vector model and explicit
GC/cycle/cumulative-quota tests. No external dependencies were added.

JSON follows local ECMA-262 §25.5, including `parse`, `stringify`, reviver
context.source and `rawJSON`/`isRawJSON`. The dependency-free `crates/json/`
core parses strict UTF-16 into a bounded flat arena with source ranges and
quotes strings with well-formed lone-surrogate escapes. The adapter supplies
ordinary objects/arrays, duplicate-key replacement (including `__proto__`),
reviver mutation snapshots, toJSON/replacer callbacks, property-list filtering,
wrapper coercion and indentation. Serialization streams to a single bounded
buffer; excluded object properties do not allocate member prefixes. Native
temporaries and parse snapshots are traced through GC. JSON traversal shares
a hard nesting budget of 48 across reentrant JSON calls; parser nesting is
also bounded. Fuel, string, node, property and heap limits remain fatal resource
errors, not catchable language failures. `BigInt` and Proxy are still absent,
so their JSON interactions are not claimed as conformant. JSON methods now use
GC-owned native Function objects: extensibility, prototype changes, name/length
deletion and redefinition are ordinary object operations. Other intrinsic
functions still using immediate Builtin values retain that mutability gap.

JSON validation includes all 65536 individual UTF-16 code-unit quote/parse
roundtrips, independent `json_codec` fuzzing, hook/GC/limit regression tests,
and 79 terminating JSON comparisons with Node with no observed mismatch.
Node is a development comparison tool, not a build or runtime dependency.

For this host algorithm, the HTML Living Standard sections 8.8 (queueMicrotask),
queue a microtask, perform a microtask checkpoint and report an exception, plus
Web IDL callback conversion/invocation, were read from verbatim downloads in
`target/web-standards/`. Sources and retrieved hashes are recorded in
[the host references](../../docs/whatwg/README.md). They are documentation, not
runtime/build dependencies. Concat follows local ECMA-262 10.4.2.3 and 23.1.3.2.

Symbol differential probes against Node v26.5.0 agreed on 84 isolated scenarios.
One divergence is retained according to the local specification: Object.prototype
toString applies `ToObject` before Get(Symbol.toStringTag), so a strict getter on
Number.prototype observes a wrapper; Node's getter observed a primitive. This
case is tested against ECMA-262 20.1.3.6 rather than counted as a differential pass.

Baseline on the development host, release build: ten isolated runs of
`let sum=0; for(let i=0;i<100000;i++){sum+=i;} sum` took 58.86 ms total,
with 77.58 µs compilation and 22 bytecode instructions. Result: 4999950000.
This is a single noisy measurement, not a cross-engine performance claim.
With function frames and tracing support, the same loop measured 81.09 ms
for ten runs (92.04 µs compilation, 24 instructions). This is slower than
the primitive-only baseline; competitive speed remains an open requirement.

A Number/Number fast path now replaces pure arithmetic/comparison operands in
place, preserving normal fuel charging and bypassing no user hooks. Seven-sample
measurements showed 12.4% lower median time for the sum loop, 15.3% for mixed
arithmetic and 9.6% for simple calls. See [the measurement record](../../docs/jrs-performance.md)
and `sh tools/jrs-bench.sh` for sources, raw samples and limitations. This does
not establish competitive overall performance. Generic-path equivalence tests
cover IEEE special values, 10,000 generated bit pairs and exact budget boundaries.
