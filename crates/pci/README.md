# pci

The configuration space of a PCI function, as logic with no memory
access. Everything reaches the bytes through the `ConfigSpace` trait, so
this crate knows neither an address space nor a mapping: the enhanced
configuration access mechanism is arithmetic here, and the adapter that
holds the mapping calls it rather than deriving the offset again.

What it reads: the type-0 header, the walk over buses and functions, the
base address registers with size probing, the capability list, the MSI-X
capability and its table entry, and the vendor capabilities of virtio 1.x.

Four rules the code keeps and the tests hold it to:

- Size probing clears the memory and I/O decode bits of the command
  register before it writes all ones and restores the register
  afterwards, including when the probe found nothing. Both halves are in
  one function, so a caller cannot do the first without the second.
- The capability walk is bounded by the number of capabilities that fit
  in the first two hundred and fifty-six bytes, so a list that points at
  itself is an error and never a hang.
- A 64-bit base address register consumes the next index, and an index a
  previous register consumed is not read again.
- `virtio` reports every structure it found, in the order the capability
  list had them, and chooses none: virtio 1.4 section 4.1.4 lets a device
  publish more than one structure of a type and makes the list order its
  order of preference, so the choice belongs to the driver.

Bridges are read and reported and not descended into. The machine of
[03 3.1.1](../../docs/03-target-platform.md) puts its devices on bus 0,
and a walk that follows secondary bus numbers would have no consumer
(D-112).

PCI-SIG releases its specifications only to members, so the layout
numbers here cannot be checked against a document kept beside the code
the way D-59 asks. Every group of constants names the document and the
revision it came from, and
[`docs/pcisig/README.md`](../../docs/pcisig/README.md) records which
documents those are and how to obtain them (D-124). The virtio
capabilities are section 4.1.4 of
[`docs/oasis/virtio-v1.4-cs01.html`](../../docs/oasis/README.md), which
is beside the code.

The feature `test-doubles` adds `RecordedConfigSpace`: the configuration
space of a `q35` machine with a virtio-net device, read out of the ECAM
window of a running machine and kept as a byte fixture.
