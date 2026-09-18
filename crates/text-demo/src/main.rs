// Throwaway rasterizer over text-core. Not part of the design:
// f64 everywhere below the layout, no cache, no hinting, no gamma.
// Its only job is to make text-core's numbers visible as pixels.
//
// Colour glyphs: the COLR paint stream is consumed exactly as text-core
// emits it — Clip/Unclip narrow a coverage mask, Group/Compose bracket an
// offscreen surface, Fill covers the mask. Porter-Duff and the separable
// blend modes are implemented; the four non-separable ones fall back to
// source-over and say so.

use std::env;
use std::error::Error;
use std::fs;

use text_core::{
    Fixed, Font, FontSet, Language, OutlineKind, Role, TextStyle,
    cff::{Cff, Command},
    colr::{Affine, Color, ColorSource, ColorStop, Colr, CompositeMode, Extend, Fill, PaintOp},
    glyf::{Glyf, Point},
    variation::VariationPoint,
};

const SUBSAMPLES: usize = 4; // vertical oversampling; x coverage is exact
const CURVE_STEPS: usize = 12;
const PAD: f64 = 8.0;
const PALETTE: u16 = 0;
const TEXT_RGB: [f64; 3] = [0.0, 0.0, 0.0];

fn f(v: Fixed) -> f64 {
    v.bits() as f64 / 4_294_967_296.0
}

// ---------------------------------------------------------------- surfaces

/// Straight-alpha RGBA, one f64 per channel.
#[derive(Clone)]
struct Surface {
    w: usize,
    h: usize,
    px: Vec<[f64; 4]>,
}

