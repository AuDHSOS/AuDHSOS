# server-memory

The allocation policy of the memory server: which memory object a client
gets, what is left over, when memory is overwritten with zeros, and who
holds what.

The crate reaches the kernel through the `Pages` trait — map, zero, unmap,
split, merge — and through nothing else, so the whole policy runs on the
host against the recording double the feature `test-doubles` provides.
That is what catalog item 6.6.23 asks for: the zeroing is a thing a test
can watch happen, in the order it happens.

Every object is overwritten with zeros before it is handed out and again
the moment it comes back (D-12). Two passes over the same bytes, and both
are meant: the pass on return is what keeps a secret out of free memory,
and the pass on hand-out is what makes the guarantee to the client hold
even for memory this server never gave out before.
