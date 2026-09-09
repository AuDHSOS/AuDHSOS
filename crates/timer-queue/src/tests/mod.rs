// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{DeadlineQueue, Error};
use std::{collections::BTreeMap, vec::Vec};

#[test]
fn stable_order_cancellation_quotas_and_token_exhaustion() {
    let mut queue = DeadlineQueue::new(3);
    let a = queue.insert(20, "a").unwrap();
    let b = queue.insert(10, "b").unwrap();
    let c = queue.insert(10, "c").unwrap();
    assert_eq!(queue.insert(0, "d"), Err(Error::Capacity));
    assert_eq!(queue.values().copied().collect::<Vec<_>>(), ["b", "c", "a"]);
    assert_eq!(queue.pop_due(9), None);
    assert_eq!(queue.remove(0), None);
    assert_eq!(queue.pop_due(10), Some((b, "b")));
    assert_eq!(queue.remove(b), None);
    assert_eq!(queue.remove(a), Some("a"));
    assert_eq!(queue.pop_due(u128::MAX), Some((c, "c")));
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
    assert_eq!(queue.pop_due(0), None);
    queue.next = u64::MAX - 1;
    let last = queue.insert(u128::MAX, "last").unwrap();
    assert_eq!(last, u64::MAX);
    assert_eq!(queue.insert(0, "no"), Err(Error::Tokens));
    assert_eq!(queue.pop_due(u128::MAX), Some((last, "last")));
    assert_eq!(DeadlineQueue::new(0).insert(0, ()), Err(Error::Capacity));
}

#[test]
fn generated_operations_match_sorted_model() {
    let mut queue = DeadlineQueue::new(30);
    let mut model = BTreeMap::new();
    let mut state = 17u64;
    let mut highest = 0;
    for _ in 0..20_000 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let time = u128::from((state >> 8) % 50);
        match state % 3 {
            0 => {
                let result = queue.insert(time, state);
                if model.len() == 30 {
                    assert_eq!(result, Err(Error::Capacity));
                } else {
                    let token = result.unwrap();
                    highest = token;
                    model.insert((time, token), state);
                }
            }
            1 => {
                let token = (state >> 16) % (highest + 1);
                let key = model.keys().copied().find(|(_, id)| *id == token);
                assert_eq!(queue.remove(token), key.and_then(|k| model.remove(&k)));
            }
            _ => {
                let expected = if model
                    .first_key_value()
                    .is_some_and(|((t, _), _)| *t <= time)
                {
                    model.pop_first().map(|((_, id), v)| (id, v))
                } else {
                    None
                };
                assert_eq!(queue.pop_due(time), expected);
            }
        }
        assert_eq!(queue.len(), model.len());
        assert_eq!(queue.is_empty(), model.is_empty());
        assert_eq!(
            queue.next_deadline(),
            model.first_key_value().map(|((t, _), _)| *t)
        );
        assert_eq!(
            queue.values().copied().collect::<Vec<_>>(),
            model.values().copied().collect::<Vec<_>>()
        );
    }
}
