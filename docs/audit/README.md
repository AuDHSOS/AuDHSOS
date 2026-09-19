# Audit findings

What an audit of the workspace found, one document per group of crates
audited together, one section per finding. Every finding is a GitHub
issue of this repository; the issue number stands in the section
heading.

These documents are a record of what was found and filed, not a plan.
A finding is closed by closing its issue; the section here stays as it
was written, so that a reader of an issue finds the state the audit
judged.

## How to read a document

Every section carries these parts, in this order:

- `Title`: the issue title, verbatim.
- `Labels`: `bug` for a defect in behavior, safety, security, or a
  document that contradicts the code; `enhancement` for performance,
  bounds that are safe today, and hygiene. The second label is the
  `part::` label of the area.
- `Body`: the issue body, verbatim. Its first paragraph states the
  defect and where, the second the consequence with a concrete trigger,
  the third the fix and the option not taken.

Every claim cites a path and a line range of the revision the document
names. A line moves as the code changes; the cited revision is what the
claim was checked against.

## What was audited

The two audits together cover every crate of the workspace but `jrs`
and `jrs-cli`, which are under development and were left out at the
user's direction. The cryptography audit ran at `bc47df5` and filed on
2026-09-18; the rest ran at the same revision and filed on 2026-09-19.

| | Findings | Bug | Enhancement |
|-|---------:|----:|------------:|
| Total | 523 | 333 | 190 |

### Cryptography

| Document | Crates | Findings | Bug | Enhancement | Issues |
|----------|--------|---------:|----:|------------:|--------|
| [`crypto.md`](crypto.md) | `crates/crypto`: ct, hash, aead, bignum, ec, dh, rng, rsa | 28 | 14 | 14 | #23 to #51 |

### Kernel and adapters

| Document | Crates | Findings | Bug | Enhancement | Issues |
|----------|--------|---------:|----:|------------:|--------|
| [`kernel-syscall.md`](kernel-syscall.md) | `kernel-syscall` | 20 | 16 | 4 | #191 to #210 |
| [`kernel-ipc-sched.md`](kernel-ipc-sched.md) | `kernel-ipc`, `kernel-sched` | 7 | 2 | 5 | #52 to #58 |
| [`kernel-objects-types.md`](kernel-objects-types.md) | `kernel-objects`, `kernel-types` | 8 | 5 | 3 | #73 to #80 |
| [`kernel-mm-hal-api.md`](kernel-mm-hal-api.md) | `kernel-mm`, `kernel-hal-api` | 14 | 4 | 10 | #59 to #72 |
| [`kernel-core-bin-sync.md`](kernel-core-bin-sync.md) | `kernel-core`, `audhsos-kernel`, `audhsos-sync` | 11 | 4 | 7 | #113 to #134 |
| [`kernel-hal-x86_64.md`](kernel-hal-x86_64.md) | `kernel-hal-x86_64`, `kernel-x86-tables`, `kernel-acpi` | 13 | 7 | 6 | #82 to #110 |
| [`boot-uefi-elf.md`](boot-uefi-elf.md) | `boot-uefi-x86_64`, `audhsos-uefi`, `audhsos-elf` | 8 | 6 | 2 | #138 to #164 |
| [`abi.md`](abi.md) | `audhsos-abi` | 10 | 4 | 6 | #81 to #96 |

### Userland

| Document | Crates | Findings | Bug | Enhancement | Issues |
|----------|--------|---------:|----:|------------:|--------|
| [`user-sys-rt.md`](user-sys-rt.md) | `user-sys-x86_64`, `user-rt` | 9 | 6 | 3 | #99 to #112 |
| [`user-programs-init.md`](user-programs-init.md) | `user-programs` library and the root task | 14 | 6 | 8 | #166 to #190 |
| [`user-programs-bins.md`](user-programs-bins.md) | `user-programs` binaries, `user-test-programs` | 11 | 6 | 5 | #132 to #163 |
| [`user-loader-mem-name-console.md`](user-loader-mem-name-console.md) | `user-loader`, `server-memory`, `server-name`, `server-console` | 12 | 10 | 2 | #115 to #136 |
| [`user-proto-display-input.md`](user-proto-display-input.md) | `user-proto`, `server-display`, `server-input`, `app-canvas`, `gfx` | 10 | 7 | 3 | #139 to #159 |
| [`fs.md`](fs.md) | `server-fs`, `fs-fat`, `fs-gpt` | 18 | 12 | 6 | #211 to #228 |
| [`server-net-programs.md`](server-net-programs.md) | `server-net`, `user-net-programs` | 12 | 11 | 1 | #165 to #187 |

