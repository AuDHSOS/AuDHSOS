# Web Crypto API implementation plan for jrs

Status: implementation proposal and open-work inventory, 2026-09-09.
This document does not claim Web Crypto support or authorize changes to the
project's safety policy. The runtime is **jrs** (javascript-rust).

## 1. Objective and completion boundary

Implement the Web Crypto API using the project's own Rust components, without
external software dependencies. Reuse the existing cryptographic primitives;
do not create a second collection of algorithms inside the JavaScript VM.

Web Crypto is a host API, not part of ECMA-262. Reading
[`ecma/ecma262.html`](ecma/ecma262.html) is necessary for language semantics,
Buffers, Promises and object behavior, but insufficient for this feature.
The implementation also needs Web Crypto, Web IDL, HTML task/realm rules,
Secure Contexts and the algorithm-specific standards.

There are two verification scopes, neither of which replaces the other:

1. The API and algorithm behavior specified by the selected Web Crypto version.
2. All applicable original tests at the selected WPT revision, including its
   additional and tentative specifications, real Window/Worker variants,
   secure/insecure contexts and serialization tests.

The project's broader goal of complete WPT and Test262 acceptance is unchanged.
A passing digest subset, successful round trips, or a shell-only run is not
completion. Unsupported algorithms and unavailable host environments must be
reported explicitly; they cannot disappear from the acceptance denominator.
The Web Crypto specification's extensibility does not justify shrinking the
project's requested test scope.

## 2. Evidence and specification baseline

### 2.1 Sources consulted for this plan

The following documents were retrieved and relevant sections read on
2026-09-09. Downloads are reference documents, not runtime dependencies. Their
temporary copies are under ignored `target/` storage. Preserve their original
notices and record a reviewed snapshot under the documentation conventions
before implementing each algorithm.

| Reference | Retrieved source / local copy | SHA-256 of retrieved bytes |
|---|---|---|
| Published Web Cryptography Level 2 | `https://www.w3.org/TR/WebCryptoAPI/` → `target/webcrypto-plan-spec.html` | `e211cbfe5185afba44f2e0eed7fde1d2d649cbd0a82d136cd9ff360e58a48fa5` |
| Web Crypto editor's draft | `https://w3c.github.io/webcrypto/` → `target/webcrypto-plan-editor.html` | `0a7ea877940f255d9f9b6477a2be55a0ddf7319fa194008fadc41e8b2a0e9a6e` |
| Secure Curves proposal | `https://wicg.github.io/webcrypto-secure-curves/` → `target/webcrypto-plan-curves.html` | `62290cc7371022f896169a52f82389c9250e1b036bc5f4f2512ceefb8182d159` |
| Modern Algorithms proposal | `https://wicg.github.io/webcrypto-modern-algos/` → `target/webcrypto-plan-modern.html` | `42864441ef777e0cf67759adc2499e707fd15c41e24ae4f610a9c040d1a466b9` |

The published URL resolved to the **First Public Working Draft of 22 April
2025**, whose dated URL is
<https://www.w3.org/TR/2025/WD-webcrypto-2-20250422/>. It must not be described as
a final Recommendation. The editor's draft and WPT IDL have differences; for
example, the inspected WPT `deriveBits` length parameter carries `EnforceRange`.
Version differences must be resolved in an explicit conformance matrix, not by
silently mixing algorithms from different editions.

Consulted Web Crypto sections include Crypto (§10), CryptoKey (§13),
SubtleCrypto and its task source (§14), algorithm normalization (§18.4),
algorithm registrations (§§20–34), cached objects, and key-format mappings.
The existing [host standards inventory](whatwg/README.md) identifies the local
Web IDL, HTML and DOM sources. Retrieve and read any missing normative NIST,
RFC or Secure Contexts document before implementing from it. This plan is not
a substitute for those algorithm specifications.

