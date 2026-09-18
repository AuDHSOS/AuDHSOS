# text-raster

Deterministic rasterization of the outlines and paint streams `text-core`
produces. Steps R1 to R13 of `docs/18-rasterization.md` build it.

The library performs no I/O, holds no setting, allocates nothing, and has no
floating point in any product path, no dependency outside the workspace, and no
unsafe code (D-177, D-179). Its input is a `LayoutView`, the faces layout named,
and the values the compositor owns; its output is coverage written into a
surface the caller supplies. Every operation therefore runs on the host against
a `Vec`.

`draw` is the one entry point the compositor needs: it walks the lines and
glyphs of a `LayoutView`, applies each run's scale and variation coordinates,
quantizes each origin to one of four horizontal subpixel positions and a whole
vertical pixel (D-174), and dispatches each glyph to the monochrome path or the
colour path.

## Surfaces

A [`Surface`] is a width, a height, a stride in bytes, a [`Format`], and a
borrowed mutable byte slice. Four formats exist (D-178):

| Format | Bytes per pixel | Contents |
|--------|-----------------|----------|
| `A8` | 1 | Coverage or alpha, linear |
| `Rgba16` | 8 | Red, green, blue, alpha as little-endian `u16`, premultiplied, linear light |
| `Rgbx8888` | 4 | Red, green, blue, one unused byte, opaque, display space |
| `Bgrx8888` | 4 | Blue, green, red, one unused byte, opaque, display space |

Every blend runs on linear-light values; a display-space channel is decoded on
the way in and encoded on the way out through the [`Gamma`] context the caller
builds from the value `server-display` owns (D-183). Alpha is premultiplied.
The two display formats are the two `gfx::PixelFormat` values, so a
compositor's framebuffer is a destination without a conversion pass.

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

## What each step contributes

| Step | What it adds |
|------|--------------|
| R1 | The surface and the one error type |
| R2 | Outlines to polylines within an eighth of a device pixel (D-181) |
| R3 | Exact-area coverage under the non-zero winding rule (D-182) |
| R4 | The four subpixel positions and the whole vertical pixel (D-174) |
| R5 | The transfer function as two tables (D-183) |
| R6 | One coverage mask composited with one text colour |
| R7 | The glyph cache, keyed and evicted as D-184 states |
| R8 | A square root, an inverse tangent in half-turns, and a power |
| R9 | Linear, radial and sweep gradients, with pad, repeat and reflect |
| R10 | The clip stack |
| R11 | The twenty-eight compositing and blending modes |
| R12 | The whole paint stream of a colour glyph |
| R13 | Drawing a `LayoutView` |

## What is refused

Each refusal names what is drawn instead (D-180): hinting, the outline as the
font states it; subpixel antialiasing over RGB stripes, grayscale coverage;
vertical layout, nothing, because no run reaches this crate with a vertical
origin; and dilation for faux-bold, the face at the weight the style asked for.
The four non-separable blend modes are not refused.

## Golden images

`src/tests/golden/` holds the exact bytes of nine surfaces as binary PPM. They
are a gate, which D-177 allows because no product path here has floating point:
one input gives one set of bytes on every host, in debug and in release.
`AUDHSOS_GOLDEN=1 cargo test -p text-raster` rewrites them; a change to one is a
change to the rendering and is reviewed as one.
