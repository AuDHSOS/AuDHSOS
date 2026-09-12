# 5. Code Organization

## 5.1 Repository layout

```
AuDHSOS/
├── Cargo.toml                 workspace: members, shared metadata, lints, profiles
├── Cargo.lock                 workspace members only
├── rust-toolchain.toml        pinned nightly, components, targets
├── .cargo/config.toml         `cargo xtask` alias; relocation model of the kernel target
├── LICENSE                    AGPL-3.0 text, verbatim from gnu.org
├── README.md
├── CONTRIBUTING.md
├── CHANGELOG.md
├── rustfmt.toml
├── docs/                      this document set and the decision register
│   ├── rfc/                   the RFCs, verbatim, with their checksums (D-59)
│   ├── oasis/                 what OASIS publishes, the same way (D-100)
│   ├── w3c/                   what the W3C publishes, the same way
│   ├── ecma/                  what Ecma International publishes, the same way
│   ├── itu/                   the JPEG Recommendations, kept although their terms forbid the copy (D-124)
│   ├── cipa/                  Exif, on the same footing (D-124)
│   └── pcisig/                what PCI-SIG releases only to members: the provenance rule that takes its place (D-124)
├── crates/
│   ├── regex/                 audhsos-regex: bounded Thompson NFA, no backtracking, independently audited and fuzzed
│   ├── event-target/          audhsos-event-target: bounded reusable listener registry and event flags, no runtime/DOM dependency
│   ├── timer-queue/           audhsos-timer-queue: bounded stable deadline queue without a clock or executor
│   ├── abi/                   audhsos-abi: syscall table, errors, rights, message layout, boot image header, boot information, address constants
│   ├── elf/                   audhsos-elf: ELF64 parser producing validated load segments
│   ├── uefi/                  audhsos-uefi: UEFI structure layouts, GUIDs, constants, the firmware clock conversion (no calls)
│   ├── gfx/                   gfx: framebuffer logic, bitmap font, damage tracking
│   ├── pci/                   pci: configuration space, BARs, capabilities, MSI-X, the virtio capabilities (document 13)
│   ├── sync/                  audhsos-sync: Global<T> and Preset<T> cells (unsafe allowed)
│   ├── time/                  audhsos-time: UnixTime, CivilTime, Instant, Duration (document 12)
│   ├── encoding/              audhsos-encoding: Base64, hex, PEM (document 12)
│   ├── deflate/               audhsos-deflate: the DEFLATE format of RFC 1951 in the zlib wrapper of RFC 1950
│   ├── collections/           audhsos-collections: fixed-capacity containers over indices (document 12)
│   ├── jrs/                   jrs: no_std + alloc JavaScript bytecode core; initial non-conforming subset, no OS process adapter yet
│   ├── symbols/               audhsos-symbols: ELF symbol table and DWARF line lookup (document 12)
│   ├── drivers/
│   │   ├── uart16550/         driver-uart16550: register logic over a port access trait
│   │   ├── i8042/             driver-i8042: PS/2 controller and decoder logic over a port access trait
│   │   ├── virtio-blk/        driver-virtio-blk: virtio 1.x block device logic over a register trait
│   │   └── virtio-net/        driver-virtio-net: virtio 1.0 network device logic over a register trait (document 13, Phase 14)
│   ├── support/
│   │   ├── testing/           test-support: property-test engine, builders, strategies, model-test runner
│   │   └── fuzz/              fuzz-support: fuzzer entry glue and corpus replay (unsafe allowed, host only)
│   ├── virtio/
│   │   └── queue/             virtio-queue: split virtqueue and initialization logic (document 12)
│   ├── fs/
│   │   ├── fat/               fs-fat: FAT32 over a block device trait (document 12)
│   │   └── gpt/               fs-gpt: GUID partition table over the same trait
│   ├── boot/
│   │   └── uefi-x86_64/       boot-uefi-x86_64: the loader (unsafe allowed)
│   ├── kernel/
│   │   ├── types/             kernel-types: PhysAddr, VirtAddr, PhysFrame, Page, ranges, alignment
│   │   ├── hal-api/           kernel-hal-api: HAL traits and their test doubles
│   │   ├── mm/                kernel-mm: memory map, frame allocator, page tables, mapper, address spaces, kernel stacks
│   │   ├── objects/           kernel-objects: pools, ids, handles, rights, object types, quotas
│   │   ├── sched/             kernel-sched: thread states, run queues, time slices
│   │   ├── ipc/               kernel-ipc: endpoints, notifications, rendezvous, message transfer
│   │   ├── syscall/           kernel-syscall: argument decoding, validation, dispatch
│   │   ├── core/              kernel-core: KernelState, boot sequence, memory bring-up, reactions to traps and ticks
│   │   ├── hal-x86_64/        kernel-hal-x86_64: the adapter (unsafe allowed)
│   │   ├── test-harness/      kernel-test-harness: in-QEMU test runner, serial protocol
│   │   └── bin/               audhsos-kernel: the binary; tests/*.rs are QEMU test kernels
│   ├── user/
│   │   ├── rt/                user-rt: typed handles, heap, message area, startup message, report lines
│   │   ├── sys-x86_64/        user-sys-x86_64: _start, trap instruction, the gate and its wrappers (unsafe allowed)
│   │   ├── proto/             user-proto: protocol encodings
│   │   ├── loader/            user-loader: tar reader, the segments a user ELF asks for
│   │   ├── servers/           the logic of the servers, host-tested, no system call
│   │   │   ├── name/          server-name
│   │   │   ├── console/       server-console
│   │   │   ├── memory/        server-memory
│   │   │   ├── display/       server-display: framebuffer owner, surfaces, cursor
│   │   │   ├── input/         server-input: subscribers, the two decoders, event rings
│   │   │   └── net/           server-net: the device, the stack, the sockets (document 13, Phase 14)
│   │   ├── programs/          user-programs: every program of the system as one
│   │   │   │                  binary each of one crate, because a program is a
│   │   │   │                  loop around a logic crate and seven crates of a
│   │   │   │                  loop each are seven manifests saying the same
│   │   │   │                  thing (D-97)
│   │   │   └── src/bin/       server-init (the root task), server-memory,
│   │   │                      server-name, server-console, server-display,
│   │   │                      server-input, app-hello, app-checks,
│   │   │                      app-paint, app-input, app-canvas,
│   │   │                      app-faulter
│   │   └── apps/              the logic of the applications, as servers/ is
│   │                          for the servers: host-tested, no system call
│   │       └── canvas/        app-canvas: the drawing state of the graphical
│   │                          demonstration and e2e client (Phase 11)
│   ├── crypto/                (document 11)
│   │   ├── ct/                crypto-ct: Choice, constant-time selection and comparison, Secret<N>
│   │   ├── hash/              crypto-hash: SHA-256, SHA-384/512, HMAC, HKDF
│   │   ├── aead/              crypto-aead: ChaCha20-Poly1305, bitsliced AES-GCM, GHASH
│   │   ├── bignum/            crypto-bignum: limbs, Montgomery arithmetic, a run-time modulus
│   │   ├── ec/                crypto-ec: fe25519, X25519, Ed25519 verify, P-256 and P-384 ECDSA verify
│   │   ├── dh/                crypto-dh: finite-field Diffie-Hellman over the MODP groups of RFC 3526
│   │   ├── rng/               crypto-rng: Entropy and Rng traits, ChaCha20 generator
│   │   └── rsa/               crypto-rsa: RSA verification, PKCS #1 v1.5 and PSS
│   ├── net/                   (documents 11, 12 and 14)
│   │   ├── der/               audhsos-der: strict zero-copy DER reader
│   │   ├── x509/              audhsos-x509: certificates, path validation, name matching
│   │   ├── tls/               audhsos-tls: TLS 1.3 client, sans-I/O
│   │   ├── wire/              net-wire: addresses of both families, cursor, internet checksum
│   │   ├── eth/               net-eth: Ethernet II frames, ARP, the neighbor cache
│   │   ├── ip/                net-ip: IPv4, reassembly, ICMPv4, routes over both families
│   │   ├── ipv6/              net-ipv6: IPv6, extension headers, ICMPv6, Neighbor Discovery, SLAAC
│   │   ├── udp/               net-udp: UDP datagrams, the socket table, the receive ring
│   │   ├── tcp/               net-tcp: the RFC 9293 state machine, its timers, Reno congestion control
│   │   ├── dns/               net-dns: the RFC 1035 message format, name compression, the stub resolver
│   │   ├── dhcp/              net-dhcp: the RFC 2131 client state machine, its options, the lease timers
│   │   ├── http/              net-http: HTTP/1.1 client encoding and parsing
│   │   ├── stack/             net-stack: interface, demultiplexing, poll
│   │   └── ssh/               audhsos-ssh: SSH-2 client, sans-I/O: the wire types and the binary packet (document 14, track S)
│   └── tools/
│       ├── xtask/             build, image (GPT + FAT32 writer, CRC32), run, test, lint, check-layering, check-deps, unsafe-budget, fuzz, coverage; policy tables
│       ├── markdown/          doc-markdown: the Markdown parser of this repository's documents
│       ├── html/              doc-html: the HTML parser of the standards this repository holds
│       ├── jrs/               jrs-cli: host executable for the JavaScript core
│       ├── svg/               doc-svg: the SVG figures of those documents, as marks of a page
│       ├── pdf/               doc-pdf: a PDF 1.7 writer, pages, fonts, outline
│       └── docpdf/            the tool `xtask pdf` starts: every document as a PDF
├── fuzz/                      fuzz target crates and corpora
├── research/                  source of other projects, kept to be read; the checks never descend into it
├── .claude/                   the coding agent: `settings.json` is tracked, `worktrees/` holds a checkout per worktree session; the checks never descend into it
└── .github/workflows/         CI definitions
```