WPT reference revision:
`7926f3ca1cd9f7f1df88db0bb0cee14f278a559b`.
The root `WebCryptoAPI/` listing was inspected through GitHub's contents API;
its saved response is `target/webcrypto-plan-wpt-directory.json`, SHA-256
`2d35ce1f074954b3c53c2eb237ea328290c1cfdcef18a35560b3359aaac998ca`.
The root listing is **not** a recursive test inventory or a test run.
Original `getRandomValues.any.js`, tentative IDL/supports tests and
`interfaces/webcrypto.idl` were also inspected. No Web Crypto tests were run
while preparing this document, and no new pass count is claimed.

### 2.2 Current implementation, not intended capability

| Area | Observed state | Consequence |
|---|---|---|
| [jrs dependencies](../crates/jrs/Cargo.toml) | No Crypto-Crate dependency or Web Crypto adapter | API installation and dispatch are new work |
| [Realm/Host](../crates/jrs/src/vm/realm.rs), [VM](../crates/jrs/src/vm.rs) | Persistent realm, explicit host callbacks, retained handles, Promise jobs and host-driven timer tasks | Reuse boundaries and checkpoints; add a separate crypto completion path |
| [Runtime limits and gaps](../crates/jrs/README.md) | TypedArrays/resizable buffers, BigInt, communicating realms and browser workers remain incomplete | BufferSource and full host-test coverage are prerequisites, not wrappers around ordinary Arrays |
| [DOMException](../crates/jrs/src/vm/dom_exception.rs) | Branded exceptions and name/code handling exist | Reuse, then verify all Web Crypto errors and any newer exception interfaces expected by WPT |
| Secure-context/origin model | No complete browser context model | Do not expose secure-only APIs everywhere to make tests pass |
| CryptoKey / structured serialization | No CryptoKey internal representation or its clone integration | Introduce opaque key storage and realm-local wrappers |

This inventory is a point-in-time source inspection in a changing worktree.
Revalidate it before implementation. Existing passing jrs tests are not evidence
that CryptoKey, CSPRNG integration or private-key operations are implemented.

## 3. Reuse map and algorithm work

The existing track is described in
[Cryptography and TLS](11-cryptography-and-tls.md). It intentionally supports
TLS-oriented operations, with verification-only asymmetric product APIs.
Web Crypto expands the threat model to attacker-directed operations on secrets.

| Component | Reusable foundation | Missing work / restriction |
|---|---|---|
| [`crypto-ct`](../crates/crypto/ct/README.md) | Constant-time comparison/selection and `Secret<N>` | Secret cloning and variable-size storage policy; erasure is explicitly best effort, not a guarantee |
| [`crypto-rng`](../crates/crypto/rng/README.md) | `Entropy`, `Rng`, ChaCha generator, rekey/reseed logic | Production entropy provider, capability integration, failure handling and concurrent/fork lifecycle |
| [`crypto-hash`](../crates/crypto/hash/README.md) | SHA-256/384/512, HMAC, HKDF | SHA-1 compatibility support and PBKDF2; Web Crypto parameter/default/length rules |
| [`crypto-aead`](../crates/crypto/aead/README.md) | AES-128/256 encryption, GHASH, AES-GCM, ChaCha20-Poly1305 | AES-192, block decryption, CBC/CTR/KW modes, generalized GCM and API format handling |
| [`crypto-rsa`](../crates/crypto/rsa/README.md) | Public keys, PKCS#1 v1.5 and PSS verification, MGF1 | Key generation, private-key import, signing, OAEP, configurable PSS salt length and broader parameters |
| [`crypto-ec`](../crates/crypto/ec/README.md) | X25519, Ed25519 verification, P-256/P-384 verification and point validation | Production secret-scalar operations/signing, key generation, ECDH, P-521 and relevant extension curves |
| [`crypto-bignum`](../crates/crypto/bignum/README.md) | Bounded public-value arithmetic | Explicitly **not constant time**; must not process RSA private exponents or secret ECDH/signing intermediates |
| [`audhsos-der`](../crates/net/der/README.md) | Borrowing, bounded ASN.1 DER reader | Dedicated SPKI/PKCS#8 key codecs and production writer; reconcile each format's acceptance rules |
| [`audhsos-encoding`](../crates/encoding/src/base64.rs) | Strict standard Base64, PEM helpers | Explicit Base64url profile for JWK; do not globally weaken the existing Base64 decoder |
| [`audhsos-json`](../crates/json/README.md) / jrs JSON | JSON parsing/quoting | JWK schema and Web IDL dictionary semantics; `importKey("jwk", ...)` takes an object, not JSON text |

