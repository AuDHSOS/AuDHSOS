# 17. The desktop on the screen

## 17.0 How to read this document

Every step below has the same six parts, in the same order: Status,
Depends on, Size, Needs, Does, Done when. A step that creates something
nameable carries Produces between Does and Done when. Nothing is implied.
Every term is defined in 17.1 and has exactly one name, used everywhere.

## 17.1 Terms

| Term | Meaning |
|------|---------|
| display server | `server-display`, the one process that writes to the framebuffer. It hands a client a surface and copies what the client presents onto the screen. |
| input server | `server-input`, the one process that reaches the PS/2 controller. It writes events into a ring per client and signals a notification. |
| compositor | `server-desk`, the process this document is about. It is a client of the display server and a server to every program that holds a window. |
| desktop | What the compositor paints: the background, the windows on it, and the bar over them. |
| window | A rectangle of the desktop that belongs to one client, with a frame the compositor draws and content the client fills. |
| frame | The whole window: the title bar, the border, and the content between them. |
| chrome | The parts of the frame the compositor draws: the border, the title bar, the title, and the close box. |
| content | The pixels of a window that the client's draw commands write. |
| window surface | The memory object the compositor holds the content of one window in. A client never sees it. |
| screen surface | The memory object the display server gave the compositor for the whole screen. |
| draw command | One operation of `gfx::draw`: a clear, a fill, an outline, a segment, or a run of text. |
| command list | At most `gfx::draw::LIST_CAPACITY` draw commands, which is what one message carries. |
| bar | The strip of `server_desk::bar::HEIGHT` rows across the top of the screen. It holds the menu titles and the clock. |
| panel | What one menu title opens: a column of entries below it. |
| clock | The digital clock at the right end of the bar, `HH:MM:SS`, from the wall clock of the kernel. |
| focus | The window the keyboard reaches. Exactly one window has it while any window is open. |
| order | The windows back to front. It is the order they are painted in, and the last of them is the one in front. |
| shell | `app-shell`, the program that holds a window and runs what is typed in it. |
| scrollback | The `app_shell::ROWS` rows above the line being typed. |

## 17.2 Goal

At the end of the track, three things are true that are not true now:

1. More than one program draws on the screen at the same time, each in a
   window of its own, and a window that is covered and uncovered again
   needs nothing of its client.
2. A program draws without holding pixels: it sends draw commands, and
   the compositor carries them out in the surface it keeps for that
   program.
3. A person types at a shell on that screen, and the shell reaches a
   Secure Shell server and an HTTP server over the network of the
   machine.

## 17.3 What is already built

These pieces are finished and were not changed by this track, except
where 17.4 says so.

| Piece | What it does | Decided in |
|-------|--------------|------------|
| `server-display` | One surface per client, presentation of damaged rectangles, the cursor sprite. | Phase 10 |
| `server-input` | The ring per client, the event records, the process watch. | Phase 11, D-108 |
| `gfx` | Pixel formats, rectangles, the damage set, surfaces, the bitmap font, presentation. | Phase 10 |
| `user-proto` | The display protocol, the input protocol, the socket protocol, the file protocol. | D-106, D-116 |
| `user_programs::socket` | A connection over the socket protocol, with the wait of D-142. | Phase 14 |
| `audhsos-ssh` | The Secure Shell client as a state machine with no I/O. | Track S, D-141 |
| `net-http` | The request writer and the response decoder. | Phase 12 |

## 17.4 What was missing

| # | What was missing | Where it is now |
|---|------------------|-----------------|
| 1 | No way to say what to draw without drawing it. | `crates/gfx/src/draw.rs`. |
| 2 | No protocol for a window. | `crates/user/proto/src/window.rs`. |
| 3 | No decision about which client the keyboard reaches. | `crates/user/servers/desk/src/state.rs`, and D-153. |
| 4 | A program could not be given a capability to the compositor, because the startup message had no role for one. | `Role::DeskServer` in `crates/abi/src/startup.rs`, `Startup::desk_server` in `crates/user/rt/src/startup.rs`. |
| 5 | Nothing in the image typed at anything. | `crates/user/apps/shell` and `crates/user/net-programs/src/bin/app_shell.rs`. |

