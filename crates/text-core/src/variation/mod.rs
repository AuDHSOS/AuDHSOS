// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Fixed-point font variation coordinates and delta stores.

mod axes;
mod gvar;
mod metrics;
pub use gvar::{Gvar, VariationPoint};
mod store;
pub use axes::{Axes, Axis, AxisValue};
pub use metrics::{Hvar, Mvar};
pub use store::{DeltaMap, ItemStore};
mod instance;
pub use instance::Instance;