### 3.1 Core algorithm matrix

For every row, implement only the operations registered for that algorithm,
and test invalid operations too. Key generation, import/export and usages are
part of support, not optional follow-up polish.

| Algorithm family | Work required for Web Crypto |
|---|---|
| SHA-1/256/384/512 | Digest adapter and byte snapshots; add SHA-1 when required by the selected compatibility target. SHA-1 availability is not a recommendation for new security designs. |
| HMAC | Secret-key generation/import/export, hash binding, optional/default length and bit-length rules, signing and constant-time verification. |
| HKDF | Raw base-key handling, hash/salt/info normalization, output-length rules, nonextractable derivation keys and `deriveKey` composition. |
| PBKDF2 | New HMAC-based implementation with iteration/block accounting, correct zero/overflow rejection, output limits and cooperative execution. Never silently reduce iterations. |
| AES-GCM | Existing API accepts a 12-byte nonce and 16-byte tag only. Add general IV processing and permitted tag lengths (32, 64, 96, 104, 112, 120, 128 bits), AES-192, AAD and counter/message limits, `ciphertext || tag` output and authenticated failure handling. Keep the narrow TLS API's contract intact. |
| AES-CTR | 16-byte counter, specified counter-bit width 1–128, carry only in that field, wrap/message limits; encryption and decryption. |
| AES-CBC | AES inverse rounds, 16-byte IV, required padding and uniform decryption failure handling; no exposed padding-error detail. |
| AES-KW | Key wrapping/unwrapping algorithm and integrity check, length restrictions, all supported AES key sizes; not an alias for GCM. |
| RSASSA-PKCS1-v1_5 | Verification compatibility review, production signing, generation/import/export and hash-bound keys. Existing certificate acceptance restrictions must not silently become Web Crypto rules. |
| RSA-PSS | Caller-supplied `saltLength`, including zero and boundary cases. Existing verification fixes salt length to digest length. Add secure signing and key lifecycle. |
| RSA-OAEP | New encrypt/decrypt paths, hash/MGF1/label rules, limits, blinding and failure-oracle resistance. |
| ECDSA | P-256/P-384/P-521, secure signing and key generation, curve/hash rules and fixed-width `r || s` signatures rather than certificate DER signatures. |
| ECDH | Secret-scalar multiplication for named curves, validated peer points, bit-length rules and derived-key construction. Existing public verification multiplication is not a secure substitute. |
| Ed25519 | Reuse verification only after checking the API's exact invalid-encoding rules; build reviewed production signing/key generation rather than enabling `test-signing`. |
| X25519 | Reuse the secret-scalar ladder; add key lifecycle, format validation, all-zero shared-secret handling and required derivation-length behavior. |

Review RSA size/exponent limits: the current public arithmetic supports up to
4096 bits and carries shape restrictions. Identify which WPT vectors and valid
API inputs exceed those assumptions. Such limits must remain visible limitations
until resolved, not be counted as conformance passes.

### 3.2 Extensions already relevant to the broad WPT goal

The inspected WPT directory contains `encap_decap`, `getPublicKey.tentative`,
`supports.tentative`, `supports-modern.tentative` and tentative IDL coverage.
An implementation of the core table alone cannot establish full acceptance.

- Secure Curves: inventory X448/Ed448 tests and parameters, including Ed448
  context behavior. These primitives are not supplied by the current EC-Crate.
