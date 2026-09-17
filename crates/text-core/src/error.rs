// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use core::fmt;

/// A font envelope failed validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontError {
    /// A gvar metric requires the decoded instance's phantom points.
    MissingOutline,
    /// The caller-provided output storage is too small.
    BufferTooSmall,
    /// A recursive font structure refers to an active ancestor.
    Cycle,
    /// A required table is absent.
    MissingTable,
    /// A table's internal structure or values are inconsistent.
    InvalidTable,
    /// A glyph identifier is outside the face's glyph set.
    GlyphIndex,
    /// A required byte range exceeds the input.
    Truncated,
    /// An offset or length cannot be represented.
    Overflow,
    /// The sfnt signature or TTC version is unsupported.
    UnsupportedFormat,
    /// A face or table count is zero.
    EmptyDirectory,
    /// A format count exceeds the documented work limits.
    LimitExceeded,
    /// A directory or table offset is not four-byte aligned.
    Misaligned,
    /// A tag is not printable ASCII with trailing spaces only.
    InvalidTag,
    /// Tags are not strictly increasing.
    TableOrder,
    /// Metadata or distinct table ranges intersect.
    Overlap,
    /// TTC signature fields are inconsistent or the signature is not at EOF.
    InvalidSignature,
    /// The requested face does not exist.
    FaceIndex,
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingOutline => "variable glyph metrics require an instance outline",
            Self::BufferTooSmall => "font output buffer too small",
            Self::Cycle => "cyclic font structure",
            Self::MissingTable => "required font table is absent",
            Self::InvalidTable => "invalid font table structure",
            Self::GlyphIndex => "glyph index out of range",
            Self::Truncated => "truncated font range",
            Self::Overflow => "font offset arithmetic overflow",
            Self::UnsupportedFormat => "unsupported font format",
            Self::EmptyDirectory => "empty font directory",
            Self::LimitExceeded => "font validation limit exceeded",
            Self::Misaligned => "unaligned font offset",
            Self::InvalidTag => "invalid font table tag",
            Self::TableOrder => "font table tags are not strictly ordered",
            Self::Overlap => "overlapping font ranges",
            Self::InvalidSignature => "invalid collection signature range",
            Self::FaceIndex => "font face index out of range",
        })
    }
}

impl core::error::Error for FontError {}
