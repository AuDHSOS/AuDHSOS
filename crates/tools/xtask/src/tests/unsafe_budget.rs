// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::unsafe_budget`.

use crate::policy::Kind;
use crate::unsafe_budget::is_function_pointer_type;
use crate::unsafe_budget::{Counts, count, strip_comments_and_strings, violations_for};

#[test]
fn unsafe_in_comments_and_strings_is_not_counted() {
    let source = "// unsafe here\n/* unsafe /* nested unsafe */ */\nlet s = \"unsafe { }\";\nlet r = r#\"unsafe\"#;\nlet c = 'u';\nfn safe<'unsafe_lt>() {}\n";
    assert_eq!(count(source), Counts::default());
}

#[test]
fn real_sites_are_counted_once_each() {
    let source = "unsafe fn f() {}\nunsafe impl Sync for T {}\nfn g() { unsafe { asm!(\"nop\"); } }\nfn h() { naked_asm!(\"ret\") }\nglobal_asm!(\".text\");\nlet unsafe_op_in_unsafe_fn = 1;\n";
    assert_eq!(
        count(source),
        Counts {
            unsafe_keywords: 3,
            asm_macros: 2,
            global_asm_macros: 1
        }
    );
}

#[test]
fn char_literals_and_lifetimes_are_told_apart() {
    let stripped = strip_comments_and_strings(
        "let q = '\"'; let l: &'a str = \"unsafe\"; let e = '\\''; unsafe {}",
    );
    assert!(stripped.contains("unsafe {}"));
    assert!(!stripped.contains("\"unsafe\""));
    assert_eq!(
        count("let x = '\\n'; let y = 'a'; struct S<'unsafe>;"),
        Counts::default()
    );
}

#[test]
fn unterminated_constructs_do_not_panic() {
    for source in ["\"open", "/* open", "r#\"open", "'", "'\\", "// end"] {
        let _ = count(source);
    }
}

#[test]
fn budgets_are_enforced_exactly() {
    let adapter = Kind::Adapter {
        unsafe_budget: 2,
        asm_budget: 1,
    };
    assert!(
        violations_for(
            "a",
            adapter,
            Counts {
                unsafe_keywords: 2,
                asm_macros: 1,
                global_asm_macros: 0
            }
        )
        .is_empty()
    );
    assert_eq!(
        violations_for(
            "a",
            adapter,
            Counts {
                unsafe_keywords: 3,
                asm_macros: 2,
                global_asm_macros: 1
            }
        )
        .len(),
        3
    );
    assert_eq!(
        violations_for(
            "l",
            Kind::Logic,
            Counts {
                unsafe_keywords: 1,
                asm_macros: 0,
                global_asm_macros: 0
            }
        )
        .len(),
        1
    );
    assert!(violations_for("h", Kind::Host, Counts::default()).is_empty());
}

#[test]
fn unsafe_function_pointer_types_are_not_sites_but_definitions_are() {
    let types = concat!(
        "pub type A = unsafe extern \"efiapi\" fn(x: u32) -> u32;\n",
        "pub type B = unsafe fn() -> ();\n",
        "pub struct S { pub f: unsafe extern \"C\" fn(*mut u8) }\n",
    );
    assert_eq!(count(types).unsafe_keywords, 0);

    let definitions = concat!(
        "pub unsafe fn one() {}\n",
        "pub unsafe extern \"C\" fn two() {}\n",
        "unsafe impl Send for S {}\n",
        "unsafe extern { fn three(); }\n",
    );
    assert_eq!(count(definitions).unsafe_keywords, 4);

    assert!(is_function_pointer_type(" extern      fn(u8)"));
    assert!(is_function_pointer_type(" fn()"));
    assert!(!is_function_pointer_type(" fn name()"));
    assert!(!is_function_pointer_type(" impl Send for S"));
    assert!(!is_function_pointer_type(" { let x = 1; }"));
    assert!(!is_function_pointer_type(" fnord()"));
}
