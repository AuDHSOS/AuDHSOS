# user-test-programs

One binary per scenario the kernel test images need a user thread for.

Each is linked at `0x40_0000`, carries no runtime, and is turned into a
flat binary by `sh tools/xtask.sh build-user-tests`, which puts it into
`target/user-tests/<name>.bin`. A test kernel embeds the bytes, maps them
into a process it creates, and watches what the thread does.

The programs are deliberately small enough to read in one go: what a test
observes has to be what the program does and nothing else.
