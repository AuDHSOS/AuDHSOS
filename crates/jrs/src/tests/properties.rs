// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::eval;
use crate::{Error, Limits, Value, compile};
use core::fmt::Write;
use test_support::{generators, property};

#[test]
fn property_loop_sum_agrees_with_integer_model() {
    property::check("jrs-loop-sum", &generators::range(0u32..=500), |count| {
        let source = format!("let s=0; for(let i=0;i<{count};i++){{s+=i;}} s");
        let expected = count.saturating_mul(count.saturating_sub(1)) / 2;
        if eval(&source) == Ok(Value::Number(f64::from(expected))) {
            Ok(())
        } else {
            Err(format!("sum disagrees for {count}"))
        }
    });
}

#[test]
fn property_utf16_escape_literals_preserve_every_code_unit() {
    property::check(
        "jrs-utf16-roundtrip",
        &generators::vec(generators::range(0u16..=u16::MAX), 0..=32),
        |units| {
            let mut source = String::from("'");
            for unit in units {
                write!(source, "\\u{unit:04x}").map_err(|e| e.to_string())?;
            }
            source.push('\'');
            if eval(&source) == Ok(Value::String(units.clone().into())) {
                Ok(())
            } else {
                Err("UTF-16 units changed".to_owned())
            }
        },
    );
}

#[test]
fn property_arbitrary_source_has_only_structured_results() {
    property::check(
        "jrs-arbitrary-source",
        &generators::bytes(0..=256),
        |bytes| {
            if let Ok(source) = core::str::from_utf8(bytes) {
                let result = compile(
                    source,
                    Limits {
                        fuel: 1000,
                        ..Limits::default()
                    },
                );
                if matches!(result, Err(Error::InvalidBytecode)) {
                    return Err("compiler violated an internal invariant".to_owned());
                }
            }
            Ok(())
        },
    );
}

#[test]
fn property_uint32_reduction_matches_integer_modulo() {
    property::check(
        "jrs-uint32-modulo",
        &generators::range(-9_007_199_254_740_991i64..=9_007_199_254_740_991),
        |integer| {
            let number: f64 = integer
                .to_string()
                .parse::<f64>()
                .map_err(|e| e.to_string())?;
            let expected =
                u32::try_from(integer.rem_euclid(4_294_967_296)).map_err(|e| e.to_string())?;
            if Value::Number(number).to_uint32() == expected {
                Ok(())
            } else {
                Err(format!("modulo conversion disagrees for {integer}"))
            }
        },
    );
}

#[test]
fn property_decimal_format_roundtrips_binary64() {
    property::check(
        "jrs-number-string-roundtrip",
        &generators::range(0u64..=u64::MAX),
        |bits| {
            let number = f64::from_bits(*bits);
            let text = Value::Number(number).to_string();
            let result = Value::string(&text).to_number();
            if (number.is_nan() && result.is_nan())
                || (number == 0.0 && result == 0.0)
                || number.to_bits() == result.to_bits()
            {
                Ok(())
            } else {
                Err(format!("binary64 {bits:x} did not roundtrip via {text}"))
            }
        },
    );
}

#[test]
fn property_array_length_and_elements_follow_a_vector_model() {
    property::check(
        "jrs-array-vector-model",
        &generators::vec(generators::range(0u32..=100), 0..=24),
        |values| {
            let body = values
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let source = format!("let a=[{body}];a.push(42);a.pop();a.join(',')");
            if eval(&source) == Ok(Value::string(&body)) {
                Ok(())
            } else {
                Err("push/pop changed elements".to_owned())
            }
        },
    );
}

#[test]
fn property_sparse_map_keeps_presence_and_maps_only_values() {
    property::check(
        "jrs-sparse-map",
        &generators::vec(generators::option(generators::range(0u32..=100)), 0..=16),
        |values| {
            let body = values
                .iter()
                .map(|v| v.map_or_else(String::new, |v| v.to_string()))
                .collect::<Vec<_>>()
                .join(",");
            let expected = values
                .iter()
                .map(|v| v.map_or_else(String::new, |v| v.saturating_add(1).to_string()))
                .collect::<Vec<_>>()
                .join(",");
            let ending = if values.last().is_some_and(Option::is_none) {
                ","
            } else {
                ""
            };
            let source = format!("[{body}{ending}].map(x=>x+1).join(',')");
            if eval(&source) == Ok(Value::string(&expected)) {
                Ok(())
            } else {
                Err(format!("sparse map differs for {source}"))
            }
        },
    );
}

