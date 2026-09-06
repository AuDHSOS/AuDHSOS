# Changelog

All notable changes to this project are documented in this file. The format
follows Keep a Changelog; the project follows Semantic Versioning.

## [Unreleased]

### Fixed

- The device interrupt handler of the kernel binary acknowledges at the local
  APIC before it signals the notification, not after. 2.7 gives the three
  steps in one order — mask the line at the I/O APIC, end the interrupt at the
  local APIC, then signal — and the binary had the last two the wrong way
  round. Nothing of Phase 6 could observe it, because that image starts no
  user thread and therefore signals nobody; the test kernel its images share
  had the order right from the start. What the wrong order costs is a wake-up
  that runs a driver with the interrupt still unacknowledged at the local
  APIC, which delays every later interrupt of that priority class until the
  handler returns.

- The scheduler asks the transition table before it takes a thread out of
  its run queue. It did it the other way round in the four operations that
  take a thread off the processor, and for two of them that was wrong: the
  table has a row out of `Ready` for `Suspend` and for `Exit`, but none for
  `Fault` and none for the four block events, because only a running thread
  can fault or block. A ready thread handed to `fault` was therefore taken
  out of its queue and then refused — left `Ready` and in no queue, which
  is the one thing the scheduler says can never be true of a thread, and
  which `pick_next` can never find again. The caller saw an error and had
  every reason to believe nothing had happened.

  The four bodies were identical and are now one, `leaves_the_processor`,
  which applies the event first and dequeues afterwards. Nothing can be
  left half done that way: `dequeue` fails only for a thread the pool does
  not hold, and the apply has just held it. An operation the table refuses
  now changes nothing at all — not the state, not a queue, not the time
  slice. `kernel_syscall::fault::stop` no longer has to check the state
  itself.


### Changed

- D-86: four unsafe budgets rise, and a process and a thread each hold one
  reference to themselves. The budgets are `kernel-hal-x86_64` (129 to 142
  unsafe, 24 to 27 `asm!`, for the three port widths that were missing and
  the six methods that reach them), `audhsos-kernel` (26 to 27, for the
  buffer a forwarded interrupt writes), `user-sys-x86_64` (11 to 13, for the
  call that hands out both return words), and `user-test-programs` (17 to 65,
  for the five new programs). The reference model is the second half: a handle
  is a reference and the last one destroys what it names, except for a process
  and a thread, which end when they are killed and when the reaper has given
  back what they held. The reaper is the reason — it needs the `Thread` object
  after the last handle to the thread has closed, to find the kernel stack and
  the buffer to give back.

- The documents follow the code of Phase 6. 2.3.4 gains the reference a
  process and a thread hold to themselves: a process ends when it is killed
  and a thread when the kernel has given back what it held, whatever handle
  still names either of them, so closing the last handle to a running thread
  does not end it and the handles that named one afterwards name nothing.
  That is the one place where 2.3.2's "closing the last handle to an object
  destroys the object" does not hold, and 2.3.2 now says where to look. 2.8's
  row for `system_info` names what it actually reports, the framebuffer
  description being Phase 9's.

  In 5.2 the dependency columns of `kernel-ipc`, `kernel-syscall`, and
  `audhsos-kernel` say what the manifests do, and 4.3 says what the test
  programs use `unsafe` for now that they write into a page they share with
  the test. In the plan, 10.6 says what the implementation settled: where the
  operations of `WaitQueue` live, why `Queue` has three variants and carries
  a badge, what `transfer` takes and why, the two questions the environment
  passes on to the architecture layer, and the buffer `fault::deliver` is
  handed.

### Added

- The five programs of the userland: the root task, the name server, the
  console driver, the memory server, and the application. Each is a thin
  loop around a logic crate — receive, decode, ask the policy, encode, send
  — and what is only here is the part that needs a capability or a pointer.

  The root task reads the archive out of the boot image and starts the four
  others in order. The memory server comes first, out of one region the root
  task keeps back for exactly that; everything after it is asked of the
  memory server, so the boot of the system is the first three tests of it.
  The console driver runs two threads, because a thread of this kernel waits
  on one thing: one waits on the endpoint and owns the controller, the other
  waits on the interrupt and sends the byte it took to the first under a
  badge of its own. Nothing is held by both, so nothing needs a lock.

- `thread_create` takes the page for the thread's IPC buffer (D-89). The
  fifth argument was reserved and had to be zero; it is now a memory object
  of one page, and zero still means the kernel takes a frame out of its own
  reserve. Without it the root task cannot do what 2.10 has always required
  of it — write the startup message into the buffer of the child before the
  child runs — because the kernel allocated that frame and the parent held
  nothing that named it. A child would then hold capabilities it could not
  tell apart.

- The kernel starts the root task. Steps 12 to 14 of the boot sequence have
  been in 2.9 since it was written and in the implementation plan nowhere:
  the boot image becomes a memory object, a process is built with the flat
  binary mapped at `ROOT_TASK_BASE`, a stack of sixteen pages below it with
  an unmapped page between, a kernel stack, an IPC buffer, and the handles it
  starts with — system control, itself, the boot image, and one memory object
  per free region of memory — and then the startup message that says which
  handle is which.

  What knows nothing of a processor is `kernel_core::root`, host-tested over
  the doubles the memory tests already use. What is left is `task.rs` in the
  kernel binary: the frame a new thread returns through, the switch of the
  stacks, the idle thread on the boot stack, and the loop the kernel becomes
  — switch to whatever the scheduler picked, halt when there is nothing.

  Two things follow from there being a user thread at last. The trap handler
  now tells a fault of the kernel from a fault of a user thread: the first
  ends the machine, the second becomes a message to the fault handler of that
  process. And the device interrupt handler gains the switch that 2.7 asks
  for as its third step, which the binary could not do while it had nobody to
  switch to.

- `server-console`, what the console driver does with the controller and
  with the bytes that arrive on it: a write that reports how far it came,
  and a ring for what came in until a client asks for it. The controller is
  `driver-uart16550` over its `Registers` trait, so the tests use the
  recording implementation that crate already had, and nothing here makes a
  system call.

  The ring drops the oldest byte when it is full and counts what it dropped.
  For a console that is the right end to lose: what a person is typing now
  is what they will look for on the screen, and a count is something a
  client can act on where a silently missing byte is not.

  `Uart16550::registers` is new beside `into_registers`, for a caller that
  has to reach a register the crate has no method for.

- `server-memory`, the allocation policy of the memory server, and
  `server-name`, the registry behind the name server. Neither makes a system
  call: the memory policy reaches the kernel through the `Pages` trait — map,
  zero, unmap, split, merge — and through nothing else, so the whole of it
  runs on the host against the recording double catalog item 6.6.23 asks
  for. The zeroing is a thing a test watches happen, in the order it
  happens: one pass over exactly the range of the object, before the handle
  goes out and again the moment it comes back.

  The free list holds whole objects sorted by physical address; a request is
  served from the first that can hold it at the alignment it asks for, and
  what lies before and behind is split off and stays free. A release joins
  the object to the neighbours it touches, which is what `memory_merge` is
  for and what keeps the store from grinding itself down.

  `Handle::MAX` is new in `audhsos-abi`: the widest handle there is, so that
  code which must name a handle in a `const` can.

- `memory_merge`, a forty-second system call: one memory object out of two
  that lie side by side. The two must be in that order, agree in kind and
  cache policy, carry the same rights, and be held by the caller and by
  nothing else — a mapping is a reference, and an object that something maps
  may not be dissolved under it. The lower one grows to cover both, the
  upper one ceases to exist, and its kernel object goes back to the quota.

  This supersedes the clause of D-12 that had objects split and never
  merged, and what forced it is not a test. An object that can only become
  smaller means a memory server that hands memory out and takes it back
  grinds its objects down to single pages and can never serve a large
  request again: fragmentation with no floor under it. Catalog item 6.6.23
  had asked since it was written that two released neighbours be handed out
  as one, and with a kernel that only splits, that is unsatisfiable (D-88).

  The const assertion in `user-sys-x86_64` earned its keep on the first
  build after the table grew: it named the missing wrapper before anything
  else could.

- `user-loader`: the ustar reader for the boot archive, a ustar writer
  beside it, and `plan`, which says what of a user ELF has to be mapped
  where. Both are parsers over borrowed bytes with no system call in them,
  so both run on the host and under the fuzzer.

  The reader is strict where a loader has to be: an archive is input the
  root task did not write, so a name that is absolute or that walks upwards
  through `..` is refused where it is read; a header whose checksum does not
  match is refused; and the walk ends at the first entry that does not read,
  because the next header is found by the size field of this one.

  `plan` adds to the ELF reader only what is about user space — the bounds,
  and two segments that share a page once rounded, which the format allows
  and a loader that maps pages cannot. It answers with a plan and not an
  action, so all of it is testable without a capability in sight.

  New fuzz target `tar`, with its own seed corpus. It found one thing on its
  first run, in the target rather than in the reader: a search stops at its
  match, so it can succeed over an archive whose later headers are broken,
  and the assertion that the two must agree was wrong. That input is in the
  corpus as `find-stops-before-a-broken-header`.

- `user-proto`, the messages the servers speak: the name protocol, the
  console protocol, and the memory protocol, each a type with `encode` and
  `decode` and no system call in it. A label carries the version in its high
  sixteen bits, the protocol in the next sixteen, and the message in the
  low sixteen, so a server refuses a version it does not speak before it
  reads a word — and the version is read before the protocol, so a client of
  a later release is told which of the two it is.

  Every reply begins with a status word, zero or the code of an error of the
  interface, and carries its payload only behind a status that says the
  request succeeded: a failed lookup carries no endpoint, a failed
  allocation no memory object. Catalog item 6.6.56 is new and covers them;
  12.9 had asked for it before the encodings could be written.

- The gate, in `user-sys-x86_64`: the address of a thread's IPC buffer and
  the forty-one system calls as methods with names, types, and `Result`. A
  wrapper writes the call number and its arguments into the buffer, executes
  `int 0x80`, and turns the status word into an error or a value; a handle
  comes back as the typed handle of its object kind. The assembly is
  unchanged — one `int 0x80`, as it has been since Phase 5.

  The address belongs to the thread and not to the process: the buffer of
  the thread in slot `n` lies `n` pages below `IPC_BUFFER_TOP`, so a process
  with two threads has two of them. A gate is therefore a value a thread
  carries, and never a global — there is no thread-local storage in this
  system to put one in, and a single global would be a silent corruption the
  moment `server-console` starts its second thread.

  The wrappers are written out one by one rather than expanded from the call
  table. What holds them to it is `COVERED`, a list of the calls the gate
  has a method for, checked against `Syscall::ALL` in a const assertion: a
  call added to the table and forgotten here fails the build, and so does a
  wrapper for a call the table no longer has.

  `program!` is the entry point that goes with it. It builds the gate,
  reads the startup message out of the buffer before the first call
  overwrites it, and hands `main` both. The older `entry!` stays for the
  programs of the kernel test images, which make calls on purpose that no
  typed wrapper would let them make.

- `user-rt`, the part of a user program that needs no machine: the nine
  typed handles the system calls take, the heap, the message area of the IPC
  buffer, the startup message as named fields, and a line of text in a fixed
  number of bytes. The crate makes no system call and knows no instruction,
  so all of it runs on the host under test and under the coverage gate.

  The heap is one free list with first fit and coalescing on release, and
  not the size classes 10.7.2 sketched: 6.6.12 requires that two released
  neighbours become one block large enough for their sum, and a class list
  hands back two blocks of the class they came from however they lie in
  memory. A release names only its offset, because a table of live blocks
  remembers the length — which also lets the allocator refuse a release of
  something it never handed out. Both tables are const generics, so a
  program that allocates twice does not carry the tables of one that
  allocates ten thousand times.

- The startup message, in `audhsos-abi::startup`. A process that has just been
  created finds it in the IPC buffer of its first thread, and it says which
  handle of its table holds which role: a handle is a number, and nothing else
  tells a new program that entry two is the name server and not the memory
  server. Whoever created the process writes it — the kernel for the root
  task, the root task for every other process.

  It is a list of pairs, role and handle, and not a list of fixed positions,
  because the processes of this system are given different things: the root
  task receives one memory object per free region of memory, of which there
  are as many as the machine has, and an application receives two endpoints
  and nothing else. A reader takes the roles it knows and passes over the
  rest, so a role added later reaches an older program as nothing at all
  rather than as a shifted field. The ten roles are a table macro beside
  `error_codes!` and `object_types!`, so name, code, and lookup come from one
  place. The label spells `STARTUP` and lies below the range the kernel keeps
  for itself, which a const assertion holds to.

