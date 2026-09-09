// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::*;
fn kind() -> Rc<[u16]> {
    Rc::from([120])
}
#[test]
fn removal_once_reentry_and_new_registration_do_not_alias() {
    let mut ls = Listeners::new();
    let once = Options {
        once: true,
        ..Options::default()
    };
    assert_eq!(ls.add(kind(), 1, once, 4, 8), Ok(true));
    assert_eq!(ls.add(kind(), 1, Options::default(), 4, 8), Ok(false));
    assert_eq!(ls.add(kind(), 2, Options::default(), 4, 8), Ok(true));
    let ids = ls.snapshot(&kind(), false);
    assert_eq!(ids.len(), 2);
    assert!(ls.take_for_invoke(ids[0]).is_some());
    assert!(ls.take_for_invoke(ids[0]).is_none());
    assert!(ls.remove(&kind(), &2, false));
    assert!(!ls.remove(&kind(), &2, false));
    assert_eq!(ls.add(kind(), 2, Options::default(), 4, 8), Ok(true));
    assert!(ls.take_for_invoke(ids[1]).is_none());
    assert_eq!(ls.len(), 1);
    assert!(!ls.is_empty());
    assert_eq!(ls.iter().next().map(|l| l.callback), Some(2));
    assert!(ls.snapshot(&kind(), true).is_empty());
    assert!(ls.snapshot(&[121], false).is_empty());
    assert_eq!(ls.add(kind(), 3, Options::default(), 1, 8), Err(Limit));
    assert_eq!(ls.add(kind(), 3, Options::default(), 4, 0), Err(Limit));
    ls.next = u64::MAX;
    assert_eq!(ls.add(kind(), 3, Options::default(), 4, 8), Err(Limit));
    let empty: Listeners<u8> = Listeners::default();
    assert!(empty.is_empty());
    let mut ls = Listeners::new();
    let id = ls.next_id();
    assert_eq!(ls.add(kind(), 1, Options::default(), 4, 8), Ok(true));
    assert_eq!(ls.next_id(), id + 1);
    assert_eq!(ls.add(kind(), 1, Options::default(), 4, 8), Ok(false));
    assert_eq!(ls.next_id(), id + 1);
    assert!(ls.remove_id(id));
    assert!(!ls.remove_id(id));
    assert_eq!(ls.add(kind(), 1, Options::default(), 4, 8), Ok(true));
    assert!(!ls.remove_id(id));
    assert_eq!(ls.len(), 1);
}
#[test]
fn event_flags_reinitialize_and_cancel_correctly() {
    let mut e = Event::new(kind(), true, true, true);
    assert!(e.bubbles() && e.cancelable() && e.composed());
    e.passive(true);
    e.prevent_default();
    assert!(!e.canceled());
    e.passive(false);
    e.prevent_default();
    assert!(e.canceled());
    assert!(e.begin());
    assert!(!e.begin());
    e.stop(true);
    assert!(e.stopped() && e.immediate());
    e.initialize(Rc::from([121]), false, false);
    assert_eq!(e.event_type(), kind());
    e.finish();
    assert!(!e.stopped() && !e.immediate() && !e.dispatching() && e.canceled());
    e.initialize(Rc::from([121]), false, false);
    assert!(!e.bubbles() && !e.cancelable() && !e.canceled() && e.composed());
    e.stop(false);
    assert!(e.stopped() && !e.immediate());
    e.prevent_default();
    assert!(!e.canceled());
}