Two things were added to crates that already existed. `Rect::span` moved
into `gfx` from the canvas, because a segment and a damage rectangle are
the same rectangle; `Bytes::filled` and `Bytes::as_str` were added to
`user-proto`, because a caller that filled an array of exactly `N` bytes
has nothing to fail at, and a caller that holds ASCII has no error case to
carry.

## 17.5 Decision D1: the compositor is a client of the display server

**The decision: the compositor takes one surface the size of the screen
from the display server and composes into it.**

Reason 1: the display server owns the framebuffer and nothing else may
write to it; a second writer would need the device memory and the mode,
which is the one thing `Role::Framebuffer` gives out.
Reason 2: the presentation of damaged rectangles already exists, and a
composition is exactly a set of damaged rectangles.
Reason 3: the cursor sprite is the display server's, so a pointer that
moves over the desktop costs one `SetCursor` and no pixels of the
compositor.

**The option not taken: the compositor replaces the display server.** It
costs the framebuffer grant, the mode, the sprite and the presentation
path, all of which work; it buys one copy of the screen per frame.

## 17.6 Decision D2: a client sends draw commands and never sees pixels

**The decision: a window's pixels live in a memory object of the
compositor, and a client says what to draw with a list of draw commands.**

Reason 1: a client that drew into a shared surface would have to be told
when the window moved, was covered, or changed size, and the compositor
would have to trust what stands in a surface it does not own.
Reason 2: the commands are the same operations `gfx` already has, so the
compositor carries them out with the code the display server draws with.
Reason 3: one message carries sixteen commands of at most fifty-six bytes
of text, which is a whole row of a shell window and more; a client that
wants more sends a second message.

**What it costs:** one message per batch of sixteen commands, against one
shared page and a presentation. A shell that types one character sends one
command; a shell that repaints its whole window sends twenty-four.

**The option not taken: a surface per window, as the display protocol
hands out.** It is what `app-canvas` uses, it is cheaper per pixel, and it
cannot be composed: the compositor would have to read a surface a faulting
client is writing at the same time.

## 17.7 Decision D3: the keyboard reaches the window in front

**The decision: the focus follows the click, and every key but the two of
the desktop reaches the window that has it.**

Reason 1: a pointer event goes to the window under the pointer, which is
what a client that draws needs; a key has no position and needs a rule.
Reason 2: the rule is the click, because the desktop has no other way to
say which window is meant.
Reason 3: the two keys of the desktop — the one that shows it and the one
that ends it — are swallowed, so no client acts on a key that was not
typed at it.

A client hears about the focus: the compositor queues `Event::Focus` for
the window that took it and for the window that lost it, so a shell draws
its caret only while it can be typed at.

## 17.8 Decision D4: nothing is painted before the key that shows the desktop

**The decision: the compositor paints nothing until `server_desk::SHOWS`
comes up.**

Reason 1: D-126 says the same of the canvas, for the same reason: the
programs that draw on the whole screen run before this one and their
pictures are checked while the machine still runs.
Reason 2: a desktop that laid its background down at startup would decide
by a race which of the two the screen holds.
Reason 3: the key is what the end-to-end run sends, so the run decides
when the desktop appears rather than the scheduler.

The compositor serves its clients from the moment it starts: a window
opened before the key is drawn into and appears when the desktop is
shown.

## 17.9 Decision D5: the compositor waits on two things with two threads

**The decision: a second thread waits on the notification with a deadline
and sends what it heard to the serving thread under a badge of its own.**

