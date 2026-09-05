# kernel-x86-tables

The bit layouts the `x86_64` kernel writes into hardware, separated from
the adapter so that they can be tested on the host: the segment descriptors
and selectors of the global descriptor table, the gate descriptors of the
interrupt descriptor table, the byte image of the task state segment, the
register offsets of the local APIC and of the I/O APIC with the encoding of
a redirection entry, the write sequence that moves the two legacy interrupt
controllers out of the way and masks them, and the vector plan that says
which vector carries what. The crate encodes and decodes; writing the
values into the registers is the adapter's work.