## 5.2 Crate catalog

| Crate | Layer | Target | `unsafe` | Host tests | May depend on |
|-------|-------|--------|----------|------------|---------------|
| `audhsos-abi` | 0 | all | no | yes | `test-support` behind the feature `test-strategies` |
| `audhsos-elf` | 0 | all | no | yes, fuzz | `test-support` behind the feature `test-strategies` |
| `audhsos-uefi` | 0 | all | no | yes (layouts) | `audhsos-abi`, `audhsos-time` |
| `audhsos-sync` | 0 | all | allowlisted | Miri | - |
| `audhsos-time` | 0 | all | no | yes | `test-support` behind the feature `test-strategies` |
| `audhsos-encoding` | 0 | all | no | yes, fuzz | `test-support` behind the feature `test-strategies` |
| `audhsos-collections` | 0 | all | no | yes | `test-support` behind the feature `test-strategies` |
| `audhsos-deflate` | 0 | all | no | yes | `test-support` behind the feature `test-strategies` |
| `kernel-types` | 1 | all | no | yes | `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `kernel-x86-tables` | 1 | all | no | yes | - |
| `kernel-acpi` | 1 | all | no | yes, fuzz | `kernel-types`; `test-support` as a dev-dependency |
| `kernel-hal-api` | 1 | all | no | doubles are tested | `kernel-types`; features `test-doubles`, `port-io` |
| `driver-uart16550` | 1 | all | no | yes | - (feature `test-doubles`) |
| `driver-i8042` | 1 | all | no | yes, fuzz | - (feature `test-doubles`) |
| `gfx` | 1 | all | no | yes | `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `audhsos-symbols` | 1 | all | no | yes | `audhsos-elf`; `test-support` as a dev-dependency |
| `virtio-queue` | 1 | all | no | yes | `audhsos-collections`; feature `test-doubles` |
| `pci` | 1 | all | no | yes, fuzz | - (feature `test-doubles`); `test-support` as a dev-dependency |
| `driver-virtio-net` (Phase 14) | 2 | all | no | yes, fuzz | `pci`, `virtio-queue` (feature `test-doubles`) |
| `driver-virtio-blk` | 2 | all | no | yes | `virtio-queue` (feature `test-doubles` as a dev-dependency); `test-support` as a dev-dependency; feature `test-doubles` |
| `fs-fat` | 1 | all | no | yes | `audhsos-time`; `test-support` as a dev-dependency; feature `test-doubles` |
| `fs-gpt` | 1 | all | no | yes | `fs-fat`, for the block device trait it reads through; `test-support` and `fs-fat` with `test-doubles` as dev-dependencies |
| `kernel-mm` | 2 | all | no | yes | `kernel-types`, `kernel-hal-api`, `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `kernel-objects` | 2 | all | no | yes | `kernel-types`, `kernel-mm`, `audhsos-abi`; `test-support` behind the feature `test-strategies` |
| `kernel-sched` | 2 | all | no | yes | `kernel-objects`, `audhsos-abi` |
| `kernel-ipc` | 3 | all | no | yes | `kernel-objects`, `kernel-sched`, `audhsos-abi`; `kernel-types` and `test-support` as dev-dependencies |
| `kernel-syscall` | 3 | all | no | yes, against a recording `Environment` | layers 0-2, `kernel-sched`, `kernel-ipc` |
| `kernel-core` | 4 | all | no | yes, with doubles | layers 0-3, `audhsos-sync` |
| `kernel-hal-x86_64` | 5 | `x86_64-unknown-none` | allowlisted | the pure parts live in `kernel-x86-tables` and `kernel-acpi` | `kernel-acpi`, `kernel-hal-api`, `kernel-types`, `audhsos-abi`, `driver-uart16550`, `audhsos-sync`, `kernel-x86-tables`, `kernel-mm`, `kernel-test-harness` |
| `kernel-test-harness` | 5 | all | no | yes | `kernel-hal-api` |
| `audhsos-kernel` | 6 | `x86_64-unknown-none` | allowlisted (the entry point, the memory and interrupt bring-up, and the test images) | QEMU | `kernel-core`, `kernel-hal-api`, `kernel-hal-x86_64`, `kernel-ipc`, `kernel-types`, `audhsos-abi`; `kernel-mm`, `kernel-objects`, `kernel-syscall`, `audhsos-sync` for the test images |
| `boot-uefi-x86_64` | b | `x86_64-unknown-uefi` | allowlisted | pure sub-modules | `audhsos-abi`, `audhsos-elf`, `audhsos-uefi`, `kernel-types`, `kernel-mm`, `kernel-hal-api` |
| `user-rt` | u0 | all | no | yes | `audhsos-abi`, `audhsos-collections`; `test-support` as a dev-dependency |
| `user-sys-x86_64` | u1 | `x86_64-unknown-none` | allowlisted | through the programs of `user-test-programs` in QEMU | `audhsos-abi`, `user-rt` |
| `user-test-programs` | u1 | `x86_64-unknown-none` | allowlisted | QEMU: they are what the kernel test images run in user mode | `audhsos-abi`, `user-rt`, `user-sys-x86_64` |
| `user-proto` | u1 | all | no | yes | `audhsos-abi`, `driver-i8042`, `gfx`, `user-rt` |
| `user-loader` | u2 | all | no | yes, fuzz | `audhsos-abi`, `audhsos-elf`; `test-support` behind the feature `test-strategies` |
| `server-name` | u2 | all | no | yes | `audhsos-abi`, `audhsos-collections`, `user-proto` |
| `server-memory` | u2 | all | no | yes, against a recording `Pages` | `audhsos-abi`, `audhsos-collections`; feature `test-doubles` |
| `server-console` | u2 | all | no | yes | `audhsos-collections`, `driver-uart16550` |
| `server-display` | u2 | all | no | yes | `audhsos-abi`, `audhsos-collections`, `gfx`, `user-proto` |
| `server-input` | u2 | all | no | yes | `audhsos-abi`, `audhsos-collections`, `driver-i8042`, `user-proto`; feature `test-doubles` |
| `server-net` (Phase 14) | u2 | all | no | yes | `audhsos-abi`, `audhsos-collections`, `audhsos-time`, `crypto-rng`, `driver-virtio-net`, `net-stack`, `pci`, `user-proto` |
| `user-programs` | u3 | `x86_64-unknown-none` | allowlisted | e2e in QEMU | the three server logic crates, `audhsos-abi`, `driver-uart16550`, `pci`, `user-rt`, `user-proto`, `user-loader`, `user-sys-x86_64` |
| `crypto-ct` | c0 | all | no | yes | - |
| `audhsos-der` | c0 | all | no | yes, fuzz | `audhsos-time`; `test-support` as a dev-dependency |
| `crypto-hash` | c1 | all | no | yes | `crypto-ct` |
| `crypto-aead` | c1 | all | no | yes | `crypto-ct` |
| `crypto-bignum` | c1 | all | no | yes | `crypto-ct`; `test-support` as a dev-dependency |
| `crypto-ec` | c2 | all | no | yes | `crypto-bignum`, `crypto-ct`, `crypto-hash`; feature `test-signing` |
| `crypto-dh` | c2 | all | no | yes | `crypto-bignum`, `crypto-ct`; `test-support` as a dev-dependency |
| `crypto-rng` | c2 | all | no | yes | `crypto-ct`, `crypto-aead`; feature `test-doubles` |
| `crypto-rsa` | c2 | all | no | yes, fuzz | `crypto-bignum`, `crypto-ct`, `crypto-hash`; feature `test-signing` |
| `audhsos-x509` | c3 | all | no | yes, fuzz | `audhsos-der`, `audhsos-time`, `crypto-hash`, `crypto-ec`, `crypto-rsa`; feature `test-certificates` |
| `audhsos-ssh` (track S) | c3 | all | no | yes | `crypto-rng`; `test-support` and `crypto-rng` with `test-doubles` as dev-dependencies. The crypto crates of 14.5 join it with the steps that need them |
| `audhsos-tls` | c4 | all | no | yes, fuzz | `crypto-ct`, `crypto-hash`, `crypto-aead`, `crypto-ec`, `crypto-rng`, `audhsos-der`, `audhsos-time`, `audhsos-x509` |
| `net-wire` | n0 | all | no | yes | `test-support` as a dev-dependency |
| `net-eth` | n1 | all | no | yes | `net-wire`, `audhsos-time`, `audhsos-collections`; `test-support` as a dev-dependency |
| `net-ip` | n2 | all | no | yes, fuzz | `net-eth`, `net-wire`, `audhsos-time`, `audhsos-collections`; `test-support` as a dev-dependency |
| `net-ipv6` | n2 | all | no | yes, fuzz | `net-ip`, `net-eth`, `net-wire`, `audhsos-time`, `audhsos-collections`; `test-support` as a dev-dependency |
| `net-udp` | n3 | all | no | yes | `net-wire`, `crypto-rng`; `test-support` and `crypto-rng` with `test-doubles` as dev-dependencies |
| `net-tcp` | n3 | all | no | yes, fuzz | `net-wire`, `audhsos-time`, `audhsos-collections`, `crypto-rng`; `test-support` and `crypto-rng` with `test-doubles` as dev-dependencies |
| `net-dns` | n4 | all | no | yes, fuzz | `net-udp`, `net-wire`, `audhsos-time`, `audhsos-collections`, `crypto-rng`; `test-support` and `crypto-rng` with `test-doubles` as dev-dependencies |
| `net-dhcp` | n4 | all | no | yes | `net-udp`, `net-wire`, `audhsos-time`, `audhsos-collections`, `crypto-rng`; `test-support` and `crypto-rng` with `test-doubles` as dev-dependencies |
| `net-http` | n4 | all | no | yes, fuzz | `net-wire`; `test-support` as a dev-dependency |
| `net-stack` | n5 | all | no | yes | every `net-` crate, `audhsos-time`, `audhsos-collections`, `crypto-rng`; `test-support` and `crypto-rng` with `test-doubles` as dev-dependencies |
| `test-support` | dev | host | no | yes | - (depends on no workspace crate, so that every crate can use it as a dev-dependency without a cycle) |
| `fuzz-support` | dev | host | allowlisted | yes, and Miri over `counters` and `sancov`, which hold its `unsafe` | - |
| `xtask` | host | host | no | yes | `audhsos-abi`, `kernel-test-harness` (the boot image header, the layout constants, and the serial protocol grammar exist once), `audhsos-symbols`, `audhsos-time`, `fs-fat`, `fs-gpt`, `user-loader` |
| `doc-markdown` | host | host | no | yes | - |
| `doc-html` | host | host | no | yes | `doc-markdown` |
| `doc-pdf` | host | host | no | yes | `audhsos-deflate` |
| `doc-svg` | host | host | no | yes | `doc-html`, `doc-pdf` |
| `docpdf` | host | host | no | yes, without a coverage gate, as `xtask` | `doc-html`, `doc-markdown`, `doc-pdf`, `doc-svg` |
| `jrs` | logic | all, with an allocator supplied by the embedding | no | yes, property and fuzz | `audhsos-regex`, `audhsos-event-target`, `audhsos-timer-queue`, `audhsos-json`, `audhsos-math`, `audhsos-utf16`; `test-support` as a dev-dependency |
| `jrs-cli` | host | host | no | yes | `jrs`, `doc-html` (WPT script extraction) |
| `audhsos-regex` | logic | all, with an allocator supplied by the embedding | no | yes, property and fuzz | none at run time; `test-support` as a dev-dependency |
| `audhsos-regex-bt` | logic | all, with an allocator supplied by the embedding | no | yes, differential and fuzz | `audhsos-regex` (shared syntax, classes, assertions, result types) |
| `audhsos-event-target` | logic | all, with an allocator supplied by the embedding | no | yes, independent model/fuzz | none |
| `audhsos-timer-queue` | logic | all, with an allocator supplied by the embedding | no | yes, independent model/fuzz | none |
| `audhsos-json` | logic | all, with an allocator supplied by the embedding | no | yes, UTF-16 roundtrip/fuzz | none |
| `audhsos-math` | logic | all, allocation-free | no | yes, binary64 differential/fuzz | none |
| `audhsos-utf16` | logic | all, with an allocator for search preprocessing | no | yes, search-model/fuzz | none |

## 5.3 Layering rules

The regular expression core is the dedicated `crates/regex/` crate, not under
support, encoding or the JavaScript VM. Its algorithm and resource contracts
are in [regex/README.md](../crates/regex/README.md): Thompson NFA/DFA only, no
backtracking fallback, no external dependency, independent fuzzing and audit.
`audhsos-regex` implements a bounded, prioritized Thompson NFA over UTF-16
code units and is registered in workspace policy. Its independent fuzz target
is `regex_nfa`. Other reusable components likewise remain in appropriately
named crates under `crates/`.

The separately authorized `crates/regex-bt/` component implements bounded
backtracking, with no reverse dependency or fallback in `crates/regex/`.
Its dependency on `audhsos-regex::syntax` reuses parsing and code-unit predicates,
not the Thompson matcher. Choice frames, assertion frames, register cells,
input, compile expansion and work are bounded. Unlike the NFA, it can have
exponential runtime and requires explicit selection; quotas are not a linearity
guarantee. jrs currently remains connected only to the automaton engine.

`crates/json/` is the independent bounded UTF-16 JSON parser/quoting core.
Its flat parse arena retains lexical ranges without recursive destruction.
JavaScript object materialization, reviver/replacer/toJSON hooks and raw JSON
branding live in `jrs/src/vm/json/`; xtask's narrower integer-only QMP codec
remains separate. The independent fuzz target is `json_codec`.

1. Dependencies point downward only. A crate may depend on crates of lower
   layers as listed in the catalog, never sideways or upward.
2. Logic crates never depend on adapter crates, with one exception:
   `kernel-core`, `kernel-hal-x86_64`, and `user-sys-x86_64` depend on
   `audhsos-sync`.
3. `kernel-hal-api` depends on `kernel-types` and nothing else. Its test
   doubles live behind the feature `test-doubles`. Generators for property
   tests live in the crate that owns the types, behind the feature
   `test-strategies`; `test-support` itself depends on no workspace crate.
4. The crates shared between loader, kernel, and userland are exactly
   `audhsos-abi`, `audhsos-elf`, `audhsos-uefi`, `audhsos-sync`,
   `audhsos-time`, `audhsos-encoding`, `audhsos-collections`,
   `kernel-types`, `kernel-hal-api`, `kernel-mm`, and `driver-uart16550`.
5. Userland crates never depend on kernel crates other than those in rule 4.
6. `cfg(target_arch = ...)` and `cfg(target_os = "uefi")` appear only in
   adapter crates and in the binaries' `Cargo.toml` target tables.
7. The cryptography and TLS crates (layers c0 to c4, document 11) depend
   on each other only, never on kernel, loader, or userland crates.
   Userland crates depend on them, not the reverse.
8. The allowed edges are a table in `xtask/src/policy.rs`. `cargo xtask
   check-layering` reads `cargo tree --edges normal,build,dev --prefix
   depth` and fails on any edge not in the table.
9. No dependency section of any manifest references a crate outside the
   workspace. `cargo xtask check-deps` verifies `Cargo.lock` and every
   manifest.
10. The network crates (layers n0 to n5, [document 12](12-parallel-work.md))
    depend on each other, on the layer-0 foundations, and on `crypto-rng`
    for unpredictable numbers, never on kernel, loader, or userland
    crates. Userland depends on them, not the reverse. `audhsos-tls` and
    the network crates never reference each other; the transport that
    joins them lives in a userland process.
11. `audhsos-symbols`, `virtio-queue`, `fs-fat`, `fs-gpt`, and `pci` are
    logic crates at layer 1. They depend on layer-0 crates only — `pci` on
    nothing at all — and are used by the xtask and, when the phases reach
    them, by driver and server processes. `fs-gpt` is the one exception to
    the layer-0 rule: it depends on `fs-fat`, which is layer 1, because the
    block device trait it reads through is defined there (D-138).
12. `driver-virtio-blk` is a logic crate at layer 2, and
    `driver-virtio-net` will be another, because a driver of a virtio
    device stands above the queue logic it drives the device through.
    `driver-virtio-blk` depends on `virtio-queue` and on nothing else: it
    reaches registers through a trait of its own and parses no capability,
    so it needs nothing of `pci` (D-139). `driver-virtio-net` needs both,
    and neither depends on the network crates of rule 10: it hands frames
    out and takes them in as byte slices, and what a frame means belongs
    to `server-net` (D-114).

## 5.4 Workspace configuration

- `[workspace.package]` holds version, edition (`2024`), license
  (`AGPL-3.0-only`), repository, and `rust-version`. Every crate inherits
  them with `.workspace = true`.
- `[workspace.dependencies]` lists only workspace members by path.
- `[workspace.lints]` holds the complete lint configuration. Every crate
  declares `[lints] workspace = true`. Crate roots contain only the
  attributes that differ per crate: `#![no_std]`, `#![forbid(unsafe_code)]`
  or the adapter header, and the crate documentation.
