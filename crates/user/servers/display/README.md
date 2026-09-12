# server-display

What the display server decides, apart from the system calls it makes. It
owns the screen: which client holds which surface, what a presentation
copies out of that surface, and where the cursor is, which of its two
sprites stands there — the arrow, or the double arrow of a resize — and
what the screen held under it before that sprite was drawn.

A client is known by the badge of the capability its messages arrive
through, so a surface belongs to whoever created it and a request that
names another client's surface is refused without looking at the pixels.
Nothing here maps memory or sends a message: the process around it hands in
the surface of the client and the surface over the framebuffer, and this
crate says what is copied where. That is why it is tested on the host, over
byte arrays that stand in for both.
