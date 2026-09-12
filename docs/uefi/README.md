# Reference documents: the UEFI Forum

The firmware interface the loader is written against and the table format
the kernel finds its interrupt controllers through, kept verbatim so that
a structure offset or a service signature can be read against its source
without a network, and so that the source cannot change under the crate
that cites it.

Two documents, one body: the UEFI Forum publishes ACPI as well as UEFI,
the specification having moved there from the companies that first wrote
it. One directory per publishing body is the rule of this tree, so both
live here.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. D-59 records the
arrangement, and D-124 records why a document is kept under it despite
its terms.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `UEFI_Spec_Final_2.11.pdf` | *Unified Extensible Firmware Interface (UEFI) Specification*, Release 2.11, UEFI Forum, Inc., 21 November 2024, 2301 pages | 2026-09-12 from `https://uefi.org/sites/default/files/resources/UEFI_Spec_Final_2.11.pdf` | 16698759 | `a64b8e442004b91becc3de9afaf8ca61b259a9a3b436accb6b3711ab5400cee9` |
| `ACPI_Spec_6.6.pdf` | *Advanced Configuration and Power Interface (ACPI) Specification*, Release 6.6, UEFI Forum, Inc., 13 May 2025, 1202 pages | 2026-09-12 from `https://uefi.org/sites/default/files/resources/ACPI_Spec_6.6.pdf` | 6955208 | `8c7542dd4de974ae47bba71bb0336637fe1e3838daad7692370ab4cf218efd35` |

## How the files were obtained

Not from this machine. `uefi.org` is behind a bot check that answers every
request from a shell with `403`, `robots.txt` included, and clearing it
means completing a human-verification widget, which is not something an
agent of this project does. Both files were downloaded by hand and placed
here, and the addresses above are the ones they came from.

This is worth writing down because these are the first documents in this
tree that a `curl` cannot fetch. The checksum is what makes that harmless:
it says which bytes were read, whoever fetched them.

## Which releases, and why

UEFI 2.11 and ACPI 6.6 are the current ones. A published release is fixed
at its address and will not be replaced there, whereas the HTML at
`https://uefi.org/specs/` is a rendering that can be rebuilt; the PDFs are
therefore what is kept. The UEFI file name carries `Final`, which is what
the Forum names a published release.

Neither release matters much for the constants. The parts of UEFI this
project implements — the system table, the boot services, the runtime
services table and its time service, the graphics output protocol, the
file protocols, and the memory map — have not changed since 2.8 in
anything the loader depends on, and the ACPI tables the kernel reads have
not changed in anything it reads since 2.0 added the XSDT. What the
release decides is the section numbers a comment cites.

## Who cites it

`audhsos-uefi` ([document 5](../05-code-organization.md)), which writes
down the parts of the interface the loader needs and pins every offset
with a layout test:

| Section | What is taken from it |
|---------|-----------------------|
| 4.3 | `EFI_SYSTEM_TABLE`, and the position of the runtime services pointer in it |
| 4.4 | `EFI_BOOT_SERVICES`, all forty-four slots in order |
| 4.5.1 | `EFI_RUNTIME_SERVICES`: the signature, and `GetTime` as the first service after the header |
| 8.3.1 | `GetTime`, `EFI_TIME`, the field ranges, `EFI_UNSPECIFIED_TIMEZONE`, the daylight bits, and `Localtime = UTC - TimeZone` |
| 12.9.2 | `EFI_GRAPHICS_OUTPUT_PROTOCOL` and its mode information |
| 13.4.1, 13.5.1, 13.5.16 | `EFI_SIMPLE_FILE_SYSTEM_PROTOCOL`, `EFI_FILE_PROTOCOL`, `EFI_FILE_INFO` |
| 7.2 | the memory map, `EFI_MEMORY_DESCRIPTOR`, and the memory types |

`kernel-acpi` ([document 5](../05-code-organization.md)), which parses the
tables the kernel needs to find its interrupt controllers:

| Section | What is taken from it |
|---------|-----------------------|
| 5.2.5.3 | the root system description pointer: the signature, the two checksums, and the revision that decides whether an XSDT is there |
| 5.2.6 | the system description table header, and the checksum over the whole table |
| 5.2.7, 5.2.8 | the root and extended root tables, and the four- and eight-byte pointers they are arrays of |
| 5.2.12 | the multiple APIC description table: the local APIC address, the flags, and the entries that follow |
| 5.2.12.2, 5.2.12.3 | the processor local APIC and the I/O APIC structures |
| 5.2.12.5 | the interrupt source override, which is what says an ISA line is wired somewhere other than the default |

The `MCFG` table `kernel-acpi` also reads is not in either document: it is
the *PCI Firmware Specification* 3.3, section 4.1.2, which
[`docs/pcisig/`](../pcisig/README.md) records as a document that cannot be
obtained. Only the header it shares with every other table comes from
ACPI 6.6, section 5.2.6.

Both crates name the section beside each group of constants, which is what
D-40 asks of a transcription. `kernel-acpi` predates this directory and
says "the specification" in places where it should now name a section;
bringing those citations up to the rule is work this directory makes
possible and does not itself do.

## Terms

Neither document is covered by this repository's licence. Both carry the
same notice, and the ACPI file carries the 2024 year the UEFI one does
even though it was published in 2025:

> Copyright © 2024, Unified Extensible Firmware Interface (UEFI) Forum,
> Inc. All Rights Reserved.

and grants, to a person implementing the specification, permission to
maintain an electronic version accessible by that person's internal
personnel and to print it in whole or in part, in each case solely for
use in connection with implementing it, and provided the specification is
not modified.

This project is such an implementation, and the copies are unmodified.
What the grant does not reach is the word *internal*: a public repository
is not internal personnel. So these copies stand where the ITU documents
in [`docs/itu/`](../itu/README.md) stand, under D-124 and for the same
reason — the copy is what D-59 is for, a clause readable without a
network at wording that cannot change under what cites it, and no
arrangement that respects the grant delivers that. What follows is stated
rather than assumed: the files are unmodified so that their notices travel
inside them, nothing is republished from this repository, and a copy goes
if the Forum objects.
