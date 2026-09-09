# jrs integration into Firefox

Status: draft implementation plan; no implementation is authorized by this document.
Date: 2026-09-09.

## 1. Objective and scope

Integrate jrs as an explicitly selectable JavaScript backend in a Firefox fork,
while retaining Gecko's DOM, HTML processing, CSS, layout, rendering, networking
and platform services. A successful experiment must execute page JavaScript in
jrs and operate on real Gecko DOM objects. Merely linking jrs or executing a
standalone expression inside Firefox does not meet that integration objective.

The initial deployment model keeps SpiderMonkey for the browser's privileged
code and default browsing. An experimental content environment uses jrs. A
complete replacement of SpiderMonkey, including Firefox internals, is a later
decision and workstream, not an implied result of the content experiment.

Not in scope:

- Reimplementing DOM, CSS, layout or rendering in JavaScript.
- Porting Firefox to AuDHSOS or changing Gecko's renderer.
- A WebExtension that merely invokes a separate jrs executable.
- Compiling jrs to WebAssembly and running it on SpiderMonkey as a claimed
  replacement engine.
- Silently falling back to SpiderMonkey for unsupported page scripts.
- Publishing patches, changing licenses, installing tools or changing either
  repository as part of this planning step.

No external software dependencies may be introduced into the jrs core or its
new FFI layer. Existing Firefox facilities may be used by the Gecko adapter;
Firefox itself already has external dependencies. These are separate dependency
boundaries and must remain visible in manifests and review.

## 2. Inspected baseline and evidence limits

Firefox checkout: `~/github.com/mozilla-firefox/firefox`.
Inspected HEAD: `fb95137a04eb8fe1196cb12f26b100c1e060295c`, dated 2026-09-03.
This records the local HEAD, not a verified clean worktree or upstream currency.

The jrs source is in the shared AuDHSOS worktree and is under active development.
Before implementation, record a reproducible revision for jrs and every internal
dependency. Historical test counts are not integration evidence and are not used
as an acceptance baseline in this plan.

Concrete integration surfaces identified in the Firefox checkout:

| Surface | Existing source | Observed coupling |
| --- | --- | --- |
| Rust linkage | `toolkit/library/rust/shared/Cargo.toml`, `shared/lib.rs`, `toolkit/library/rust/moz.build` | Rust components are linked through gkrust; `jsrust_shared` is an existing SpiderMonkey component, not an engine-selection interface. |
| Script loading | `dom/script/ScriptLoader.h`, `dom/script/ScriptSettings.cpp` | Script loading and execution contexts use SpiderMonkey compile, loader and context types. |
| DOM bindings | `dom/bindings/Codegen.py`, `dom/bindings/BindingUtils.h` | Generated bindings use `JSObject`, `JS::Value`, reserved slots, wrappers and JIT metadata. |
| GC and scheduling | `xpcom/base/CycleCollectedJSRuntime.h`, `xpcom/base/CycleCollectedJSContext.cpp` | Native roots, cycle traversal, job queues, rejection tracking and context lifetimes depend on SpiderMonkey. |
| Security wrappers | `js/xpconnect/wrappers/WrapperFactory.h` | Cross-origin and Xray wrapper behavior is coupled to SpiderMonkey compartments and proxy handlers. |
| Workers and messaging | `dom/workers/RuntimeService.cpp`, `dom/base/StructuredCloneHolder.h` | Worker runtimes and structured cloning use SpiderMonkey interfaces. |
| Privileged modules and debugging | `js/xpconnect/loader/mozJSModuleLoader.h`, `js/public/Debug.h` | System modules and debugger integration are also engine-specific. |

These are investigation entry points, not a complete file-change inventory.
Firefox's prescribed `searchfox-cli` was unavailable during planning. No bootstrap
or installation was performed. A full symbol/call-site inventory and platform
build assessment belong to phase F0; this plan does not assume a generic Gecko
engine abstraction already exists.

Current jrs provides a safe Rust core, opaque realm-owned values, compilation,
explicit host functions and persistent realms. The inspected `Host::call` contract
does not allow arbitrary synchronous host reentry into the borrowed realm.
Its Rust `Value`/`Rc` representation is not a stable C ABI, and its current GC is
not connected to Gecko's cycle collector. Language completeness, communicating
realms, browser scheduling and engine performance must be assessed separately.