impl Surface {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![[0.0; 4]; w * h],
        }
    }

    fn write_ppm(&self, path: &str) -> std::io::Result<()> {
        let mut out = format!("P6\n{} {}\n255\n", self.w, self.h).into_bytes();
        for p in &self.px {
            // over white
            for c in 0..3 {
                let v = p[c] * p[3] + 1.0 * (1.0 - p[3]);
                out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        fs::write(path, out)
    }
}

/// Composite `src` onto `dst` with source-over, both straight alpha.
fn over(dst: &mut [f64; 4], src: [f64; 4]) {
    let a = src[3] + dst[3] * (1.0 - src[3]);
    if a <= 0.0 {
        *dst = [0.0; 4];
        return;
    }
    for c in 0..3 {
        dst[c] = (src[c] * src[3] + dst[c] * dst[3] * (1.0 - src[3])) / a;
    }
    dst[3] = a;
}

// ------------------------------------------------------------ coverage mask

/// Scanline coverage of a set of closed polygons, non-zero winding.
/// Four subsamples in y, exact area in x.
fn coverage(w: usize, h: usize, polys: &[Vec<(f64, f64)>]) -> Vec<f64> {
    let mut cov = vec![0.0f64; w * h];
    let mut edges: Vec<(f64, f64, f64, f64)> = Vec::new();
    for poly in polys {
        for i in 0..poly.len() {
            let (x0, y0) = poly[i];
            let (x1, y1) = poly[(i + 1) % poly.len()];
            if y0 != y1 {
                edges.push((x0, y0, x1, y1));
            }
        }
    }
    if edges.is_empty() {
        return cov;
    }
    let top = edges.iter().map(|e| e.1.min(e.3)).fold(f64::MAX, f64::min);
    let bottom = edges.iter().map(|e| e.1.max(e.3)).fold(f64::MIN, f64::max);
    let first_row = top.floor().max(0.0) as usize;
    let last_row = (bottom.ceil().max(0.0) as usize).min(h);
    let weight = 1.0 / SUBSAMPLES as f64;

    let mut crossings: Vec<(f64, i32)> = Vec::new();
    for row in first_row..last_row {
        for s in 0..SUBSAMPLES {
            let y = row as f64 + (s as f64 + 0.5) / SUBSAMPLES as f64;
            crossings.clear();
            for &(x0, y0, x1, y1) in &edges {
                if (y0 <= y) == (y1 <= y) {
                    continue;
                }
                let t = (y - y0) / (y1 - y0);
                crossings.push((x0 + t * (x1 - x0), if y1 > y0 { 1 } else { -1 }));
            }
            crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut winding = 0;
            let mut start = 0.0;
            for &(x, dir) in &crossings {
                if winding == 0 {
                    start = x;
                }
                winding += dir;
                if winding == 0 {
                    let a = start.max(0.0);
                    let b = x.min(w as f64);
                    if b > a {
                        let first = a.floor() as usize;
                        let last = ((b.ceil() as usize).max(first + 1)).min(w);
                        for px in first..last {
                            let l = a.max(px as f64);
                            let r = b.min(px as f64 + 1.0);
                            if r > l {
                                cov[row * w + px] += (r - l) * weight;
                            }
                        }
                    }
                }
            }
        }
    }
    cov
}

// --------------------------------------------------------------- flattening

fn quad(out: &mut Vec<(f64, f64)>, a: (f64, f64), c: (f64, f64), b: (f64, f64)) {
    for step in 1..=CURVE_STEPS {
        let t = step as f64 / CURVE_STEPS as f64;
        let u = 1.0 - t;
        out.push((
            u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
            u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
        ));
    }
}

fn cubic(out: &mut Vec<(f64, f64)>, a: (f64, f64), c1: (f64, f64), c2: (f64, f64), b: (f64, f64)) {
    for step in 1..=CURVE_STEPS {
        let t = step as f64 / CURVE_STEPS as f64;
        let u = 1.0 - t;
        out.push((
            u * u * u * a.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * b.0,
            u * u * u * a.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * b.1,
        ));
    }
}

/// One TrueType contour, including implied on-curve midpoints.
/// `place` maps a font-unit point to device space.
fn contour(points: &[Point], place: &dyn Fn(f64, f64) -> (f64, f64)) -> Vec<(f64, f64)> {
    let device = |p: &Point| place(f(p.x), f(p.y));
    let mid = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);

    let n = points.len();
    let mut out: Vec<(f64, f64)> = Vec::new();
    if n == 0 {
        return out;
    }
    let start_index = points.iter().position(|p| p.on_curve);
    let start = match start_index {
        Some(i) => device(&points[i]),
        None => mid(device(&points[0]), device(&points[n - 1])),
    };
    let base = start_index.map_or(0, |i| i + 1);
    out.push(start);

    let mut current = start;
    let mut control: Option<(f64, f64)> = None;
    for k in 0..n {
        let p = &points[(base + k) % n];
        let d = device(p);
        if p.on_curve {
            match control.take() {
                Some(c) => quad(&mut out, current, c, d),
                None => out.push(d),
            }
            current = d;
        } else if let Some(c) = control.replace(d) {
            let implied = mid(c, d);
            quad(&mut out, current, c, implied);
            current = implied;
        }
    }
    if let Some(c) = control {
        quad(&mut out, current, c, start);
    }
    out
}