- Modern API surface: encapsulate/decapsulate Bits and Key operations,
  `getPublicKey`, static `SubtleCrypto.supports` overloads, extra key usages,
  encapsulation result dictionaries and raw-public/private/seed/secret formats.
- The inspected `supports-modern` test explicitly names ML-DSA-44/65/87,
  ML-KEM-512/768/1024, hybrid KEMs MLKEM768-P256, MLKEM768-X25519,
  MLKEM1024-P384, and ChaCha20-Poly1305. A support query must be consistent with
  implemented operations, not return hard-coded success while calls fail.
- The retrieved living Modern Algorithms draft additionally describes SLH-DSA,
  AES-OCB, SHA-3, cSHAKE, TurboSHAKE, KangarooTwelve, KMAC and Argon2. Recursively
  inventory the pinned tests and their cited versions before committing the
  exact extension matrix. Do not assume every new draft section is exercised
  by the older WPT pin, or quietly omit tests that are present.

These are separate substantial implementation tracks, particularly post-quantum
algorithms and memory-hard derivation. The core can be delivered incrementally,
but the extension backlog stays open under the full-test goal.

## 4. Target architecture and ownership

The following new paths are **proposed**, not existing packages or APIs:

```text
JavaScript / Web IDL / realm and secure-context policy
  crates/jrs/src/vm/webcrypto/       bindings, normalization hooks, Promise/GC bridge
                 |
  crates/crypto/webcrypto/          typed operations, key store, validation
                 |                 no jrs::Value, Realm or JavaScript callbacks
       +---------+---------+
       |                   |
  existing crypto-*    crates/crypto/key-format/
  primitives          SPKI / PKCS#8 / JWK byte and field validation
       |                   |
  crypto-rng/ct       audhsos-der / audhsos-encoding

Host boundary: entropy + secure-context policy + bounded executor/completion queue
```

Use `no_std + alloc` for reusable logic where appropriate, and preserve
`#![forbid(unsafe_code)]` in safe Crates. The core API accepts owned byte
snapshots, normalized Rust parameter types and opaque key handles. A synchronous
backend is useful for unit tests; the JavaScript API still follows its specified
Promise/completion semantics and must not block an event loop with unbounded work.

Primitive fixes belong in existing Crates; key codecs and common operation
validation belong in reusable Crates. ASN.1 or cryptographic math must not be
hidden in `support/`, the VM, or unrelated encoding modules. Encoding owns the
generic Base64url codec, not cryptographic key permissions.

When packages are actually introduced, update workspace manifests and locks,
xtask dependency/layering policy, crate catalog, coverage registration and
dedicated fuzz targets. There must be no registry dependency, subprocess
OpenSSL backend, copied third-party implementation, or implicit platform crypto
library. Platform entropy access is a capability, not permission to replace
the requested Rust implementation with a platform Web Crypto service.

## 5. Runtime and binding prerequisites

### 5.1 Binary storage

- Implement ArrayBuffer backing stores and branded TypedArray/DataView objects
  with checked byte offsets, element lengths and aliasing. Ordinary JS Arrays
  are not BufferSource substitutes.
- Implement the relevant integer and floating TypedArrays. The inspected random
  test constructs Float16Array/Float32Array/Float64Array/DataView to test rejection,
  and accepts BigInt64Array/BigUint64Array; missing constructors are still failures.
- Add detachment and out-of-bounds/resizable-view handling. Track SharedArrayBuffer
  separately: apply the chosen IDL's shared/resizable admission rules, never
  indiscriminately accept every buffer or read shared memory as an ordinary Slice.
- Provide checked read-copy and write operations for exactly the view window.
  Test that bytes outside a subview are untouched. Detached or resized buffers
  must not leave Rust Borrows or stale backing-store pointers alive.
- Copy `data`, signatures and BufferSource algorithm members at each operation's
  specified step. Getters can mutate buffers during normalization: one global
  rule such as “copy everything last” is wrong. No borrowed JS data may survive
  into background work.

