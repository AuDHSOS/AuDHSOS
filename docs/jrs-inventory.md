<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!-- Copyright (C) 2026 Manuel Baesler and contributors -->

# jrs: inventory before M1

Date: 13 September 2026. Scope: the state
[jrs-architecture.md](jrs-architecture.md) section 18 step 1 asks for before the
semantic contract of M1 is fixed. It records what exists, who owns it, and where
JavaScript can re-enter. It decides nothing; the decisions of section 1.2 stay
open.

Method: read from the workspace at the commit this document is added in. Every
statement below names the file it comes from.

## 1. Public surface

`crates/jrs/src/lib.rs` exports `compile`, `compile_script`, `Program`,
`Script`, `Error`, `SymbolValue`, `Value`, `ObjectValue`, `FunctionValue`,
`Host`, `Realm`, `Runtime`, `SilentHost` and `Limits`. Every other module is
private except `engine`, which is `pub mod` and therefore part of the surface
without being part of the documented API.

Two entry points compile: `compile` for `Runtime::run` and `compile_script` for
`Realm::evaluate`. They differ in one flag, `realm`, which
`bytecode.rs:491` threads into `compile_parsed`. The flag selects a different
global binding model, which is the split section 2 of the architecture names.

## 2. Ownership today

`Execution` (`vm.rs:250`) holds 60 fields in one structure: the host reference,
the limits, the operand stack, the heap, the frames, the realm owner token, and
41 fields of realm state, from `object_prototype` and `global` through the
constructor storages to the DOM event and abort prototypes. Jobs, timers,
rejections, the symbol registry, the global lexical map and the retained-handle
map live in the same structure. Three fields at its end, `register_vm`,
`register_agent` and `register_feedback`, hold the second engine.

`engine::agent::Agent` (`engine/agent.rs`) holds a `GenerationalHeap` and a
`Realm`. It is the shape the architecture asks for. It owns no jobs, no host, no
global object and no execution context stack.

## 3. Heap roots

Two heaps exist.

The stack backend's heap is marked from `mark_realm_roots` (`vm.rs:1174`) and
`mark_execution_roots` (`vm.rs:1258`): the realm fields listed above, the
operand stack, the frames, `native_roots` (`vm.rs:266`), the job queue and the
retained handles. `native_roots` is the manual "remember the length, push, later
truncate" pattern the architecture asks to be encapsulated; `vm.rs:1673` pops it
by hand.

The register engine's heap has `roots: Vec<Value>` with `push_root`,
`root_value`, `enter_scope` and `exit_scope` (`engine/heap.rs`), plus the
interpreter's register file, accumulator and context chain, which
`collect_young` (`engine/interpreter.rs:225`) passes to `scavenge_with_roots`.
Realm intrinsics are immortal objects held by `Root` indices
(`engine/realm.rs`).

The two heaps share nothing. A value cannot cross between them; `vm.rs:560`
converts a register result back with `register_primitive`, which answers `None`
for anything that is not a primitive.

## 4. Native re-entry

Rust calls back into JavaScript at four places, each guarded by the same
`native_depth >= 12` counter: `vm.rs:1982`, `vm/classes.rs:310`,
`vm/dynamic_function.rs:27` and `vm/realm.rs:464`. The counter is a Rust-stack
depth limit, not a frame budget, which is what M5 replaces.

The register engine has no re-entry at all. `RegisterVM::call_intrinsic`
(`engine/interpreter.rs`) takes `&self` and cannot open a frame, which is why
an intrinsic there cannot run a user `valueOf`, a getter or a callback, and why
the lowering refuses any call that would need one.

## 5. Two implementations of the same semantics

This is the finding that decides the order of the remaining work.

`crates/jrs/src/engine/` is 12,248 lines against 16,383 in `vm.rs` and
`vm/`. It is not a lowering of the existing semantics onto a different
instruction format. It is a second object model (`engine/object.rs`,
`engine/shape.rs`, `engine/elements.rs`), a second heap (`engine/heap.rs`), a
second string store (`engine/string.rs`), a second Realm (`engine/realm.rs`)
and a second implementation of the following clauses, each of which the stack
backend also implements:

| Clause | Stack backend | Register engine |
|---|---|---|
| 20.1.3 `%Object.prototype%`: `hasOwnProperty`, `isPrototypeOf`, `propertyIsEnumerable`, `toString` | `vm/properties.rs`, `vm/instance.rs` | `engine/interpreter.rs` |
| 22.1.3 `%String.prototype%`: 18 methods | `vm/strings.rs`, `vm/strings/` | `engine/interpreter.rs` |
| 23.1.3 `%Array.prototype%`: `at`, `includes`, `indexOf`, `join`, `lastIndexOf`, `pop`, `push`, `reverse`, `slice`, `values` | `vm/arrays.rs`, `vm/arrays/` | `engine/interpreter.rs` |
| 23.1.5 Array Iterator | `vm/iterators.rs` | `engine/interpreter.rs` |
| 6.1.5.1 well-known Symbols | `vm/symbols.rs` | `engine/realm.rs` |
| 20.5 Error and the six `NativeError` prototypes | `vm/errors.rs` | `engine/realm.rs` |
| 10.1.11.1 own-key order | `vm/enumeration.rs` | `engine/heap.rs` |

Section 1 of the architecture requires one semantics, and section 8 requires
one maintained production format rather than a collection of equal interpreters.
A register lowering is allowed, but only after an A/B proof and without a
second language semantics behind it. The register path as it stands is the
second semantics.

## 6. What the register path executes

`register_script_features` (`bytecode.rs:5691`) refuses, when `realm` is true,
every `Declare`, every `Var`, every function declaration and every lexical
block. `Realm::evaluate` compiles with `realm` true, and the Test262 runner uses
`Realm::evaluate`. **No Test262 file executes on the register engine.** The
34.56% the README reports is the stack backend's number, measured with the
register path present but unused.

Outside a realm, `lower_register_script` (`bytecode.rs:5826`) additionally
refuses any script whose completion value is not primitive
(`bytecode.rs:5734`), because `register_primitive` cannot carry an object across
the heap boundary. A script that answers an object, calls a user function as a
method, reads `this`, or touches a global therefore stays on the stack backend.

The register path is reached by `Runtime::run` for standalone scripts that lower
completely. That set is exercised by `crates/jrs/src/tests/register_backend.rs`
and by the `jrs_backend` differential fuzz target, and by nothing else.

## 7. Consequence for the plan

The migration cannot remove stack-backend code by growing the register engine's
builtin catalogue, because the register engine never becomes the production path
for a realm, and every builtin added to it is a duplicate rather than a port.
The order the architecture gives — M1 semantic contract, M2 source and code, M3
one object model and root discipline, M4 agent and realms, M5 resumable builtins
— is what makes a single semantics possible; the register lowering is M8 and
sits after them.

The pilot of section 18 remains the next step. This document is its first item.
