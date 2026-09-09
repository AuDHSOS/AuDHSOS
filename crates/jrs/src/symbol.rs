// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Symbols have identity, not string equality. Heap handles make symbol keys
//! visible to the tracing collector without retaining weak associations.

use alloc::rc::Rc;

/// Opaque Symbol identity and optional UTF-16 description.
#[derive(Clone, Debug)]
pub struct SymbolValue {
    pub(crate) handle: crate::heap::Handle,
    pub(crate) owner: Rc<()>,
    pub(crate) description: Option<Rc<[u16]>>,
    pub(crate) registered: bool,
}

impl PartialEq for SymbolValue {
    fn eq(&self, other: &Self) -> bool {
        self.handle == other.handle && Rc::ptr_eq(&self.owner, &other.owner)
    }
}

impl SymbolValue {
    pub(crate) fn descriptive(&self) -> Rc<[u16]> {
        let mut units: alloc::vec::Vec<_> = "Symbol(".encode_utf16().collect();
        if let Some(description) = &self.description {
            units.extend_from_slice(description);
        }
        units.push(41);
        units.into()
    }
}
