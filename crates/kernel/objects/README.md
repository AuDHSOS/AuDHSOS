# kernel-objects

The storage of kernel objects: a fixed-capacity pool whose ids carry a
generation, so that an id of a freed slot is rejected even after the slot
has been reused, and a quota that counts what a process owns. The pool
never allocates; its capacity is a const generic, so that the sizes are
decided where the kernel is assembled and not here.
