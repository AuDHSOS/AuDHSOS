# gfx

What is drawn, and where it is drawn to. The crate holds the pixel formats
the UEFI Graphics Output Protocol reports, rectangles and the damage set
that says which of them changed, a surface over a byte buffer with fill,
blit, and clipping, the project's own bitmap font of 95 glyphs, and the
presentation step that copies the damaged rectangles of one surface into
anything that takes rows of pixels.

It reaches no hardware and holds no buffer of its own: a surface borrows
the bytes it draws into, so the same code draws into the framebuffer of the
display server, into a back buffer, and into a byte array a host test owns.
Every byte access is a checked offset into that slice, so no rectangle,
however placed, reaches past the surface it was given.
