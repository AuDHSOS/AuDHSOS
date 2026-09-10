# app-canvas

What the graphical demonstration draws, apart from the system calls it
makes. It takes the events of the input protocol one at a time and turns
them into pixels of a surface: the pointer moves a position that is clamped
to the screen, a button held down joins one position to the next with a
line, a key that types a character puts that character at a text cursor,
and the escape key puts the background back and starts the text over.

Nothing here maps memory or sends a message. The program around it hands in
the surface and takes back what changed, which is why this is tested on the
host over a byte array that stands in for the pixels of the screen.
