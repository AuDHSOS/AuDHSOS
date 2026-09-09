// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Shared same-realm `ArraySpeciesCreate`. Cross-realm constructors and Proxy
//! `IsArray` remain unsupported along with those facilities elsewhere in the VM.
use super::{Error, Execution, Value, indexing::index_number};

impl Execution<'_> {
    pub(super) fn array_species_create(
        &mut self,
        receiver: &Value,
        length: u64,
    ) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        let result = (|| {
            if !self.is_array(receiver) {
                return self.array_create_wide(length);
            }
            let mut ctor = self.get(receiver, &Value::string("constructor").units())?;
            self.native_roots.push(ctor.clone());
            if matches!(ctor, Value::Object(_) | Value::Function(_)) {
                let key = Value::Symbol(self.well_known("species")?);
                ctor = self.get_key(&ctor, &key)?;
                if matches!(ctor, Value::Null) {
                    ctor = Value::Undefined;
                }
            }
            if matches!(ctor, Value::Undefined) {
                return self.array_create_wide(length);
            }
            self.construct_sync(ctor.clone(), &[Value::Number(index_number(length))], ctor)
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn array_create_wide(&mut self, length: u64) -> Result<Value, Error> {
        let length = u32::try_from(length).map_err(|_| Error::Range {
            message: "invalid array length",
        })?;
        self.new_array(length)
    }
}