- Two user threads that meet, and a driver at ring three. Five new programs
  under `user-test-programs`: `ipc_client` and `ipc_server`, which are a call
  and a reply with a badge, four words, and a handle to a memory object both
  processes end up mapping; `fault_handler`, which receives the fault messages
  of another process and either answers, repairs, or ends it;
  `write_unmapped`, whose fault a handler can do something about; and
  `notification_waiter`, which takes ISA line zero, binds it to a bit of a
  notification, programs the interval timer through a range of I/O ports, and
  waits for the bit.

  The interval timer and not the local APIC timer: that one is the kernel's
  own, it carries a vector of its own, and the vector plan gives it no global
  system interrupt, so no interrupt object can name it. ISA line zero reaches
  the I/O APIC through the interrupt source override the tables carry, and
  programming it is three port writes — which makes the one program cover the
  interrupt path, the port path, and the notification path from ring three.

  `tests/ipc.rs` reads what the pair left: the badge, the words, the handle
  that names the same memory in both processes, a message of no words, one of
  the widest payload, one word too many, a receive that refuses to wait, a
  sender killed while it waited, and an endpoint destroyed under a waiter. The
  two queues of one endpoint cannot both hold a waiter at once — whichever
  side arrives second meets the first — so the QEMU test shows them one at a
  time and the host test of `kernel-ipc` shows them together.
  `tests/isolation.rs` gains the fault handler: the message with the reserved
  label, the kind, and the address; a reply that resumes the thread into the
  instruction it faulted on, where it faults again; a handler that gives up
  and ends the process; and one that maps a page at the address the fault
  named, answers, and lets the thread run on.

  A program that has more to report than two return words hold writes into a
  page the kernel shares with it: its own message area is where the messages
  of the test are, and a log kept there would be overwritten by the very
  message it is about.

- The twenty-one calls the system call table was still missing, which
  completes it: `process_set_fault_handler`, `endpoint_create`,
  `endpoint_badge`, the six endpoint operations, the four notification
  operations, the three interrupt operations, the three port operations,
  `memory_create_device`, and `system_info`. `kernel_syscall::calls::
  UNIMPLEMENTED` is empty, and no call of the interface answers
  `Unsupported` any more.

  `Environment` grows the seam to a second buffer — `with_buffer`, which
  reaches the IPC buffer of a frame — beside the port, interrupt, memory-map
  and firmware questions the new calls ask. `Reply` grows `blocked`, which is
  what a call whose caller has no result yet leaves: the dispatcher writes
  neither the status word nor the return words, and the thread that
  completes the rendezvous writes them into the buffer of the thread it
  wakes. It also grows the words of a result that does not fit into two
  return words, which go into the message area of the caller's own buffer
  with label zero and handle count zero: `system_info` reports twenty and
  says so in its first return word, and `thread_info` reports the three of a
  fault beside the state and the kind.

  A handle is now a reference. `handle_close` and the close of a dying
  process's list release it, `handle_duplicate`, `process_install_handle`,
  and the handle installation of a transfer take one, and when the last one
  goes the object is destroyed — which for an endpoint, a notification, and a
  reply object wakes everyone who waited (D-75). A process and a thread each
  hold one reference to themselves besides, because a process ends when it is
  killed and a thread when the reaper has given back what it held, whatever
  handle still names it; `Pool::force_release` is what those two do.

  `fault::deliver` builds the fault message in the faulting thread's own
  buffer — the reserved label of its kind, the address, the instruction
  pointer, and the error code — and performs a `call` on the fault handler
  endpoint of its process. The thread is then blocked as any caller is, and
  the reply resumes it at the instruction it faulted on. A process with no
  handler, a handler endpoint that is gone, and a reply pool with no slot
  left all end the same way, in `Faulted`. `Exception::fault` in
  `kernel-core` is the mapping from vector to kind, and
  `KernelEnvironment` gains the devices of the machine and the ACPI pointer
  the bring-up read.

  In QEMU, `every_syscall` walks the whole table: every call answers exactly
  one error, and every call whose success a single thread can observe answers
  a success as well. The six endpoint calls whose success is a rendezvous are
  the exception — a thread that sends with nobody receiving waits for ever —
  and the `ipc` image covers those.

- The two hardware seams the system call layer of Phase 6 needs.
  `kernel-hal-api` gains `paging::FrameBytes`, which reaches a frame as
  bytes — the seam a second IPC buffer is read and written through — with
  the double `MemoryFrameBytes` and the implementation of the physical
  window. And it gains `device::Devices`, one bound over
  `InterruptController` and `PortAccess`, because the four calls that need
  them are four calls of one table and the kernel environment carries what
  they need in one field; the double is `RecordingDevices` over the two that
  exist. The feature `port-io` stops being unused: `kernel-hal-x86_64`
  enables it.

  In the adapter, `Ports` is the `PortAccess` implementation and
  `DeviceAccess` joins it with the interrupt controller. `instructions`
  gains `read_port_u16`, `write_port_u16`, and `read_port_u32` beside the
  three it had, which are three new `asm!` sites; the budgets rise with
  them.

- The crate `kernel-ipc` at `crates/kernel/ipc`, layer 3: the rendezvous
  state machine over the object pools and the scheduler. Endpoints with a
  queue of senders and one of receivers, reply objects, notifications,
  message transfer, interrupt delivery, and what a destroyed object owes the
  threads that waited on it.

  Every endpoint operation is two steps, because a message moves through two
  IPC buffers and nothing in this crate reaches a frame or an address space.
  The first step finds the peer or queues the caller; the caller then copies;
  the second step says what became of the two threads. Between the two steps
  the endpoint holds neither of them, which is what makes a refused copy
  leave nothing behind — and `undo_meeting` puts the peer back at the front
  of its own priority, where the meeting took it from, because nothing runs
  between the meeting and the copy.

  `transfer` is the one function that moves a message, and its order is what
  makes a refused message change nothing: the header is validated first,
  then every handle is resolved in the sender's list and checked for
  `TRANSFER`, and only then are the words copied and the handles installed.
  A handle without that right therefore fails before any other handle of the
  same message is installed. Installation stops when the receiver's list is
  full: the message is delivered, the receiver's handle count says how many
  arrived, and its status word carries `PARTIAL`, which is the error flag of
  2.6.2 and not an error. The reserved label range is checked at the entry
  of `ipc_send` and `ipc_call` and not here, because the fault message the
  kernel builds carries such a label by construction.

  Every operation returns an `Outcome`: whether the caller blocks, its
  status and return words when it does not, which thread became ready and
  what goes into its buffer, and whether the caller should switch. A
  destroyed object returns `Waiters`, which hands out the threads it left one
  at a time — there can be as many of them as the machine has threads, and
  each needs its own buffer written (D-75).

  Two things the writing settled. `WaitQueue` keeps its operations in
  `kernel-objects`, beside the pool its links thread through, because a
  method has to live in the crate that defines the type and the queue is a
  field of `Endpoint`; `kernel-ipc` is the rendezvous that uses it.  And a
  thread queued as a sender has to say what it asked for, because whichever
  side arrives second completes the meeting: `Queue` therefore has three
  variants, `Senders` and `Callers` in the senders queue and `Receivers` in
  the other, and a cancellation treats the first two alike.

- One row in the transition table of `kernel-sched`: `BlockedSend` plus
  `BlockReply` is `BlockedReply`. It is the only transition from one blocked
  state to another, and it is the queued caller of `ipc_call`, which waited
  for a receiver and waits for the answer once one has taken its message —
  without ever having been ready in between, which is what makes the send
  and the wait of a call atomic from the receiver's point of view.

- The six object structures of Phase 6 in `kernel-objects`: `Endpoint` with
  a queue of senders and one of receivers, `Reply`, `Notification`,
  `Interrupt`, `IoPortRange`, and `SystemControl`, which holds nothing and
  therefore has no pool — a capability to it is the whole of the right to
  create interrupts, port ranges, and device memory. Their five pools take
  their sizes from `config` directly rather than becoming parameters of
  `Objects`: together they stay under 200 KiB against the 1.2 MiB the four
  parameterized pools reach, so a host test can still hold a machine with
  all of them at full size, and the signature of everything that touches
  the machine stays four parameters wide instead of nine.

  A `Thread` now carries two link pairs. The run queues of `kernel-sched`
  use `queue_links`, the wait queues use `wait_links`, and a thread is in
  at most one of the two at a time, because a thread in a run queue is
  `Ready` and a thread in a wait queue is blocked. They are separate
  fields because `Scheduler::dequeue` corrects the head, the tail, and the
  ready bitmap of a run queue after it unlinks: handed a thread whose
  links pointed into an endpoint queue it would splice that queue and
  leave the endpoint naming a thread that is no longer in it. Beside the
  links the thread carries `wait`, which names what it waits on so that a
  cancellation finds the queue without searching every endpoint, and
  `fault`, which is what `thread_info` and a fault message report.

  `WaitQueue` orders by priority and then by arrival: a thread goes behind
  the last thread of a priority at least its own, so the walk is bounded by
  the length of the queue and no second structure carries the order. The
  position is fixed at that moment; a later `thread_set_priority` does not
  move a thread that already waits. The queue lives beside the pool its
  links thread through and not in `kernel-ipc`, because its operations need
  that pool.

  `Objects` gains `retain`, `destroy`, and `capacities`. `destroy` is the
  one place a reference count of zero turns into the destruction of an
  object; what destruction owes the threads that waited travels back to the
  caller in `Destroyed`, because waking a thread needs the scheduler and
  writing its status word needs its IPC buffer. `counts` reports nine
  numbers rather than four, the eight pools and the handle arena, which is
  what `system_info` will report. `HandleArena::close_all` becomes
  `close_next`, so that every entry a dying process held passes through the
  caller's release path one at a time: a count is not enough once a handle
  is a reference.

- The label range the kernel keeps for its own messages, in
  `audhsos_abi::ipc_buffer`: `KERNEL_LABEL_BASE` is the first reserved
  label, `FAULT_LABEL_BASE` is the same value, and the label of a fault
  message is that base plus the code of its `FaultKind`, so the six kinds
  occupy the first six labels and 250 are left for the kernel messages of
  later phases. `is_kernel_label`, `fault_label`, `fault_kind_of`, and
  `Message::is_kernel_label` read it. The range is reserved rather than
  merely documented because `ipc_send` and `ipc_call` refuse such a label
  before anything is copied; the check is at their entry and not in the
  transfer, which exists to carry the one message that has to have one.

  Beside it two smaller pieces the phase needs: `Error::Cancelled`, which
  is what a thread finds in its status word when `thread_suspend` took it
  out of a wait queue, and `layout::MAX_RESULT_WORDS`, the width of a
  system call result that does not fit into two return words and goes into
  the message area of the caller's own buffer instead.

- The fuzz target `rsa` and three real chains, step R6 of the RSA track,
  which is the end of it.

  The target builds a key and a signature out of the same input and
  verifies one against the other under whichever of the six schemes the
  first byte names. Nothing in it asserts that a signature verifies — a
  random string almost never is one, and asserting that it is not would be
  asserting that the fuzzer is unlucky. What it asserts is that a key that
  parses reads back as the key it was built from, and what it watches for
  is time: everything it reaches is bounded before it runs, so a
  verification that took long would be a bound that is missing. The corpus
  holds a valid signature under each of the six schemes, the eight
  malformed encodings the unit tests refuse, and two inputs that are not a
  key at all.

  `tools/tls-probe` reaches `www.ietf.org`, `www.rust-lang.org`, and
  `www.bbc.co.uk` — the three chains section 11.15 named as the cost of
  the omission. They do not exercise the same thing.
  `www.rust-lang.org` is RSA the whole way, a 2048-bit leaf under a
  2048-bit intermediate under the 4096-bit `ISRG Root X1`.
  `www.bbc.co.uk` is the same shape under `GlobalSign Root R46` with a
  `sha384WithRSAEncryption` link in it. `www.ietf.org` serves an ECDSA
  chain whose root is cross-signed with RSA, so the same host verifies
  through RSA under `ISRG Root X1` and through P-384 under
  `ISRG Root X2` — two anchors, two algorithms, one connection.

  The default anchor changes from `GTS Root R4` to `GTS Root R1`, because
  Google moved `*.google.de` from a P-384 root to an RSA one while this
  work was going on. The probe's own default host stopped verifying
  without a line of its code changing, which is the plainest argument for
  this whole track there is. `GTS Root R4` stays in `anchors/` for the
  record, `ISRG Root X2` joins it as the P-384 anchor that a live chain
  still reaches, and each of the five is recorded with its fingerprint and
  the trust store it was taken from.

- `audhsos-tls` offers and honours the RSA signature schemes, step R5 of
  the RSA track, and the seam section 11.11 named as open is closed.

  Six code points join `signature_algorithms` in the `ClientHello`, so it
  now carries nine. The three `rsa_pss_rsae_*` are accepted in a
  `CertificateVerify` over an RSA key. The three `rsa_pkcs1_*` are
  refused there, by an arm of their own so that the refusal is visible
  rather than a fall-through — RFC 8446 section 4.2.3 gives those code
  points exactly one meaning, that a certificate may be signed that way,
  and forbids them in a handshake signature (D-82). Offering what one
  refuses is not a contradiction here: the offer is about the chain, the
  refusal is about the signature the server makes itself, and both
  directions have a test.

  `MAX_SPKI` goes from 128 to 550, which is what an RSA-4096 subject
  public key information occupies — fifteen for the algorithm identifier,
  five hundred and twenty-six for the inner sequence of two integers, five
  for the bit string, four for the outer sequence. The 160-byte buffer
  that holds the signed content does not change: the transcript hash is
  the cipher suite's, which is SHA-256 or SHA-384 and never SHA-512,
  however the signature is hashed.

  The replay of RFC 8448 now checks the server's signature. It did not
  before, and the reason was that the trace signs with
  `rsa_pss_rsae_sha256`. The `CertificateVerify` of the simple 1-RTT
  handshake verifies through `crypto-rsa` under the key section 2 of that
  document prints, against the transcript this crate computes, and fails
  against a transcript one bit different. The key is a thousand and
  twenty-four bits, below what a certificate may carry (D-79), so the
  chain around the signature stays stubbed and the signature inside it
  does not. The whole path is walked in the state-machine test instead,
  twice: against a server with a two-thousand-and-forty-eight-bit RSA
  chain and against one at four thousand and ninety-six.

  `audhsos-tls` gained no dependency for any of this. The crate table of
  section 11.3 said it would take `crypto-rsa`; a handshake signature is
  verified the way a certificate signature is, through
  `SubjectPublicKey::verify`, so the edge that carries RSA into the client
  is the one to `audhsos-x509` that was already there. The table is
  corrected.

