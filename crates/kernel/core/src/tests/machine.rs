// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::machine`.

use audhsos_sync::{Preset, UncontendedToken};

use crate::machine::{MACHINE, Machine, with_machine};

#[test]
fn the_cell_holds_the_machine_from_the_start_and_one_borrow_at_a_time() {
    // One test uses the cell, because it is one `static` and the tests of
    // a crate share it.
    let counts = with_machine(|machine| {
        let inside = MACHINE.borrow(&UncontendedToken);
        assert!(
            inside.is_err(),
            "a second borrow while the first is alive is refused"
        );
        machine.objects.counts()
    });
    assert_eq!(counts, Some([0; 9]));
    assert!(!MACHINE.is_borrowed(), "the borrow was released");

    let idle = with_machine(|machine| machine.scheduler.idle());
    assert_eq!(idle, Some(None), "no idle thread until boot names one");
}

#[test]
fn a_machine_of_its_own_starts_empty() {
    // A second cell, to show that the constructor and not the one cell is
    // what makes a machine empty. It is a `static` because a machine is
    // over a mebibyte and never travels over a stack (D-66).
    static OWN: Preset<Machine> = Preset::new(Machine::new());
    let machine = OWN.borrow(&UncontendedToken).unwrap();
    assert_eq!(machine.objects.counts(), [0; 9]);
    assert!(machine.scheduler.is_idle());
    assert_eq!(machine.scheduler.current(), None);
}
