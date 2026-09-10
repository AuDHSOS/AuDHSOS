# Reference documents: Texas Instruments

The 16550 serial controller, kept verbatim so that a register or a status
bit can be read against its source without a network, and so that the
source cannot change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. D-59 records the
arrangement for `docs/rfc/`, this directory is the same arrangement for a
datasheet rather than a standard, and D-124 records why this document is
kept under it despite its terms.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `tl16c550d.pdf` | SLLS597E, *TL16C550D, TL16C550DI Asynchronous Communications Element with Autoflow Control*, Texas Instruments, April 2004, revised December 2008, 60 pages | 2026-09-10 from `https://www.ti.com/lit/ds/symlink/tl16c550d.pdf` | 2201945 | `a49ab32dac5b1c131cc20cdd6d7ab29987f202b6d7c3c42fa21d110d88adced8` |

The checksum is here so that a reader can tell the file has not been
edited since. It was fetched three times and the three fetches agreed
byte for byte; it is what the server delivered, unaltered.

## Why this part and not the PC16550D

The crate is called `driver-uart16550` and the machine of
[03-target-platform.md 3.2](../03-target-platform.md) has a 16550 on
COM1. The part that name comes from is National Semiconductor's
PC16550D, and Texas Instruments has published it since it acquired
National in 2011. That document is no longer obtainable from TI. On
2026-09-10 every route to it answered 404: the product folder
`https://www.ti.com/product/PC16550D`, the literature shortcut
`https://www.ti.com/lit/gpn/PC16550D`, the symlink
`https://www.ti.com/lit/ds/symlink/pc16550d.pdf`, and the literature
number SNLS378C under `https://www.ti.com/lit/ds/snls378c/snls378c.pdf`.
The same URL shapes serve the parts TI still sells, so the pattern is
right and the document is gone. Copies sit on distributor and aggregator
sites; none of them is the publishing body, and D-124 asks whether the
document can be obtained, which of its publisher it cannot.

The TL16C550D is TI's own 16550 and is served. It specifies the same
register file the crate implements — the eight registers at their
offsets, the divisor latch, the FIFO control register, and the line
status bits — because that register file is what makes a part a 16550.
Where the crate cites a bit or a sequence, this document says it.

Two limits on that, so nobody reads more into the substitution than
belongs in it. The TL16C550D has autoflow control, which the PC16550D
has not and the crate does not use; its automatic RTS and CTS behaviour
is a feature of this part alone. And a citation of it is a citation of a
compatible part, not of the one QEMU models, which is a 16550A. Where
the two could differ, the code says which it follows.

## What it settles

The transmit path polls the line status register before each byte
([`write_byte`](../../crates/drivers/uart16550/src/uart.rs)), and what
one observation of bit 5 licenses decides whether that poll is needed
per byte or per sixteen. Section *Line Status Register (LSR)*, bit 5,
page 37:

> In the FIFO mode, THRE is set when the transmit FIFO is empty; it is
> cleared when at least one byte is written to the transmit FIFO.

`init` enables the FIFOs, so the driver is in FIFO mode, and the FIFO
holds sixteen bytes. This is the sentence that has to hold before the
polled loop is allowed to write more than one byte per observation.

## Terms

This document is not covered by this repository's licence. TI serves it
to anyone at no charge and without registration, and its *Important
Notice and Disclaimer*, last updated 10/2025, restricts what may be done
with it:

> These resources are subject to change without notice. TI grants you
> permission to use these resources only for development of an
> application that uses the TI products described in the resource. Other
> reproduction and display of these resources is prohibited.

So this is the arrangement `itu/` and `cipa/` stand in and not the one
`rfc/`, `oasis/`, `w3c/` and `ecma/` stand in: the file is kept because
D-124 decides on obtainability rather than on licence, and the
restriction is quoted here rather than argued with. What follows from it
is that the copy stays inside this repository and is not redistributed
under this repository's licence, that nothing is transcribed from it
into the code beyond the register names and bit numbers a citation
needs, and that a reader who wants their own copy fetches it from the
URL above rather than from here.

## Why it is here

`driver-uart16550` implements the initialization sequence, the polled
transmit path, the receive path, and the interrupt registers of this
controller, and until now cited nothing for any of it: the register
block, the FIFO control value `0xC7`, the line status bit numbers, and
the 1.8432 MHz reference clock behind the divisor all stood in the
source with no source of their own. The file follows the same reasoning
as the RFCs. When a comment or a test cites a bit, that bit must be
readable from the repository at the wording that was read, without
asking a server that has moved on — which, for the PC16550D, is exactly
what happened.

## What is still uncited

`driver-i8042` has the same gap and is not answered here. It implements
the controller's command and status registers, the `AUX` bit that says
which of the two devices a byte came from, and the scan-code and packet
decoders, and it cites nothing either. Whether its documents — Intel's
8042 datasheet and the IBM PS/2 technical reference — can be obtained
from their publishers is the D-124 question for them, and it is open.
