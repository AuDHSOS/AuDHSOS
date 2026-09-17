// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Affinity, Cursor, LayoutView, Rect, engine::add};
use crate::{Fixed, TextError};
impl LayoutView<'_> {
    fn boundary(self, byte: usize) -> bool {
        byte == 0
            || byte == self.info.text_len
            || self
                .clusters
                .iter()
                .any(|c| c.start == byte || c.end == byte)
    }
    /// Return all visual cursor positions at an original grapheme boundary.
    /// # Errors
    /// Rejects non-boundary offsets, invalid views, and insufficient output.
    pub fn carets(self, byte: usize, out: &mut [Cursor]) -> Result<usize, TextError> {
        if byte > self.info.text_len || !self.boundary(byte) {
            return Err(TextError::InvalidInput);
        }
        let mut count = 0;
        for cluster in self.clusters {
            let Some(affinity) = (if cluster.start == byte {
                Some(Affinity::Downstream)
            } else if cluster.end == byte {
                Some(Affinity::Upstream)
            } else {
                None
            }) else {
                continue;
            };
            let line = self
                .lines
                .get(cluster.line)
                .ok_or(TextError::InvalidInput)?;
            let right = (affinity == Affinity::Downstream) == (cluster.level & 1 != 0);
            *out.get_mut(count).ok_or(TextError::BufferTooSmall)? = Cursor {
                byte,
                line: cluster.line,
                x: if right { cluster.right } else { cluster.left },
                y: line.y,
                height: line.height,
                affinity,
            };
            count = add(count, 1)?;
        }
        for (index, line) in self.lines.iter().enumerate() {
            if line.start == byte && line.start == line.end {
                *out.get_mut(count).ok_or(TextError::BufferTooSmall)? = Cursor {
                    byte,
                    line: index,
                    x: Fixed::ZERO,
                    y: line.y,
                    height: line.height,
                    affinity: Affinity::Downstream,
                };
                count = add(count, 1)?;
            }
        }
        if self.info.text_len == 0 {
            *out.get_mut(count).ok_or(TextError::BufferTooSmall)? = Cursor::default();
            count = add(count, 1)?;
        }
        Ok(count)
    }
    /// Return merged visual rectangles for a range of whole graphemes.
    /// # Errors
    /// Rejects invalid/non-boundary ranges, inconsistent views, and insufficient output.
    pub fn selection(
        self,
        range: core::ops::Range<usize>,
        out: &mut [Rect],
    ) -> Result<usize, TextError> {
        if range.start > range.end
            || range.end > self.info.text_len
            || !self.boundary(range.start)
            || !self.boundary(range.end)
        {
            return Err(TextError::InvalidInput);
        }
        let mut count: usize = 0;
        let mut prior_line = None;
        for cluster in self.clusters {
            if cluster.start < range.start
                || cluster.end > range.end
                || cluster.left == cluster.right
            {
                continue;
            }
            let line = self
                .lines
                .get(cluster.line)
                .ok_or(TextError::InvalidInput)?;
            if prior_line == Some(cluster.line)
                && let Some(prior) = count.checked_sub(1).and_then(|i| out.get_mut(i))
                && prior.x.checked_add(prior.width)? >= cluster.left
            {
                prior.width = prior
                    .x
                    .checked_add(prior.width)?
                    .max(cluster.right)
                    .checked_sub(prior.x)?;
                continue;
            }
            *out.get_mut(count).ok_or(TextError::BufferTooSmall)? = Rect {
                x: cluster.left,
                y: line.y,
                width: cluster.right.checked_sub(cluster.left)?,
                height: line.height,
            };
            count = add(count, 1)?;
            prior_line = Some(cluster.line);
        }
        Ok(count)
    }
}