Reason 1: a thread of this kernel waits on exactly one thing, and the
compositor waits on its endpoint, on the input ring and on the ends of its
clients.
Reason 2: `server-input` and `server-display` already do this, the first
for two interrupt lines and the second for the ends of its clients, so the
shape is the system's and not this program's.
Reason 3: the deadline is what moves the clock: it passes every `TICK`
microseconds whether an event arrived or not, and the wake-up it produces
is what reads the wall clock and repaints the bar.

One notification carries both kinds of news: the input server signals bit
zero, and the end of the client in slot `n` signals bit `n + 1`.

**The option not taken: `ipc_try_recv` in a loop with a sleep.** It costs
one thread less and buys a latency of one sleep on every request a client
makes.

## 17.10 The order of the steps

| Step | Name | Status | Depends on | Size |
|------|------|--------|------------|------|
| W1 | The draw commands | built | nothing | M |
| W2 | The window protocol | built | W1 | M |
| W3 | The desktop | built | W2, D3 (17.7), D4 (17.8) | L |
| W4 | The shell | built | W2 | M |
| W5 | The compositor process | built | W3, D1 (17.5), D5 (17.9) | L |
| W6 | The shell process | built | W4, W5 | L |
| W7 | The image and the boot | built | W5, W6 | S |
| W8 | The end-to-end run | built | W7 | M |
| W9 | A request under TLS | not built | W6 | M |

W1, W2, W3 and W4 are logic crates under the coverage gate and run on the
host. W5 and W6 are the two processes; everything they add is a system
call.

## 17.11 W1. The draw commands

Status: built.
Depends on: nothing.
Size: M.

### Needs (already built)

- `gfx::Surface`, for the fill, the blit and the damage set.
- `gfx::draw_text` and the font, for a run of text.

### Does

1. Define `Command`: `Clear`, `Fill`, `Frame`, `Line`, `Text`.
2. Define `Text`, at most `TEXT_CAPACITY` bytes of ASCII, because the
   font draws the printable characters of ASCII and a message carries the
   text by value.
3. Define `List`, at most `LIST_CAPACITY` commands.
4. Carry one command out with `draw`, a whole list with `draw_all`. Every
   operation clips to the surface first and records what it wrote in the
   damage set of that surface.
5. Move the Bresenham walk of `app-canvas` into `line`, and the rectangle
   that encloses two pixels into `Rect::span`.

### Produces

`crates/gfx/src/draw.rs`, and `Rect::span` in `crates/gfx/src/rect.rs`.

### Done when

A list of one of every command draws into a byte array and the damage of
that array is what the commands wrote.

## 17.12 W2. The window protocol

Status: built.
Depends on: W1.
Size: M.

### Needs (already built)

- `user_proto::label`, for the protocol number and the status word.
- `user_proto::Bytes`, for the title.
- The two-handle transfer of the input protocol.

### Does

1. Add `Protocol::Window`, number 9.
2. Define the four messages: `Open`, `Draw`, `Close`, `Poll`.
3. `Open` carries the size, the title, a notification reduced to `SIGNAL`
   and the client's process reduced to `INFO`, and answers with the
   number of the window and the size that was granted.
4. `Draw` carries the number and a command list.
5. `Poll` answers with the next event of the window, or with nothing.
6. Define `Event`: `Key`, `Pointer`, `Focus`, `Closed`.

### Produces

`crates/user/proto/src/window.rs`.

### Done when

Every request and every reply comes back out of a buffer as it went in,
and a message that names a command kind, an event kind or a key code this
protocol does not have is refused.

## 17.13 W3. The desktop

Status: built.
Depends on: W2, D3 (17.7), D4 (17.8).
Size: L.

### Needs (already built)

- `gfx`, for the rectangles, the surfaces and the font.
- `audhsos-time`, for the civil time the clock shows.
- `audhsos-collections::ArrayVec`, for the windows and their events.

### Does

1. Hold at most `WINDOWS` windows in one order, back to front, one per
   badge.
2. Take one event of the input protocol and answer with an `Outcome`:
   what has to be painted, which clients have an event waiting, whether
   the pointer moved, and whether the desktop ended.