The inspected jrs manifest lists internal `audhsos-regex`, `audhsos-json`,
`audhsos-event-target`, `audhsos-math` and `audhsos-utf16` dependencies. Recheck that
list at the revision chosen for implementation; copy neither the entire operating
system workspace nor its build policy into Firefox.

## 3. Target architecture

```text
Firefox parent / privileged code: SpiderMonkey during the experiment
                        |
              Gecko IPC and security policy
                        |
Experimental content environment
  Gecko ScriptLoader / WebIDL bindings / DOM / event loop / cycle collector
                        |
                  Gecko jrs adapter
                        |
                versioned C ABI / jrs-ffi
                        |
             jrs core + internal reusable crates

Gecko DOM -> CSS -> layout -> paint -> compositor: retained unchanged in role
```

The adapter exposes real Gecko objects to jrs. It does not translate arbitrary
JavaScript objects through JSON and does not reuse the standalone jrs Event host
as a substitute for Gecko events. There must be one authoritative DOM identity
and one browser event-delivery path in the experimental environment.

### Execution ownership

Select the backend before creating the associated script globals. Use a dedicated
experimental process/environment; do not switch engines between individual
scripts, callbacks or modules in the same realm.

Process selection alone does not solve interoperability. F0 must inventory
privileged JavaScript, extension execution worlds and other SpiderMonkey use
inside content processes. Connected globals that permit synchronous object access
need either the same backend or a fully specified cross-engine bridge. The first
experiment must reject unsupported configurations explicitly and be described as
restricted; production cannot evade web semantics by splitting related globals.

During coexistence, engines must not share raw heap pointers. Gecko IPC,
structured clone and capability-checked native identities form explicit
boundaries. Extension access, same-origin frames, navigation, BFCache and process
switching require their own ownership tests.

### Component boundaries

Proposed locations; none exist solely by virtue of this plan:

| Component | Proposed ownership and location | Responsibility |
| --- | --- | --- |
| Language runtime | AuDHSOS `crates/jrs/` | ECMAScript semantics, compiled code, execution, values, GC, interrupts and embedding hooks. |
| Reusable runtime components | AuDHSOS `crates/*` | Regex, UTF-16, JSON, math and other reusable logic; no Gecko imports. |
| C ABI adapter | AuDHSOS `crates/jrs-ffi/` | Opaque handles, call boundaries, ownership validation and conversion between ABI and core types. |
| Gecko integration | Proposed Firefox `dom/jrs/` plus affected existing subsystems | C++ adapter, browser capabilities, binding support and lifecycle integration. Final placement is agreed in F0. |
| Source packaging | Pinned, auditable in-tree source snapshot in Firefox | Explicit manifests and internal path dependencies without an absolute path to the developer's AuDHSOS worktree. |

The jrs core remains `#![forbid(unsafe_code)]`. Necessary raw-pointer operations
belong only in an explicitly reviewed FFI adapter under the project's unsafe
policy. This is a proposed policy addition, not permission to weaken the core.

## 4. Required integration contracts

### 4.1 ABI, values and host reentry

Specify the ABI before implementing Gecko-facing calls:

- Opaque runtime, agent, realm, compiled-code and value handles; ABI versioning
  and capability negotiation.
- Separate borrowed and retained roots, generation checks, invalid-handle errors,
  thread affinity and deterministic shutdown ordering.
- Lossless UTF-16 strings, including lone surrogates, and bounded buffer APIs.
- Operations for properties, symbols, accessors, calls, construction, modules,
  exceptions and source metadata. Host objects require internal-method hooks,
  not just plain object properties populated at creation time.
- A synchronous `JavaScript -> Gecko -> JavaScript` reentry design that never
  creates overlapping mutable Rust borrows. Document safepoints and root scopes
  across every callback; the existing external `Host::call` is insufficient.
- Distinct normal, throw, interrupt, OOM and unsupported results. Rust panics must
  not unwind through C++; C++ exceptions must not unwind through Rust.
- Allocator ownership and deallocation on the originating side of the ABI.

