# driver-uart16550 and driver-i8042 audit findings

Repository: AuDHSOS/AuDHSOS. Audit of driver-uart16550, driver-i8042 at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #322
Title: driver-i8042: device answers are read without the AUX bit, so a keyboard byte is taken as the mouse's answer
Labels: bug, part::drivers
Body:
`Controller::read` at `crates/drivers/i8042/src/controller.rs:311-314` waits for `OUTPUT_FULL` and reads the data register without looking at the `AUX` bit of the status it just read. `keyboard_command`, `mouse_command`, `expect` and the identifier reads of `start_mouse` at `crates/drivers/i8042/src/device.rs:480`, `crates/drivers/i8042/src/device.rs:497`, `crates/drivers/i8042/src/device.rs:507`, `crates/drivers/i8042/src/device.rs:460` and `crates/drivers/i8042/src/device.rs:466` all go through it, so every answer during `init` is attributed to the device the driver is talking to and not to the device the status names. The module doc at `crates/drivers/i8042/src/controller.rs:29-32` and the README at `crates/drivers/i8042/README.md:12-14` state that the byte and the bit are read together and never one without the other; `take` at `crates/drivers/i8042/src/controller.rs:265-271` does that, `read` does not.

`init` enables the keyboard port at `crates/drivers/i8042/src/controller.rs:237` and `start_keyboard` ends with `ENABLE_REPORTING` at `crates/drivers/i8042/src/device.rs:446`, so the keyboard reports from then on while the interrupt bits of the configuration byte are still off. `init` then enables the mouse port and calls `start_mouse` at `crates/drivers/i8042/src/controller.rs:241-242`, which resets the mouse and waits for its acknowledgement, its self-test result and its identifier at `crates/drivers/i8042/src/device.rs:456-460`. A key pressed during that wait puts a scancode in the output buffer; `read` returns it as the mouse's answer; `mouse_command` returns `Error::NotAcknowledged(scancode)` at `crates/drivers/i8042/src/device.rs:500`; `init` records `devices.mouse = false` at `crates/drivers/i8042/src/controller.rs:244` and leaves the mouse interrupt off at `crates/drivers/i8042/src/controller.rs:252-254`. The input server uses that result at `crates/user/programs/src/bin/server_input.rs:125`, so one key press during boot loses the mouse until the next boot.

Fix: add a `read_from(&mut self, aux: bool) -> Result<u8, Error>` to `Controller` that reads the status once, takes the byte only when `status & AUX != 0` equals `aux`, and otherwise discards the byte and polls again under the same `MAX_POLLS` bound; `keyboard_command`, `expect` and `start_keyboard` call it with `false`, `mouse_command` and the reads of `start_mouse` with `true`. The option not taken, disabling the keyboard port with `DISABLE_KBD` while the mouse is started, costs two more commands and still leaves the window between `ENABLE_KBD` and the last configuration write.

---

## F02 — issue #324
Title: driver-uart16550: the receive path discards the line status error bits and returns a break or a corrupt byte as data
Labels: bug, part::drivers
Body:
`read_byte` at `crates/drivers/uart16550/src/uart.rs:301-306` reads the line status register, tests bit 0 only, and returns the data register. SLLS597E page 37 defines bit 1 as the overrun error, bit 2 as the parity error, bit 3 as the framing error, bit 4 as the break indicator and bit 7 as the FIFO error, and states of each of bits 1 to 4 that it "is cleared every time the CPU reads the contents of the LSR" and, of bits 2 to 4 in FIFO mode, that the error "is revealed to the CPU when its associated character is at the top of the FIFO". The read at `crates/drivers/uart16550/src/uart.rs:302` is the one that clears them, so the caller has no way to learn them. `UartError` at `crates/drivers/uart16550/src/uart.rs:151-156` has no variant for a line error.

A break on the line loads, per SLLS597E page 37, "only one 0 character" into the FIFO with bit 4 set; `read_byte` returns `Ok(0x00)`. The console server takes it at `crates/user/servers/console/src/console.rs:83-85` and puts it into the ring for its clients at `crates/user/servers/console/src/console.rs:91-100` as a typed byte. A byte with a framing or parity error is delivered the same way, and an overrun, which is a lost byte, is reported nowhere, while the same struct counts the bytes its own ring drops at `crates/user/servers/console/src/console.rs:92-96`.