- `audhsos-x509` reads and verifies RSA, step R4 of the RSA track. `oid`
  gains `rsaEncryption`, the three `sha*WithRSAEncryption` arcs,
  `id-RSASSA-PSS`, `id-mgf1`, and the three hash identifiers, each with
  the section that assigns it.

  `SignatureAlgorithm::parse` had one rule for every algorithm — it
  called `finish` on the identifier's fields and so demanded the
  parameters be absent — and now has a rule apiece. Absence stays the
  only right answer for ECDSA and Ed25519. For the three RSA identifiers
  NULL and absent are both right, which is what RFC 4055, section 5
  says and what the old rule got wrong. `id-RSASSA-PSS` is a third rule:
  its parameters are required, and only the three sets that pair a hash
  with MGF1 over that same hash and a salt as long as its output are read
  (D-81). A set with a salt of another length, a mask over another hash,
  an unknown hash, or a mask function that is not MGF1 is refused.

  `SubjectPublicKey` gains an `Rsa` variant holding the modulus and the
  exponent as borrowed slices, parsed from the inner `RSAPublicKey` of
  RFC 3279, section 2.3.1. The DER reader needed nothing new for it:
  `read_integer` already strips the leading zero of a positive integer
  and refuses a negative one. `check_usable` applies the size bound of
  D-79 — two thousand and forty-eight to four thousand and ninety-six
  bits — which is what makes an out-of-range anchor a refusal rather than
  a path that reaches nothing, and it is here rather than in `crypto-rsa`
  for the reason that decision gives.

  The builder gains RSA keys over the fixed pairs of D-83, and three
  sizes stop fitting: `MAX_CERTIFICATE` goes from a kibibyte to two,
  `TestKey::public_key` from a 97-byte array to a 526-byte one, and the
  signature buffers from 128 bytes to 512. Two key pairs, of two thousand
  and forty-eight and four thousand and ninety-six bits, join the test
  sources with the `openssl genrsa` invocation that made them.

- `crypto-rsa` gains MGF1 and EMSA-PSS-VERIFY, step R3 of the RSA track.
  RFC 8017, appendix B.2.1 is the mask and section 9.1.2 is the
  verification, with three of that section's parameters fixed rather than
  read: the mask uses the same hash that made the digest, the salt is as
  long as that hash's output, and the trailer is `0xBC`. That is what
  RFC 8446 fixes for the three `rsa_pss_rsae_*` schemes, which are the
  only PSS this client offers (D-81). The salt length in particular is
  not recovered from the position of the separator, because a verifier
  that recovered it would accept a salt of any length.

  Each of the five inconsistencies section 9.1.2 names has a test, and
  each is a block laid out by hand and then signed with a private
  exponent, so what verification meets is a signature that recovers
  exactly the block the test meant: a trailer that is not `0xBC`, a
  leftmost bit set where `emBits` says there is none, padding before the
  separator that is not zero, a separator that is not `0x01`, a salt one
  byte short and one byte long, and a recovered salt that `H` was not
  computed over.

  A two-thousand-and-forty-eight-bit key pair joins the test sources for
  the one case the RFC 8448 key cannot carry: PSS with SHA-512 needs a
  hundred and thirty bytes of encoding and that key has a hundred and
  twenty-eight. It was made once outside this repository with
  `openssl genrsa 2048` and is recorded with the command that made it,
  which is the arrangement D-83 sets out.

- `crypto-rsa`, step R2 of the RSA track: the public key with the bounds
  that belong to the primitive, and PKCS #1 v1.5 verification. The key
  takes a modulus that is odd, has its top bit set, and is no wider than
  the arithmetic allows, and an exponent that is odd and at least three.
  It does not take the lower bound on the key size: that is a rule about
  certificates and belongs where certificates are judged (D-79). One test
  in the file says so by name — the thousand-and-twenty-four-bit key of
  RFC 8448, section 2 is accepted here, and no chain will ever carry it.

  The encoding is built and compared, never parsed (D-80). The three
  `DigestInfo` prefixes are the byte strings RFC 8017, section 9.2 note 1
  writes out, each with that citation beside it, and a test checks the
  block this crate builds against them field by field. What the choice
  buys is the eight refusals underneath it, each of them an encoded
  message a decoding verifier would accept, signed with the private
  exponent of that key so the arithmetic recovers it exactly: a padding
  of seven `0xff` bytes, a `DigestInfo` moved into the block, bytes after
  the digest, a separator overwritten, a first byte that is not `0x00`, a
  second that is not `0x01`, a `DigestInfo` whose `SEQUENCE` carries an
  indefinite length — which PKCS #1 v1.5 allows and this crate refuses,
  the incompatibility D-80 states — and Bleichenbacher's forgery against
  an exponent of three, which is not signed at all but is the integer cube
  root of a block with a short padding and a long tail.

  Verification has one error for every way it can fail. Which way it was
  is exactly what the sender would like to know.

  Signing is behind `test-signing` and is one call into `pow_wide` with
  the private exponent. No key is generated (D-83).

- `crypto-bignum`, step R1 of the RSA track: limb arithmetic over slices,
  and a `Modulus` that arrives at run time. `crypto-ec` kept its four
  `Params` implementations, whose modulus, inverse, and conversion
  constant are known when the code is compiled, and gave up the three
  operations underneath them — `montgomery`, `add_limbs`, and `subtract`,
  which already took their modulus as an argument. Those now live in the
  new crate over `&[u64]` rather than `[u64; N]`, together with the
  comparison the in-place shape of the other two needs, and `crypto-ec`
  imports them. Its P-256 and P-384 suites are what says the move changed
  nothing: they pass unaltered.

  What is new above them is `Modulus`, which derives at run time what
  `Params` writes down. `n0inv` is `-m^-1` modulo `2^64` by Hensel
  doubling — an odd modulus is its own inverse modulo eight, so five steps
  carry three correct bits past sixty-four — and `R2` is `2^(128*used)` by
  that many modular doublings, eight thousand one hundred and ninety-two
  of them for a four-thousand-and-ninety-six-bit key, paid once. One width
  is held and the used count decides every loop bound (D-78), so a
  two-thousand-and-forty-eight-bit key costs a
  two-thousand-and-forty-eight-bit multiplication and a
  four-thousand-and-ninety-six-bit frame.

  `pow` is left-to-right square-and-multiply and is not constant time; the
  crate documentation states that as the design and says what holds it up,
  which is the boundary rather than the code — no secret enters this
  crate. The exponentiation has two doors: `pow` takes the exponent as a
  `u64`, which is what verification needs, and `pow_wide` takes it as
  bytes, which is what signing a test certificate with a private exponent
  needs. Section 11.15.2 records the second one, which the plan implied in
  prose and left out of its sketch.

  The tests are a schoolbook reference in the test module — quadratic
  multiplication, binary long division, no Montgomery form anywhere — and
  properties against it at one thousand and twenty-four,
  two thousand and forty-eight, three thousand and seventy-two, and four
  thousand and ninety-six bits. The round trip of catalog 6.6.55 signs
  with the private exponent of the key RFC 8448, section 2 prints and
  verifies with its public one.

- RFC 8017, RFC 4055, RFC 5756, and RFC 3279 join the reference documents
  under `docs/rfc/`, each fetched twice and recorded with its checksum.
  They are what RSA verification reads: PKCS #1 for the primitive and both
  encodings, the two X.509 documents that name the algorithms and their
  parameters, and RFC 3279 section 2.3.1 for `rsaEncryption` and
  `RSAPublicKey`. The README gains a section for the four and a second for
  what was read and left out, and the RFC 3279 entry under *What P-384
  does not need* is corrected: it was kept out because RFC 5480 restates
  `ECDSA-Sig-Value`, and that reason says nothing about the RSA key, which
  no later document restates.

  Nothing is implemented yet. The documents are here so that the plan can
  cite sections rather than recollection — among them one correction the
  plan needed: RFC 4055 section 5 requires the parameters of
  `sha256WithRSAEncryption` and its two siblings to be NULL and requires
  an implementation to accept them absent as well, so the parser has to
  take both forms and not, as first written down, only the present one.

- RFC 2313, PKCS #1 version 1.5, joins the reference documents, fetched
  twice and recorded with its checksum. It was listed as not needed, and
  as an implementation reference it still is not: RFC 8017 delegates
  nothing to it and names it only informatively. It is here because D-80
  refuses a `DigestInfo` that is BER but not DER, and that refusal is a
  claim about this document. From the source the claim is sharper than
  from RFC 8017's summary of it. Version 1.5 does not merely permit the
  looser encoding: its sections 10.2.3 and 10.2.4 define verification as a
  BER decode followed by a digest comparison, so the two specifications
  describe two different operations. The refusal is a deliberate
  incompatibility and section 11.15.1 now says so. The document also
  settles what could until now only be inferred — it carries no table of
  `DigestInfo` prefixes, indeed no byte string at all, so the constants of
  RFC 8017 section 9.2 note 1 have one source and not two.

- RSA verification is planned rather than deferred: section 11.15 of
  document 11, decisions D-77 to D-83, catalog section 6.6.55, and steps
  R1 to R6 in the roadmap and in the order of work. Nothing is
  implemented.

  Two things the planning turned up are worth naming here, because both
  contradict what was written down before. `crypto-ec::montgomery` cannot
  serve an RSA modulus as it stands: it is generic over the limb count,
  which is what section 11.2 said, but its `Params` carries the modulus
  and the conversion constants as associated constants, and a certificate
  brings its modulus at run time. The arithmetic therefore moves to a
  crate of its own (D-77), keeping the three free functions that already
  take a modulus as an argument. And the RSA key printed in RFC 8448
  section 2 is 1024 bits, not 2048 as this changelog's previous entry and
  the reference README first said: it is below what a certificate may
  carry (D-79), which makes it a vector for the primitive and never a
  chain. Section 11.2, section 11.11, and the reference README are
  corrected accordingly.

- The model-test runner refuses a run that reached nothing.
  `ModelTest::required` names the states a run's sequences have to arrive
  at and `ModelTest::reached` reads back what the model arrived at; a run
  in which every sequence passed and one of those names was never reached
  fails as `ModelFailure::Vacuous`. That is the one way a model test
  rots — the generator drifts, or the component grows a state the
  operations no longer reach — and until now reaching nothing looked
  exactly like reaching everything and finding no fault. Both hooks
  default to nothing, so the seven existing model tests are unchanged.

- A model test of the neighbor cache under Neighbor Discovery
  (catalog 6.6.54): one neighbor, the five states of RFC 4861,
  section 7.3.2 and the schedule of its section 10 written beside the
  cache rather than out of it, driven by generated sequences of packets,
  advertisements, solicitations, waits, and polls. It names all eleven
  states and events it must arrive at, and a second run against a model
  with the stale-to-delay transition removed shows that a regression
  there is found and shrunk to a handful of operations.

- Every system call of this kernel, made from ring three.
  `tests/syscalls.rs` runs `every_syscall`, a program that walks the whole
  table and writes down a pair per call — the number and the status word —
  so the tests read the log rather than the order the program went in.
  Sixty answers to forty-one calls: each of the twenty this phase
  implements answered a success once and an error once, and each of the
  twenty-one it leaves to Phase 6 answered exactly the refusal the table
  of the interface asks for — `Unsupported` where the call is reachable,
  `WrongObjectType` where its first argument names an object type of a
  later phase. The host tests reach every path of every call with a
  recording double; what this shows is that a user thread reaches them at
  all.

- `net-udp`, the first transport of the network stack (D5, 12.6.7). The
  datagram of RFC 768 under the checksum rule of each family: over IPv4 a
  zero means the sender computed none and the datagram is taken as it is,
  over IPv6 a zero is a datagram to discard, because RFC 8200,
  section 8.1 took away the header checksum that used to catch a
  corrupted address underneath it. A sum that comes out zero goes on the
  wire as all ones in both, so that the omitted form keeps a spelling of
  its own. The length field is what a reader believes: bytes behind it are
  padding a link layer left standing and are cut away, and a length that
  reaches past what arrived is an error — the checksum covers exactly what
  the field claims, so the two always agree.

  A fixed table of sockets above it. A socket bound to one of this host's
  addresses takes only what named that address; one bound to none takes
  everything that reaches its port, broadcast and multicast included,
  which is the form a DHCP client needs before it has an address at all.
  A port is held once, whichever address holds it. An ephemeral port is
  drawn from the dynamic range of RFC 6335, which is exactly `2^14` ports
  wide, so the low fourteen bits of two random bytes name one without the
  bias a remainder would introduce; a port already taken is answered by
  drawing again rather than by walking to the next one, as RFC 6056,
  section 3.3.1 asks, because a port beside a taken one is a port an
  observer who saw the first can guess (D-51).

  The receive ring lies in memory the caller supplied and holds
  self-describing records — the two addresses the datagram arrived
  between, the port it came from, and the payload. A record that no longer
  fits behind the last begins at the front of the buffer instead of being
  cut in two, so a payload always comes back as one slice, and one limit
  governs the whole ring instead of a slot size that is wrong for every
  datagram but one. A full ring drops the newest and counts it, and a
  datagram that would not fit an empty ring is refused at entry.

  The crate sends nothing, and therefore depends on no internet layer at
  all: it writes a datagram for a pair of addresses and reports a datagram
  for a port nobody holds as such. Which of the two senders carries the
  one out, and whether RFC 1122, section 3.2.2 allows an ICMP error for
  the other, are decisions of layers that hold the header fields those
  rules are about (D-84). Catalog 6.6.45.

