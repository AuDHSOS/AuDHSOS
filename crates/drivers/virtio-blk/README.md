# driver-virtio-blk

The virtio block device as logic: what the registers of one mean, which
features it asks for, how a request is framed, and the order the device
is brought up in. It touches no device and no memory the device can see.
Registers reach it through the [`Registers`](registers::Registers) trait,
one structure and one offset at a time, and the queue is
`virtio-queue`'s, so nothing here computes an address.

The specification is `docs/oasis/virtio-v1.4-cs01.html`, section 5.2 for
the device and section 4.1 for the PCI transport, and every group of
constants names the section it came from.

## What a caller does with it

`Blk::reset` writes zero to the status and `Blk::is_reset` says when the
device has finished: the wait between them is the caller's, because a
logic crate has no clock. `Blk::initialize` then walks steps 2 to 8 of
section 3.1.1 over the state machine of `virtio-queue`, configures the
one request queue with the rings the caller allocated, and reads the
capacity. From there `Blk::submit` puts a request into the queue and
`Blk::notify` tells the device it is there; the completion comes back
through `Queue::next_used`, and the status byte the device wrote is read
by the caller and judged by `request::status`.

The three parts of a request are the caller's memory: it writes the
header with `Request::write_header`, hands over the addresses in a
`Chain`, and reads the status byte back. What this crate contributes is
which of the three the device reads and which it writes, and the order
they go into the chain — the order section 5.2.6 gives them.

## What it refuses

A write to a device that offered `VIRTIO_BLK_F_RO`. Data that is not a
whole number of 512-byte sectors, a flush that carries data, a read or a
write that carries none, and a flush of a sector other than zero: each is
a rule of section 5.2.6.1 that a device would answer with a status byte
and a lost request. A request that reaches past the last sector, which
that section forbids in as many words.

## What it does not do

One request queue. `VIRTIO_BLK_F_MQ` is refused, so section 5.2.2 leaves
exactly one, and the driver drives it. Of the eight request types, three
are here — read, write and flush. Discard, write zeroes, secure erase,
get id and get lifetime each need a feature this driver does not take or
a framing rule of their own, and a file system server needs none of them.
`features.rs` names every bit the device may offer, including the ones
that are turned down, so that an omission reads as a decision.

Nothing here allocates, and the largest thing on the stack is one
sixteen-byte header.
