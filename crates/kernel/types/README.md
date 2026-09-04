# kernel-types

Address types with validated constructors: `PhysAddr`, `VirtAddr`,
`PhysFrame`, `Page`, `PhysFrameRange`, `PageRange`, and `Alignment`. A value
of one of these types is always well-formed: physical addresses fit the
physical address width, virtual addresses are canonical, frames and pages
are aligned, and ranges never overflow or cross the canonical hole.
Generators for property tests are available behind the feature
`test-strategies`.