- `net-tcp`, the state machine of RFC 9293 (D6, 12.6.8). The eleven
  states, active and passive open, and the sequence arithmetic every one
  of their decisions rests on — a comparison of distances on a circle of
  `2^32` and never of magnitudes, so that the wrap at the top of the range
  costs nothing and the one case the relation cannot decide is the one a
  window of 65535 bytes cannot produce.

  The two windows are byte rings the caller supplied. A segment that
  arrives out of order is written straight to its place in the receive
  ring, because its sequence number says where that place is, and a short
  list of ranges remembers which parts have arrived; when the gap in front
  of them closes the ranges grow together and the bytes are readable
  without having been copied a second time. A segment's payload on the way
  out is a borrow into the send ring and never a copy, which is why a run
  stops at the physical end of the buffer rather than wrapping around it:
  the cost is one short segment per wrap, and what it buys is that a
  retransmission is the same borrow again.

  The retransmission timer is RFC 6298 with Karn's rule — a segment sent
  twice yields no measurement, and the backoff it earned stands until an
  unambiguous one arrives — and the attempts are counted per segment as
  RFC 1122, section 4.2.3.5 counts them, so a connection that loses one
  segment in each of nine rounds is not a connection that gives up.
  Congestion control is Reno of RFC 5681: slow start, congestion
  avoidance, and a fast recovery in which the third duplicate
  acknowledgment sends the one segment the duplicates point at and not
  everything behind it. Acknowledgments are delayed to the second
  full-sized segment or half a second, whichever comes first, and never
  delayed for a segment that opens a gap, fills one, or arrives at a
  window this end had closed. A closed window is probed by one byte that
  does not count as sent, so the next probe carries the same byte again.

  Resets follow RFC 5961: a reset is believed only at the exact number
  expected, and one merely inside the window draws an acknowledgment,
  which costs a blind attacker the sequence number as well as the window.
  A `SYN` inside the window of an open connection is answered and never
  acted on.

  Two send pointers are kept apart, and both distinctions are ones that
  leave two ends answering each other for ever when they are not made. A
  segment that carries nothing carries the highest number ever sent and
  not the number to send next, which a retransmission timeout has wound
  back; and an acknowledgment is judged against that same highest number,
  so that an acknowledgment of everything that went out before the rewind
  is good news rather than a segment to argue with (D-85).

  Above it a connection table: a segment finds its connection by the
  four-tuple, failing that the connection listening on its destination
  port, and failing that a reset — which is the processing of state
  `CLOSED` and belongs here, because which numbers that reset carries is
  decided by header fields nothing above this crate reads.

  Ninety-nine tests and a model test. The model runs two instances of the
  machine back to back over a link that delays, duplicates, reorders, and
  drops, under a generated schedule, and holds that both streams arrive
  whole and in order, that both ends reach `CLOSED`, and that neither ever
  sends a segment at an instant at which it said it had nothing to say.
  Nothing sleeps: a sixty-second `TIME-WAIT` and a one-second timeout both
  pass in microseconds of wall clock. Fuzz target `tcp_segment` drives the
  parser, its option list, and the state machine behind them.
  Catalog 6.6.46.

- RFC 9293 under `docs/rfc/`, which is TCP in one document: the seven
  memos that amended RFC 793 over forty years, folded back into the text
  the state machine of section 3.3.2, the segment-arrives procedure of
  section 3.10.7, and the acceptance test of section 3.4 are read from.

### Changed

- Miri runs the tests of the modules that hold `unsafe`, not every test of
  the crates around them (D-76). `MIRI_TARGETS` carries a test-name filter
  per crate: `audhsos-sync` runs whole, being two `unsafe` sites and the
  cell around them, and `fuzz-support` runs `tests::counters` and
  `tests::sancov`, which test its two files that hold `unsafe`. Everything
  else in that crate — the mutators, the corpus, the pool, the dictionary,
  the options, the generator — is safe Rust that `test --host` and the
  coverage gate already cover, and interpreting it bought nothing.

  What it cost was the check. `tests::mutate` reached the same global as
  `tests::sancov` through `sancov::with_trace`, over sixty thousand
  mutation rounds, and that one module was nearly the whole of the step:
  `miri` went from 243 seconds to 4, and the full check from just over six
  minutes to two. The filters are held to the code by
  `unsafe_budget::miri_gaps`, which reads every product file of a filtered
  crate and fails the step when one holds `unsafe` and no filter names its
  module, naming the filter that would close it — so `unsafe` cannot
  appear in a new module and quietly leave Miri's reach.

- A kernel stack is eight pages, not four, and `Pool::release` no longer
  hands the object back (D-73). Both come out of one measurement, taken
  when the system call tests first made `process_create` from ring three
  and the machine answered with a double fault at the guard page: the
  deepest path from the system call gate carries a `Process` — four
  kilobytes of region table and thread slots — through three frames in a
  row of the object pool, and measured 33 KiB in the unoptimized build the
  tests run. `Pool::release` returning the value was two of those frames
  and bought nobody anything; it now returns whether the last reference is
  gone, which leaves the deepest path at 23 KiB. A thread therefore costs
  nine frames of the reserve instead of five.

- `net-ipv6`, the IPv6 half of the network stack (D4, 12.6.6). The header
  of RFC 8200 and its extension header chain, walked to the upper layer
  over hop-by-hop options, routing, fragment, and destination options and
  bounded twice — eight headers and 512 bytes — because a chain is a
  linked list an attacker writes. `ICMPv6` of RFC 4443 with the echo pair
  and the three errors, summed over the pseudo-header of both addresses,
  which is the one difference from `ICMPv4` a checksum routine has to
  know. Neighbor Discovery of RFC 4861 to the solicited-node group,
  writing into the neighbor cache of `net-eth` rather than keeping a
  second one, with the hop limit of 255 checked before anything else.
  Stateless address configuration of RFC 4862 from a router
  advertisement, with the prefix, the router, the lifetimes, the link
  MTU, and the recursive DNS servers of RFC 8106, and duplicate address
  detection before an address is used. A held address cannot be expired
  by one advertisement: the floor of RFC 4862, section 5.5.3 (e) is what
  stands between a forged valid lifetime of a second and a host that is
  taken off the network by a single packet. Path MTU discovery of RFC 8201 in
  the send path and not beside it: the sender asks the estimate table for
  every packet, so a packet larger than the path is one this crate cannot
  write. And the send path itself, which cuts a datagram through the
  fragment header, addresses a multicast group by the arithmetic of
  RFC 2464, and asks the same routing table and the same neighbor cache
  `net-ip` does.

- The fuzz target `ipv6`, which drives four parsers: the packet with its
  chain walk, the `ICMPv6` message, the Neighbor Discovery message with
  its option walk, and the reassembler. It exists for the chain, which is
  the one part of this format a byte stream can drive in circles. Its
  seeds are named in words, one per shape a test cites.

- `net_eth::on_conflict`, which is the half of RFC 4861, section 7.2.5 I
  the cache could not make for itself: a neighbor advertisement without
  the override bit leaves a disagreeing hardware address alone and takes
  a `Reachable` entry to `Stale` all the same, so a contradicted mapping
  is checked before the next packet rather than trusted for the rest of
  the reachable time.

- `net_eth::multicast_hardware`, the mapping of RFC 2464, section 7 from
  an IPv6 multicast group to the Ethernet address it is reached at. It is
  arithmetic and not a table, so a solicitation reaches a station this
  host has never heard of without any membership list to keep in step.

- `net_ip::Piece` and `Fragments::with_header`, which are what made the
  reassembly buffers and the fragmentation arithmetic serve both families
  instead of one (D-69). A piece is what either header describes once the
  family-specific part has been read; the header length in front of it is
  an argument, twenty bytes for IPv4 and forty-eight for IPv6.

- RFC 2464, RFC 4443, RFC 4862, RFC 8106, and RFC 8201 under `docs/rfc/`,
  each fetched twice with the two fetches compared, with their bytes,
  their SHA-256, their line count, and what each is kept for (D-59).

- D-72: what the IPv6 half leaves out and why — no redirects, no
  temporary addresses of RFC 4941, no jumbograms, and a flow label
  written as zero.

- Two threads of equal priority take turns, and a thread of higher
  priority takes the processor. `tests/concurrency.rs` runs two threads of
  one process over a page they share: they take alternate tickets from a
  counter in that page, which is what a run queue that hands the processor
  to the thread that has waited longest does. `tests/preemption.rs` starts
  the timer and runs `spin`, a program that makes no system call at all:
  two of them at one priority both get the processor again and again, and
  one of them at a low priority is displaced by a thread of a higher one
  that becomes ready. The kernel writes down which thread the timer found
  running, one entry per turn, and that log is the evidence.

- `two_threads` and `spin`, the two user programs those tests run, and the
  timer, the turn log, and the tick hook the shared test kernel needed for
  them.

### Changed

- The reassembly buffers of `net-ip` are keyed by an `IpAddr` pair and a
  thirty-two bit identification, so that `net-ipv6` uses them rather than
  keeping a second set. `Assembled::Reassembled` no longer carries a copy
  of the first fragment's header: the caller holds the piece that
  completed the datagram, the fields reassembly is keyed on are equal in
  every piece by definition, and the ones that are not are read by no
  layer above. That removed a twenty-byte copy per buffer and the flag
  that said whether it had happened.

### Fixed

- The `ipv4` fuzz target is built with the coverage instrumentation the
  engine steers by. Every other target had its entry in the release
  profile of `fuzz/Cargo.toml` and this one did not, so the fuzzer was
  steering by the coverage of the crates under test alone and not by the
  target's own branches.

- A page table is emptied where it lies (D-70). `PageTable::clear` writes
  the empty entry into every slot in place; assigning `PageTable::default()`
  to a table reached through the physical window made a build without
  optimization materialize the four-kilobyte value on the stack, more than
  once: `Mapper::walk` carried a twenty-kilobyte frame and
  `kernel_half::free_subtree` one per level of its recursion. A kernel
  stack is sixteen kilobytes, so any system call that mapped or unmapped
  anything ran off the end of it. The concurrency tests found it, because
  they are the first in which the reaper runs on the kernel stack of a user
  thread and not on the boot stack; the machine answered with a double
  fault at the guard page.

- One switch per call to the switch loop of the test kernel. Asking the
  scheduler a second time before returning takes the processor away from
  the thread that has just got it, and two threads of equal priority trade
  it back and forth without either of them reaching user mode again.

- A user thread that faults stops, and the machine runs on. The trap
  report carries the code segment of the frame, so the kernel can tell a
  fault at ring zero from a fault at ring three: the first ends the
  machine, the second ends one thread. `kernel_syscall::fault::stop` is
  where that thread stops — `ThreadState::Faulted`, keeping its kernel
  stack, its buffer, and its pool slot, so that whoever created it can
  look at it and either resume it or kill it. Phase 6 puts the fault
  handler endpoint above it; the state stays what a fault nobody took ends
  in.

- `tests/isolation.rs`: the two programs that go looking for the boundary.
  `read_kernel_memory` reads an address of the kernel half, which its
  page tables carry but not for ring three, and takes a page fault;
  `hlt_in_user` runs an instruction only ring zero may, and takes a
  general protection fault. Both threads stop in `Faulted`, keep what they
  hold, and a thread started afterwards runs and ends in the same machine.
  Five tests, and the second fault is handled as readily as the first.

- `tests/support`: the kernel the Phase 5 test images share — the memory
  bring-up, the machine, a process built by hand, the switch into ring
  three, the system call gate, and the trap handler. `tests/user.rs` and
  `tests/isolation.rs` differ in what they watch, not in what they run on.

- The first user thread of this system runs. `tests/user.rs` builds a
  process by hand — an address space carrying the kernel half, a flat
  program mapped read and execute at `0x40_0000`, a stack, an IPC buffer —
  writes the frame onto the kernel stack of a thread, and switches into
  ring three. The thread writes a mark into its own buffer, gives up the
  processor with `thread_yield`, gets it back, and ends itself with
  `thread_exit`; the kernel clears away what it held. Two tests watch it.

- `user-sys-x86_64` and `user-test-programs`: what a user program needs
  from the machine, and one binary per scenario. `sh tools/xtask.sh
  build-user-tests` turns each into a flat binary under `target/user-tests`
  and checks the base address of the linker script against its own; the
  QEMU tests build them first and tell the kernels where they are.