/// CFF / CFF2 cubic path commands.
fn cff_polys(commands: &[Command], place: &dyn Fn(f64, f64) -> (f64, f64)) -> Vec<Vec<(f64, f64)>> {
    let d = |p: text_core::cff::Position| place(f(p.x), f(p.y));
    let mut polys = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    for command in commands {
        match *command {
            Command::Move(p) => {
                if cur.len() > 1 {
                    polys.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                cur.push(d(p));
            }
            Command::Line(p) => cur.push(d(p)),
            Command::Curve(c1, c2, p) => {
                if let Some(&start) = cur.last() {
                    cubic(&mut cur, start, d(c1), d(c2), d(p));
                }
            }
            Command::Close => {
                if cur.len() > 1 {
                    polys.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
        }
    }
    if cur.len() > 1 {
        polys.push(cur);
    }
    polys
}

/// One glyph's outline as device-space polygons.
#[allow(clippy::too_many_arguments)]
fn glyph_polys(
    face: &Font,
    glyph: u16,
    coords: &[Fixed],
    points: &mut [Point],
    contours: &mut [usize],
    scratch: &mut [VariationPoint],
    commands: &mut [Command],
    place: &dyn Fn(f64, f64) -> (f64, f64),
) -> Result<Vec<Vec<(f64, f64)>>, Box<dyn Error>> {
    Ok(match face.outline_kind() {
        OutlineKind::TrueType => {
            let glyf = Glyf::parse(face)?;
            // `outline_instance` needs an `fvar` table even for an empty
            // coordinate slice, so a static face takes the plain path.
            let outline = if coords.is_empty() {
                glyf.outline(glyph, points, contours)?
            } else {
                glyf.outline_instance(glyph, coords, points, contours, scratch)?
            };
            let mut polys = Vec::new();
            let mut start = 0usize;
            for &end in &contours[..outline.contours] {
                polys.push(contour(&points[start..=end], place));
                start = end + 1;
            }
            polys
        }
        OutlineKind::PostScript => {
            let cff = Cff::parse(face)?;
            let used = cff.outline_instance(glyph, coords, commands)?;
            cff_polys(&commands[..used], place)
        }
    })
}

// ----------------------------------------------------------------- colour

fn rgb_of(color: Color) -> ([f64; 3], f64) {
    let rgb = match color.source {
        ColorSource::Palette { red, green, blue } => [
            f64::from(red) / 255.0,
            f64::from(green) / 255.0,
            f64::from(blue) / 255.0,
        ],
        ColorSource::Foreground => TEXT_RGB,
    };
    (rgb, f(color.alpha))
}

/// Colour at position `t` on a colour line.
fn sample(stops: &[ColorStop], extend: Extend, t: f64) -> ([f64; 3], f64) {
    if stops.is_empty() {
        return ([0.0; 3], 0.0);
    }
    let first = f(stops[0].offset);
    let last = f(stops[stops.len() - 1].offset);
    let span = last - first;
    let t = if span <= 0.0 {
        first
    } else {
        match extend {
            Extend::Pad => t.clamp(first, last),
            Extend::Repeat => first + (t - first).rem_euclid(span),
            Extend::Reflect => {
                let u = (t - first).rem_euclid(2.0 * span);
                first + if u > span { 2.0 * span - u } else { u }
            }
        }
    };
    let mut lo = 0usize;
    while lo + 1 < stops.len() && f(stops[lo + 1].offset) < t {
        lo += 1;
    }
    let hi = (lo + 1).min(stops.len() - 1);
    let (a, aa) = rgb_of(stops[lo].color);
    let (b, ba) = rgb_of(stops[hi].color);
    let x0 = f(stops[lo].offset);
    let x1 = f(stops[hi].offset);
    let k = if (x1 - x0).abs() < 1e-12 {
        0.0
    } else {
        ((t - x0) / (x1 - x0)).clamp(0.0, 1.0)
    };
    (
        [
            a[0] + (b[0] - a[0]) * k,
            a[1] + (b[1] - a[1]) * k,
            a[2] + (b[2] - a[2]) * k,
        ],
        aa + (ba - aa) * k,
    )
}

/// The inverse of an affine transform, as six f64.
fn invert(t: Affine) -> Option<[f64; 6]> {
    let (xx, yx, xy, yy, dx, dy) = (f(t.xx), f(t.yx), f(t.xy), f(t.yy), f(t.dx), f(t.dy));
    let det = xx * yy - xy * yx;
    if det.abs() < 1e-12 {
        return None;
    }
    Some([
        yy / det,
        -yx / det,
        -xy / det,
        xx / det,
        (xy * dy - yy * dx) / det,
        (yx * dx - xx * dy) / det,
    ])
}

fn apply_inv(m: [f64; 6], x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

/// Separable blend of one channel; Porter-Duff modes never reach here.
fn blend_channel(mode: CompositeMode, cs: f64, cb: f64) -> f64 {
    match mode {
        CompositeMode::Screen => cs + cb - cs * cb,
        CompositeMode::Overlay => blend_channel(CompositeMode::HardLight, cb, cs),
        CompositeMode::Darken => cs.min(cb),
        CompositeMode::Lighten => cs.max(cb),
        CompositeMode::ColorDodge => {
            if cb <= 0.0 {
                0.0
            } else if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        CompositeMode::ColorBurn => {
            if cb >= 1.0 {
                1.0
            } else if cs <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - cb) / cs).min(1.0)
            }
        }
        CompositeMode::HardLight => {
            if cs <= 0.5 {
                2.0 * cs * cb
            } else {
                let (s, b) = (2.0 * cs - 1.0, cb);
                s + b - s * b
            }
        }
        CompositeMode::SoftLight => {
            let d = if cb <= 0.25 {
                ((16.0 * cb - 12.0) * cb + 4.0) * cb
            } else {
                cb.sqrt()
            };
            if cs <= 0.5 {
                cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
            } else {
                cb + (2.0 * cs - 1.0) * (d - cb)
            }
        }
        CompositeMode::Difference => (cs - cb).abs(),
        CompositeMode::Exclusion => cs + cb - 2.0 * cs * cb,
        CompositeMode::Multiply => cs * cb,
        _ => cs,
    }
}

/// Combine `src` over `backdrop` with `mode`, straight alpha.
fn composite(mode: CompositeMode, src: [f64; 4], backdrop: [f64; 4]) -> [f64; 4] {
    let (sa, ba) = (src[3], backdrop[3]);
    // Porter-Duff coefficients: result = fs*src + fb*backdrop, premultiplied.
    let (fs, fb) = match mode {
        CompositeMode::Clear => (0.0, 0.0),
        CompositeMode::Src => (1.0, 0.0),
        CompositeMode::Dest => (0.0, 1.0),
        CompositeMode::SrcOver => (1.0, 1.0 - sa),
        CompositeMode::DestOver => (1.0 - ba, 1.0),
        CompositeMode::SrcIn => (ba, 0.0),
        CompositeMode::DestIn => (0.0, sa),
        CompositeMode::SrcOut => (1.0 - ba, 0.0),
        CompositeMode::DestOut => (0.0, 1.0 - sa),
        CompositeMode::SrcAtop => (ba, 1.0 - sa),
        CompositeMode::DestAtop => (1.0 - ba, sa),
        CompositeMode::Xor => (1.0 - ba, 1.0 - sa),
        CompositeMode::Plus => (1.0, 1.0),
        // Blend modes: mix the colours, then source-over.
        _ => (1.0, 1.0 - sa),
    };
    let separable = !matches!(
        mode,
        CompositeMode::Clear
            | CompositeMode::Src
            | CompositeMode::Dest
            | CompositeMode::SrcOver
            | CompositeMode::DestOver
            | CompositeMode::SrcIn
            | CompositeMode::DestIn
            | CompositeMode::SrcOut
            | CompositeMode::DestOut
            | CompositeMode::SrcAtop
            | CompositeMode::DestAtop
            | CompositeMode::Xor
            | CompositeMode::Plus
    );
    let a = (sa * fs + ba * fb).clamp(0.0, 1.0);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let mut out = [0.0; 4];
    for c in 0..3 {
        let cs = if separable {
            blend_channel(mode, src[c], backdrop[c])
        } else {
            src[c]
        };
        out[c] = ((cs * sa * fs + backdrop[c] * ba * fb) / a).clamp(0.0, 1.0);
    }
    out[3] = a;
    out
}

// -------------------------------------------------------------------- main

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: text-demo <font.ttf[,font2,...]> [text] [px] [max_width_px] [weight]")?;
    let text = args
        .next()
        .unwrap_or_else(|| "Handgloves — Grüße, 0O 1Il 8B".to_owned());
    let px: i32 = args.next().map_or(Ok(32), |s| s.parse())?;
    let max_width: Option<i32> = match args.next() {
        Some(s) => Some(s.parse()?),
        None => None,
    };
    let weight: u16 = args.next().map_or(Ok(400), |s| s.parse())?;

    // The chain, in order: first path is the base face, the rest are fallbacks.
    let paths: Vec<&str> = path.split(',').collect();
    let blobs: Vec<Vec<u8>> = blobs_of(&paths)?;
    let faces: Vec<Font> = blobs
        .iter()
        .map(|b| Font::parse(b))
        .collect::<Result<_, _>>()?;
    for (i, (p, face)) in paths.iter().zip(&faces).enumerate() {
        let colour = if face.table(*b"COLR").is_some() {
            " COLR"
        } else {
            ""
        };
        println!("face {i}: {:?}{colour} {p}", face.outline_kind());
    }
    let set = FontSet {
        ui: &faces,
        mono: &faces,
        generation: 1,
    };
    let style = TextStyle {
        role: Role::Ui,
        size: Fixed::from_i32(px),
        lang: Language::parse("de")?,
        weight,
    };

    // Everything above this line is text-core. Everything below is the toy.
    let laid = text_core::layout(&set, &style, &text, max_width.map(Fixed::from_i32))?;
    let view = laid.view();
    println!(
        "{} lines, {} glyphs, box {:.2} x {:.2} px",
        view.info.lines,
        view.info.glyphs,
        f(view.info.width),
        f(view.info.height)
    );
    for (i, run) in view.runs.iter().enumerate() {
        let coords: Vec<f64> = run.coordinates().iter().map(|c| f(*c)).collect();
        println!(
            "  run {i}: bytes {}..{} face {} ({}) level {} script {:?} simple {} missing {} scale {:.5} coords {:?}",
            run.start,
            run.end,
            run.face,
            paths.get(run.face).copied().unwrap_or("?"),
            run.level,
            run.script,
            run.simple,
            run.missing,
            f(run.scale),
            coords
        );
    }

    let w = (f(view.info.width) + 2.0 * PAD).ceil() as usize;
    let h = (f(view.info.height) + 2.0 * PAD).ceil() as usize;
    let mut page = Surface::new(w, h);

    let mut points = vec![Point::default(); 16384];
    let mut contours = vec![0usize; 1024];
    let mut scratch = vec![VariationPoint::default(); 16384];
    let mut commands = vec![Command::Close; 16384];
    let mut ops = vec![PaintOp::Unclip; 8192];
    let mut stops = vec![ColorStop::default(); 4096];
    let mut colour_glyphs = 0usize;
    let mut unbounded = 0usize;

    for line in view.lines {
        let glyphs = &view.glyphs[line.glyph_start..line.glyph_start + line.glyph_count];
        for g in glyphs {
            if g.hidden {
                continue;
            }
            let run = &view.runs[g.run];
            let face = &faces[run.face];
            let scale = f(run.scale);
            // g.y is already the absolute origin: text-core adds the line
            // baseline in `position()`. Do not add it again. Every run sits
            // on one alphabetic baseline (D-171), so there is no per-face
            // offset to apply.
            let ox = PAD + f(g.x);
            let oy = PAD + f(g.y);
            let place = move |x: f64, y: f64| (ox + x * scale, oy - y * scale);

            // A colour definition wins over the plain outline.
            let colr = if face.table(*b"COLR").is_some() {
                Some(Colr::parse(face)?)
            } else {
                None
            };
            let coloured = match &colr {
                Some(c) => c.covers(g.id)?,
                None => false,
            };

            if coloured {
                let c = colr.as_ref().expect("checked");
                let painted = c.paint(g.id, PALETTE, run.coordinates(), &mut ops, &mut stops)?;
                if !painted.bounded {
                    unbounded += 1;
                    continue;
                }
                colour_glyphs += 1;
                paint_stream(
                    &mut page,
                    face,
                    run.coordinates(),
                    &ops[..painted.ops],
                    &stops[..painted.stops],
                    &place,
                    scale,
                    ox,
                    oy,
                    &mut points,
                    &mut contours,
                    &mut scratch,
                    &mut commands,
                )?;
                continue;
            }

            let polys = glyph_polys(
                face,
                g.id,
                run.coordinates(),
                &mut points,
                &mut contours,
                &mut scratch,
                &mut commands,
                &place,
            )?;
            let cov = coverage(w, h, &polys);
            for (i, a) in cov.iter().enumerate() {
                if *a > 0.0 {
                    over(
                        &mut page.px[i],
                        [TEXT_RGB[0], TEXT_RGB[1], TEXT_RGB[2], a.min(1.0)],
                    );
                }
            }
        }
    }

    page.write_ppm("out.ppm")?;
    println!("wrote out.ppm ({w} x {h}), {colour_glyphs} colour glyphs, {unbounded} unbounded");
    Ok(())
}

fn blobs_of(paths: &[&str]) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
    let mut out = Vec::new();
    for p in paths {
        out.push(fs::read(p)?);
    }
    Ok(out)
}