- Bare-metal targets abort on panic by definition; host test crates keep
  unwinding for `should_panic` tests.
- Cargo features are limited to five in the kernel, the loader, and
  userland: `debug-uart` and `test-exit` on the kernel binary and adapter,
  `test-doubles` and `port-io` on `kernel-hal-api`, `test-strategies` on
  crates that own types used in property tests. The cryptography track
  adds `test-signing` on `crypto-ec` and `test-certificates` on
  `audhsos-x509`, and reuses `test-doubles` on `crypto-rng`. Both exist to generate test data, both are off in every
  product build, and `cargo xtask check-layering` fails if a crate other
  than a test target or the xtask enables them. Host tests use `#![cfg_attr(not(test), no_std)]` and need
  no feature. No feature changes behavior in release builds.

## 5.5 Conventions

### 5.5.1 File header

Every Rust, TOML, shell, and YAML file starts with:

```
// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
```

(with the comment syntax of the file type). `cargo xtask lint` checks the
header.

### 5.5.2 Naming

- Crates: kebab-case with the prefixes shown in the catalog.
- Modules: snake_case, one concept per module. A module that needs a
  paragraph to describe its purpose is two modules.
- Types name the invariant they carry: `PhysFrame` rather than `u64`,
  `Rights` rather than `u32`. Constructors validate and return `Result`.