### 5.2 Crypto objects and normalization

- Install `crypto` with the required realm identity and descriptors, and real
  `Crypto`, `SubtleCrypto` and `CryptoKey` interface/prototype brands. Cover
  name/length, extensibility, inherited methods, wrong receivers and constructor
  behavior through original IDL tests.
- Preserve the exposure distinction: `crypto`/`getRandomValues` are not blanket
  secure-context-only; `subtle`, `randomUUID`, SubtleCrypto and CryptoKey have
  secure-context requirements. A host-supplied trusted context record must decide
  exposure. A JavaScript property named `isSecureContext` must not grant authority.
- Normalize algorithms per operation with canonical names and the specified
  case-insensitive name matching. Convert dictionaries with required/default
  members, nested hash identifiers, sequences, enums and `EnforceRange` rules.
- Preserve Web IDL member-access order and the multiple dictionary conversions
  required by normalization. Do not memoize away an observable getter because
  both dictionaries happen to contain `name`.
- Build a per-method error-order table: argument/brand conversion, snapshots,
  normalization, key checks, crypto failure and result publication. Follow Web
  IDL's rules for Promise-returning operations; do not turn every failure into
  a synchronous throw or every resource failure into a rejection.

### 5.3 CryptoKey and secret ownership

Each key needs trusted internal type, algorithm, extractability, usages and
opaque material handle. Public properties are not permission storage.
The inspected spec includes cached `algorithm` and `usages` objects: repeated
access must obey the cache semantics, while mutation of a returned object must
not alter the internal algorithm or grant usages. Preserve algorithm-specific
key-pair rules, including different public/private usages and extractability.

Keep raw secret material outside ordinary JS object properties, diagnostic
formatting and freely cloned `Value` data. Use generation-checked handles and
an ownership model that supports pending operations and permitted cross-realm
cloning. Do not retain every generated key forever merely because a Host once
received it. Collection, failed import, canceled work and realm destruction must
release storage and clear sensitive temporaries according to the documented
erasure policy.

`extractable: false` forbids export through the relevant API paths; it does not
prevent authorized use or automatically forbid structured cloning. Implement
CryptoKey serialization/deserialization as specified, including independent
realm-local cached objects. Structured cloning is not JSON export and must not
leak raw bytes or serialize Rust addresses. Do not promise that nonextractable
keys survive arbitrary same-process memory disclosure.

## 6. Entropy, jobs, errors and resource security

### 6.1 Synchronous random APIs

`getRandomValues` must validate the actual TypedArray kind before the size
quota, accept only the integer kinds listed by the spec, enforce **65,536 bytes**
per call, fill the selected view and return that same object. Distinguish Web
IDL rejection from `TypeMismatchError` and `QuotaExceededError`. Check whether
the selected WPT revision expects the newer QuotaExceededError interface in
addition to its legacy DOMException behavior.

`randomUUID` needs 16 secure random bytes, version-4 and variant bits, and the
specified lowercase hyphenated representation. A common UUID formatter can be
reused outside jrs if one is introduced.

The present RNG-Crate documents no production entropy source. Complete the
platform work described in [document 13](13-the-network-on-the-machine.md),
or a suitable explicit development-host entropy capability, before claiming
secure randomness. No fixed seed, timestamp, counter or `Math.random` fallback.
Handle unavailable entropy, short fills, reseed failure, request exhaustion and
fork/snapshot/concurrency lifecycle without publishing predictable or partially
initialized results. Test doubles remain test-only. Statistical tests do not
prove cryptographic unpredictability.

### 6.2 Crypto operations and completion

The crypto task source is distinct from Promise microtasks and timers. The
specification does not require globally ordered completion of independent
operations. Do not assert FIFO completion unless an actual dependency requires it.

1. On the VM thread, perform specified conversions/snapshots and create the
   appropriate native Promise without consulting an overridden global Promise.
