// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tags, in the low tag number form that is all a certificate uses.

/// The identifier octet of a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tag(u8);

/// The bit that marks a constructed value.
const CONSTRUCTED: u8 = 0x20;
/// The bits that carry the class.
const CLASS: u8 = 0xC0;
/// The class of a context-specific tag.
const CONTEXT: u8 = 0x80;
/// The tag number that introduces the high tag number form.
const HIGH_FORM: u8 = 0x1F;

impl Tag {
    /// `BOOLEAN`.
    pub const BOOLEAN: Tag = Tag(0x01);
    /// `INTEGER`.
    pub const INTEGER: Tag = Tag(0x02);
    /// `BIT STRING`.
    pub const BIT_STRING: Tag = Tag(0x03);
    /// `OCTET STRING`.
    pub const OCTET_STRING: Tag = Tag(0x04);
    /// `NULL`.
    pub const NULL: Tag = Tag(0x05);
    /// `OBJECT IDENTIFIER`.
    pub const OBJECT_IDENTIFIER: Tag = Tag(0x06);
    /// `UTF8String`.
    pub const UTF8_STRING: Tag = Tag(0x0C);
    /// `PrintableString`.
    pub const PRINTABLE_STRING: Tag = Tag(0x13);
    /// `TeletexString`, which old certificates still carry.
    pub const TELETEX_STRING: Tag = Tag(0x14);
    /// `IA5String`.
    pub const IA5_STRING: Tag = Tag(0x16);
    /// `UTCTime`.
    pub const UTC_TIME: Tag = Tag(0x17);
    /// `GeneralizedTime`.
    pub const GENERALIZED_TIME: Tag = Tag(0x18);
    /// `SEQUENCE`, which is always constructed.
    pub const SEQUENCE: Tag = Tag(0x30);
    /// `SET`, which is always constructed.
    pub const SET: Tag = Tag(0x31);

    /// The tag with the given identifier octet.
    #[must_use]
    pub const fn new(octet: u8) -> Tag {
        Tag(octet)
    }

    /// A context-specific tag, constructed or primitive, as explicit and
    /// implicit tagging respectively produce them.
    #[must_use]
    pub const fn context(number: u8, constructed: bool) -> Tag {
        let form = if constructed { CONSTRUCTED } else { 0 };
        Tag(CONTEXT | form | (number & HIGH_FORM))
    }

    /// The identifier octet.
    #[must_use]
    pub const fn octet(self) -> u8 {
        self.0
    }

    /// Whether the value is constructed.
    #[must_use]
    pub const fn is_constructed(self) -> bool {
        self.0 & CONSTRUCTED != 0
    }

    /// Whether the tag is context-specific.
    #[must_use]
    pub const fn is_context(self) -> bool {
        self.0 & CLASS == CONTEXT
    }

    /// Whether the tag uses the high tag number form, which this reader
    /// refuses.
    #[must_use]
    pub const fn is_high_form(self) -> bool {
        self.0 & HIGH_FORM == HIGH_FORM
    }
}