- Functions are verbs. Predicates start with `is_`, `has_`, or `can_`.
- Abbreviations are limited to `abi`, `hal`, `ipc`, `mm`, `tcb`, `tlb`,
  `apic`, `irq`, `elf`, `id`, `uefi`; in the cryptography track,
  `ct`, `aead`, `ec`, `rng`, `der`, `tls`, `hmac`, `hkdf`, `oid`, `spki`;
  and in the parallel tracks of document 12, `arp`, `dhcp`, `dns`, `fat`,
  `http`, `ip`, `mac`, `mss`, `mtu`, `rto`, `tcp`, `udp`.
- Tests: `<subject>_<condition>_<expected>`, for example
  `frame_allocator_exhausted_returns_out_of_frames`.

### 5.5.3 Test files

Unit tests live in `src/tests/<module>.rs`, declared by `#[cfg(test)]
mod tests;` in the crate root and by `src/tests/mod.rs`. Product source
files contain no test code, so that coverage measures product code only.
Tests reach private items through `pub(crate)` visibility where needed.

### 5.5.4 Errors

- Each logic crate defines one exhaustive `enum Error` with `Display`. No
  string errors, no boxed errors, no error codes as integers outside `abi`.
- `kernel-syscall` maps every crate error to `abi::Error` through one `From`
  implementation per crate. The mapping table is exhaustively tested.
