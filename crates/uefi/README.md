# audhsos-uefi

The parts of the UEFI interface the loader needs, written down once and
checked against the specification by layout tests: the system table, the
boot services table, the runtime services table, the protocols the loader
opens, the memory descriptor, the status codes, and the GUIDs. The crate
never calls firmware; it describes it. Next to the layouts it holds the
pure work the loader would otherwise do by hand: reading the memory map
with the stride the firmware reports, turning it into the boot regions the
kernel expects, encoding a file name as UTF-16, and turning the `EFI_TIME`
of `GetTime` into a count of seconds from the Unix epoch.

That last one is here rather than in the loader on purpose. It is a
calendar conversion, which is arithmetic worth testing on the host under a
coverage gate; leaving it beside the `unsafe` call would have put it in a
crate no test runs. The specification it is written against is kept in
`docs/uefi/`, and every offset and constant names its section.
