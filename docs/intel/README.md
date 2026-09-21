# Reference documents: Intel

The x86 kernel uses the pinned Intel Software Developer's Manual for AP startup,
interrupt commands, descriptor loading and translation invalidation.

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `325462-092-sdm-vol-1-2abcd-3abcd-4.pdf` | Intel 64 and IA-32 Architectures Software Developer's Manual, combined volumes, order 325462-092 | 2026-09-21 from `https://cdrdv2.intel.com/v1/dl/getContent/671200` | 26664910 | `16a9336104750613ae2f2bab6eb7a1b21a7e1ef60ced35e9ab2e0d8c7efcec68` |

## Lookups

| Operation | Volume 3 section |
|-----------|------------------|
| INIT–SIPI–SIPI startup | 11.4.4.1–11.4.4.2, table 11-1 |
| Long-mode activation | 12.8.5 |
| Interrupt command register | 13.6.1 |
| Multiprocessor TLB invalidation | 5.10.4.2 |

The PDF is reference material, retained verbatim under its publisher's terms.
The build does not read or link the PDF.
