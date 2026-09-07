# server-display

What the display server decides, apart from the system calls it makes. It
owns the screen: which client holds which surface, what a presentation
copies out of that surface, and where the cursor is and what stood under it
before the sprite was drawn there.

A client is known by the badge of the capability its messages arrive
through, so a surface belongs to whoever created it and a request that
names another client's surface is refused without looking at the pixels.
Nothing here maps memory or sends a message: the process around it hands in
the surface of the client and the surface over the framebuffer, and this
crate says what is copied where. That is why it is tested on the host, over
byte arrays that stand in for both.