Fix: read the line status once in `read_byte`, and when any of bits 1 to 4 is set, read the data register so the FIFO advances and return `Err(UartError::Line(status))` with a new variant carrying the bits; the option not taken, returning the byte together with a flag, makes every caller check two values.

---

## F03 — issue #328
Title: driver-uart16550: no bounded drain of the receive FIFO, so a client takes one byte per interrupt with two free slots
Labels: enhancement, part::drivers
Body:
`init` writes `FIFO_CONTROL_ENABLE` = `0xC7` at `crates/drivers/uart16550/src/uart.rs:44` and `crates/drivers/uart16550/src/uart.rs:202-203`, which sets the receiver trigger level to 14 bytes (SLLS597E page 34, table of bits 7 and 6). The crate's only receive operation is `read_byte` at `crates/drivers/uart16550/src/uart.rs:301-306`, one byte per call. Both clients take one byte per wake: the console server's interrupt thread at `crates/user/programs/src/bin/server_console.rs:370-387` and `Console::take_from_controller` at `crates/user/servers/console/src/console.rs:83-85`.

SLLS597E page 34 states that the received data available interrupt "is cleared when the FIFO drops below its programmed trigger level" and that the time-out interrupt needs "more than four continuous character times" without a read. When the line asserts, the FIFO holds 14 of 16 bytes; at 115200 baud a character takes 86.8 µs, so two more arrivals fill the FIFO within 174 µs and a third arrival before the next single-byte read is an overrun, which page 37 defines as occurring "only after the FIFO is full, and the next character has been completely received". An interrupt round trip of the console server, a notification wait, two port system calls, an IPC send and an interrupt acknowledgement, longer than 174 µs loses input during a paste of more than 16 bytes, and F02 keeps the loss silent.

Fix: add `read_bytes(&mut self, into: &mut [u8]) -> usize` to `Uart16550` that reads the data register while line status bit 0 is set, at most `FIFO_DEPTH` times per call, and let both clients drain with it per interrupt; the option not taken, a trigger level of 1 (`FCR` = `0x07`), raises one interrupt per byte.

---

## F04 — issue #330
Title: driver-i8042: MAX_POLLS is a count of status reads whose length in time the adapter decides
Labels: enhancement, part::drivers
Body:
`MAX_POLLS` at `crates/drivers/i8042/src/controller.rs:110-111` bounds `wait_input` and `wait_output` at `crates/drivers/i8042/src/controller.rs:355-376` by the number of status reads. The interval between two reads is the cost of one `Ports::read_status`, which the crate leaves to the adapter: the input server's adapter makes one system call per read at `crates/user/programs/src/bin/server_input.rs:654-658`, and the scripted double answers at once. The crate states no time the bound stands for.

`expect(controller, RESET_PASSED)` at `crates/drivers/i8042/src/device.rs:443` and `crates/drivers/i8042/src/device.rs:457` waits for the self-test result a device sends after `RESET`, and the device, not the controller, decides how long its self-test takes. Whether 100,000 status reads outlast that self-test depends on the adapter's cost per read, so the same `init` reports a keyboard present through one adapter and absent through another (`crates/drivers/i8042/src/controller.rs:238`).

Fix: add `fn pause(&mut self)` to `Ports` with a documented length, call it once per iteration of `wait_input`, `wait_output` and `flush`, and state the resulting timeout as `MAX_POLLS` times that length; the option not taken, raising `MAX_POLLS`, changes the count and not what it measures.

---

## F05 — issue #332
Title: driver-uart16550: interrupt_pending reads the identification register, which clears a pending THRE interrupt without saying so
Labels: enhancement, part::drivers
Body:
`interrupt_pending` at `crates/drivers/uart16550/src/uart.rs:333-335` reads the interrupt identification register and returns bit 0 only. SLLS597E page 34 states that the THRE interrupt "is cleared ... when the THR is written to ... or the IIR is read", and page 35 states that bits 0 to 3 encode the source. The crate decodes none of bits 1 to 3.

A client that enables the transmit interrupt through `enable_interrupts(_, true)` at `crates/drivers/uart16550/src/uart.rs:316-325` and calls `interrupt_pending` sees `true`, and the read has already cleared the THRE indication and the interrupt line; the client cannot learn that the source was the transmitter and has nothing left to acknowledge. No client enables the transmit interrupt today: the console server calls `enable_receive_interrupt` at `crates/user/servers/console/src/console.rs:43`.