- The address of its IPC buffer is what the kernel hands a thread in its
  first argument register: the buffer cannot be asked for, because asking
  needs it. The buffers of a process are the top pages of its address
  space (`ipc_buffer_address`), the kernel maps one when a thread is
  created, and the frame the trampoline returns through carries the
  address in a word of its own.

- `Scheduler::adopt`, which is how the kernel tells the scheduler that the
  processor is already running a thread — the idle thread of the bring-up
  is the kernel itself, and the first switch needs somewhere to write its
  context.

- `kernel-syscall`: the entry point of the system call interface and the
  twenty calls of this phase. The checks run in the order 2.8 documents —
  number, argument count, handle, object type, rights, arguments, quota —
  and a test for each pair holds that the first failing check is the one
  reported. The argument count is checked without a count in the buffer:
  the words above what the call reads must be zero, which is what a caller
  built against another version of the table looks like.

- `Environment`, the one trait the calls reach everything through that is
  not an object: address spaces, mappings, kernel stacks, frames, and the
  debug console. The kernel implements it over its memory bring-up; the
  tests implement it with a recorder, which is what makes every error path
  of every call reachable on the host. Ninety-four tests do that, and each
  one that expects a refusal also holds that no frame and no kernel stack
  was left behind.

- `reaper::reap` gives back what a thread that has ended held. The tests
  found that `process_kill` and `thread_exit` returned the kernel stack of
  the very thread that was standing on it while the kernel wrote its
  answer: the stack would have gone back into the reserve and been handed
  out again under a running handler. A thread that ends now keeps its
  stack, its IPC buffer, and its slot until the kernel has switched away
  from it, and the state `Exited` is the whole record of what is left to
  clear away.

- `net-ip`, step D3 of track D: the IPv4 header of RFC 791, fragmentation
  and reassembly, `ICMPv4` of RFC 792 under the restrictions of RFC 1122,
  the routing table both families share, and the send path that joins them
  to the neighbor cache of `net-eth`.
- Options are located and skipped, never interpreted. This host originates
  none and needs none to read a datagram, and a parser that understood them
  would be reading a field an attacker writes. An ICMP error quotes a header
  and eight bytes, so the total length that header declares is longer than
  what arrived: `Quoted::parse` reads it where `Datagram::parse` correctly
  refuses to, which is what lets a transport fail a connection fast instead
  of timing out.
- An overlapping fragment discards the whole datagram. RFC 791 leaves the
  case open, and every answer but discarding lets a packet filter be walked
  past — a first fragment shows one transport header and a second overwrites
  it. A duplicate that agrees byte for byte is dropped instead and costs
  nothing.
- Generating an ICMP error goes through the five restrictions of RFC 1122,
  section 3.2.2, which the memo says take precedence over every other reason
  to send one, and a token bucket bounds what is left; it refills at a steady
  rate and keeps the remainder, so the rate does not drift.
- The routing table holds IPv4 and IPv6 together with longest-prefix match
  and no metric, since a host with one interface has nothing to weigh. The
  send path takes an `IpAddr` pair and is the signature both families will
  use; it carries IPv4 today, because the header it writes is the IPv4 one,
  and `net-ipv6` will write its own and ask this same table and cache.
- The fuzz target `ipv4` drives four parsers, because a byte stream reaches
  each of them by a different door: the datagram, the quoted header an error
  carries, the `ICMPv4` message behind the payload, and the reassembler,
  which is the one with state. Twenty million inputs in twenty-five seconds
  found no crash; the seed corpus is nine named datagrams and the fuzzer's
  own findings stay out of the repository, as they do for every other
  target.
- RFC 791, RFC 792, and RFC 1122 join the reference documents under
  `docs/rfc/`, each fetched twice and recorded with its checksum.

- `net-eth`, step D2 of track D: Ethernet II frames over RFC 894, ARP over
  RFC 826, and the neighbor cache both families share. Reception is a filter
  and not a parse — a frame that is short, oversized, addressed elsewhere,
  or of a type no layer here reads is dropped and not reported, because a
  link carries other stations' traffic and a stack that reported each piece
  of it would report nothing worth reading. The destination filter takes any
  multicast group rather than the ones this station joined: the groups
  follow from the addresses an IPv6 host holds, which this layer does not
  know, so membership is checked one layer up.
- The neighbor cache is one cache for IPv4 and IPv6, keyed by `IpAddr`, with
  the five states and the schedule of RFC 4861 (D-69). It says *ask again
  for this address, now, here or on the link* and the caller writes whichever
  packet that family takes, so ARP is a user of the cache and not its owner.
  One packet waits per neighbor and a second replaces it rather than
  queueing, since the first is what a retransmission produces again. Two
  boundaries are stated where they are done: the reachable time is not drawn
  at random as RFC 4861, section 6.3.2 asks, because the crate takes no
  randomness; and `Reachable` is the only state an unsolicited claim cannot
  change, which is the whole of the resistance to a stolen mapping that a
  link layer can offer.
- RFC 826, RFC 894, and RFC 4861 join the reference documents under
  `docs/rfc/`, each fetched twice and recorded with its checksum.

- The system call table of `audhsos-abi`: one `syscalls!` macro over the
  forty-one calls that derives the enum, the number lookup, the name, the
  argument count, and what the first argument names — nothing, a handle of
  any type, or a handle of one object type. The kernel dispatcher and the
  userland wrappers of Phase 7 are built from this table and from nothing
  else. There is no `dispatch!` helper the plan named: a `match` over an
  exhaustive enum is already checked for completeness, and a macro around
  it would only make the errors worse.

- The IPC buffer of `audhsos-abi`: the fixed offsets of the page a thread
  and the kernel exchange everything through, with `Buffer` and `BufferMut`
  over `[u8; 4096]`. Every accessor stays inside the page, so a buffer
  whose fields hold arbitrary bytes is readable without a panic; an index
  past an area reads `None` and writes nothing. The status word carries the
  error code in its low half, where zero is success, and partial progress
  in bit 32, because partial progress is a success and cannot be an error
  code while a caller still reads one word.

- `ThreadState`, `FaultKind`, and `Fault` in `audhsos-abi`, and the error
  codes `InvalidState` and `NotRunnable`. Every code is non-zero, so a
  return word that was never written names no state and no fault.

- `Preset<T>` in `audhsos-sync`: a cell whose value is there from the
  start, without an initialization step, without the `Option` of
  `Global<T>`, and with a `const` constructor (D-66). A `static` whose
  value is all zeros reaches the `.bss` and never travels over a stack,
  which is what the object pools of the kernel need: `Global::init` takes
  its value by move, and the pools are larger than the boot stack.

- The kernel objects in `kernel-objects`: the handle arena of the whole
  machine with the owner check a shared arena needs (D-58), `Process` with
  its page-table root and region table (D-65), `Thread` with the kernel
  stack pointer as its whole saved context (D-67), and `MemoryObject`. The
  pools and the arena are `const`-constructible and all zeros when empty
  (D-66): a generation counts up when a slot is handed out rather than
  starting at one, and the free list is implicit through a high-water mark,
  while released slots keep the first-in first-out order the catalog asks
  for.

- `Objects` holds every pool and the arena, parameterized by its four sizes
  so that a test can keep a machine of a handful of slots on its stack; the
  kernel uses the alias `MachineObjects`, which measures 1 224 280 bytes.
  A test that built one with `Box::new` overflowed its two-mebibyte stack,
  which is the failure D-66 predicts for the boot stack one level down.
  `kernel_core::machine::MACHINE` is the `Preset` cell that holds it beside
  the scheduler.

- `kernel-sched`: thirty-two priority queues threaded through the thread
  entries, a bitmap of the priorities that hold someone, time slices, and
  the state transition table. Every legal transition is one row, and every
  one of the hundred and thirty pairs of state and event the table does not
  name is an error rather than a panic.

- IPv6 is in the first network version beside IPv4 (D-69), which supersedes
  the first clause of D-50. The decision was taken now rather than after D8
  because the layers above the wire would otherwise bake `Ipv4Addr` into
  eight crates and their tests: they carry `IpAddr` and `IpCidr` instead,
  and the transports, the resolver, and the facade are written once. Two
  consequences are recorded with it — the neighbor cache of `net-eth` is one
  cache for both families, keyed by `IpAddr` and holding the states of
  RFC 4861, because Neighbor Discovery is `ICMPv6` and drives it from above
  where ARP drives it from beside; and IPv6 configures itself from a router
  advertisement with SLAAC and RFC 8106, so there is no DHCPv6. Track D
  gains `net-ipv6` as step D4 with catalog item 6.6.54, and the steps after
  it move up by one.
- `net-wire` carries both families accordingly. `Ipv6Addr` reads and writes
  the canonical text of RFC 5952, section 4 — leading zeros suppressed, `::`
  used to its maximum and never for a single zero group, the leftmost of two
  equal runs, lower case — and refuses every other spelling RFC 4291
  permits, the dotted form of an IPv4-mapped address included, since
  `::ffff:1.2.3.4` and `::ffff:102:304` would otherwise be two texts for one
  address. `Ipv6Addr::solicited_node` derives the multicast group of
  RFC 4291, section 2.7.1. `IpAddr`, `IpCidr`, and `IpVersion` are the
  family-agnostic types; there is no mapped form, and a pair of addresses
  that is not one family is `MixedFamilies` rather than a conversion.
- The checksum gained the IPv6 pseudo-header of RFC 8200, section 8.1: two
  128-bit addresses, a 32-bit upper-layer length, and the upper-layer
  protocol, which is not the packet's next-header field when an extension
  header stands between them. `transport` dispatches on the family.
  `Protocol` gained `ICMPV6` and the four extension header numbers, and
  `has_pseudo_header` knows that `ICMPv6` sums over one where `ICMPv4` does
  not — RFC 4443, section 2.3 changed that, and it is the difference that
  would otherwise be found by a wrong checksum on the wire.
- RFC 4291, RFC 5952, and RFC 8200 join the reference documents under
  `docs/rfc/`, each fetched twice and recorded with its checksum.

- `net-wire`, the first step of track D and the layer every network crate
  above it rests on. `MacAddr`, `Ipv4Addr`, `Ipv4Cidr`, and `Port` have one
  canonical text each in both directions, so `010.0.0.1` is refused rather
  than read two ways and a prefix length above 32 is an error rather than a
  mask of all ones (D-68). `EtherType` and `Protocol` are wrappers over the
  number on the wire and not enumerations, which is what lets `net-eth` drop
  an unregistered frame quietly instead of failing to parse it. `Reader` and
  `Writer` either take exactly what was asked for or take nothing and leave
  the position where it was, so a truncated frame stops a parser instead of
  unwinding it, and `Writer::patch_u16` puts a checksum back into the field
  it was computed around.
- The internet checksum of RFC 1071, with the pseudo-header form UDP and TCP
  need. The accumulator carries the end-around carry at every word instead
  of deferring it as the memo recommends, so it provably stays inside
  sixteen bits and every addition is a checked one; and it holds the odd
  byte a call ended on, so a pseudo-header, a header, and a payload may
  arrive in three calls without an odd length shifting the words of the next
  (D-68). The vectors are the worked example of RFC 1071, section 3 —
  including its last table, which splits the same eight bytes across an odd
  boundary and is what checks the held byte.
- RFC 1071 joins the reference documents under `docs/rfc/` on the
  arrangement of D-59, fetched twice and recorded with its checksum. RFC 791,
  RFC 768, and RFC 9293 each state this checksum and give no numbers for it;
  the memo is where the worked example is.

- `xtask check --quiet`, and with it a quiet mode of the whole xtask. A run
  keeps one line per step, `lint: ok` to `fuzz --regression: ok`, and the
  output of a step reaches the terminal only when that step fails, where it
  arrives whole and preceded by the command line that produced it. The full
  check writes 3314 lines warm and eleven under `--quiet`; the long form is
  worth watching and worth nothing in a log or in the context of an agent.
  What a run prints is now decided in one place (`out`): progress goes
  through `note!`, which a quiet run drops, and everything that explains a
  failure stays a plain `eprintln!`, which it never touches. `Cmd::run`
  reads a child's streams instead of inheriting them while the xtask is
  quiet, and prints them if the child fails.

- The fuzzing engine of this project, in `fuzz-support`, ported from
  libFuzzer (D-63). Apple's clang carries no libFuzzer runtime, so
  `-Clink-arg=-fsanitize=fuzzer` failed at the link step and no target
  could be fuzzed on the machine this system is written on; what a runtime
  would have supplied is now Rust in this workspace. `sancov` holds the
  callbacks the compiler emits calls to and `counters` the ranges it
  registers; above them are the thirteen mutations and how one is drawn,
  the table of the values the target was last seen comparing against, the
  bucketing of counters into features, a corpus in which a feature belongs
  to the smallest input that reaches it, the value profile, the loop, the
  merge, and the shrink. The command line is libFuzzer's, so what is
  written down about running a target still holds, and a flag this engine
  has not is refused by name rather than ignored. On the `der` target it
  runs 1.25 million inputs a second, which is what libFuzzer reached on
  the same target on the same machine.
- The port is a derived work of Apache-2.0 code, so the eight files that
  carry one name both licences in their header, `NOTICE` at the root
  records what was ported and from where, and the SPDX check knows the
  second header form and which files may use it (`policy::PORTED_FILES`).
- The `unsafe` of a fuzz target is gone. The engine is called from Rust,
  so nothing above `sancov` and `counters` sees a pointer and a length,
  and the macro that writes a target's entry points is safe code.

