// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::sections`.

use crate::sections::{BSS_MIN, DATA_MAX, Sizes, violations};

#[test]
fn a_kernel_whose_pools_are_in_the_bss_passes() {
    let sizes = Sizes {
        data: 11_016,
        bss: 1_499_504,
    };
    assert!(violations(sizes).is_empty());
}

#[test]
fn a_pool_that_reached_the_data_is_refused() {
    // The state issue 73 reports: `.data` carries 1.37 MiB of pool bytes
    // the loader copies, and `.bss` holds what is left.
    let sizes = Sizes {
        data: 1_437_728,
        bss: 20_824,
    };
    let violations = violations(sizes);
    assert_eq!(violations.len(), 2);
    assert!(violations[0].contains(".data is 1437728 bytes"));
    assert!(violations[1].contains(".bss is 20824 bytes"));
}

#[test]
fn each_bound_is_refused_on_its_own() {
    let data = Sizes {
        data: DATA_MAX + 1,
        bss: BSS_MIN,
    };
    assert_eq!(violations(data).len(), 1);
    let bss = Sizes {
        data: DATA_MAX,
        bss: BSS_MIN - 1,
    };
    assert_eq!(violations(bss).len(), 1);
}

#[test]
fn a_missing_section_is_zero_bytes_and_refused() {
    let sizes = Sizes { data: 0, bss: 0 };
    assert_eq!(
        violations(sizes).len(),
        1,
        "only the `.bss` bound is broken"
    );
}
