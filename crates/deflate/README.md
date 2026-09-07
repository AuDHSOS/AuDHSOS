# audhsos-deflate

The compressed data format of RFC 1951, and the wrapper of RFC 1950 that
a PDF calls `FlateDecode` and a PNG carries in its image chunks. Both
documents are in the house, under `docs/rfc/`, and every number here is
transcribed from them with the section it comes from named beside it.

Nothing allocates. The caller gives the buffer the result is written into
and, for compressing, the table the compressor finds runs with; the
functions answer how many bytes they wrote. `bound` says how much room a
result can possibly need.

## Compressing

One pass, and one decision per position: is there a run of at least three
bytes that has been seen before within the last thirty-two kilobytes? If
there is, the longest one found is written as a length and a distance; if
there is not, the byte is written as itself. What makes that quick is the
one table the compressor keeps — for every three bytes seen, where they
were last seen, and from there a chain back through every earlier place
with the same three. The chain is walked a hundred and twenty-eight steps
at most, which is where finding longer runs stops paying for the time it
costs.

The codes are the fixed ones the format itself names, so no table has to
be written into the stream. A block that carried its own table would be
smaller — perhaps a third smaller on prose — and it would also be a
second algorithm; this one is what makes the difference between a file of
six megabytes and one of two.

An input that will not compress is written unchanged, in blocks that say
so. The result is then a handful of bytes longer than the input and never
more, which is what `bound` counts.

## Reading

The decoder reads all three kinds of block, including the one this crate
never writes. That is deliberate: a decoder that only read what the
compressor here produces could not be held against anybody else's, and a
stream from a real compressor is the only proof that these codes are the
codes the format means. The tests use both directions — what this crate
writes must read back as what went in, and what a real compressor writes
must read back as what it started from.

## What it does not do

It does not compress in pieces: the whole input is one call and one
block. It does not read a stream that names a dictionary it was not given.
It does not write a block with its own code table, and it does not do
gzip, which is RFC 1952 and a different wrapper around the same thing.