- The repository is ready for the worktree sessions of a coding agent: a
  session branches from the local `HEAD`, which `.claude/settings.json`
  states because nothing here is pushed, and writes a whole further
  checkout under `.claude/worktrees/`. `.gitignore` keeps that checkout
  and the permissions of one machine out of the index, and `.claude` joins
  the directories the checks never descend into. Without the exclusion the
  walker judges the copy of a ported file by the path it has in the
  worktree, which is not the path `PORTED_FILES` names, and the SPDX check
  fails on a file that is correct.
- `crypto-ec` gains ECDSA over P-384, which is what a chain that ends at a
  P-384 root takes. `p384::PublicKey::from_sec1` reads the uncompressed
  point of ninety-seven bytes, `verify` checks a signature against a
  digest of any width up to the order, and `sign` behind `test-signing` is
  deterministic per RFC 6979 over SHA-384. The constants are RFC 5903,
  section 3.2, and RFC 5114, section 2.7, states them independently and
  agrees. The vectors are RFC 6979, appendix A.2.6 — the ten signatures of
  two messages under five hashes — and RFC 5903, appendix 8.2, whose two
  key pairs and shared point are three scalar multiplications the tests
  did not compute.
- `crypto-ec` grows two modules the two ECDSA curves share:
  `montgomery` is the modular arithmetic, generic over the number of
  limbs, and `jacobian` is the group law of a short Weierstrass curve
  whose `a` is minus three. `p256` and `p384` are now the constants and
  the ECDSA on top of them. P-256 is unchanged in behaviour: the same RFC
  6979 vectors pass over the shared code. An element carries the width of
  its encoding as a parameter, and a width that does not match its limbs
  fails to compile.
- `audhsos-x509` reads and verifies P-384 keys: `oid::SECP384R1`,
  `SubjectPublicKey::EcdsaP384`, and the pairings of that key with
  `ecdsa-with-SHA256` and `ecdsa-with-SHA384`. The curve named in the
  algorithm and the width of the point must agree, so a P-384 identifier
  over a P-256 point is a bad key rather than either curve. The test
  certificate builder gains `TestKey::EcdsaP384Sha384`, so a whole chain
  of that curve is one the test suites build rather than vendor; the TLS
  client is driven through a handshake over such a chain.
- `audhsos-x509`: `TrustAnchor::from_certificate` reads the subject and the
  key out of a certificate and stops there. The certificate's own
  signature is never verified, which is what lets a root that reaches a
  system only as a cross-signed certificate serve as an anchor: the
  `GTS Root R4` in Google's chain is signed by GlobalSign with RSA, and
  `Certificate::parse` refused it for an algorithm nothing would have
  looked at. The key is checked for being one this crate can verify with,
  so an unusable anchor is refused while the caller still holds the file
  rather than becoming a path that reaches nothing. Decision D-62.
- `tools/tls-probe`: a host program that drives the sans-I/O client over a
  real socket, so that the stack is answered by a server instead of by a
  recording. It opens TCP, runs the handshake, and speaks enough HTTP/1.1
  to show a status line and a body. Everything above the socket is this
  repository's code; the host supplies the socket, the wall clock, and
  `/dev/urandom` under `crypto-rng`. It is a workspace of its own, like
  `fuzz/`, because it is the only crate in the tree that links `std`. Its
  anchor is `GTS Root R4`, taken from the host's own trust store: the full
  chain for `google.de` verifies, the P-384 signature on the intermediate
  included. What a run still does not prove is that the anchor is one the
  world trusts, there being no root program yet; the README of the probe
  writes that down, with the negative cases that show the checks are real.
- `kernel-acpi` (Phase 4): the ACPI tables the kernel needs to find its
  interrupt controllers, parsed in safe Rust. `parse_rsdp` reads the root
  pointer of revision zero or two with both of its checksums;
  `SdtHeader::parse` reads and checks the header every table starts with;
  `RootTable` walks the RSDT or the XSDT, four-byte entries or eight-byte
  ones as the signature says; `madt::parse` reads the multiple APIC
  description table into fixed capacities of four I/O APICs and sixteen
  interrupt source overrides. An entry of length zero is an error, because
  a walk that accepted one would never end; an entry that leaves the table
  is an error; more I/O APICs than the kernel holds are an error and not a
  truncation; an entry of a type the parser does not read is skipped by
  its length.
- `kernel-acpi`: `Madt::route_isa` answers where an ISA line goes and how
  it is taken, which is the one piece of interrupt routing that is logic
  rather than register writes, and is therefore tested on the host.
- `kernel-x86-tables` gains the register blocks of the two APICs and the
  encoding of a redirection entry, the write sequence that moves the two
  legacy controllers out of the way and masks them, and the vector plan:
  exceptions `0..=31`, legacy controllers `0x20..=0x2F`, timer `0x30`,
  I/O APIC lines `0x40 + gsi`, system call `0x80`, spurious `0xFF`.
  `kernel-hal-x86_64::vectors` is that module, so that the plan is one
  table with host tests behind it.
- `kernel-hal-x86_64::acpi` finds the tables of the machine through the
  physical window and hands their bytes to the parsers. No range is read
  before it has been checked against the memory the firmware reported, so
  a root pointer that names nothing makes the kernel report rather than
  fault.
- `kernel-hal-x86_64::apic`: `LocalApic` and `IoApic` over their register
  windows, one volatile access per `unsafe` block, and `Apics`, which
  implements both `InterruptController` and `Timer`. A line is routed
  masked, and the routing resolves ISA lines through the overrides of the
  table. `Apics::line_state` reads a redirection entry back out of the
  hardware, so a test can assert what the I/O APIC took rather than what
  the kernel meant to write.
- `kernel-hal-x86_64::timer`: the local APIC timer measured once against
  channel two of the interval timer, ten milliseconds with a bounded poll,
  then programmed periodic at `TICKS_PER_SECOND`. Every poll is bounded, so
  a machine whose channel two does not run reports instead of hanging.
- `kernel-hal-x86_64::interrupts`: the bring-up that reads the tables, maps
  the two register windows through a mapping the kernel supplies, quiets
  the legacy controllers, turns the local APIC on, and masks every I/O APIC
  line. `acknowledge` sends the end-of-interrupt, and sends none for the
  spurious vector, which is the one that expects none.
- `kernel-hal-x86_64::instructions`: `rdmsr` and `wrmsr`, and
  `InterruptGuard`, which turns interrupts off for as long as a borrow of
  kernel state lasts and back on afterwards if they were on. This is the
  guard the safety policy names in 4.6.
- `kernel-hal-x86_64::traps` gains a handler for every device vector of the
  plan and a second registration point, `set_interrupt_handler`: a device
  interrupt carries its vector and nothing else, which is not what a trap
  report carries.
- `kernel-hal-x86_64::testing::raise_interrupt` raises a vector from
  software, with the vector as an inline constant. The interrupt tests
  therefore assert what they mean to assert instead of asserting that a
  handler is installed.
- `kernel-core::tick` counts a timer tick and gives the scheduler its turn,
  which is nothing until Phase 5. There are two tick counts: the adapter
  counts what the hardware delivered, because `Timer::ticks` is the
  adapter's method and the adapter may not depend on `kernel-core`, and
  `KernelState` counts what the kernel processed. `KernelState` gains the counts of ticks,
  of device interrupts, and of spurious interrupts, and `with_state` hands
  the state out the way `with_memory` hands out the memory.
- `kernel-core::memory`: `KernelMemory::map_device` maps a device register
  window into the physical window, uncached, and registers it as a region
  of kind `Device`. The window the loader builds covers memory, because it
  is sized from the memory map; an aperture above it is the kernel's own to
  map, at the address the window rule gives it.
- The kernel image brings the interrupt hardware up, starts the timer, and
  waits for its first ticks before it reports that the boot is complete.
- QEMU test image `interrupts`: the tables name the hardware and the unit
  is on; a second tick arrives after the end-of-interrupt; the tick counter
  grows while the kernel does nothing; a masked timer delivers nothing and
  unmasking starts it again; a vector raised from software reaches the
  handler of that vector, for `0x30`, `0x40`, and `0xFF`; a routed line
  carries the vector and the wiring the table names, comes up masked,
  follows `mask` and `unmask`, and refuses a second routing. Catalog
  6.6.11, the APIC items of 6.6.16, and the interrupt items of 6.6.21.
- Fuzz target `madt` over the ACPI parsers, with fifteen seeds. The target
  reads the bytes twice: as they are, so that signature, length, and
  checksum are exercised, and once with those three repaired, so that the
  fuzzer reaches the walk over the entries without having to guess a
  checksum.

- `audhsos-collections` (track E3): `ArrayVec`, `RingBuffer`, `BitSet`,
  `IndexList` with its `Link`, and `IndexMap`. Fixed capacity, no
  allocation, no `unsafe`, and no panic: a `push` that does not fit
  returns `Err(Full)` and every accessor returns an `Option`, so nothing
  here can abort a system call or a packet. Track E is complete, and track
  D is unblocked from D1.
- `audhsos-collections`: `IndexList` holds no values. It holds a head, a
  tail, a length, and an identifier, while the links live in a slice of
  `Link` the caller keeps beside its own array — which is what a run queue
  over a fixed array of threads is, and what an endpoint wait queue over
  the same array is (D-48). Several lists may run over one slice, one per
  priority, and a `Link` records which list its node is in, so a node
  handed to the wrong list is refused instead of being stolen from the
  right one.
- `audhsos-collections`: `BitSet` is parameterised by its number of 64-bit
  words rather than by its number of bits, because `[u64; BITS.div_ceil(64)]`
  is an expression over a const parameter and stable Rust cannot size an
  array with one. The alternative was an incomplete language feature in
  the crate every other crate rests on. `BitSet::BITS` reports the size and
  every operation is checked against it.
- `audhsos-collections`: the owning containers store `Option<T>`, which
  costs one discriminant per slot and buys the right to hold a `T` with no
  default without a line of `unsafe`. The price is that there is no
  `as_slice`, and it is stated in the crate documentation rather than left
  to be discovered.
- Catalog 6.6.41 gains the further items of `IndexList` and `BitSet`, the
  requirement that every owning container carry a value that is neither
  `Copy` nor `Default`, and the note that the model generators reach
  beyond the container as well as inside it.

- `audhsos-encoding` (track E2): strict Base64 of RFC 4648, hex, and PEM of
  RFC 7468, each writing into a buffer the caller owns and answering how
  many bytes it wrote. Nothing allocates and nothing panics. Strict means
  one sequence of bytes has one text: Base64 refuses whitespace, a missing
  or excessive pad, a pad anywhere but at the end, and a final quantum
  whose unused bits are not zero.
- `audhsos-encoding`: PEM in the strict form. The end line must name the
  label of the begin line, every body line but the last is exactly 64
  characters, only the last may carry a pad, and nothing but line
  terminators may follow the end line. Explanatory text before the begin
  line is skipped, which RFC 7468 permits and a certificate file with a
  preamble needs. A block decodes into two borrows: the label into the
  input, the bytes into the caller's buffer.
- `audhsos-encoding`: a text with no end line reports the missing line and
  not the length of a body line. The body is located before it is read, so
  the rule for the last body line is applied only to a block that has one.
- `fuzz/pem`: the target track E owed, with a corpus of twelve texts — a
  block, a preamble, `CRLF` terminators, and the eight shapes that are
  refused. It asserts what strictness means: an accepted block re-encodes
  to a text that decodes to the same bytes, and a Base64 text the decoder
  accepts re-encodes to exactly itself.
- Catalog 6.6.40 gains the two non-canonical quanta by name, the split
  between a length error and a character error, the PEM rules that were
  not in the first list, and the check that the generator of near-valid
  blocks reaches both an accepted and a refused text.

- `audhsos-time` (track E1): `CivilTime`, `UnixTime`, `Instant`, and
  `Duration`, and the integer calendar that joins the first two. The rule
  is the proleptic Gregorian one over the years 0 to 9999, which is the
  range a `GeneralizedTime` can write down; the arithmetic is March-based,
  so a year ends with its leap day and the day of the year needs no month
  table. Every intermediate value is bounded by the year range the module
  validates before it computes, and a result outside the range is an error
  rather than a wrap.
- `audhsos-time`: the crate reads no clock. Time enters every interface as
  a parameter, which is what lets a sixty-second backoff be exercised in
  microseconds of wall clock. An `Instant` counts microseconds from an
  origin the caller chooses, and its `Add` saturates, because a timer that
  saturates fires late while one that wraps fires immediately and forever.
- `audhsos-time`: generators of all four types behind the feature
  `test-strategies`, for track D and `fs-fat`.
- Catalog 6.6.39 gains the day-by-day walk of the calendar, the ends of the
  range, the resolution of a `UnixTime`, and the items of the generators.
  Every day from 1601-01-01 to 9999-12-31 round-trips and is the successor
  of the day before it.

- `audhsos-symbols` (track G2): an address to a function, a file, and a
  line. The symbol table gives the function, the DWARF line program of
  version 4 or 5 gives the file and the line. The state machine runs once
  per lookup and keeps only the row it needs, so the crate allocates
  nothing and borrows everything from the bytes it was handed. No inline
  frames and no call-frame information, and therefore no stack unwinding.