- Functions that can fail return `Result`. A function that cannot fail does
  not return `Result`.

### 5.5.5 Lint set

Configured once in the workspace. Level `deny` unless stated.

- Rust: `unsafe_code` (deny; adapter crates allow it locally),
  `unsafe_op_in_unsafe_fn`, `missing_docs`, `unreachable_pub`,
  `unused_crate_dependencies`, `rust_2024_compatibility`, `warnings`.
- Clippy: `all`, `pedantic`, `nursery` (warn, individual lints raised to
  deny as they prove useful), `cargo`, plus the safety set from rule R7:
  `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`,
  `unreachable`, `indexing_slicing`, `arithmetic_side_effects`,
  `as_conversions`, `undocumented_unsafe_blocks`,
  `multiple_unsafe_ops_per_block`.
- Exceptions use `#[expect(lint, reason = "...")]`.

### 5.5.6 Documentation

- Every public item has a doc comment. Module docs start with the
  invariants the module maintains.
- `cargo doc --workspace --document-private-items` runs with
  `RUSTDOCFLAGS="-D warnings"` in CI.
- Doc examples on host-testable crates run as doctests.
- Each crate has a `README.md` that `lib.rs` includes as crate
  documentation.

### 5.5.7 Logging

Project-defined logging macros: `klog!` in `kernel-core` writes through the
`DebugConsole` trait when the feature is on and compiles to nothing
otherwise; `user_sys_x86_64::write_line` sends a line to the log
endpoint of the startup message, or through `debug_log` while a program
still has none.

