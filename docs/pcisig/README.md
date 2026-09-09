# Reference documents: PCI-SIG

This directory holds no documents, and that is the point of it.

The crate [`pci`](../05-code-organization.md) implements the
configuration space layout, the base address registers, the capability
list, and the MSI-X capability of the PCI specifications. D-124 states
what decides whether a document is kept beside the code: whether it can
be obtained. The two documents this crate cites cannot be.

## What the crate cites, and where it is

| Document | Revision | What the crate takes from it |
|----------|----------|------------------------------|
| *PCI Express Base Specification* | 6.0 | the type-0 configuration header, the base address registers and their size probing, the capability list, the MSI-X capability and its table entry, the enhanced configuration access mechanism and its address arithmetic |
| *PCI Firmware Specification* | 3.3 | the `MCFG` ACPI table: the allocation structure, its base address, segment group, and bus range |

Both are at `https://pcisig.com/specifications`, which states the terms:
members read them online at no cost through the Specification Library,
and non-members order them against payment. Neither is needed to build,
test, or run this system; they are needed to check that a constant is
what the crate says it is.

One PCI specification is free to non-members, the *PCI Code and ID
Assignment Specification*, and it is not one of the two above. It is
released through a form at
`https://pcisig.com/pci-code-and-id-assignment-specification-agreement`
that requires a first and last name, an email address, a company, and
agreement to its terms. Nothing in the crate cites it.

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

## Vendor documentation, and what it is good for

Two FPGA vendors publish, without charge or registration, product guides
for their PCI Express cores that carry part of what the specifications
define. They are not a substitute for the documents above and are not
kept here. What they are good for is stated precisely, because the
temptation to treat them as the source is the thing to guard against.

| Document | What it carries | Read on |
|----------|-----------------|---------|
| *7 Series FPGAs Integrated Block for PCI Express LogiCORE IP Product Guide*, PG054 v3.3, clause *PCI Configuration Space*, `https://docs.amd.com/r/en-US/pg054-7series-pcie` | the type-0 and type-1 configuration space headers with every offset, and the position of the MSI-X capability in the configuration space: capability ID and next pointer, message control, table offset with its BAR indicator, PBA offset with its BAR indicator | 2026-09-09 |
| *Stratix V Avalon-ST Interface for PCIe Solutions User Guide*, Altera document 683093, section 8.1.3, `https://docs.altera.com/r/docs/683093/current/implementing-msi-x-interrupts` | the sixteen-byte MSI-X table entry and its four fields, the address of the nth entry as `base[BAR] + 16n`, the PBA address as `PBA_base + 8·floor(m/64)` with bit `m mod 64`, table size as the value read plus one, `Vector_Control` as the mask, and the four-kilobyte alignment of the table base | 2026-09-09 |

Neither carries the BAR size probing, the address arithmetic of the
enhanced configuration access mechanism, the `MCFG` table, or the bit
positions within the capability's message control register.

### The part that matters most is the citation

Both documents name the specification they implement, and the Altera one
names the clause: MSI-X, its capability structure and its table
structures are section 6.8.2 of the *PCI Local Bus Specification*,
revision 3.0. That is what is otherwise missing when the standard is not
in the house. D-40 requires a transcribed constant to name its document,
revision and section, and the project's rule is that a standard is never
cited from memory. A section number recalled rather than read fails both.
So a vendor document that prints the number turns a citation that could
only have been guessed into one with a source behind it, and the source
is recorded in the doc comment beside the constant.

### What they do not settle

They describe two implementations, not the standard. Two cores can agree
with each other and both depart from the specification, and neither
document is evidence about the third case. PG054 states its compliance as
*PCI Express Base Specification*, revision 2.1, where the crate cites
revision 6.0; the header layout and the MSI-X capability are unchanged
across those revisions, but the document is evidence for the older one.

They are therefore a third check and rank below the two above: a constant
still names the specification rather than the product guide, and the
captured `q35` configuration space is still what fails a wrong layout in
the tests.

## A restriction on use, not only on copying

The specifications page carries a condition that binds a reader who has
lawful access, and it is recorded here because it constrains what may be
done with the documents rather than whether a copy may be made:

> PCI-SIG Specifications shall not be used for the purpose of creating,
> training, enhancing, developing, maintaining, or contributing to any
> commercially available Artificial Intelligence systems without the
> express, written consent of PCI-SIG in advance.

The same page defines "commercially available" as offered, or intended
to be offered, for sale, licence or commercial use, and excludes internal
use and development for internal use. This system is not an artificial-
intelligence system and nothing here is training data for one.

## The other directories

[`docs/rfc/`](../rfc/README.md) holds what the RFC Editor publishes,
[`docs/oasis/`](../oasis/README.md) what OASIS publishes, and
[`docs/w3c/`](../w3c/README.md), [`docs/ecma/`](../ecma/README.md),
[`docs/itu/`](../itu/README.md) and [`docs/cipa/`](../cipa/README.md) the
same for those four. Nothing under any of them is compiled, linked, or
read at run time, and rule R8 of the safety policy is untouched:
`Cargo.lock` still lists only workspace members.
