# kernel-x86-tables

The bit layouts of the `x86_64` descriptor tables, separated from the
hardware adapter so that they can be tested on the host: the segment
descriptors and selectors of the global descriptor table, the gate
descriptors of the interrupt descriptor table, and the byte image of the
task state segment. The crate encodes and decodes; loading the tables into
the registers is the adapter's work.