## 5.6 Avoiding duplication

| Situation | Approach |
|-----------|----------|
| The same algorithm runs in the loader, the kernel, and under test | one generic implementation over HAL traits; adapters and doubles differ, the algorithm does not (mapper, memory map normalization) |
| ELF parsing in the loader and in userland | `audhsos-elf`, one parser |
| UART register handling in the kernel debug console and in the userland console driver | `driver-uart16550` over a port access trait; two adapters (direct port I/O, `IoPortRange` system calls) |
| i8042 register handling and PS/2 decoding | `driver-i8042` over its own port access trait, following the UART pattern; one adapter over `IoPortRange` system calls |
| PCI configuration space, for the bus walk and for a driver's own registers | `pci` over a `ConfigSpace` trait; one adapter, the volatile accessor of `user-sys-x86_64` over the mapped ECAM window, and a recorded configuration space as the double (D-112, D-113) |
| Virtqueue arithmetic in the network driver and in a later block driver | `virtio-queue` over `QueueMemory`; a driver supplies the offset arithmetic of its own DMA region and nothing else (D-115) |
| Volatile access to a mapped device region | one accessor in `user-sys-x86_64`; every driver above it keeps `forbid(unsafe_code)` and reaches its registers through a trait (D-113) |
| Pixel operations in the display server and in applications | `gfx`: one surface type, one font, one damage tracker; the display server and applications draw with the same code |
| System call numbers, names, argument counts, kernel dispatch | one declarative table in `audhsos-abi` (a `syscalls!` macro) consumed by the kernel dispatcher; the wrappers of `user-sys-x86_64` are written out by hand and a constant assertion holds them to the same table (D-92) |
| Object types, their rights masks, and `TryFrom<u32>` conversions | one declarative table in `audhsos-abi` |
| Error mapping | one `From` implementation per crate pair, tested by a table |
| Test doubles | one implementation in `kernel-hal-api` behind `test-doubles` |
| FAT32 structures in the image writer and in a later file system server | `fs-fat` over a block device trait; the image writer of the xtask is its first user and a file system server will be the second |
| Partition table structures and the CRC-32 they are checked with, in the image writer and in a later file system server | `fs-gpt` over the same trait; the xtask keeps the image's own choices and no structure of the format (D-138) |
| Calendar arithmetic in certificate validity, file timestamps, and network timers | `audhsos-time`; every interface takes time as a parameter, no crate reads a clock |
| Fixed-capacity containers in kernel queues, the network stack, and userland | `audhsos-collections`; one model-tested implementation per container |
| Base64 and PEM in the trust-anchor tool and in generated test data | `audhsos-encoding` |
| Test fixtures, generators, and the model-test runner | `test-support` crate |
| Lint, metadata, dependency lists | workspace inheritance |
| Protocol encodings | `user-proto` defines each message once as a type with `encode`/`decode` |
| Repeated `match` on object type | `KernelObject::as_<type>()` accessors generated from the object table |
| Policy tables (layering, unsafe budgets, SPDX header, fuzz targets) | `xtask/src/policy.rs`, type-checked constants |