- `audhsos-elf`: the section header table, which the loader does not read
  and a symbolizer cannot do without. `sections()` needs the magic, the
  class, and the byte order and nothing about segments, so a file without
  a loadable segment still yields its sections.
- `audhsos-symbols`: `demangle` writes a Rust symbol name back readable,
  in the `v0` scheme of RFC 2603 and the legacy `_ZN` scheme, straight
  into a formatter and therefore without allocating. Generic arguments are
  dropped and a name the parser does not understand is written unchanged.
  Every one of the 1002 symbols of the kernel image reads back.
- `xtask`: `symbolize <elf> <address>...` answers by hand, and a QEMU run
  that fails now resolves every address of the kernel half in its serial
  output against the image it ran. A file that cannot be read or carries
  no symbols produces nothing, because the report is a comment on a run
  that already failed.
- Catalog 6.6.53 gains the section header table items, the forms and the
  odd opcodes of a line program, and the items of the xtask.

- `fuzz-support` (track G1): the `LLVMFuzzerTestOneInput` entry glue, the
  `fuzz_target!` macro that writes it once, and the corpus replay. The one
  `unsafe` of the project's fuzzing turns the fuzzer's pointer and length
  into a slice and answers the empty slice for a length of zero or a null
  pointer; Miri covers it.
- `fuzz/`: a workspace of its own with the targets whose parsers exist,
  `elf`, `boot_image_header`, and `boot_info`, each with a seed corpus
  under `fuzz/corpus/`. A target built with the coverage instrumentation
  and `--cfg fuzzing` is a libFuzzer binary; the same source built without
  them replays a corpus and needs no fuzzer runtime.
- `xtask`: the directories the checks never descend into are a table in
  `policy.rs`, where D-24 puts a policy, instead of a constant of the
  file walker. `research`, which holds source of other projects kept to be
  read, joins `target` and `.git`: that source carries the license headers
  of those projects and not this one, and a check of this project has no
  business in it.
- `xtask`: `fuzz --regression` replays the stored corpus of every target
  and is the tenth step of `check`. `fuzz` itself now passes the corpus
  directory to the fuzzer and uses the coverage instrumentation flags
  rather than `-Zsanitizer=fuzzer`, which rustc does not accept: the
  libFuzzer runtime comes from the platform's clang, and a machine without
  it fails at the link step.

- `audhsos-abi`: `KERNEL_STACKS_BASE`, `KERNEL_STACK_PAGES`,
  `KERNEL_STACK_SLOT_PAGES`, `KERNEL_STACK_SLOTS`, and
  `MAX_PHYS_WINDOW_BYTES`.
- `kernel-mm`: the kernel stack pool. A slot is one unmapped guard page
  followed by four mapped pages; an allocation that runs out of frames
  leaves no slot taken and no page mapped. `NoFrames`, a frame source for
  a walk that creates no table, and `Mapper::frames_mut`.
- `kernel-core`: `config`, the number of each kind of kernel object and
  the size of the fixed tables of the kernel address space; `memory`, the
  bring-up. It takes the reserve out of the normalized map in the size the
  boot image header asks for, adopts the loader's page tables by walking
  the kernel image, the physical window, the boot stack, and the boot
  information page into a kernel region table, drops the loader's identity
  mapping without giving a frame back, and stores the result in the
  `MEMORY` cell.
- `kernel-hal-x86_64`: `PhysicalWindow` with `frame_bytes_mut`, which
  replaces `WindowAccess`; `active_root` and `activate` beside `LocalTlb`;
  and `memory`, the walk of the active tables through the window that the
  bring-up and the boot information page need.
- `audhsos-kernel`: the kernel takes its memory over after the boot report
  and reports the reserve, the regions of its address space, and the
  identity mapping it dropped. Three test kernels, `memory`,
  `memory_fault`, and `kernel_stack`, cover the memory and kernel stack
  items of catalog 6.6.21.
- `kernel-core`: `KernelMemory::allocate_stack` and `release_stack`, which
  build the mapper out of the root frame and the reserve and drive the
  stack pool.
- `kernel-hal-x86_64`: `testing::write_byte`, the write a test image needs
  to show that a page it mapped carries what it wrote and that a guard
  page faults.
- Catalog 6.6.21 gains the kernel stack items and the boot information
  address item.
- Plan 10.5.0: what the kernel reserve carries and what the kernel image
  carries, with the arithmetic behind D-57.
- Planning documents and the decision register under `docs/`.
- `docs/11-cryptography-and-tls.md`: the design and implementation plan
  for the TLS 1.3 client track (constant-time primitives, hashes and
  HKDF, AEADs, elliptic curves, random generator, DER, X.509, the sans-I/O
  protocol crate), its test catalog entries 6.6.30 to 6.6.38, its roadmap
  track 8.17, and decisions D-36 to D-44.
- `docs/12-parallel-work.md`: the design and implementation plan for the
  work that runs beside the kernel phases — the admission test for
  parallel work, the sans-I/O network stack (`net-wire`, `net-eth`,
  `net-ip`, `net-udp`, `net-tcp`, `net-dns`, `net-dhcp`, `net-http`,
  `net-stack`), the shared foundations (`audhsos-time`,
  `audhsos-encoding`, `audhsos-collections`), the device logic without
  devices (`virtio-queue`, `fs-fat`), the tooling (`fuzz-support`,
  `audhsos-symbols`), the phase work that may be pulled forward, and the
  capacity rule; its test catalog entries 6.6.39 to 6.6.53, its roadmap
  tracks 8.18 to 8.22, and decisions D-45 to D-54.
- `crypto-ct`: `Choice`, constant-time comparison, selection, exchange and
  copy, `Secret<N>` with a best-effort erase on drop.
- `crypto-hash`: SHA-256, SHA-384, SHA-512, HMAC, and HKDF, against the
  vectors of FIPS 180-4, RFC 4231, and RFC 5869.
- `crypto-aead`: `ChaCha20`, `Poly1305`, and the `ChaCha20-Poly1305`
  authenticated cipher of RFC 8439, sealing and opening in place, with
  verification before decryption.
- `crypto-aead`: AES-128 and AES-256 bitsliced over four blocks without a
  lookup table, table-free GHASH, and AES-128-GCM and AES-256-GCM, against
  the published test cases of the mode.
- `crypto-ec`: the field of `2^255 - 19` and X25519 with a constant-time
  Montgomery ladder, against the vectors of RFC 7748.
- `crypto-ec`: Ed25519 verification and deterministic signing behind
  `test-signing`, with strict canonicality and small-order checks, against
  the vectors of RFC 8032.
- `crypto-ec`: P-256 with Montgomery arithmetic for both moduli, Jacobian
  point arithmetic, ECDSA verification, and deterministic signing per
  RFC 6979 behind `test-signing`.
- `crypto-rng`: the `Entropy` and `Rng` traits, a `ChaCha20` generator that
  rekeys after every request and mixes fresh material into its key when it
  reseeds, and the doubles the protocol tests will need.
- `audhsos-der`: a strict, zero-copy reader for the distinguished encoding
  rules, with bounded nesting and one encoding per value. Its time
  conversion waits on `audhsos-time` (D-46); section 11.14 of document 11
  lists that seam and the others.
- `audhsos-x509`: certificate parsing and signature verification, chain
  validation against caller-supplied trust anchors, and RFC 6125 name
  matching, with a builder behind `test-certificates` that writes and signs
  the certificates the tests use.
- `audhsos-tls`: the record layer, the record protection, the key schedule
  of RFC 8446 section 7.1, and the handshake transcript.
- `audhsos-tls`: the wire codec and the handshake messages, read against
  the server side of the trace of RFC 8448.
- `audhsos-tls`: the client state machine, the alerts, and the sans-I/O
  interface. The handshake of RFC 8448 is reproduced, and a whole
  connection runs against a server built in the tests.
- Fuzz targets `der`, `x509`, `tls_record`, and `tls_handshake`, the four
  that catalog 6.6.35 to 6.6.38 requires of track C. Beyond "no input may
  panic" each one carries an invariant: a DER value is shorter than what
  it was read from, a parsed certificate is a view of its input and
  verifies against no empty trust store, a record's length is its header
  and its body and what this crate seals it opens again, and no handshake
  reader hands back more than the message it was given. The corpora start
  from the trace of RFC 8448 and from certificates the builder writes.

- `docs/rfc/`: the standards this system implements, verbatim, with their
  source and checksum recorded (D-59). RFC 8448, whose trace is the test
  the TLS client must reproduce; and the four documents P-384 takes — RFC
  5903 for the curve parameters, RFC 5480 for the `secp384r1` identifier
  and the uncompressed point encoding, RFC 5758 for `ecdsa-with-SHA384`,
  and RFC 6979 appendix A.2.6 for the signature vectors, which is the
  table one curve up from the A.2.5 the P-256 tests already read. The
  index says what each document contributes, and which ones were read and
  left out.
- Decision D-56: the key exchange of the TLS client is `x25519` alone.
- Workspace foundation: pinned toolchain, workspace lint set, SPDX headers,
  license, CI workflow.
- `audhsos-abi`: error codes, rights, object types, handles, layout
  constants.
- `kernel-types`: physical and virtual addresses, frames, pages, ranges,
  alignment.
- `kernel-hal-api`: hardware abstraction traits with test doubles.
- `audhsos-sync`: the `Global<T>` cell for global state.
- `test-support`: property-test engine with integrated shrinking and
  model-test runner.
- Unit tests live in `src/tests/` so that coverage measures product code
  only.
- `xtask`: `lint`, `check-layering`, `check-deps`, `unsafe-budget`,
  `test`, `coverage`, `miri`, `doc`, `check`.
- `audhsos-abi`: boot image header and boot information structure with
  validating parsers and writers, and generators behind the feature
  `test-strategies`.
- `kernel-objects`: fixed-capacity object pool with generation-checked ids,
  reference counts, and first-in-first-out slot reuse; quotas.
- `kernel-mm`: memory map normalization, kernel reserve selection, bitmap
  frame allocator, `x86_64` page-table entries behind an
  architecture-neutral trait, the mapper over the HAL traits with bounded
  range operations, and the region table of an address space.
- `kernel-types`: `PhysFrame::ZERO` and `PhysFrameRange::EMPTY`.
- `kernel-hal-api`: `MemoryFrameAccess::with_lazy_tables`, which
  materializes a page table on the first modifying access to a frame of a
  declared memory range.
- `audhsos-elf`: validating ELF64 parser with an image builder behind the
  feature `test-strategies`.
- `driver-uart16550`: register logic of the 16550 serial controller with a
  recording register double behind the feature `test-doubles`.
- `kernel-x86-tables`: encoding and decoding of the global descriptor
  table, the interrupt descriptor table, and the task state segment.
- `audhsos-uefi`: layouts, status codes, and identifiers of the UEFI
  interfaces the loader uses, the memory map reader that honors the
  firmware's stride, the conversion into boot regions, and UTF-16
  encoding; the Graphics Output Protocol structures, `LocateProtocol`, and
  the conversion of a graphics mode into a framebuffer description.
- `audhsos-abi`: the boot information carries the framebuffer the firmware
  set up. The fixed part grows to 136 bytes; the version stays 1.
- `xtask`: the disk image writer with its own CRC-32, GUID partition
  table, and FAT32 file system, the boot image writer, and the `image`
  subcommand; the policy table names the target every crate is built for,
  and the host commands skip the crates that are not built for the host.
- `kernel-core`: the boot report, the trap report, and the cell holding the
  global kernel state.
- `kernel-test-harness`: the test runner of a kernel image and the serial
  line protocol it writes.
- `kernel-hal-api`: a mutable reference to a debug console or an exit
  device is one, so that the kernel can hand one out without giving it
  away.
- `kernel-hal-x86_64`: the first adapter crate. Privileged instruction
  wrappers, the descriptor tables, the trap handlers, the boot information
  as a `Platform`, the serial debug console, the exit device, and page
  table memory through the physical window.
- `audhsos-kernel`: the kernel image with its linker script, the entry the
  loader jumps to, and the panic handler.
- `audhsos-abi`: `BOOT_STACK_TOP`, `BOOT_STACK_PAGES`, and
  `BOOT_INFO_VADDR`.
- `xtask`: the `build` subcommand, and a check that the constants the
  linker scripts repeat agree with the ABI.
- `boot-uefi-x86_64`: the loader. It reads the kernel and the boot image
  from the boot volume, places the kernel image in one physical range,
  builds the physical memory window, an identity mapping, the kernel
  segments, the boot stack, and the boot information page with the
  kernel's own mapper, writes the boot information from the memory map it
  reads last, and enters the kernel.
- `xtask`: `qemu-runner`, `run`, and `test --qemu`. The runner wraps a
  test kernel into a disk image, runs the reference machine with a time
  limit, reads the serial protocol, and maps the exit status; `test
  --qemu` runs every test kernel and the three images the loader has to
  reject; `check` runs it last.
- `kernel-hal-x86_64`: `testing`, what a kernel test image needs: the
  harness over the debug console and the exit device, the `test_kernel!`
  macro that writes the entry point and the panic handler once, the trap
  hook a test image registers, and the instructions that raise the
  exceptions the trap tests expect.
- `audhsos-kernel`: ten test kernels under `tests/`: boot, console,
  descriptors, breakpoint, divide error, invalid opcode, general
  protection, page fault, the double fault of a kernel stack overflow, and
  a panic in an image that expects one.