Browser-visible identity must survive repeated native wrapping. Strings and
temporary values must not be copied at every DOM access without measurement.
An attempted SpiderMonkey JSAPI compatibility layer would also need friend APIs,
GC and layout assumptions; it is not presumed to be a small shim. Prefer an
explicit backend boundary and a separate jrs binding backend where feasible.

### 4.2 DOM and WebIDL

Extend the binding generation pipeline with a jrs backend; do not maintain edited
generated files or manually duplicate all interface definitions.

Implement WebIDL conversions, overloads, dictionaries, sequences, callback
interfaces, exceptions, branded receivers, wrappers and exposure rules. Include
exotic objects such as WindowProxy and indexed/named properties, wrapper caches,
expandos, cross-realm identities and custom-element callback reentry.

The first DOM milestone may cover Document, Element, Text, selected attributes,
events and geometry queries. That is a validation subset, not the final binding
coverage. Style mutation followed by a synchronous layout query must use Gecko's
normal behavior, not a deferred rendering approximation.

### 4.3 Memory management

Design bidirectional tracing between jrs and Gecko's native object graph. The
combined system must identify cycles such as DOM node -> listener -> closure ->
DOM node, without retaining everything forever or freeing reachable objects.

Define root registration, tracing barriers, finalization/unlink order, weak edges,
wrapper-cache lifetimes and collection coordination. Do not assume jrs must copy
SpiderMonkey's GC design, but provide equivalent invariants for Gecko integration.
Include incremental collection pauses, OOM, memory pressure, realm destruction,
navigation, BFCache eviction and worker termination in validation.

### 4.4 Scheduling, threads and termination

Let Gecko own event-loop checkpoints. Supply a jrs execution mode in which a
native callback or nested evaluation does not independently drain all jobs.
Integrate Promise jobs, rejection reporting, event errors and incumbent/entry
settings without changing their order.

Define runtime ownership per thread/agent, worker startup/shutdown, structured
clone, transferable buffers, SharedArrayBuffer and Atomics. Current `Rc`-based
values may not be passed directly between threads. Rust thread-safety cannot be
obtained by merely replacing `Rc` with `Arc`.

Provide interrupts and allocation quotas appropriate for a long-lived browser.
A lifetime-cumulative shell fuel budget must not eventually disable an otherwise
healthy page. Native operations need cooperative cancellation or separate bounded
execution; a bytecode interrupt cannot stop arbitrary C++ work.

### 4.5 Security and lifecycle

Keep Gecko's principal, origin, sandbox, CSP, Trusted Types and permission
decisions authoritative. Carry the necessary context through compilation,
callbacks, module loading, host-object access and error reporting. Replacing
wrappers must preserve access checks, not only visible property values.

Validate same-origin and cross-origin frame behavior, navigation, window proxies,
privileged/unprivileged transitions, extension worlds and teardown. No unsupported
security path may default to granting access. An arbitrary JavaScript realm is
not by itself a security boundary.

### 4.6 Loading, debugging and performance

Integrate classic scripts, async/defer, modules, dynamic imports, source URLs,
line/column information, decoding, cache invalidation and browser policy checks.
Version compiled-code caches by engine/build; do not feed SpiderMonkey cache
artifacts to jrs. Preserve Gecko's network, CORS and integrity checks.

Add engine identification to tests and diagnostics. Provide stacktraces,
breakpoints, stepping, scope/object inspection, source maps, profiler markers and
memory reporting before treating the backend as usable beyond an experiment.

Measure startup, parsing, execution, DOM calls, layout-triggering workloads,
memory, GC pauses and responsiveness against the same Firefox revision with
SpiderMonkey. JIT is not a prerequisite for the first milestone; competitive
performance is a separate measured requirement, not a consequence of using Rust.

## 5. Phases and acceptance gates

All phases are planned. No checkbox below records completed implementation.

### F0 — Feasibility, boundaries and reproducible baseline

- [ ] Pin Firefox, jrs, internal crates, Test262 and WPT revisions and platforms.
- [ ] Inventory SpiderMonkey APIs used by loaders, generated bindings, wrappers,
  GC, workers, debugging and privileged content-process code.