## 5.7 Build automation

All developer and CI commands go through `cargo xtask`, on the development
machine through the wrapper scripts `tools/xtask.sh` and
`tools/xtask-check.sh` that [07 section
7.5](07-toolchain-and-environment.md#75-findings-about-the-development-machine)
describes. The xtask uses only the standard library and the toolchain
binaries (`cargo`, `rustc`, `rustfmt`, `cargo-clippy`, `cargo-miri`,
`llvm-profdata`, `llvm-cov`, `llvm-objcopy`) plus QEMU.

| Subcommand | Purpose |
|------------|---------|
| `build [--release]` | build the loader, the kernel, the userland binaries, and the boot image |
| `image` | assemble the boot image (root task ELF plus tar archive) and the disk image (GPT, FAT32 file system, loader, kernel, boot image) |
| `run [--release] [--display]` | boot the system in QEMU with the serial console on the terminal; `--display` opens QEMU's display window instead of `-display none` |
| `qemu-runner <elf>` | the Cargo runner for the kernel target: wraps a test kernel into a disk image, runs QEMU with a timeout, parses the serial protocol, maps the exit status |
| `build-user-tests` | build the user programs the test images run and turn each into a flat binary under `target/user-tests/` |
| `test [--host] [--qemu] [--e2e] [--release]` | run the selected test levels; default runs the host level; `--release` builds the end-to-end run from the release profile. `--e2e` runs the whole system twice: once with a graphics adapter, where it takes a picture of the screen through the machine protocol and holds it against what the program that draws said it drew, and once with `-vga none`, where the machine has no framebuffer and the run still has to end by itself |
| `lint` | `rustfmt --check`, `clippy` with the workspace lint set, SPDX header check |
| `check-layering` | verify the layering table against `cargo tree`, verify `forbid(unsafe_code)` in every logic crate, reject assembly files, verify the adapter-function-to-QEMU-test tables |
| `check-deps` | verify that `Cargo.lock` and all manifests reference workspace members only |
| `unsafe-budget` | count `unsafe` blocks and `asm!` sites per adapter crate against the policy table |
| `fuzz [--target <name>] [--time <s>] [--regression] [--merge <directory>] [--minimize <file>]` | build fuzz targets with `-Zsanitizer=fuzzer` and run them; `--regression` replays the stored corpus instead, which is what `check` runs |
| `coverage` | build host tests with `-C instrument-coverage`, merge profiles with `llvm-profdata`, export LCOV with `llvm-cov`, enforce thresholds |
| `miri` | run the tests of the `unsafe` modules of the host-executable adapter crates under Miri, after checking that no module holding `unsafe` is left out |
| `doc` | build documentation with warnings as errors |
| `pdf [options]` | every Markdown document of `docs/` and every standard beside them as a PDF under `target/pdf/`; the options go to the tool, which explains them with `--help` |
| `jrs [options]` | build and run the release-mode `jrs-cli` host executable; `--help` describes source input, fuel and measurement options |
| `jrs-check [--fix-format]` | package-scoped formatting, tests, strict Clippy and `jrs` cross-check for `x86_64-unknown-none` |
| `regex-check [--fix-format]` | the isolated regex core's formatting, tests, strict Clippy and `x86_64-unknown-none` cross-check |
| `symbolize <elf> <address>...` | the function, file, and line of every address, which a failing QEMU run is reported through |
| `check [--quiet]` | everything CI runs, in CI order; `--quiet` leaves one line per step and prints the output of a step only when it fails |

A subcommand refuses an option it does not know, so a mistyped
`check --qiuet` stops with the usage text instead of running the whole
check and reporting a success for something nobody asked for.

The xtask verifies at start that `RUSTUP_TOOLCHAIN`, which rustup's proxies
set for child processes, names the pinned channel, and stops with
instructions otherwise. Every Cargo it starts receives `RUSTC` and
`RUSTDOC` pointing into the same toolchain, so a foreign `rustc` earlier
on the `PATH` is never used.

## 5.8 Version control

- Conventional Commits: `feat(mm): ...`, `fix(ipc): ...`, `test(sched):
  ...`, `docs: ...`, `chore: ...`, `refactor(objects): ...`. The scope is the
  crate's short name. The body states what changes. A footer
  `Decision: D-07` links a decision register entry when one applies.
- Trunk-based development on `main` with short-lived branches. Every merge
  passes `sh tools/xtask-check.sh`.
- `CHANGELOG.md` follows Keep a Changelog and is updated in the same commit
  as the change.

## 5.9 Definition of done for a change

1. Tests exist for the new behavior and for every edge case listed in the
   catalog for the affected component.
2. Documentation of every touched public item is current.
3. `sh tools/xtask-check.sh` passes locally.
4. No `unsafe` budget increase without a decision register entry.
5. The change contains no duplicated logic that the review could point at.
6. The changelog entry exists.
