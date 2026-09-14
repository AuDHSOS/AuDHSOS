# user-net-programs

The three programs of the network: `server-net`, which drives the virtio
network device and answers the socket protocol; `app-net`, which uses a
socket; and `app-ssh`, the Secure Shell client of document 14, which
reads what it is given off the scratch volume (D-146), opens one
connection, and runs a command on the far side.

They are a package of their own and not three more binaries of
`user-programs` for one reason: a binary of this workspace names every
dependency of its package, so a package is the unit that decides what a
program carries. The network stack and the driver under these two are
megabytes of an image every program of the volume is read out of one
message at a time, and a program that draws a rectangle has no use for
them (D-97's rule, at the one place it splits).

What the three share with every other program — the client of the memory
server, the mapping, the serving loop, the priorities — is the library of
`user-programs`, which they depend on. What is theirs alone is here: the
region the device reads and writes in [`net_dma`], and the registers of
the device in [`net_registers`].
