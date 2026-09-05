# audhsos-tls

A TLS 1.3 client that moves no bytes. It is handed what arrived and hands
back what to send; opening the connection, waiting on it, and closing it
belong to whoever owns the transport.

That shape is not a convenience. A protocol that reads and writes for
itself can only be tested against something that answers, and the
interesting cases — a record that arrives in three pieces, a peer that
sends the right message at the wrong moment, a tag that fails — are
exactly the ones a live peer will not produce on request. Sans-I/O makes
every one of them a function call.

The client offers one version, three cipher suites, and one key exchange
group. It presents no certificate of its own, resumes no session, and
sends no early data. Document 11 says what that leaves out and why, and
section 11.14 lists the seams the whole track is still waiting on.

Nothing here allocates. Buffers come from the caller, and their minimum
sizes are constants of this crate.
