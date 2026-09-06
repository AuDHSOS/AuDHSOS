# net-http

An HTTP/1.1 client that encodes a request and decodes a response, and
knows nothing about a connection. Bytes come in as they arrive and go out
into a buffer the caller supplied; which transport carried them is a
question this crate never asks.

The decoder is incremental because a response arrives in whatever pieces
the network cut it into. It takes at most one line of the head per call,
so no byte of the body is ever copied into the buffer the head is
assembled in, and a response split at any boundary decodes to what the
whole of it decodes to.

Framing is decided once, from the head, under the rules of RFC 9112,
section 6.3 — and where those rules leave two readings, this crate takes
neither. A response carrying both `Content-Length` and `Transfer-Encoding`
is refused rather than resolved by precedence, and so are two
`Content-Length` fields that disagree. That is the whole of request
smuggling: it is not an attack on a parser that is wrong, it is an attack
on two parsers that are each right in a different way. A message with one
framing has one length whoever reads it.

What is left is three framings and no ambiguity. A status of 1xx, 204 or
304, and any response to a `HEAD`, has no body at all. `Transfer-Encoding:
chunked` frames the body in chunks, extensions and trailers included and
both ignored. `Content-Length` frames it by count. And a response with
none of those runs until the connection closes, which is why the caller
has to say when it did: without that, a body that ended and a body that
was cut off look the same.

The head is read strictly. A status line longer than 256 bytes, a header
line longer than 1024, more headers than the decoder holds, and the
obsolete line folding of RFC 7230 are each refused. A header name is
`tchar` and nothing else; a value carries no control character but the
horizontal tab. The same rules apply on the way out, so a header this
crate writes is one it would read back.

Redirects are reported and never followed. A redirect is a decision about
what the caller is willing to fetch and from where, and a client that
follows one on its own has made that decision for it. There are no content
encodings in this version (D-50).