#[test]
fn property_reverse_matches_sparse_vector_and_backward_search() {
    property::check(
        "jrs-sparse-reverse",
        &generators::vec(generators::option(generators::range(0u32..=5)), 0..=24),
        |values| {
            let literal = values
                .iter()
                .map(|v| v.map_or_else(String::new, |v| v.to_string()))
                .collect::<Vec<_>>()
                .join(",");
            let tail = if values.last().is_some_and(Option::is_none) {
                ","
            } else {
                ""
            };
            let mut checks = format!(
                "let a=[{literal}{tail}];a.reverse();if(a.length!=={})throw 7;",
                values.len()
            );
            for (i, v) in values.iter().rev().enumerate() {
                write!(checks, "if(({i} in a)!=={})throw 7;", v.is_some())
                    .map_err(|e| e.to_string())?;
                if let Some(v) = v {
                    write!(checks, "if(a[{i}]!=={v})throw 7;").map_err(|e| e.to_string())?;
                }
            }
            let last = values
                .iter()
                .rev()
                .rposition(|v| *v == Some(3))
                .map_or_else(|| "-1".into(), |i| i.to_string());
            write!(checks, "a.lastIndexOf(3)==={last}").map_err(|e| e.to_string())?;
            if eval(&checks) == Ok(Value::Boolean(true)) {
                Ok(())
            } else {
                Err(checks)
            }
        },
    );
}

#[test]
fn property_fill_and_copy_within_match_sparse_snapshot_model() {
    property::check(
        "jrs-fill-copy-sparse",
        &generators::vec(generators::option(generators::range(0u32..=7)), 0..=20),
        |values| {
            let length = values.len();
            let middle = length / 2;
            let literal = values
                .iter()
                .map(|v| v.map_or_else(String::new, |n| n.to_string()))
                .collect::<Vec<_>>()
                .join(",");
            let tail = if values.last().is_some_and(Option::is_none) {
                ","
            } else {
                ""
            };
            // Source is snapshotted in the oracle; the VM must choose direction
            // to produce the same overlapping copy without snapshotting JS values.
            for (target, start, end) in [
                (1usize.min(length), 0usize, length),
                (0, middle, length),
                (middle, 0, length),
                (length, 0, length),
            ] {
                let mut expected = values.clone();
                let count = end.saturating_sub(start).min(length.saturating_sub(target));
                expected[target..target + count].copy_from_slice(&values[start..start + count]);
                let mut source =
                    format!("let a=[{literal}{tail}];a.copyWithin({target},{start},{end});");
                write!(source, "if(a.length!=={length})throw 7;").map_err(|e| e.to_string())?;
                for (i, value) in expected.iter().enumerate() {
                    write!(source, "if(({i} in a)!=={})throw 7;", value.is_some())
                        .map_err(|e| e.to_string())?;
                    if let Some(n) = value {
                        write!(source, "if(a[{i}]!=={n})throw 7;").map_err(|e| e.to_string())?;
                    }
                }
                source.push_str("true");
                if eval(&source) != Ok(Value::Boolean(true)) {
                    return Err(source);
                }
            }
            let mut source = format!("let a=[{literal}{tail}];a.fill(9,{middle});");
            for (i, value) in values.iter().enumerate() {
                let expected = if i >= middle { Some(9) } else { *value };
                write!(source, "if(({i} in a)!=={})throw 7;", expected.is_some())
                    .map_err(|e| e.to_string())?;
                if let Some(n) = expected {
                    write!(source, "if(a[{i}]!=={n})throw 7;").map_err(|e| e.to_string())?;
                }
            }
            source.push_str("true");
            if eval(&source) == Ok(Value::Boolean(true)) {
                Ok(())
            } else {
                Err(source)
            }
        },
    );
}

#[test]
fn property_predicate_searches_match_sparse_vector_model() {
    property::check(
        "jrs-predicate-search-sparse",
        &generators::vec(generators::option(generators::range(0u32..=7)), 0..=20),
        |values| {
            let literal = values
                .iter()
                .map(|v| v.map_or_else(String::new, |n| n.to_string()))
                .collect::<Vec<_>>()
                .join(",");
            let tail = if values.last().is_some_and(Option::is_none) {
                ","
            } else {
                ""
            };
            let mut source = format!("let a=[{literal}{tail}];");
            for sought in [None, Some(0), Some(3), Some(8)] {
                let sought_text = sought.map_or_else(|| "undefined".into(), |n| n.to_string());
                for (method, index_method, found) in [
                    (
                        "find",
                        "findIndex",
                        values.iter().position(|v| *v == sought),
                    ),
                    (
                        "findLast",
                        "findLastIndex",
                        values.iter().rposition(|v| *v == sought),
                    ),
                ] {
                    let index = found.map_or_else(|| "-1".into(), |k| k.to_string());
                    let value = if found.is_some() {
                        &sought_text
                    } else {
                        "undefined"
                    };
                    write!(source, "if(a.{index_method}(v=>v==={sought_text})!=={index})throw 1;if(a.{method}(v=>v==={sought_text})!=={value})throw 2;")
                        .map_err(|e| e.to_string())?;
                }
            }
            let length = values.len();
            for (k, value) in values.iter().enumerate() {
                let expected = value.map_or_else(|| "undefined".into(), |n| n.to_string());
                let relative = length.saturating_sub(k);
                write!(source, "if(a.at({k})!=={expected}||a.at(-{relative})!=={expected})throw 3;if(({k} in a)!=={})throw 4;", value.is_some())
                    .map_err(|e| e.to_string())?;
            }
            source.push_str("true");
            if eval(&source) == Ok(Value::Boolean(true)) {
                Ok(())
            } else {
                Err(source)
            }
        },
    );
}

