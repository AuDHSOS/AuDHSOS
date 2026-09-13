# driver-virtio-net

The virtio network device as logic: what the registers of one mean, which
features it asks for, how a frame is framed, and the order the device is
brought up in. It touches no device and no memory the device can see.
Registers reach it through the [`Registers`](registers::Registers) trait,
one structure and one offset at a time; the queues are `virtio-queue`'s;
the frame buffers reach it through the [`Frames`](frames::Frames) trait,
one buffer at a time. Nothing here computes an address.

The specification is `docs/oasis/virtio-v1.4-cs01.html`, section 5.1 for
the device and section 4.1 for the PCI transport, and every group of
constants names the section it came from.

## What a caller does with it

`Net::reset` writes zero to the status and `Net::is_reset` says when the
device has finished: the wait between them is the caller's, because a
logic crate has no clock. `Net::initialize` then walks steps 2 to 8 of
section 3.1.1 over the state machine of `virtio-queue`, configures the
receive queue and the transmit queue with the rings the caller allocated,
and reads the MAC address. `Net::fill` puts every receive buffer into the
available ring before the first frame arrives.

From there `Net::receive` answers the frame behind the twelve-byte header
of the next used element and puts the buffer it came in back into the
available ring in the same call, so the device is never left with fewer
buffers than the driver believes. `Net::send` drains the completions of
the transmit queue, writes a zeroed header and the frame into a free
buffer, and adds the two as one chain; `Net::notify` tells the device
which queue has something in it.

## What it refuses

Every offered feature bit but `VIRTIO_F_VERSION_1` and
`VIRTIO_NET_F_MAC`, each by name. `VIRTIO_NET_F_MRG_RXBUF` is what makes
one receive buffer hold one whole frame, so refusing it is what lets the
receive path read one used element per frame; no control queue is
negotiated, which is what makes the device two queues and not three
(D-114). A frame longer than a buffer, a used element shorter than the
header, and one longer than the buffer it names are each refused before a
byte of the frame reaches a caller.

## What it does not do

Link status changes, statistics, checksum and segmentation offloads,
multiqueue, and the control queue. Each is a named refusal in
`features.rs`, so that an omission reads as a decision.

Nothing here allocates, and the largest thing on the stack is one
twelve-byte header.
