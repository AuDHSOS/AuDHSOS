# server-desk

What the compositor decides, apart from the system calls it makes. It owns
the screen the display server gave it: which client holds which window,
where that window stands, which of them is in front, what the menu bar and
its clock show, and what one movement of the pointer or one keystroke
changes.

A client is known by the badge of the capability its messages arrive
through, so a window belongs to whoever opened it and a request that names
another client's window is refused without looking at the pixels. A client
never sees those pixels: it sends draw commands, and the process around
this crate carries them out in the surface it holds for that window.

Nothing here maps memory or sends a message: the process hands in the
surface over the framebuffer and the surface of each window, and this crate
says what is painted where. That is why it is tested on the host, over byte
arrays that stand in for both.
