// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One glyph's outline as device-space edges, whichever table states it.
//!
//! This crate reads no font table itself: `text-core` decodes the outline
//! (D-173) and this module only chooses which of its two decoders to call and
//! hands the result to the flattening of R2.

use text_core::{
    Fixed, Font, OutlineKind,
    cff::{Cff, Command},
    colr::Affine,
    glyf::{Glyf, Point},
    variation::VariationPoint,
};

use crate::{
    error::RasterError,
    flatten::{Edge, flatten_cff, flatten_glyf},
};

/// Caller-owned storage the outline decoders of `text-core` write into.
#[derive(Debug)]
pub struct OutlineScratch<'a> {
    /// TrueType control points.
    pub points: &'a mut [Point],
    /// TrueType contour endpoints.
    pub contours: &'a mut [usize],
    /// `gvar` decoding scratch.
    pub variation: &'a mut [VariationPoint],
    /// CFF path commands.
    pub commands: &'a mut [Command],
}

/// Decode one glyph and flatten it into `edges`, returning how many it wrote.
///
/// `transform` maps font units to device pixels. A face that states neither
/// outline table writes no edge.
/// # Errors
/// Returns `Font` for a malformed outline, `BufferTooSmall` when a caller
/// slice cannot hold the result, and `Overflow` when a coordinate leaves
/// Q32.32.
pub fn outline_edges(
    font: &Font<'_>,
    glyph: u16,
    coordinates: &[Fixed],
    transform: Affine,
    scratch: &mut OutlineScratch<'_>,
    edges: &mut [Edge],
) -> Result<usize, RasterError> {
    match font.outline_kind() {
        OutlineKind::TrueType => {
            let Ok(glyf) = Glyf::parse(font) else {
                return Ok(0);
            };
            let outline = glyf.outline_instance(
                glyph,
                coordinates,
                scratch.points,
                scratch.contours,
                scratch.variation,
            )?;
            let points = scratch
                .points
                .get(..outline.points)
                .ok_or(RasterError::BufferTooSmall)?;
            let contours = scratch
                .contours
                .get(..outline.contours)
                .ok_or(RasterError::BufferTooSmall)?;
            flatten_glyf(points, contours, transform, edges)
        }
        OutlineKind::PostScript => {
            let Ok(cff) = Cff::parse(font) else {
                return Ok(0);
            };
            let count = cff.outline_instance(glyph, coordinates, scratch.commands)?;
            let commands = scratch
                .commands
                .get(..count)
                .ok_or(RasterError::BufferTooSmall)?;
            flatten_cff(commands, transform, edges)
        }
    }
}