- `audhsos-abi`: the boot stack is 64 pages, not 16. An unoptimized test
  image needs more than 64 KiB before it reaches the harness.
- CI installs QEMU and the UEFI firmware and names the firmware the
  distribution installed, so that `check` can run `test --qemu`.
- Planning for graphics output and input devices: roadmap Phases 9 to 11,
  decisions D-29 to D-33, catalog sections 6.6.24 to 6.6.29, and the
  framebuffer fields of the boot information structure in the documents.

### Changed

- The range operations of the memory calls walk the iterators of
  `PageRange` and `PhysFrameRange` instead of adding to a page number per
  step, which removes six branches that could not be taken, and `frame_at`
  no longer checks an alignment its caller has already checked.
- `Thread` carries the entry point and the user stack it starts on, because
  the frame the first switch returns through is synthesized when the thread
  starts and not when it is created. `Pool` gained `iter` and `ids`.

- The object counts moved from `kernel-core::config` to
  `kernel-objects::config`, so that the structure holding the pools can name
  its own sizes; `kernel-core::config` re-exports them and keeps the numbers
  of the memory bring-up. `THREADS_PER_PROCESS` and `REGIONS_PER_PROCESS`
  are in `audhsos-abi`, because a process sees both when an operation is
  refused.
- `CachePolicy` moved from `kernel_mm::page_table` to `kernel-types`: the
  policy belongs to the memory and not to the table that maps it, and a
  memory object reaches it there without depending on the page tables. The
  old path stays as a re-export. `kernel-objects` in turn depends on
  `kernel-mm`, because `Process` holds the real region table.

- The address space of a process lives in the `Process` object as a
  page-table root and a region table; there is no address-space object, no
  pool for one, and no `AddressSpaceId` (D-65). Plan 10.5.2 named an id it
  defined nowhere. The kernel half is shared by copying the two page-map
  level four entries the kernel occupies, 256 for the window and 511 for
  the stacks, the boot information page, and the image; a kernel stack
  allocated later changes only tables below entry 511 and needs no second
  copy. Loading a root becomes the HAL trait `AddressSpaceControl` with a
  double, and the kernel loads one only when the incoming thread belongs to
  another process.
- An empty object pool is all zeros and its constructor is `const`, so the
  pools reach the `.bss` without passing through the boot stack (D-66). A
  `Pool<MemoryObject, 4096>` is around 230 KiB and the handle arena 512 KiB
  against a boot stack of 256 KiB, and `Global::init` takes its value by
  move. The generation of a slot now counts up in `allocate`, the free list
  is implicit through a high-water mark, `RegionTable` carries `kernel:
  bool` instead of `user: bool`, and `audhsos-sync` gains a
  `const`-initialized cell without the `Option` that `Global` has.
- The saved machine context of a thread is one word, the kernel stack
  pointer, and `Thread.context` is a `VirtAddr` (D-67). `kernel-objects`
  keeps its dependencies, and no type parameter for the machine context
  travels through the pools, the scheduler, the dispatcher, and
  `kernel-core`. The HAL trait writes the synthesized frame into the kernel
  stack as a slice of `u64` values, which is host-testable.
- The catalog marks where a phase boundary runs through an item, the way
  6.6.21 already did for the privileged instruction in user mode. The
  isolation item becomes two: a fault stops the thread in `Faulted` and
  leaves the system running, which Phase 5 carries, and the fault handler
  endpoint receives the message, which is marked from Phase 6. The system
  call items of 6.6.21 and 6.6.9 ask for a success and a failure test per
  call the phase implements, and for `Unsupported` from every call it does
  not.
- Phase 5 implements twenty of the forty-one system calls, and the plan and
  the roadmap name them instead of describing them as groups.
  `process_set_fault_handler` moves to Phase 6, where it belongs: it is a
  process call by name and an IPC call by nature, because the endpoint it
  names is a Phase 6 object.
- Roadmap 8.6 carries the status line every finished phase carries.
- The documents name the wrapper scripts under `tools/` where they named
  rustup's Cargo proxy: `sh tools/xtask.sh <subcommand>`, and
  `sh tools/xtask-check.sh` for the full check (D-64, amending D-35). The
  scripts put `~/.cargo/bin` in front of the `PATH`, so the proxy finds the
  pinned nightly and not MacPorts' `rustc`, and they are tracked, so a
  worktree session has them. 07 section 7.5 holds what they do and the rule
  for the mentions that still read `cargo xtask <subcommand>`: those are
  what CI runs and what a subcommand is called. README, `CONTRIBUTING.md`,
  `CLAUDE.md`, and the acceptance criteria of 08 and 10 name the scripts.

- `xtask fuzz` no longer links a runtime from the platform's clang, and
  the coverage instrumentation moved out of `RUSTFLAGS` into per-package
  settings in `fuzz/Cargo.toml`, with `fuzz/.cargo/config.toml` turning on
  the Cargo feature that allows them. The instrumentation now goes on the
  code under test and the targets and not on the engine that measures it,
  which is worth about three runs in four: on the `der` target the engine
  was the greater part of the counters that were cleared and read for
  every input, and none of them ever said anything about the input.

- `audhsos-der` no longer defines a time type. `Timestamp` is gone;
  `read_time`, `from_utc_time`, and `from_generalized_time` yield the
  `CivilTime` of `audhsos-time`, which closes the seam decision D-46
  opened. The parser kept the syntax — the form RFC 5280 allows, the
  digits, the `Z` suffix, the two-digit year window — and gave up the field
  ranges to the calendar. A day is now checked against the true length of
  its month, so the thirty-first of April and the twenty-ninth of February
  of a year that is not leap are refused where the old check against
  thirty-one let them through.
- `audhsos-x509` and `audhsos-tls` take the type from its owner:
  `Certificate::not_before` and `not_after`, the `now` of `verify_chain`,
  and the `now` of `ClientConfig` are a `CivilTime`. No signature changed
  shape, because the fields and their order did not.
- Section 11.14 loses the first of its four seams. What remains missing is
  not a conversion but a clock: no crate of this project reads one, so the
  value still enters from outside.

### Fixed

- `reap` no longer asks the scheduler which thread is running. A thread
  that ends leaves the processor in the same breath, so `current` is
  already empty when `thread_exit` returns — and the sweep then gave back
  the kernel stack the system call was standing on and took its pages out
  of the tables. The machine answered with a double fault in
  `Mapper::walk`. The caller now says which thread it is standing on: the
  caller of a system call, or the thread the kernel has just switched to.

- A subcommand that takes no option no longer ignores one. `lint`,
  `check-layering`, `check-deps`, `unsafe-budget`, `coverage`, `miri`,
  `doc`, and `check` read their arguments through one check that refuses
  what it does not know, as `run`, `build`, and the others already did.
  `check --quiet` before the option existed ran the whole check and
  reported success for a run nobody had asked for, and a typo did the same.

- `tools/cargo.sh` follows the other two wrappers: it `exec`s Cargo instead
  of running it, and leaves standard error where it was rather than folding
  it into standard output. Its exit status was already Cargo's, since a
  redirection is not a pipe. 07 section 7.5 names all three scripts.

- The wrapper scripts under `tools/` report what a run did. Both ended in
  `… 2>&1 | tail -60`, and a pipeline in `sh` exits with the status of its
  last command, so `sh tools/xtask-check.sh` returned zero however the
  check went: `&&` after it ran, and a red check could be committed. Each
  script now `exec`s Cargo, so output and exit status are the xtask's own.
  The pager and the colors stay off; nothing else is added, and shortening
  the output is the caller's business, which is where the status survives
  it.

- `audhsos-tls`: an alert from the peer was reported as
  `the record was not expected here`, and answered with one. Every alert
  but `close_notify` became `UnexpectedMessage`, which says the wrong
  thing — a server's `handshake_failure` is a message the client asked
  for — and then `fail` wrote `unexpected_message` back into a connection
  the peer had already closed, which RFC 8446 section 6.2 forbids: both
  sides close at once on a fatal alert. `TlsError::PeerAlert` now carries
  the code the peer sent, named where this client knows the name, and
  `Alert::for_error` answers `None` for it, which is what stops the reply.
  Seven bytes went back during a handshake and twenty-four after one; now
  none do.
- `audhsos-tls`: the signature scheme of a `CertificateVerify` was read for
  its hash and not for its curve. RFC 8446 section 4.2.3 has an ECDSA code
  point name both, and this client offers `ecdsa_secp384r1_sha384`
  (0x0503); a server could present a P-256 certificate, sign with it over a
  SHA-384 digest, and label the result 0x0503, and the handshake completed.
  Nothing was forged by it — the signature still had to verify under the
  leaf's real key, and the leaf still had to pass the path check — but the
  client was not holding a peer to the scheme it named. The scheme and the
  key now have to be the pair the code point stands for, and a mismatch is
  `illegal_parameter`. X.509 is unchanged and must be: `ecdsa-with-SHA384`
  binds no curve there, and certificates are signed that way. The constants
  are renamed `ECDSA_SECP256R1_SHA256` and `ECDSA_SECP384R1_SHA384`, the
  second of which was documented as `ecdsa_secp256r1_sha384` — a scheme
  that does not exist, which is where the confusion sat. Decision D-61.
- `audhsos-tls`: a `Certificate` message was refused whole when any entry
  in it failed to parse. RFC 8446 section 4.4.2 makes the entries behind
  the leaf an aid to path building and allows ones that belong to no path,
  and a server that sends its own root sends a certificate this client
  cannot read: Google Trust Services puts a P-384 root above a P-256
  chain, and `crypto-ec` has P-256 and Ed25519. Every such server was
  unreachable. An entry that does not parse is now passed over. The leaf
  still has to parse, and the path still has to reach an anchor through
  signatures that verify, so nothing a path check decided has changed.
- `audhsos-tls`: the alert that ends a connection went out under the
  handshake keys, and those are dropped the moment the application keys
  exist, so after the handshake it was written in the clear and the server
  could not read why its peer had gone. It now uses the keys of the epoch
  the connection has reached.
- `audhsos-tls`: the `change_cipher_spec` record of middlebox
  compatibility mode was accepted anywhere and whatever it carried. RFC
  8446 section 5 closes its window with the peer's `Finished` and allows
  it one value, and both are enforced.
- `audhsos-tls`: the sequence number is spent before its nonce is used
  rather than after, so that no path can hand the same nonce out twice.
  The last record of an epoch is still written; every call after it is
  refused.

- Plan 10.4 said that raising an interrupt vector from software was "not
  possible without asm" and settled for a weaker assertion about the
  spurious vector. That was written before `kernel-hal-x86_64::testing`
  existed, which now holds four `asm!` sites of its own and is allowlisted
  for them. The section says what the test is to assert, carries the
  `int` with the vector as an inline constant that the pinned toolchain
  accepts, and 4.5 lists the site.

- `kernel-hal-x86_64`: the boot information page was reported as a memory
  region with its virtual address, which `PhysAddr::new` rejects, so the
  region was silently dropped and no boot report ever showed a
  `boot-info` line. The entry point now walks the loader's tables for
  `BOOT_INFO_VADDR` and reports the frame that walk names.

### Changed

- The object pools live in the `.bss` of the kernel image, not in the
  kernel reserve (D-57): a `Pool<T, N>` is a typed array, and putting one
  into raw frames would need `unsafe` in a logic crate. Document 2.4.1 said
  the reserve holds them and is corrected. The reserve holds page tables,
  kernel stacks, and the IPC buffers of threads.
- The handle slots of the machine are one shared arena of `HANDLE_ENTRIES`
  (16384) and `HANDLES_PER_PROCESS` (4096) is the ceiling the quota
  enforces against it, not memory set aside per process (D-58). This
  supersedes the handle part of D-57: 1024 handles were too few for
  `server-memory`, which holds one handle per memory object it hands out
  and can therefore reach `MEMORY_OBJECTS`, and 4096 inside every `Process`
  would have been 8 MiB of `.bss` for the one process that needs them.
  Plan 10.5.2 describes the arena, and 2.3.2 is worded for it.
- `THREADS` and `KERNEL_STACKS` are 256 and `KERNEL_STACK_SLOTS` follows
  them, because every thread costs five frames of the reserve and 1024 of
  them would need 5120 against the 4048 the reference machine has.
  `PROCESSES` is 64 and `HANDLES_PER_PROCESS` is 1024, because the handle
  table lives in the `Process` object and 256 tables of `1 << 16` entries
  would be 512 MiB of `.bss`. The reference machine keeps its 256 MiB.
- `kernel-hal-x86_64::WindowAccess` becomes `PhysicalWindow` in a module
  of its own and gains the byte access the IPC buffers and the boot image
  header need; `paging` keeps the lookaside buffer and the page-table
  root.
- The kernel binary carries `kernel-hal-api`, `kernel-mm`, and
  `kernel-types` as dev-dependencies for its test kernels; the kernel
  image itself keeps its three dependencies.
- `UnixTime` moves out of `audhsos-der` into the new `audhsos-time`
  crate, and the trust-anchor PEM decoding into `audhsos-encoding`
  (D-46, D-47); document 11 is amended accordingly.
- The crate catalog, the repository layout, the layering rules, and the
  duplication table of document 5 list the crates of document 12; the
  shared-crate list of layering rule 4 gains the three foundations.
- The roadmap and the document index state the status of track C as
  implemented through step T2, which the crates already were.
