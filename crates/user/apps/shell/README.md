# app-shell

What the shell is, apart from the system calls the program around it makes.
It holds the line being typed, the scrollback above it, and the parser that
says what a line asks for: a Secure Shell session, a request for a
document, or one of the five things the shell does itself. A locator under
TLS parses like any other; what speaks it is the program around this, and
today that program refuses one.

It draws nothing and reaches nothing. One window event goes in and a step
comes out that says whether the window has to be painted, whether a line is
ready to run, or whether the window is gone; the picture leaves as draw
commands of `gfx`, which the compositor carries out. That is why it is
tested on the host, over lists of commands rather than over pixels.
