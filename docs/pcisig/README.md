# Reference documents: PCI-SIG

This directory holds no documents, and that is the point of it.

The crate [`pci`](../05-code-organization.md) implements the
configuration space layout, the base address registers, the capability
list, and the MSI-X capability of the PCI specifications. Every other
standard this system implements is kept verbatim beside the code, so that
a constant can be checked against its source without a network and so that
the source cannot change under a crate that cites it (D-59, D-100).
[`docs/rfc/`](../rfc/README.md) holds what the RFC Editor publishes,
[`docs/oasis/`](../oasis/README.md) what OASIS publishes,
[`docs/w3c/`](../w3c/README.md) and [`docs/ecma/`](../ecma/README.md) the
same for those two.

PCI-SIG does not publish its specifications freely. They are available to
members, and their terms do not allow a copy to be redistributed in a
repository. So the rule that holds everywhere else cannot hold here, and
what takes its place is written down instead of left implicit (D-117).

## What the crate cites, and where it is

| Document | Revision | What the crate takes from it |
|----------|----------|------------------------------|
| *PCI Express Base Specification* | 6.0 | the type-0 configuration header, the base address registers and their size probing, the capability list, the MSI-X capability and its table entry, the enhanced configuration access mechanism and its address arithmetic |
| *PCI Firmware Specification* | 3.3 | the `MCFG` ACPI table: the allocation structure, its base address, segment group, and bus range |

Both are obtained from `https://pcisig.com/specifications`, which requires
a membership account. Neither is needed to build, test, or run this
system; they are needed to check that a constant is what the crate says it
is.

## The rule that takes the place of a copy

- Every constant of the crate `pci` names, in its own documentation
  comment, the document and the revision it comes from and the section
  within it. A number without that provenance is a defect, and the review
  of the crate treats it as one.
- The second check is empirical and lives in the tests: a configuration
  space captured from a real `q35` machine with a virtio-net device on it
  is a fixture of the crate, and the enumeration, the base address
  registers, the capability list, and the MSI-X capability are all read
  back out of it. A layout that is wrong fails there whether or not
  anybody can read the specification.
- What the virtio specification defines is not affected. The vendor-
  specific capabilities of virtio 1.x are section 4.1.4 of the OASIS
  document, which is in [`docs/oasis/`](../oasis/README.md) verbatim and
  is cited the ordinary way.

Nothing in this directory is compiled, linked, or read at run time, and
rule R8 of the safety policy is untouched: `Cargo.lock` still lists only
workspace members.
