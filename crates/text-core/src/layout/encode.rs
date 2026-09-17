// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{LayoutView, engine::add};
use crate::{Fixed, Role, TextError, unicode::Script};
struct Writer<'a> {
    bytes: Option<&'a mut [u8]>,
    at: usize,
}
impl Writer<'_> {
    fn put(&mut self, value: &[u8]) -> Result<(), TextError> {
        let end = add(self.at, value.len())?;
        if let Some(bytes) = self.bytes.as_deref_mut() {
            bytes
                .get_mut(self.at..end)
                .ok_or(TextError::BufferTooSmall)?
                .copy_from_slice(value);
        }
        self.at = end;
        Ok(())
    }
    fn index(&mut self, n: usize) -> Result<(), TextError> {
        self.put(
            &u64::try_from(n)
                .map_err(|_| TextError::Overflow)?
                .to_le_bytes(),
        )
    }
    fn fixed(&mut self, value: Fixed) -> Result<(), TextError> {
        self.put(&value.bits().to_le_bytes())
    }
}
#[expect(
    clippy::as_conversions,
    reason = "generated Script has repr(u16); versioned numeric wire ID"
)]
const fn script_id(script: Script) -> u16 {
    script as u16
}
impl LayoutView<'_> {
    /// Size of the version-two little-endian numeric encoding.
    /// # Errors
    /// Returns a numeric size overflow.
    pub fn encoded_len(self) -> Result<usize, TextError> {
        self.encode(None)
    }
    /// Encode fields explicitly, excluding Rust padding and private scratch fields.
    /// # Errors
    /// Returns insufficient output capacity or numeric size overflow.
    pub fn write_bytes(self, out: &mut [u8]) -> Result<usize, TextError> {
        self.encode(Some(out))
    }
    fn encode(self, out: Option<&mut [u8]>) -> Result<usize, TextError> {
        let mut w = Writer { bytes: out, at: 0 };
        w.put(b"TEXT\x02\0\0\0")?;
        w.put(&18_u16.to_le_bytes())?;
        w.put(&self.info.generation.to_le_bytes())?;
        w.put(&[match self.info.role {
            Role::Ui => 0,
            Role::Mono => 1,
        }])?;
        w.index(self.info.text_len)?;
        w.fixed(self.info.width)?;
        w.fixed(self.info.height)?;
        for n in [
            self.runs.len(),
            self.lines.len(),
            self.glyphs.len(),
            self.clusters.len(),
        ] {
            w.index(n)?;
        }
        for run in self.runs {
            for n in [run.start, run.end, run.face] {
                w.index(n)?;
            }
            w.put(&run.generation.to_le_bytes())?;
            w.put(&script_id(run.script).to_le_bytes())?;
            w.put(&run.language.tag())?;
            w.put(&[run.level, u8::from(run.simple), u8::from(run.missing)])?;
            w.fixed(run.scale)?;
            w.index(run.coordinates().len())?;
            for coord in run.coordinates() {
                w.fixed(*coord)?;
            }
        }
        for line in self.lines {
            for n in [
                line.start,
                line.end,
                line.glyph_start,
                line.glyph_count,
                line.cluster_start,
                line.cluster_count,
            ] {
                w.index(n)?;
            }
            for v in [line.y, line.height, line.baseline, line.width] {
                w.fixed(v)?;
            }
            w.put(&[u8::from(line.overflow)])?;
        }
        for glyph in self.glyphs {
            for n in [glyph.run, glyph.start, glyph.end] {
                w.index(n)?;
            }
            w.put(&glyph.id.to_le_bytes())?;
            for v in [glyph.x, glyph.y, glyph.advance] {
                w.fixed(v)?;
            }
            w.put(&[glyph.level, u8::from(glyph.hidden)])?;
        }
        for cluster in self.clusters {
            for n in [cluster.start, cluster.end, cluster.line] {
                w.index(n)?;
            }
            w.fixed(cluster.left)?;
            w.fixed(cluster.right)?;
            w.put(&[cluster.level])?;
        }
        Ok(w.at)
    }
}