#[test]
fn property_splice_matches_sparse_vector_model() {
    property::check(
        "jrs-splice-sparse",
        &generators::vec(generators::option(generators::range(0u32..=7)), 0..=20),
        |values| {
            let length = values.len();
            let literal = values
                .iter()
                .map(|v| v.map_or_else(String::new, |n| n.to_string()))
                .collect::<Vec<_>>()
                .join(",");
            let tail = if values.last().is_some_and(Option::is_none) {
                ","
            } else {
                ""
            };
            for (start, count, items) in [
                (0usize, 0usize, vec![Some(8), Some(9)]),
                (
                    length / 2,
                    3usize.min(length.saturating_sub(length / 2)),
                    vec![],
                ),
                (1usize.min(length), length.saturating_sub(1), vec![Some(8)]),
                (
                    length / 2,
                    1usize.min(length.saturating_sub(length / 2)),
                    vec![Some(8), Some(9)],
                ),
                (length, 0, vec![Some(8)]),
                (0, length, vec![]),
            ] {
                let mut expected = values.clone();
                let deleted = expected
                    .splice(start..start + count, items.iter().copied())
                    .collect::<Vec<_>>();
                let mut arguments = String::new();
                for item in &items {
                    write!(arguments, ",{}", item.unwrap_or(0)).map_err(|e| e.to_string())?;
                }
                let mut source =
                    format!("let a=[{literal}{tail}];let d=a.splice({start},{count}{arguments});");
                for (name, model) in [("a", &expected), ("d", &deleted)] {
                    write!(source, "if({name}.length!=={})throw 1;", model.len())
                        .map_err(|e| e.to_string())?;
                    for (k, value) in model.iter().enumerate() {
                        write!(source, "if(({k} in {name})!=={})throw 2;", value.is_some())
                            .map_err(|e| e.to_string())?;
                        if let Some(value) = value {
                            write!(source, "if({name}[{k}]!=={value})throw 3;")
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
                source.push_str("true");
                if eval(&source) != Ok(Value::Boolean(true)) {
                    return Err(source);
                }
            }
            Ok(())
        },
    );
}

#[test]
fn property_sort_and_copying_sort_match_a_stable_sparse_model() {
    property::check(
        "jrs-sort-sparse-stable",
        &generators::vec(generators::option(generators::range(0u32..=5)), 0..=32),
        |values| {
            let literal = values
                .iter()
                .enumerate()
                .map(|(i, value)| value.map_or_else(String::new, |v| format!("{{k:{v},id:{i}}}")))
                .collect::<Vec<_>>()
                .join(",");
            let tail = if values.last().is_some_and(Option::is_none) {
                ","
            } else {
                ""
            };
            let mut expected = values
                .iter()
                .enumerate()
                .filter_map(|(i, v)| v.map(|k| (k, i)))
                .collect::<Vec<_>>();
            expected.sort_by_key(|(k, _)| *k);
            let mut source = format!(
                "let a=[{literal}{tail}],b=a.toSorted((x,y)=>x.k-y.k);a.sort((x,y)=>x.k-y.k);"
            );
            write!(
                source,
                "if(a.length!=={}||b.length!=={})throw 1;",
                values.len(),
                values.len()
            )
            .map_err(|e| e.to_string())?;
            for i in 0..values.len() {
                if let Some((k, id)) = expected.get(i) {
                    write!(
                        source,
                        "if(a[{i}].id!=={id}||b[{i}].id!=={id}||a[{i}].k!=={k})throw 2;"
                    )
                    .map_err(|e| e.to_string())?;
                } else {
                    write!(
                        source,
                        "if(({i} in a)||!Object.hasOwn(b,{i})||b[{i}]!==undefined)throw 3;"
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            source.push_str("true");
            if eval(&source) == Ok(Value::Boolean(true)) {
                Ok(())
            } else {
                Err(source)
            }
        },
    );
}

#[test]
fn property_copying_methods_match_sparse_vector_oracle() {
    property::check(
        "jrs-copying-array-model",
        &generators::vec(generators::option(generators::range(0u32..=7)), 0..=24),
        |values| {
            let size = values.len();
            let literal = values
                .iter()
                .map(|v| v.map_or_else(String::new, |n| n.to_string()))
                .collect::<Vec<_>>()
                .join(",");
            let tail = if values.last().is_some_and(Option::is_none) {
                ","
            } else {
                ""
            };
            let mut cases = vec![(
                "toReversed()".to_owned(),
                values.iter().copied().rev().collect::<Vec<_>>(),
            )];
            let middle = size / 2;
            if size > 0 {
                let mut replaced = values.clone();
                replaced[middle] = Some(9);
                cases.push((format!("with({middle},9)"), replaced));
            }
            for (start, skip, insert) in [
                (0, size, vec![]),
                (
                    middle,
                    size.saturating_sub(middle).min(2),
                    vec![Some(8), Some(9)],
                ),
                (size, 0, vec![Some(9)]),
                (0, 0, vec![]),
            ] {
                let mut model = values.clone();
                model.splice(start..start + skip, insert.iter().copied());
                let mut args = format!("{start},{skip}");
                for item in insert {
                    write!(args, ",{}", item.unwrap_or(0)).map_err(|e| e.to_string())?;
                }
                cases.push((format!("toSpliced({args})"), model));
            }
            for (method, expected) in cases {
                let mut source = format!(
                    "let a=[{literal}{tail}],b=a.{method};if(b===a||b.length!=={}||a.length!=={size})throw 1;",
                    expected.len()
                );
                for (index, value) in expected.iter().enumerate() {
                    let value = value.map_or_else(|| "undefined".to_owned(), |n| n.to_string());
                    write!(
                        source,
                        "if(!Object.hasOwn(b,{index})||b[{index}]!=={value})throw 2;"
                    )
                    .map_err(|e| e.to_string())?;
                }
                for (index, value) in values.iter().enumerate() {
                    write!(source, "if(({index} in a)!=={})throw 3;", value.is_some())
                        .map_err(|e| e.to_string())?;
                    if let Some(value) = value {
                        write!(source, "if(a[{index}]!=={value})throw 4;")
                            .map_err(|e| e.to_string())?;
                    }
                }
                source.push_str("true");
                if eval(&source) != Ok(Value::Boolean(true)) {
                    return Err(source);
                }
            }
            Ok(())
        },
    );
}

#[test]
fn property_flatten_matches_nested_sparse_model() {
    property::check(
        "jrs-flat-nested-sparse",
        &generators::vec(generators::option(generators::range(0u32..=7)), 0..=24),
        |values| {
            let literal = values
                .iter()
                .map(|v| v.map_or_else(String::new, |n| n.to_string()))
                .collect::<Vec<_>>()
                .join(",");
            let tail = if values.last().is_some_and(Option::is_none) {
                ","
            } else {
                ""
            };
            let numbers = values.iter().flatten().copied().collect::<Vec<_>>();
            let mut source = format!(
                "let sparse=[{literal}{tail}],a=[,sparse,[sparse],undefined];let flat=a.flat(Infinity),mapped=sparse.flatMap((v,k)=>[,v,k]);"
            );
            write!(
                source,
                "if(flat.length!=={})throw 1;",
                numbers.len() * 2 + 1
            )
            .map_err(|e| e.to_string())?;
            for (index, value) in numbers.iter().chain(&numbers).enumerate() {
                write!(
                    source,
                    "if(flat[{index}]!=={value}||!Object.hasOwn(flat,{index}))throw 2;"
                )
                .map_err(|e| e.to_string())?;
            }
            let last = numbers.len() * 2;
            write!(source,"if(flat[{last}]!==undefined||!Object.hasOwn(flat,{last}))throw 3;if(mapped.length!=={last})throw 4;").map_err(|e|e.to_string())?;
            let mut out = 0;
            for (index, value) in values.iter().enumerate() {
                if let Some(value) = value {
                    write!(
                        source,
                        "if(mapped[{out}]!=={value}||mapped[{}]!=={index})throw 5;",
                        out + 1
                    )
                    .map_err(|e| e.to_string())?;
                    out += 2;
                }
                write!(
                    source,
                    "if(({index} in sparse)!=={})throw 6;",
                    value.is_some()
                )
                .map_err(|e| e.to_string())?;
            }
            source.push_str("true");
            if eval(&source) == Ok(Value::Boolean(true)) {
                Ok(())
            } else {
                Err(source)
            }
        },
    );
}
