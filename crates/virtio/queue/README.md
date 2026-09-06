# virtio-queue

The split virtqueue of virtio 1.x and the device initialization state
machine, as logic with no device access. The queue reaches the memory it
shares with the device only through the `QueueMemory` trait, and the
state machine reaches the device status and feature registers only
through `DeviceRegisters`. The adapter that maps a device implements
both, and it lives in a driver process; nothing here knows a physical
address or a register offset.

The ring layout lives in this crate rather than in the adapter, because
the layout is what the specification fixes and what a test can pin down:
the trait hands over three byte regions, and the encoding and decoding of
descriptors, of the available ring, and of the used ring happen here.

The free descriptors are a `BitSet` inside the queue, not a list threaded
through the `next` fields of the shared descriptor table. The device
cannot write the descriptor table, but it can write the used ring, and a
used element that names a descriptor which is already free is the one
thing that would corrupt a free list threaded through shared memory. Here
it is one bit test and a refusal.

The specification is `docs/oasis/virtio-v1.4-cs01.html`, and every group
of constants here names the section it came from.

Everything the device writes into the used ring is checked before it is
believed: the descriptor it names must be one of ours and not already
free, the index must not have moved backwards or past the chains that are
out, and the length it reports must fit the room the chain gave it. That
last one is the number a caller uses to read, and a device may report
less than it wrote but never more than it was given.

Three things are deliberately absent (D-52): packed rings, indirect
descriptors, and `EVENT_IDX`. `Device::negotiate` refuses to negotiate any
of the three rather than ignoring them, so a driver that asks for one
learns it here instead of at the first descriptor.

The feature `test-doubles` adds a queue memory in ordinary bytes that can
also act as the device — it completes chains into the used ring, sets the
no-notify flag, and can scribble a chain so that it does not end — and a
register file that can be scripted to reject the feature set or to demand
a reset.

Two things this crate does not do, because it cannot: it does not wait,
and it does not order memory by itself. A transport that wants the status
read back as zero after a reset waits in the adapter, and the memory
ordering between a ring entry and the index that publishes it goes
through `QueueMemory::barrier`, whose default does nothing because a host
test needs nothing.