### Network and security protocols

| Document | Crates | Findings | Bug | Enhancement | Issues |
|----------|--------|---------:|----:|------------:|--------|
| [`der-x509.md`](der-x509.md) | `audhsos-der`, `audhsos-x509` | 17 | 4 | 13 | #229 to #245 |
| [`tls.md`](tls.md) | `audhsos-tls` | 28 | 22 | 6 | #250 to #321 |
| [`ssh.md`](ssh.md) | `audhsos-ssh` | 14 | 11 | 3 | #246 to #268 |
| [`net-wire-eth-ip.md`](net-wire-eth-ip.md) | `net-wire`, `net-eth`, `net-ip` | 22 | 17 | 5 | #272 to #327 |
| [`net-ipv6.md`](net-ipv6.md) | `net-ipv6` | 16 | 15 | 1 | #326 to #367 |
| [`net-tcp.md`](net-tcp.md) | `net-tcp` | 26 | 17 | 9 | #442 to #477 |
| [`net-udp-dns-dhcp.md`](net-udp-dns-dhcp.md) | `net-udp`, `net-dns`, `net-dhcp` | 7 | 6 | 1 | #271 to #289 |
| [`net-http-stack.md`](net-http-stack.md) | `net-http`, `net-stack` | 24 | 20 | 4 | #358 to #386 |

### Device logic

| Document | Crates | Findings | Bug | Enhancement | Issues |
|----------|--------|---------:|----:|------------:|--------|
| [`virtio-queue-blk.md`](virtio-queue-blk.md) | `virtio-queue`, `driver-virtio-blk` | 8 | 5 | 3 | #295 to #315 |
| [`virtio-net-pci.md`](virtio-net-pci.md) | `driver-virtio-net`, `pci` | 13 | 8 | 5 | #331 to #355 |
| [`uart-i8042.md`](uart-i8042.md) | `driver-uart16550`, `driver-i8042` | 9 | 4 | 5 | #322 to #341 |

### Foundations

| Document | Crates | Findings | Bug | Enhancement | Issues |
|----------|--------|---------:|----:|------------:|--------|
| [`collections-time-encoding.md`](collections-time-encoding.md) | `audhsos-collections`, `audhsos-time`, `audhsos-encoding` | 12 | 5 | 7 | #387 to #398 |
| [`deflate-symbols.md`](deflate-symbols.md) | `audhsos-deflate`, `audhsos-symbols` | 7 | 4 | 3 | #412 to #420 |
| [`json-math-event-timer.md`](json-math-event-timer.md) | `audhsos-json`, `audhsos-math`, `audhsos-event-target`, `audhsos-timer-queue` | 6 | 3 | 3 | #399 to #404 |
| [`regex-utf16.md`](regex-utf16.md) | `audhsos-regex`, `audhsos-regex-bt`, `audhsos-utf16` | 7 | 6 | 1 | #405 to #411 |
| [`text-core-parsing.md`](text-core-parsing.md) | `text-core`: font parsing | 7 | 4 | 3 | #478 to #484 |
| [`text-core-shaping.md`](text-core-shaping.md) | `text-core`: shaping and layout | 14 | 6 | 8 | #485 to #498 |
| [`text-raster.md`](text-raster.md) | `text-raster` | 15 | 9 | 6 | #437 to #458 |

### Tools

| Document | Crates | Findings | Bug | Enhancement | Issues |
|----------|--------|---------:|----:|------------:|--------|
| [`xtask.md`](xtask.md) | `xtask` | 18 | 14 | 4 | #417 to #436 |
| [`doc-tools.md`](doc-tools.md) | `doc-markdown`, `doc-html`, `doc-pdf`, `doc-svg`, `docpdf` | 32 | 24 | 8 | #515 to #546 |
| [`support-tools.md`](support-tools.md) | `test-support`, `fuzz-support`, `kernel-test-harness`, `membench`, `norec`, `text-demo` | 16 | 9 | 7 | #499 to #514 |
## Method

1. Every non-test source file of a group was read whole; tests were read
   where they state intent.
2. The host tests of each crate were run once. A failing test is a
   finding.
3. Each claim was re-opened at its cited lines and confirmed before it
   was written. A claim that could not be confirmed from the code was
   dropped.
4. Findings were filed as issues in the order of severity within a
   group.

The order of the groups above is the order they were audited in, from
the crates a defect costs most in to the crates it costs least in.
