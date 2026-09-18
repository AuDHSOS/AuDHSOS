# text-raster

Deterministic rasterization of the outlines and paint streams `text-core`
produces. Steps R1 to R13 of `docs/18-rasterization.md` build it; R1 is
implemented.

The library performs no I/O, holds no setting, allocates nothing, and has no
floating point, no dependency outside the workspace, and no unsafe code
(D-177, D-179). Its input is a `LayoutView`, the faces layout named, and the
values the compositor owns; its output is coverage written into a surface the
caller supplies. Every operation therefore runs on the host against a `Vec`.

A [`Surface`] is a width, a height, a stride in bytes, a [`Format`], and a
borrowed mutable byte slice. Four formats exist (D-178):

| Format | Bytes per pixel | Contents |
|--------|-----------------|----------|
| `A8` | 1 | Coverage or alpha, linear |
| `Rgba16` | 8 | Red, green, blue, alpha as little-endian `u16`, premultiplied, linear light |
| `Rgbx8888` | 4 | Red, green, blue, one unused byte, opaque, display space |
| `Bgrx8888` | 4 | Blue, green, red, one unused byte, opaque, display space |

Every blend of this crate runs on linear-light values; a display-space channel
is decoded on the way in and encoded on the way out (D-183). Alpha is
premultiplied. The two display formats are the two `gfx::PixelFormat` values,
so a compositor's framebuffer is a destination without a conversion pass.

Access is by texel, which is the stored value of a format and not a colour.
Reads outside the surface return `None`; writes outside it, and writes of a
texel the format does not carry, return an error. A surface never writes the
bytes of a row past its last visible column.

```rust
use text_raster::{Format, Surface, Texel};

let mut bytes = [0_u8; 4 * 3];
let mut surface = Surface::new(&mut bytes, 4, 3, 4, Format::A8)?;
surface.set_texel(1, 2, Texel::Coverage(255))?;
assert_eq!(surface.texel(1, 2), Some(Texel::Coverage(255)));
assert_eq!(surface.texel(4, 2), None);
# Ok::<(), text_raster::RasterError>(())
```

What this crate refuses, with what it draws instead (D-180): hinting, the
outline as the font states it; subpixel antialiasing over RGB stripes,
grayscale coverage; vertical layout, nothing, because no run reaches this
crate with a vertical origin; and dilation for faux-bold, the face at the
weight the style asked for. The four non-separable blend modes are not
refused.
