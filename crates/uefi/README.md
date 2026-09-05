# audhsos-uefi

The parts of the UEFI interface the loader needs, written down once and
checked against the specification by layout tests: the system table, the
boot services table, the protocols the loader opens, the memory
descriptor, the status codes, and the GUIDs. The crate never calls
firmware; it describes it. Next to the layouts it holds the pure work the
loader would otherwise do by hand: reading the memory map with the stride
the firmware reports, turning it into the boot regions the kernel expects,
and encoding a file name as UTF-16.