3. Route a key to the window that has the focus, and a pointer event to
   the window under the pointer, in the coordinates of its content.
4. Take a click on a title bar as a raise, a focus and the start of a
   drag; a click on a close box as `Event::Closed` for that client; a
   click on the bar as a menu that opens.
5. Hold two menus: what the desktop does, and the windows that are open.
6. Paint in three steps — the desktop over a region, the chrome of each
   window in order, the bar over everything — and leave the content to
   the process, which holds the surfaces.

### Produces

`crates/user/servers/desk`, with `bar`, `window` and `state`.

### Done when

Every rule above is checked on the host over byte arrays that stand in
for the screen, and the crate passes the coverage gate.

## 17.14 W4. The shell

Status: built.
Depends on: W2.
Size: M.

### Needs (already built)

- `user_proto::keyboard`, for the character a key stands for.
- `gfx::draw`, for the commands a row of text becomes.

### Does

1. Hold the scrollback, the line being typed, and the layout.
2. Take one window event and answer with a `Step`: paint, run the line,
   or the window is going away.
3. Parse a line into a `Command`: `ssh [user@]host[:port] [command]`,
   `get http[s]://host[:port][/path]`, `echo`, `time`, `clear`, `help`,
   `quit`. A locator names the scheme under TLS or the one without it;
   which of the two the program speaks is W6's.
4. Render the window as draw commands, one row at a time, and say where
   to go on when a list is full.
5. Paint one row for a keystroke and the whole window for anything that
   was printed.

### Produces

`crates/user/apps/shell`, with `command`, `screen` and `state`.

### Done when

Every line the parser accepts and every line it refuses is checked, a
scrollback that overflows drops its oldest row, and a render of a full
window comes out as the rows it holds.

## 17.15 W5. The compositor process

Status: built.
Depends on: W3, D1 (17.5), D5 (17.9).
Size: L.

### Needs (already built)

- `Mapping` and the memory server client of `user-programs`.
- The serving loop of `user_programs::serve`.
- `thread_create` and a static the second thread reads its endpoint from,
  as `server-display` uses it.

### Does, in order

1. Register under the name `desk`.
2. Ask the display server what the screen is. A machine without one ends
   the program, and its clients hear `NotFound` when they open a window.
3. Take a surface the size of the screen, hand over a capability to its
   own process, and map it.
4. Subscribe to the input server with a notification of its own and map
   the ring.
5. Start the waking thread of D5.
6. Answer `Open` by asking the memory server for the pixels of the
   window, mapping them, and watching the end of the client on bit
   `slot + 1` of the same notification.
7. Answer `Draw` by carrying the commands out in that mapping.
8. Answer `Poll` from the queue of the window, and `Close` by giving the
   pixels back.
9. On every wake-up: release the windows of clients that are gone, feed
   every event of the ring to the desktop, read the wall clock, compose
   what changed, and present it.
10. End when the key that ends the desktop has come up and every client
    has given its window back, or `GOODBYE` ticks later, whichever comes
    first.

### Produces

`crates/user/programs/src/bin/server_desk.rs`, `Role::DeskServer`,
`Startup::desk_server`, and the `windows` column of the start table of the
root task.

### Done when

The machine starts it, it says what the screen is, and a client that was
given a badged capability to it opens a window.

## 17.16 W6. The shell process

Status: built.
Depends on: W4, W5.
Size: L.

### Needs (already built)

- The socket protocol client of `user_programs::socket`.
- `audhsos-ssh` and the files of D-146 on the scratch volume.
- `net-http` and the resolver of the network server.

### Does

1. Open a window of `app_shell::WIDTH` by `app_shell::HEIGHT` at the
   compositor, handing over a notification and its own process.
2. Wait on the notification, take every event out of the window, and feed
   them to the shell.
3. Run the line the shell answers with: the four commands it does itself,
   a Secure Shell session, or an HTTP request.
