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
- `if`, `while`, three-part `for`, unlabelled `break` and `continue`.
- Untagged template literals with nested substitutions, cooked escapes, line
  normalization and string-hint conversion in evaluation order. Tagged
  templates and their raw/cooked template-object identity are not implemented.
- `instanceof` with ordinary prototype-chain semantics (without Symbol hooks),
  `switch` with strict selectors, fallthrough and shared lexical case scope,
  `for…in` key enumeration, and synchronous iterable `for…of`. For-of declarations
  support nested array binding patterns and elisions, not defaults or rest
  inside patterns. Symbol.iterator is called with the original receiver; the
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
  functions with simple, default and rest parameters, `return`, hoisting, first-class calls
  and closures over shared mutable bindings. `for (let ...)` creates
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
  iterator side effects. Object spread and rest inside binding patterns remain
  unimplemented.
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
destructured parameters,
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
Parse-negative rejections are conservatively unsupported while the parser still
combines invalid syntax with unsupported grammar. Runtime-negative errors must
occur at runtime with the expected constructor name; harness/resource failures
cannot satisfy them. Regex implementation restrictions now use fatal
`Error::Unsupported`, not catchable `SyntaxError`, so they cannot produce false
negative-test passes. Other parser/builtin completeness gaps remain open.

Object.prototype.propertyIsEnumerable and isPrototypeOf use the ordinary property
and prototype operations, with Symbol keys, primitive boxing and required error
ordering. Both are GC-owned mutable native Function objects. Immediate native
constructors now follow their actual prototype for inherited property lookup.
Global declaration collisions are validated before function/var definability
checks, and top-level arrows no longer incorrectly permit new.target.

The full run uses `--fuel 1000000` per realm as a bounded diagnostic baseline;
resource exhaustion is a failure, not a pass. The current focused JSON run covers
165 original files / 330 strict and non-strict variants: 272 passed, 54 failed,
4 unsupported. No expected-failure masks or feature exclusions are used.

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
