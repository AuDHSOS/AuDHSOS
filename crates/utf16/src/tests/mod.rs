// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::*;
#[test]
fn exhaustive_small_search_matches_naive_oracle() {
    for n in 0..7 {
        for mask in 0..1u32 << n {
            let text: Vec<u16> = (0..n).map(|i| u16::from(mask & (1 << i) != 0)).collect();
            for m in 0..5 {
                for pm in 0..1u32 << m {
                    let needle: Vec<u16> = (0..m).map(|i| u16::from(pm & (1 << i) != 0)).collect();
                    let p = Pattern::new(&needle, 100, &mut 1000).unwrap();
                    for start in 0..=text.len() + 1 {
                        let from = start.min(text.len());
                        let expected = (from..=text.len())
                            .find(|i| text.get(*i..i + needle.len()) == Some(needle.as_slice()));
                        assert_eq!(p.find(&text, start, &mut 1000).unwrap(), expected);
                        let expected = (0..=from)
                            .rev()
                            .find(|i| text.get(*i..i + needle.len()) == Some(needle.as_slice()));
                        assert_eq!(p.rfind(&text, start, &mut 1000).unwrap(), expected);
                    }
                }
            }
        }
    }
}
#[test]
fn work_storage_and_surrogate_boundaries() {
    assert!(Pattern::new(&[1], 0, &mut 10).is_err());
    assert!(Pattern::new(&[1], 1, &mut 0).is_err());
    let needle = [1, 1, 1, 2];
    let p = Pattern::new(&needle, 10, &mut 100).unwrap();
    assert!(p.find(&[1; 100], 0, &mut 1).is_err());
    let mut text = alloc::vec![1;10000];
    text.push(2);
    let mut work = 40000;
    assert_eq!(p.find(&text, 0, &mut work), Ok(Some(9997)));
    assert!(work > 0);
    assert_eq!(
        code_point_at(&[0xd83d, 0xde00], 0),
        Some((0x1f600, 2, false))
    );
    assert_eq!(code_point_at(&[0xd83d, 0xde00], 1), Some((0xde00, 1, true)));
    assert_eq!(code_point_at(&[0xd800, 65], 0), Some((0xd800, 1, true)));
    assert_eq!(code_point_at(&[65], 0), Some((65, 1, false)));
    assert_eq!(code_point_at(&[], 0), None);
    for u in 0..=u16::MAX {
        let (c, n, bad) = code_point_at(&[u], 0).unwrap();
        assert_eq!(c, u32::from(u));
        assert_eq!(n, 1);
        assert_eq!(bad, (0xd800..=0xdfff).contains(&u));
    }
}

#[test]
fn budgets_fail_during_preprocessing_fallback_and_reverse_scan() {
    assert!(Pattern::new(&[1, 1, 2], 3, &mut 3).is_err());
    let p = Pattern::new(&[1, 2], 2, &mut 100).unwrap();
    assert!(p.rfind(&[1, 1, 2], 3, &mut 0).is_err());
    assert!(p.find(&[1, 1, 2], 0, &mut 1).is_err());
    assert_eq!(p.find(&[1], 0, &mut 0), Ok(None));
    let p = Pattern::new(&[], 0, &mut 0).unwrap();
    assert_eq!(p.find(&[], 99, &mut 0), Ok(Some(0)));
}
