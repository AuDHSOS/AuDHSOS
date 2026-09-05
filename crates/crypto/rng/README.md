# crypto-rng

The randomness a TLS handshake needs: one generator, and the two traits
that separate where bytes come from from what consumes them.

`Entropy` is the platform: a source of unpredictable bytes, which on this
system does not exist yet and arrives with the network stack as `RDSEED`
behind a system call. `Rng` is what the protocol asks for. Between them
sits `ChaChaRng`, which stretches a seed into as much output as the
handshake needs.

Two properties are built in rather than left to the caller. The generator
rekeys itself from its own output after every request, so a state captured
later does not reveal what was handed out earlier. And it reseeds from its
entropy source after a fixed number of bytes, mixing the fresh material
into the key it already has rather than replacing it, so a source that
turns out to be predictable cannot take the state over.

No product code constructs a generator yet. The crate ships the algorithm
and the traits; the platform source is specified when the network stack
is scheduled.
