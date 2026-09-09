# Event target core

Safe `no_std + alloc`, dependency-free listener registry and event flags. The
embedding supplies callback identity, invocation, type conversion, GC tracing,
parent paths and exception reporting. This crate never invokes user code.

Listeners are deduplicated by type/callback/capture, not passive/once. Each phase
snapshots registration IDs; deleted IDs stay absent even after re-registration.
Once listeners are removed before invocation, including recursive dispatch.
Registry size and type length are bounded; IDs never wrap. Lookup/removal by ID
is O(log n); duplicate detection is a bounded O(n) scan. Snapshot allocation is
O(n), bounded by the listener limit. Capturing and bubbling phases take separate
snapshots, so a callback added in capture can run in the subsequent target phase.

Event flags model cancellation, passive listeners, propagation stops and the
dispatch guard. The jrs adapter currently supplies standalone targets with no
parent path. No DOM tree, shadow retargeting, activation, `AbortSignal`, browser
event-handler attributes or event loop is implied by this component.

Reference: DOM Living Standard, Event/EventTarget, add/remove listener, dispatch
and inner invoke algorithms, retrieved via docs/whatwg/README.md.
