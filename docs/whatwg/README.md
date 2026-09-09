# Host standards used by jrs

These standards are reference documents only. They are not compiled, linked,
or read by the runtime. Implementations remain independent safe Rust with no
external software dependencies. The original documents retain their own notices
and licensing; this repository's software licence does not replace them.

For the microtask host implementation, the following unmodified pages were
downloaded on 2026-09-09 and read locally in `target/web-standards/`:

| File | Source | SHA-256 of retrieved bytes |
| --- | --- | --- |
| timers-and-user-prompts.html | https://html.spec.whatwg.org/multipage/timers-and-user-prompts.html | ac7c69384851de024840c5a3f255faf4bff6c4c0a54d0f27c2e96dfe6415d9b7 |
| webappapis.html | https://html.spec.whatwg.org/multipage/webappapis.html | 367e592035f875d89fd0e70fdffbf96e0d0aaf9c1fba36732019835e78a66328 |
| webidl.html | https://webidl.spec.whatwg.org/ | bd4d0345c5a2164bccea124842afe4c6f1d7aa2373ce182bfd6f049055fdd05a |
| dom.html | https://dom.spec.whatwg.org/ | bfde1117739cf93463ed17ba0af7d663abf098fbd126f2bcf92e93ce7d787487 |

Consulted sections: HTML 8.8 Microtask queuing, “queue a microtask”, “perform a
microtask checkpoint”, “report an exception”; Web IDL 3.2.19 callback function
conversion and 3.12 invoking callback functions. The implementation has a single
realm, so DOM event dispatch and cross-realm callback settings remain gaps.

For standalone event support, DOM Event, CustomEvent, constructing events,
EventTarget, flatten options, add/remove listeners, dispatch, invoke/inner invoke
and event flags were read. Web IDL DOMException custom bindings (§3.14.1) and
interface/legacy names (§2.8.1, §4.4) were read for invalid dispatch errors.
Only parentless EventTarget paths are implemented; Window/tree/retargeting and
AbortSignal integration remain gaps.

To retrieve a missing page, create `target/web-standards/`, then download its
listed URL with `curl -L --fail --show-error --max-time 30 URL -o PATH` (through
the project's `rtk proxy` command prefix). Check its SHA-256 with `shasum -a 256`
and read the relevant sections locally before using them. These are living
standards: if bytes differ, do not claim they are the recorded snapshot. Review
the changed algorithms and record the new retrieval/hash when implementing from
the newer version. Downloads are kept in ignored build storage, not vendored
software or a runtime dependency.
