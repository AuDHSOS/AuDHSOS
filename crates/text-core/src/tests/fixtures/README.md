# Envelope fixture

`envelope.sfnt` is project-authored format data: a 51-byte sfnt envelope with
two opaque tables, `TEST` at offset 44 (four bytes) and `zzzz` at offset 48
(three bytes). Payload bytes are 1 through 7. The recorded checksum is
`0x12345678` for each table; search hints are deliberately `0xffff`.

Source: `docs/microsoft/otff.html:905`, Table Directory. The fixture tests
envelope parsing and is not a renderable font. The host test reads the file
and checks the same literal numbers as the independently constructed fixture.


`cff2-spec.bin` is the 226-byte worked example from
`docs/microsoft/cff2.html`, “Example CFF2 table”. SHA-256:
`523da78ecf92d7fcdd6e0291d8e80973d931ed5354a16c0c9ab5ecdb568de229`.

`noto-cjk-subset.otf` contains .notdef, A, 中, and 日 from Noto Sans CJK JP
Regular, under the SIL Open Font License in `OFL-Noto.txt`; embedded name
table copyright notices are retained. Retrieved 2026-09-17 from
`https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/OTF/Japanese/NotoSansCJKjp-Regular.otf`.
Original SHA-256: `68a3fc98800b2a27b371f2fb79991daf3633bd89309d4ffaa6946fd587f375b5`.
Subset SHA-256: `ddac457d3fad21c7156e4e7ee856b8e6833a9db655a4e6e2e5deb44a5ed0b1c2`.
FontTools' host-only Subsetter with default options and `populate(text="A中日")`
produced the subset; FontTools is not a build or runtime dependency.
The host test checks literal coordinates from the original glyphs.


`noto-sans-variable-subset.ttf` retains A, g, é and their components from
Noto Sans, under `OFL-NotoSans.txt` with embedded copyright notices.
Retrieved 2026-09-17 from
`https://raw.githubusercontent.com/google/fonts/main/ofl/notosans/NotoSans%5Bwdth,wght%5D.ttf`.
Original SHA-256: `bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d`.
Subset SHA-256: `0aa877599c4b59d6ba151d0f0d236059d6732206f0dc84c419d4d31fd58239a9`.
FontTools' default Subsetter with `populate(text="Agé")` produced the subset.
Literal endpoint coordinates and advances were independently obtained from
FontTools' glyph set at normalized wght=1, wdth=-1.

T10 shaping fixtures (2026-09-17):

- `DejaVu-shaping.ttf`: subset of `/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf`;
  source SHA-256 `bb01e8ee9284e494256d4d34e8db8613b3a78148721250bfee91a875bccc3481`,
  subset SHA-256 `13312fa39c5f3998b97f7aee3e366aaca7c89c46502888ee30aa6f3793c6a38f`.
  License: `LICENSE-DejaVu.txt`. Characters: Latin oracle strings, Greek alpha/tonos,
  Cyrillic short I, Arabic salam/beh-fatha-teh, Hebrew shalom with points,
  parentheses, space, ZWJ and ZWNJ.
- `NotoCJK-shaping.otf`: subset of the same Noto CJK source recorded above;
  SHA-256 `a7027cbc01d2dd722653d2474fd05529468dfd81c68637f3110fba204778aadd`.
  License: `OFL-Noto.txt`. Characters: `骨直令辻A中日あア가각각ᄓᅢᇇ〮〯`.
- Generation: fontTools subset with all layout features and name IDs retained.
  Expected glyph IDs, advances and offsets were independently obtained with
  HarfBuzz 14.4.0 at the face's units-per-em. Product/tests do not link HarfBuzz.