2. Reserve input/output/key/job budgets before expensive allocation or submission.
3. Execute only normalized Rust data in the backend. Long operations need bounded
   cooperative slices or an explicit host executor; a Promise around a blocking
   RSA/PBKDF2 call is not asynchronous scheduling.
4. Return a generation-checked operation ID plus an owned result/error through
   a bounded completion queue. Worker threads must never access `Rc`-owned realm
   state, JS objects or raw VM handles.
5. On the owning realm's task turn, validate liveness, allocate branded results,
   settle once, and run the existing Promise microtask checkpoint.
6. On teardown, discard late completions safely and release keys/snapshots. Do
   not invent a JS AbortSignal parameter for standard methods that have none.

Existing `Host::call` cannot synchronously reenter a borrowed Realm. Define a
dedicated submission/polling contract instead of calling back into JS from a
Crypto-Crate or hiding completion in `setTimeout`. Embedded hosts and WPT must
both drive the same completion mechanism.

### 6.3 Errors and quotas

Map validation failures to the specified TypeError/DOMException/rejection or
`verify` false result. In particular, invalid key usages/types differ from an
incorrect signature; authentication or padding failures must not expose backend
detail. Never return partial unauthenticated plaintext.

Define limits for live keys, secret bytes, input snapshots, queued output bytes,
pending jobs, RSA size/attempts, PBKDF2 work and any extension's memory cost.
Use checked arithmetic for byte/bit conversions, counters and allocation sizes.
Charge native work, not merely a single bytecode call. Resource exhaustion is
not a successful negative conformance test. Fatal host/invariant errors should
remain distinguishable from catchable language/API errors under jrs's policy.
Allocation quotas do not by themselves make the allocator fallible or prevent OOM.

### 6.4 Private-operation review gate

**Do not enable `test-signing` in the product dependency graph.** Current
RSA/bignum and EC verification paths permit data-dependent branches and are
not a secure private-key implementation. Add reviewed constant-time secret
paths, secure scalar/prime/nonce generation, RSA blinding and fault checks as
appropriate to the algorithm. Validate all attacker-controlled keys and points.

Update the existing verification-only design decisions explicitly when adding
private operations. Do not silently reinterpret document 11 as already allowing
them. Review generated machine code, memory access, variable-time arithmetic
and error behavior; source-level branchlessness and green unit tests alone are
not a constant-time proof. Arrange an independent cryptographic review before
production release.

`Secret<N>` currently uses best-effort clearing and permits Clone. Audit copies,
expanded keys, stack temporaries, allocator reuse and crash/log output. A stronger
erasure or locked-memory guarantee may require an explicitly approved platform
boundary under the existing unsafe policy; do not quietly add unsafe code to a
safe Crate or advertise guarantees the implementation does not provide.

## 7. Key formats and operation composition

- Implement algorithm-specific raw, SPKI, PKCS#8 and JWK import/export. PEM is not
  itself a standard Web Crypto KeyFormat. Extension raw formats remain distinct.
- Build SPKI/PKCS#8 codecs over the DER foundation with OID/parameter validation,
  canonical integers, size/depth limits and required trailing-data behavior.
  X.509 chain validation and TLS certificate policy do not belong in importKey.
- JWK needs Base64url with the correct no-padding representation, RSA unsigned
  integers, fixed-size EC/OKP coordinates/scalars and symmetric key bytes.
  Validate `kty`, `crv`, `alg`, `use`, `key_ops`, `ext`, public/private material
  consistency and private CRT fields per algorithm. Do not export private fields
  from a public key or accept a contradictory JWK by ignoring its policy fields.
- `deriveKey` composes permitted derivation, target key length and import rules;
  it must not bypass `deriveKey`/`deriveBits` usage distinctions.
- `wrapKey`/`unwrapKey` compose format handling and their registered crypto
  operation, preserving exportability, wrapping usages and exact error order.
  They are not calls to public, overridable JS `exportKey`/`encrypt` properties.