/// Draw one resolved colour glyph.
#[allow(clippy::too_many_arguments)]
fn paint_stream(
    page: &mut Surface,
    face: &Font,
    coords: &[Fixed],
    ops: &[PaintOp],
    stops: &[ColorStop],
    place: &dyn Fn(f64, f64) -> (f64, f64),
    scale: f64,
    ox: f64,
    oy: f64,
    points: &mut [Point],
    contours: &mut [usize],
    scratch: &mut [VariationPoint],
    commands: &mut [Command],
) -> Result<(), Box<dyn Error>> {
    let (w, h) = (page.w, page.h);
    // Clip stack: the intersection of every open Clip.
    let mut clips: Vec<Vec<f64>> = vec![vec![1.0; w * h]];
    // Target stack: the page, then one surface per open Group.
    let mut targets: Vec<Surface> = Vec::new();

    for op in ops {
        match *op {
            PaintOp::Clip { glyph, transform } => {
                // The outline is in font units under `transform`.
                let inner = move |x: f64, y: f64| {
                    let (tx, ty) = affine(transform, x, y);
                    place(tx, ty)
                };
                let polys = glyph_polys(
                    face, glyph, coords, points, contours, scratch, commands, &inner,
                )?;
                let cov = coverage(w, h, &polys);
                let top = clips.last().expect("clip stack");
                let next: Vec<f64> = cov
                    .iter()
                    .zip(top.iter())
                    .map(|(a, b)| (a.min(1.0)) * b)
                    .collect();
                clips.push(next);
            }
            PaintOp::Unclip => {
                clips.pop();
                if clips.is_empty() {
                    return Err("unmatched Unclip".into());
                }
            }
            PaintOp::Group => targets.push(Surface::new(w, h)),
            PaintOp::Compose(mode) => {
                let source = targets.pop().ok_or("Compose without a source group")?;
                let backdrop = targets.pop().ok_or("Compose without a backdrop group")?;
                let mut result = Surface::new(w, h);
                for i in 0..w * h {
                    result.px[i] = composite(mode, source.px[i], backdrop.px[i]);
                }
                let dst = targets.last_mut().map_or(&mut *page, |s| s);
                for i in 0..w * h {
                    over(&mut dst.px[i], result.px[i]);
                }
            }
            PaintOp::Fill { transform, fill } => {
                let mask = clips.last().expect("clip stack").clone();
                let inv = invert(transform);
                let dst = targets.last_mut().map_or(&mut *page, |s| s);
                for row in 0..h {
                    for col in 0..w {
                        let a = mask[row * w + col];
                        if a <= 0.0 {
                            continue;
                        }
                        // Pixel centre back into the fill's own space.
                        let fx = (col as f64 + 0.5 - ox) / scale;
                        let fy = (oy - (row as f64 + 0.5)) / scale;
                        let (gx, gy) = match inv {
                            Some(m) => apply_inv(m, fx, fy),
                            None => (fx, fy),
                        };
                        let (rgb, alpha) = match fill {
                            Fill::Solid(c) => rgb_of(c),
                            Fill::Linear {
                                x0,
                                y0,
                                x1,
                                y1,
                                x2,
                                y2,
                                line,
                            } => {
                                let (p0, p1, p2) =
                                    ((f(x0), f(y0)), (f(x1), f(y1)), (f(x2), f(y2)));
                                // p3 = p0 + projection of p0p1 onto the line
                                // perpendicular to p0p2, through p0.
                                let n = (p2.0 - p0.0, p2.1 - p0.1);
                                let d = (p1.0 - p0.0, p1.1 - p0.1);
                                let nn = n.0 * n.0 + n.1 * n.1;
                                let k = if nn <= 0.0 {
                                    0.0
                                } else {
                                    (d.0 * n.0 + d.1 * n.1) / nn
                                };
                                let v = (d.0 - n.0 * k, d.1 - n.1 * k);
                                let vv = v.0 * v.0 + v.1 * v.1;
                                let t = if vv <= 0.0 {
                                    0.0
                                } else {
                                    ((gx - p0.0) * v.0 + (gy - p0.1) * v.1) / vv
                                };
                                let s = &stops[line.first..line.first + line.count];
                                sample(s, line.extend, t)
                            }
                            Fill::Radial {
                                x0,
                                y0,
                                r0,
                                x1,
                                y1,
                                r1,
                                line,
                            } => {
                                let t = two_point_conical(
                                    (f(x0), f(y0)),
                                    f(r0),
                                    (f(x1), f(y1)),
                                    f(r1),
                                    (gx, gy),
                                );
                                let s = &stops[line.first..line.first + line.count];
                                match t {
                                    Some(t) => sample(s, line.extend, t),
                                    None => ([0.0; 3], 0.0),
                                }
                            }
                            Fill::Sweep {
                                x,
                                y,
                                start,
                                end,
                                line,
                            } => {
                                // Angle in half-turns, counter-clockwise.
                                let ang = (gy - f(y)).atan2(gx - f(x))
                                    / std::f64::consts::PI;
                                let ang = if ang < 0.0 { ang + 2.0 } else { ang };
                                let (s0, s1) = (f(start), f(end));
                                let t = if (s1 - s0).abs() < 1e-12 {
                                    0.0
                                } else {
                                    (ang - s0) / (s1 - s0)
                                };
                                let s = &stops[line.first..line.first + line.count];
                                sample(s, line.extend, t)
                            }
                        };
                        let total = alpha * a;
                        if total > 0.0 {
                            over(&mut dst.px[row * w + col], [rgb[0], rgb[1], rgb[2], total]);
                        }
                    }
                }
            }
        }
    }
    if !targets.is_empty() {
        return Err("unbalanced Group".into());
    }
    Ok(())
}

