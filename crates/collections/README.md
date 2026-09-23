# audhsos-collections

The containers a kernel and a network stack need, in the shape both can
use: fixed capacity, no allocation, no `unsafe`, and no panic. A `push`
that does not fit returns the value with `Full`, and every accessor
returns an `Option`. Nothing here can abort a system call or a packet.

`ArrayVec`, `RingBuffer`, `BitSet`, and `IndexMap` own their storage.
`IndexList` does not: it is a doubly linked list whose links are `u32`
indices into a slice of `Link` that the caller owns. The kernel run and
wait queues use links in `Thread` (D-74); the scheduler deadline list
also uses thread links (D-131). Phase 13 bus enumeration is the next
intended `IndexList` caller (D-131). Lists over one slice need distinct
identifiers.

Two limits are stated rather than left implicit. The owning containers
store `Option<T>`, which costs one discriminant per slot and buys the
right to hold a `T` with no default without a line of `unsafe`; the price
is that there is no `as_slice`, and `iter` is the way through. And
`BitSet` is parameterised by its number of 64-bit words rather than by
its number of bits, because stable Rust cannot size an array from an
expression over a const parameter.