4. For a session: read `SSHTRUST.TXT` and `SSHKEY.BIN` off the volume,
   connect to the host and port of the line, run the command of the line,
   and write both of its streams into the scrollback.
5. For a request: resolve the host unless the line spells an address,
   connect, write the request, and write the status, the length and the
   beginning of the body into the scrollback. A locator under TLS is
   refused with a line saying so: the handshake of `audhsos-tls` needs the
   trust anchors of the volume, which is W9 and is not built.
6. Send what the shell wants painted, in as many messages as it takes.
7. End when the window is gone, and report to the root task.

### Produces

`crates/user/net-programs/src/bin/app_shell.rs`.

### Done when

The program opens a window, `help` writes six rows into it, and a line
that names a host reaches that host.

## 17.17 W7. The image and the boot

Status: built.
Depends on: W5, W6.
Size: S.

### Does

1. Add `server-desk` and `app-shell` to the programs that lie on the
   volume, in the order the start table names them.
2. Add both to the start table of the root task: the compositor after the
   display server, the input server and the network server; the shell
   after the compositor.
3. Register both crates in the policy of the xtask, with the coverage
   gate on the two logic crates.
4. Raise the `unsafe` budget of `user-programs` by the six sites of the
   compositor, named in the policy.

### Done when

`sh tools/xtask.sh check-deps`, `check-layering` and `unsafe-budget` pass,
and the boot says it started both.

## 17.18 W8. The end-to-end run

Status: built.
Depends on: W7.
Size: M.

### Does

1. Wait for the compositor to say what the screen is and for the shell to
   say where its window stands.
2. Press the key that shows the desktop and wait for the line that says
   it was painted.
3. Take a picture of the screen and check four things against the crate
   rather than against numbers written here: the bar begins with the
   color the bar is filled with, the line under it is the edge color, the
   clock shows ink, and the frame and the close box of the window carry
   their colors.
4. Type `echo hello` and a return, and wait for the shell to repeat the
   line it ran.
5. Press the key that ends the desktop and wait for the compositor and
   the shell to say they are done.

### Done when

Both boots of `sh tools/xtask.sh test --e2e` pass with the desktop driven
this way, and the machine still ends by itself.

## 17.19 W9. A request under TLS

Status: not built.
Depends on: W6.
Size: M.

### Needs (already built)

- `audhsos-tls::client`, which is a state machine with no I/O, as the
  Secure Shell client is.
- The trust anchors of the boot volume and the root of the run, which
  `app-tls` reads (D-148, D-150).

### Does

1. Read the anchor table off the volume, as `app-tls` does.
2. Wrap the connection of W6 in a TLS connection when the locator names
   the scheme under TLS, and write the same HTTP request through it.
3. Say which certificate the handshake refused, where it refuses one.

### Done when

`get https://host/` writes a status line into the scrollback, and a host
whose chain reaches no anchor is refused with the reason.

## 17.20 Risks

| # | Risk | Effect | What reduces it |
|---|------|--------|-----------------|
| 1 | The compositor presents the whole screen for a change of one row. | A keystroke costs a copy of the screen. | The composition writes only what it painted, and the damage set of the surface is what the presentation copies; the bar and the window that changed are what a keystroke paints. |
| 2 | A client stops asking for its events. | The queue of that window fills. | The queue holds `EVENTS` events and drops the oldest, counting what it dropped; nothing else in the desktop waits for that client. |
| 3 | A client faults while its pixels are mapped here. | The compositor reads a surface nobody writes any more. | The pixels are the compositor's memory object, not the client's, so a client that is gone changes nothing; the watch gives the object back. |
| 4 | Nobody presses the key that ends the desktop. | The machine never ends, because the shell reports only when its window is gone. | The end-to-end run presses it in both boots; a machine without a screen ends the compositor at once and the shell hears `NotFound`. |
| 5 | The wall clock of the machine is wrong by a zone. | The clock shows another hour than the development machine. | `clock_wall` reports where its belief came from, and the run checks that the clock shows ink rather than what it reads. |
