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