fn affine(t: Affine, x: f64, y: f64) -> (f64, f64) {
    (
        f(t.xx) * x + f(t.xy) * y + f(t.dx),
        f(t.yx) * x + f(t.yy) * y + f(t.dy),
    )
}

/// Largest t with the point on the circle interpolated between the two.
fn two_point_conical(
    c0: (f64, f64),
    r0: f64,
    c1: (f64, f64),
    r1: f64,
    p: (f64, f64),
) -> Option<f64> {
    let cd = (c1.0 - c0.0, c1.1 - c0.1);
    let dr = r1 - r0;
    let pd = (p.0 - c0.0, p.1 - c0.1);
    let a = cd.0 * cd.0 + cd.1 * cd.1 - dr * dr;
    let b = pd.0 * cd.0 + pd.1 * cd.1 + r0 * dr;
    let c = pd.0 * pd.0 + pd.1 * pd.1 - r0 * r0;
    if a.abs() < 1e-12 {
        if b.abs() < 1e-12 {
            return None;
        }
        let t = c / (2.0 * b);
        return if r0 + t * dr >= 0.0 { Some(t) } else { None };
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    for t in [(b + root) / a, (b - root) / a] {
        if r0 + t * dr >= 0.0 {
            return Some(t);
        }
    }
    None
}