- [ ] Decide backend/process selection, related-global ownership and coexistence
  rules; identify paths requiring substantial Gecko refactoring.
- [ ] Resolve source packaging, toolchain feasibility and licensing questions.
- [ ] Specify the ABI, tracing/reentry invariants and minimal DOM validation set.
- [ ] Establish benchmark workloads and numerical regression budgets before
  implementation results are available.

Gate: reviewed feasibility/design record with explicit unsupported environments,
security boundaries and owners for cross-cutting changes. No claim of a drop-in
replacement and no unexamined assumption that a content process contains only
unprivileged page JavaScript.

### F1 — Optional build and isolated execution

- [ ] Package jrs and its internal dependencies independently of the OS workspace.
- [ ] Add the FFI crate, explicit C header and Gecko adapter skeleton.
- [ ] Link through the existing Firefox Rust build and expose an experimental
  build switch, disabled by default. The switch name is chosen during implementation.
- [ ] Execute test scripts in an isolated native integration harness; verify
  initialization, values, exceptions, interrupts and repeated teardown.

Gate: default Firefox still builds and runs unchanged; the enabled build executes
jrs scripts reproducibly. No web-exposed entry point is enabled yet. This milestone
proves linkage and lifecycle only, not browser integration.

### F2 — Browser-grade embedding and memory safety

- [ ] Implement synchronous host reentry with audited rooting and unwind behavior.
- [ ] Supply host-object hooks and stable wrapper identity.
- [ ] Connect jrs GC to native tracing/cycle collection and memory reporting.
- [ ] Implement Gecko-controlled job queues and interrupt/termination handling.
- [ ] Test nested callbacks, forced GC, OOM and teardown under sanitizers/fuzzing.

Gate: no stale handles, cross-thread heap access, uncollectable test cycles or
premature finalization in the integration corpus. Security assumptions are reviewed
before loading untrusted content, not deferred until broad WPT execution.

### F3 — Real Gecko DOM in an experimental content environment

- [ ] Route classic page scripts to jrs in a dedicated opt-in environment.
- [ ] Generate the initial WebIDL binding subset and connect real Gecko objects.
- [ ] Validate DOM creation/mutation, native event callbacks, styles and synchronous
  layout reads; verify resulting rendering comes from Gecko.
- [ ] Implement principal checks, source metadata, errors and navigation teardown.
- [ ] Assert engine ownership in tests; reject unsupported configurations without
  re-executing scripts in SpiderMonkey.

Gate: controlled HTML pages execute in jrs, mutate real Gecko DOM and render
correctly. Negative security tests and memory-lifetime tests pass. Partial DOM
coverage is clearly labeled and is not browser-wide acceptance.

### F4 — Web-platform breadth

- [ ] Expand generated bindings to required interfaces and exotic objects.
- [ ] Add module loading, dynamic imports and complete script scheduling.
- [ ] Support frames, related realms, navigation/BFCache and cross-origin wrappers.
- [ ] Support workers, messaging, structured clone and transferables.
- [ ] Address extension worlds and any privileged SpiderMonkey code crossing the
  experimental environment; no undocumented cross-engine object bridge.
- [ ] Cover remaining ECMAScript/Intl features and WebAssembly integration required
  for the intended Firefox behavior.

Gate: browser WPT runs and Firefox integration suites have complete, reproducible
result accounting. Test262 runs against the same jrs revision. Missing features,
timeouts, crashes and skipped applicable tests remain acceptance gaps.

### F5 — Product readiness and measured speed

- [ ] Complete debugger, profiler, crash and memory diagnostics.
- [ ] Run security, fuzz, sanitizer, leak, long-session and shutdown testing.
- [ ] Evaluate startup, Speedometer, JS/DOM throughput, peak memory and GC/frame
  latency against the pre-agreed budgets and supported-platform matrix.
- [ ] Document release/rollback behavior and maintenance ownership for Firefox updates.

Gate: explicit approval for the intended deployment scope. A passing build or
selected WPT result cannot substitute for this gate.

### F6 — Optional complete SpiderMonkey replacement