Fix: replace the `bool` with an enum decoded from bits 3:0 (none, line status, received data, character time-out, transmitter empty, modem status), so one read tells the caller what it consumed; the option not taken, documenting the side effect on the `bool`, leaves the source unknown.

---

## F06 — issue #334
Title: driver-uart16550: the testing catalog names two doubles the tests do not run against
Labels: bug, part::drivers
Body:
`docs/06-testing-strategy.md:553-554` states of 6.6.17: "The same tests run against the kernel adapter double and the userland system call double." The crate's tests run against `RecordingRegisters` at `crates/drivers/uart16550/src/tests/uart.rs:8` and `crates/drivers/uart16550/src/tests/uart.rs:18-22` and against the test-local `BulkRegisters` at `crates/drivers/uart16550/src/tests/uart.rs:213-233`. The kernel adapter `crates/kernel/hal-x86_64/src/console.rs:40-52` has no double and no test, and the console server's tests use `RecordingRegisters` again at `crates/user/servers/console/src/tests/console.rs:6`.

A reader of the catalog looks for a kernel adapter test and a system call double that do not exist.

Fix: replace the sentence with the two doubles the tests use, `RecordingRegisters` and `BulkRegisters`; the option not taken, writing the two doubles, adds tests the port adapters cannot run on the host.

---

## F07 — issue #336
Title: driver-i8042: the delta doc and parameter name say sign-magnitude while the code computes two's complement
Labels: bug, part::drivers
Body:
`delta` at `crates/drivers/i8042/src/mouse.rs:565-576` is documented as "nine bits, sign and magnitude apart" and names its byte `magnitude`. The body at `crates/drivers/i8042/src/mouse.rs:571-575` computes `i16::from(byte) - 256` when the sign bit is set, which is the nine-bit two's complement the PS/2 packet carries.

For byte `0x01` with the sign bit set, a sign-magnitude reading is -1 and the code answers -255; the test at `crates/drivers/i8042/src/tests/mouse.rs:107-124` confirms -256 for `0x00` with the sign set, which no magnitude reading gives.

Fix: rename the parameter to `low` and document the delta as a nine-bit two's complement value whose sign bit stands in the first byte; the option not taken, changing the arithmetic to sign-magnitude, decodes every negative delta wrongly.

---

## F08 — issue #339
Title: driver-i8042: the scancode lookup is a linear scan per byte
Labels: enhancement, part::drivers
Body:
`KeyCode::from_set2` and `KeyCode::from_set2_extended` at `crates/drivers/i8042/src/keyboard.rs:221-235` search `KeyCode::ALL` and call `encoding()` on every entry until one matches, O(n) with n = 105 per byte. `Decoder::feed` calls one of them for every non-prefix byte at `crates/drivers/i8042/src/keyboard.rs:315-316`, `crates/drivers/i8042/src/keyboard.rs:336` and `crates/drivers/i8042/src/keyboard.rs:347`, and the input server calls `feed` for every keyboard byte at `crates/user/servers/input/src/state.rs:185`.

Each keyboard byte costs up to 105 match evaluations on the interrupt path of the input server where one index costs one.

Fix: build two `[Option<KeyCode>; 256]` tables from the `key_codes!` table in a `const` block, one per prefix, and index them, O(1); the option not taken, sorting `ALL` by scancode and binary searching, is O(log n) and needs a second ordering of the table.

---

## F09 — issue #341
Title: driver-uart16550: enable_receive_interrupt duplicates enable_interrupts(true, false)
Labels: enhancement, part::drivers
Body:
`enable_receive_interrupt` at `crates/drivers/uart16550/src/uart.rs:309-312` writes `INTERRUPT_RECEIVE` to the interrupt enable register; `enable_interrupts(true, false)` at `crates/drivers/uart16550/src/uart.rs:316-325` writes the same value.

Two names for one register write; the console server uses one at `crates/user/servers/console/src/console.rs:43` and the test at `crates/drivers/uart16550/src/tests/uart.rs:290-291` exercises both.

Fix: remove `enable_receive_interrupt` and call `enable_interrupts(true, false)` in the console server; the option not taken, keeping it as a documented shorthand, keeps two names.
