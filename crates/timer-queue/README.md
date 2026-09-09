# Deadline queue

Bounded `no_std + alloc` queue without external dependencies, a clock or executor.
Unsigned 128-bit deadlines let clients add wide durations without narrowing.
Equal deadlines retain insertion order. Monotonic tokens are never reused;
exhaustion is an explicit error. Cancellation removes storage immediately, not
by leaving tombstones in a heap. Two B-tree indexes provide O(log n) insertion,
removal and due extraction; iteration exposes payloads for client-owned GC.
The caller supplies timestamps, timeout semantics, callback execution and work
quotas. The entry limit bounds logical storage, not allocator failure.