- Serialization tests must cover nonextractable keys without using public
  export/import as an implementation shortcut.

## 8. Work packages and acceptance gates

All boxes are open work. A package is complete only when its gate is evidenced.
The order gives usable increments without redefining final scope.

| ID | Deliverable | Depends on | Acceptance evidence |
|---|---|---|---|
| WC-01 | Pin normative sources; recursively inventory WPT, IDL and variants; record draft conflicts | — | Every selected test maps to a feature and source version; no silent exclusions |
| WC-02 | ArrayBuffer, TypedArray/DataView and BufferSource conversion/snapshot API | WC-01 | Relevant Test262 plus detachment, resize, aliasing, BigInt/Float16 rejection and view-boundary tests |
| WC-03 | Production entropy adapter and reviewed RNG lifecycle | WC-01 | OS integration, injected failures, deterministic test doubles and reseed/fork tests; no insecure fallback |
| WC-04 | Crypto interfaces, exposure policy, normalization and getRandomValues/randomUUID | WC-02, WC-03 | Original random/UUID/secure-context and applicable IDL tests |
| WC-05 | Backend core, opaque key store, quota model and crypto completion tasks | WC-02 | GC/teardown/concurrency tests, late-result rejection, bounded work, original digest tests |
| WC-06 | SHA/HMAC/HKDF/PBKDF2 and symmetric key import/export | WC-03–05 | Vectors, algorithm parameters, usages, snapshots, derivation and format tests |
| WC-07 | General AES-GCM plus CTR/CBC/KW; wrap/unwrap integration | WC-05, WC-06 | All permitted key/tag/IV/counter cases, negative/authentication tests, unchanged TLS regressions |
| WC-08 | Production private arithmetic, RSA/EC/Ed/X operations and key generation | WC-03, WC-05; reviewed policy changes | Constant-time review, vectors, invalid-key tests, format interop and complete registered operations |
| WC-09 | Structured serialization, communicating realms, Window/Worker and secure-context integration | WC-04–08 and host foundations | Original serialization/context/worker tests, with genuine host semantics |
| WC-10 | Pinned tentative/modern extension matrix and implementation | WC-01, WC-05, WC-08 | Every extension test classified and required algorithms implemented; truthful support queries |
| WC-11 | Security review, performance work and full regression acceptance | All | Evidence bundle below; no remaining required failed/unsupported/unrun tests |

WC-03 entropy and WC-02 binary storage can progress independently. Private-key
work must wait for the secret-arithmetic review boundary, but public verification
and digest integration need not wait for RSA key generation. Missing platform
entropy is not a reason to fake randomness; backend tests can use explicit doubles
while the integration remains open.

## 9. Verification, fuzzing and performance

### 9.1 Tests and execution infrastructure

Inventory the whole WPT `WebCryptoAPI/` tree: `digest`, `derive_bits_keys`,
`encrypt_decrypt`, `generateKey`, `import_export`, `sign_verify`,
`wrapKey_unwrapKey`, `serialization`, `secure_context`, `encap_decap`, root
normalization/cached-slot/random/UUID/historical tests and both IDL harnesses.
Include shared `util` files, vectors and any tests elsewhere referencing Web
Crypto. Preserve original bytes and copyright notices, fetch at the pinned
revision and verify content hashes.

The current jrs WPT shell is not a complete browser runner. Audit and implement
metadata/variant handling, URL and secure-context selection, Window/Worker
environments, IDL loading, structured cloning and required helper APIs such as
TextEncoder/Decoder. Unsupported modules, Proxy-dependent checks or realm APIs
remain runtime work, not reasons to rewrite the harness. Crypto completion must
be pumped before accepting completion; timeouts, zero results, harness errors
and unhandled rejections remain failures. Respect per-test timeout metadata
within explicit host resource budgets.