Only start after an explicit decision to expand beyond the experimental content
backend. Port XPConnect/system-module integration, browser UI, extensions and
remaining SpiderMonkey-dependent code. Audit SpiderMonkey and WebAssembly usage
in every process before removing build dependencies. The F1–F5 architecture must
not be advertised as SpiderMonkey-free Firefox.

## 6. Verification and evidence

| Requirement | Required evidence |
| --- | --- |
| No new external jrs dependencies | Audited package graph, source provenance and reproducible offline-capable build inputs. |
| Real backend selection | Runtime/realm ownership assertions covering compilation, calls, modules, jobs and callbacks; no hidden fallback. |
| Safe FFI and reentry | Boundary tests, forced-GC/OOM cases, sanitizer results, fuzzing and unsafe-code review. |
| Correct native/JS lifetimes | Cycle collection tests, leak checks, navigation/worker shutdown and weak-reference behavior. |
| Security parity | Principal/origin/wrapper tests including cross-origin and privileged transitions. |
| ECMAScript completeness | Full pinned Test262 result manifest, including flags, variants, negative phases and required host APIs. |
| Browser behavior | Applicable WPT executed inside Firefox with jrs, plus relevant Gecko mochitests, reftests, browser tests and xpcshell tests. |
| Competitive speed | Repeated same-machine comparisons, distributions and memory/latency measurements; workload and threshold decisions recorded beforehand. |
| Default-build stability | Existing SpiderMonkey configuration passes its established checks with jrs disabled. |

Use Firefox's `mach` build/test workflow during implementation. Resolve exact suite
commands and supported platforms against the pinned checkout in F0. Unit fixtures
validate transport and lifecycle; they do not replace upstream conformance tests.
Browser WPT must use actual page/worker execution, not the existing jrs shell runner.

Separate test discovery, unsupported features, expected platform applicability,
execution failures and passes. No expected-failure mask changes the requirement
for complete conformance. Publish logs, revisions, build options, engine identity,
test manifests and resource limits with each acceptance report.

## 7. Decisions and blockers that must remain explicit

1. **Regex requirements:** general ECMAScript backreferences cannot be implemented
   by a pure finite automaton. The existing Thompson-NFA/DFA-only and no-backtracking
   rule remains in force. Full Test262/web compatibility cannot be claimed while
   this conflict remains unresolved. Do not add a fallback, silently exclude tests
   or specialize behavior to test inputs. Independent integration work can proceed.
2. **Licensing:** jrs currently declares `AGPL-3.0-only`; inspected Firefox files
   use MPL-2.0. Review redistribution obligations, Mozilla acceptance policy and
   rights to any proposed dual licensing before distributing a combined product or
   seeking upstream inclusion. A repository owner cannot automatically relicense
   contributions owned by others. This plan makes no legal compatibility ruling.
3. **Toolchain:** jrs currently declares Rust 1.100 and the OS workspace pins a
   Nightly; Firefox declares a workspace minimum of 1.90.0. Inspect the actual
   Firefox build toolchain and jrs feature requirements. Choose a compatible
   toolchain or a reviewed portability change; neither declaration proves a build works.
4. **Deployment:** choose whether the target is a private experiment, a maintained
   Firefox fork or a Mozilla-upstream proposal. Upstream acceptance is external
   coordination, not an outcome this plan can promise.
5. **Performance:** set numerical budgets in F0. Neither current shell benchmarks
   nor a successful FFI smoke test establish browser performance.

## 8. Recommended first implementation package

After this plan is approved, begin with F0 and a small F1 package: reproducible
source packaging, ABI/lifetime specification, optional linkage and native harness
tests. Do not first replace `JS_NewContext`, broadly rewrite `JS::Value`, or route
arbitrary webpages to jrs. Those changes depend on the F2 contracts.

Keep implementation commits separable into runtime/FFI changes, Firefox build
integration, native lifetime/reentry support, and WebIDL/ScriptLoader integration.
Update this plan's gates only when their evidence exists. Planning approval is
not approval to publish patches or to remove existing SpiderMonkey functionality.

Related local sources: [jrs](../crates/jrs/README.md),
[dependency manifest](../crates/jrs/Cargo.toml),
[test-input guidance](test-ext/README.md),
[testing strategy](06-testing-strategy.md), and
[unsafe-code policy](04-safety-policy.md).