Use independent published known-answer vectors and interoperable engines as
test oracles, never runtime dependencies. Round trips alone are insufficient:
the same bug in sign/verify or encrypt/decrypt can make them pass. Verify exported
keys/signatures/ciphertexts in another implementation and import its results.
Generated keys and UUIDs need property/interoperability tests, not identical
bytes from unrelated RNGs. Use fixed entropy only in deliberately injected tests.

High-value negative cases include mutation in getters, mutation after submission,
detachment/resize, wrong receivers, forged CryptoKey objects, mutated cached
metadata, wrong usages, nonextractable wrapping, invalid JWK combinations,
malformed ASN.1, bad tags/padding/signatures, output-length overflow, exhausted
entropy, worker loss, duplicate completions and realm destruction during work.

### 9.2 Dedicated fuzz targets

Proposed targets under `fuzz/`, using `crates/support/fuzz`:

- `webcrypto_formats`: DER/JWK/Base64url/raw-key parsing, bounded allocations,
  strict error behavior and valid import/export round trips.
- `webcrypto_operations`: typed operation/parameter/key combinations, limits,
  corrupted signatures/ciphertexts and reference vectors.
- Extend primitive targets for generalized GCM, AES modes, PBKDF2 and new
  private arithmetic; test secret/public boundaries separately.
- Extend `jrs_source` and add a host-job model target for snapshots, GC,
  completion races, canceled contexts and handle generation reuse.

Seeds must include valid keys and operations to reach beyond format rejection.
Keep deterministic model replay in the project check. Fuzzing without a crash
does not establish cryptographic security, correct distributions or constant time.

### 9.3 Performance contract

Measure primitive throughput and complete API latency separately. Include
small/large digests, GCM with AAD and tag variants, PBKDF2, RSA sign/verify/keygen,
ECDH, concurrent requests, RNG bursts and format import/export. Report cold and
warm initialization, copies, peak live bytes, queue depth, cancellation cleanup
and event-loop responsiveness. Keep required input snapshots; optimize their
implementation rather than removing observable semantics.

Reuse expanded keys and hash states only where algorithm/key lifetime permits;
cache entries need byte limits and must be cleared with the key. Compare the
same algorithms, key sizes, parameters and correctness workload against a
reference engine. Use paired alternating measurements, record every sample,
and do not run fuzzing or full checks concurrently. Hardware acceleration, if
later needed, belongs behind an audited platform boundary with a tested portable
fallback; no new unsafe allowance is implied by this plan.

### 9.4 Completion evidence

Before declaring Web Crypto complete, require:

- [ ] Source/IDL/algorithm/version matrix and complete WPT inventory checked in.
- [ ] Every required operation/format/parameter implemented, not merely advertised.
- [ ] Original Web Crypto tests pass in every required environment and variant;
      no hidden failure masks, unsupported counts or missing completion callbacks.
- [ ] Buffer, BigInt, Promise, object and other prerequisite Test262 tests pass;
      broader Test262/WPT goals remain separately accounted for.
- [ ] Production entropy proven available, with tested failure behavior.
- [ ] No product `test-signing`/test-RNG feature and no secret data sent through
      public-only variable-time arithmetic.
- [ ] Key lifecycle, structured serialization, teardown, quotas and host-job
      isolation verified under GC and fault injection.
- [ ] Independent security review and documented limitations; no unsupported
      zeroization, FIPS or constant-time certification claims.
- [ ] Primitive vectors, interoperability tests, fuzz campaigns and corpus replay
      pass, with versions/seeds/results retained.
- [ ] Performance and responsiveness targets measured and agreed, not inferred
      from an arithmetic-loop benchmark or unmeasured claims of speed.
- [ ] Full project check passes, preserving dependency/layering/unsafe gates and
      at least 91% line / 86% branch coverage for each gated reusable Crate.

Use the project's wrappers for all builds and checks, for example
`rtk sh tools/xtask-check.sh --quiet`. New focused commands/targets must first be
registered; none of the proposed Web Crypto fuzz targets or backend APIs exists
merely because it is named here. This document changes no implementation,
manifest, safety-policy decision or runtime behavior.
