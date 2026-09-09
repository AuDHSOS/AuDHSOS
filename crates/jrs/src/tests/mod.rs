// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Error, Host, Limits, Runtime, SilentHost, Value, compile};

mod abort;
mod accessors;
mod array_constructor;
mod array_copying;
mod array_flatten;
mod array_from;
mod array_mutation;
mod array_search;
mod array_species;
mod arrays;
mod async_function;
mod bound_construct;
mod boxing;
mod builtins;
mod classes;
mod dynamic_function;
mod embedding;
mod eval_script;
mod events;
mod exceptions;
mod exponentiation;
mod function_source;
mod functions;
mod heap;
mod iterators;
mod json;
mod microtasks;
mod number_parsing;
mod objects;
mod promise_species;
mod promises;
mod properties;
mod realm;
mod reflect;
mod regexp;
mod regexp_dispatch;
mod regexp_intrinsics;
mod regexp_replace;
mod regexp_split;
mod reverse;
mod scripts;
mod sorting;
mod splice;
mod spread;
mod strings;
mod switch;
mod symbols;
mod templates;
mod timers;
mod weakmap;

fn eval(source: &str) -> Result<Value, Error> {
    let limits = Limits::default();
    Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)
}

#[test]
fn numeric_operators_precedence_and_coercion() {
    for (source, expected) in [
        ("1 + 2 * 3", 7.0),
        ("(1 + 2) * 3", 9.0),
        ("20 / 2 / 2", 5.0),
        ("-7 % 3", -1.0),
        ("7 % -3", 1.0),
        ("- -2", 2.0),
        ("+true + null", 1.0),
        ("'12' - '2'", 10.0),
        ("1e2 + .5", 100.5),
        ("0xFf + 0b10 + 0o10", 265.0),
        ("1_000 + 0xF_F", 1255.0),
        ("Number()", 0.0),
        ("Number(' 0x10 ')", 16.0),
        ("Number('\u{feff}2')", 2.0),
        ("let i = '4'; ++i", 5.0),
        ("let i = '4'; i++", 4.0),
        ("let x = 8; x /= 2; x *= 3; x -= 1; x %= 4; x", 3.0),
    ] {
        assert_eq!(eval(source), Ok(Value::Number(expected)), "{source}");
    }
}

#[test]
fn numbers_ieee_special_values_preserved() {
    for source in [
        "0 / 0",
        "undefined + 1",
        "+'inf'",
        "+'1_0'",
        "+'+0x10'",
        "+'\u{85}'",
        "Infinity % 2",
        "Number('0x')",
    ] {
        assert!(
            matches!(eval(source), Ok(Value::Number(v)) if v.is_nan()),
            "{source}"
        );
    }
    for (source, bits) in [
        ("-0", (-0.0f64).to_bits()),
        ("-0 % 2", (-0.0f64).to_bits()),
        ("1 / -0", f64::NEG_INFINITY.to_bits()),
        ("1 / 0", f64::INFINITY.to_bits()),
        ("1 / -Infinity", (-0.0f64).to_bits()),
    ] {
        assert!(
            matches!(eval(source), Ok(Value::Number(v)) if v.to_bits() == bits),
            "{source}"
        );
    }
}

#[test]
fn comparisons_primitive_conversion_and_nan() {
    for (source, expected) in [
        ("NaN === NaN", false),
        ("0 === -0", true),
        ("null == undefined", true),
        ("null === undefined", false),
        ("'2' == 2", true),
        ("'2' === 2", false),
        ("false == ''", true),
        ("true == 1", true),
        ("1 == true", true),
        ("false == null", false),
        ("NaN <= 1", false),
        ("NaN >= 1", false),
        ("2 > 1", true),
        ("1 < 2", true),
        ("2 >= 2", true),
        ("2 <= 2", true),
        ("1 != 2", true),
        ("1 !== '1'", true),
        ("'10' < '2'", true),
        ("'\u{10000}' < '\u{e000}'", true),
        ("isNaN('x')", true),
        ("isFinite('12')", true),
        ("isFinite(Infinity)", false),
        ("Boolean('')", false),
        ("Boolean('0')", true),
        ("!NaN", true),
        ("!undefined", true),
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(expected)), "{source}");
    }
}

#[test]
fn strings_utf16_surrogates_escapes_and_coercion() {
    for (source, expected) in [
        ("'hello ' + 'world'", "hello world"),
        ("'a' + 1 + true + null + undefined", "a1truenullundefined"),
        ("'\\x41\\u0042\\u{43}'", "ABC"),
        ("'a\\\r\nb'", "ab"),
        ("'\\n\\r\\t\\b\\f\\v\\0'", "\n\r\t\u{8}\u{c}\u{b}\0"),
        ("'\\q'", "q"),
        ("String()", ""),
        ("String(-0)", "0"),
        ("String(NaN)", "NaN"),
        ("String(Infinity)", "Infinity"),
        ("String(-Infinity)", "-Infinity"),
        ("typeof null", "object"),
        ("typeof absent", "undefined"),
        ("typeof 1", "number"),
        ("typeof true", "boolean"),
        ("typeof ''", "string"),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
    assert_eq!(
        eval("'\\ud800' + '\\udc00'"),
        Ok(Value::string("\u{10000}"))
    );
    assert_eq!(eval("'\\ud800'"), Ok(Value::String(vec![0xd800].into())));
    assert_eq!(eval("'\\u{d800}'"), Ok(Value::String(vec![0xd800].into())));
    assert_eq!(eval("'\\u{1f600}'"), Ok(Value::string("😀")));
}

#[test]
fn lexical_scopes_shadow_tdz_and_early_errors() {
    assert_eq!(
        eval("let x = 3; { let x = 4; x += 5; } x"),
        Ok(Value::Number(3.0))
    );
    assert_eq!(eval("let x; x"), Ok(Value::Undefined));
    for source in [
        "x; let x = 1",
        "let x = x",
        "let x = 1; { x; let x = 2; }",
        "typeof x; let x",
        "missing",
        "missing = 1",
        "missing++",
    ] {
        assert!(
            matches!(eval(source), Err(Error::Reference { .. })),
            "{source}"
        );
    }
    for source in ["const x = 1; x = 2", "const x = 1; x++"] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
    assert!(matches!(eval("let x; let x;"), Err(Error::Syntax { .. })));
}

#[test]
fn loops_control_flow_and_slot_reset() {
    for (source, expected) in [
        ("let sum=0; for(let i=0;i<100;i++){sum+=i;} sum", 4950.0),
        (
            "let i=0; let s=0; while(i<10){i++;if(i==3){continue;}if(i==5){break;}s+=i;}s",
            7.0,
        ),
        (
            "let s=0; for(let i=0;i<5;i++){if(i==2)continue;s+=i;}s",
            8.0,
        ),
        ("let s=0; for(;;){s++;if(s==3)break;}s", 3.0),
        ("let i=0; for(i=1;i<3;i++){} i", 3.0),
        (
            "let s=0; for(let i=0;i<3;i++){for(let j=0;j<3;j++){if(j==1)break;s++;}}s",
            3.0,
        ),
        (
            "let i=0; while(i<3){let x; if(x !== undefined)break; x=9;i++;}i",
            3.0,
        ),
        ("if (false) 3; else 4", 4.0),
        ("if(true) if(false) 3; else 4;", 4.0),
    ] {
        assert_eq!(eval(source), Ok(Value::Number(expected)), "{source}");
    }
    assert!(matches!(
        eval("for(let i=0;i<1;i++){} i"),
        Err(Error::Reference { .. })
    ));
    assert!(matches!(
        eval("let i=0; while(i<2){ if(i==1)x; let x=3; i++; }"),
        Err(Error::Reference { .. })
    ));
}

#[test]
fn logical_operators_return_values_and_skip_effects() {
    for (source, expected) in [
        ("0 || 4", Value::Number(4.0)),
        ("0 && missing", Value::Number(0.0)),
        ("true || missing", Value::Boolean(true)),
        ("null ?? 3", Value::Number(3.0)),
        ("0 ?? missing", Value::Number(0.0)),
        ("false ?? missing", Value::Boolean(false)),
        ("undefined ?? 4", Value::Number(4.0)),
        ("true ? 4 : missing", Value::Number(4.0)),
        ("false ? missing : 4", Value::Number(4.0)),
        ("let x=0; true || (x=4); x", Value::Number(0.0)),
        ("(null ?? false) || 4", Value::Number(4.0)),
        ("null ?? (false || 4)", Value::Number(4.0)),
        ("void 3", Value::Undefined),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn lexical_trivia_asi_and_empty_program() {
    for source in ["", ";;;", "// hi", "/* hi */", "#! /usr/bin/jrs\n", "{}"] {
        assert_eq!(eval(source), Ok(Value::Undefined));
    }
    for source in [
        "let x = 2\nx + 3",
        "let x=2/*\n*/x+3",
        "let x=2\u{2028}x+3",
        "\u{feff}2 + 3",
        "let x=2\n+3; x",
        "let x=2;{x+3}",
    ] {
        assert_eq!(eval(source), Ok(Value::Number(5.0)), "{source}");
    }
    assert_eq!(eval("let x=1; let y=2; x\n++y; y"), Ok(Value::Number(3.0)));
}

#[test]
fn malformed_source_is_rejected_before_execution() {
    for source in [
        "/*",
        "'",
        "'a\nb'",
        "'\\x0g'",
        "'\\u'",
        "'\\u{}'",
        "'\\u{110000}'",
        "'\\1'",
        "1_",
        "1__0",
        "0_1",
        "01",
        "0x",
        "0b2",
        "0o8",
        "1e",
        "1e+",
        "1foo",
        "1n",
        "let",
        "const x",
        "let true",
        "let if",
        "let let",
        "let x=;",
        "let x=1 let y=2",
        "(",
        "{",
        "}",
        "1=2",
        "1++",
        "break;",
        "continue;",
        "if(true)let x=1",
        "for(;;)const x=1",
        "for(let x=1\nx<2;x++){}",
        "true ?? false || 1",
        "false && null ?? 1",
        "class X{constructor(){} constructor(){}}",
        "let ä=1",
        "let \\u0061=1",
        "@",
        "return 1",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    let mut host = RecordingHost::default();
    let result = compile("print(1); let x; let x;", Limits::default());
    if let Ok(program) = &result {
        let _ = Runtime::new(Limits::default()).run(program, &mut host);
    }
    assert!(result.is_err());
    assert!(host.calls.is_empty());
}

#[derive(Default)]
struct RecordingHost {
    calls: Vec<Vec<Value>>,
}
impl Host for RecordingHost {
    fn print(&mut self, arguments: &[Value]) -> Result<(), Error> {
        self.calls.push(arguments.to_vec());
        Ok(())
    }
}

#[test]
fn host_arguments_order_and_runtime_reuse() -> Result<(), Error> {
    let limits = Limits::default();
    let program = compile("let x=0; print(x++, ++x, x); x", limits)?;
    let mut host = RecordingHost::default();
    let mut vm = Runtime::new(limits);
    assert_eq!(vm.run(&program, &mut host)?, Value::Number(2.0));
    assert_eq!(vm.run(&program, &mut host)?, Value::Number(2.0));
    assert_eq!(
        host.calls,
        vec![vec![Value::Number(0.0), Value::Number(2.0), Value::Number(2.0)]; 2]
    );
    assert!(program.instruction_count() > 0);
    assert_eq!(program.binding_count(), 1);
    Ok(())
}

#[test]
fn resource_limits_are_errors_and_vm_recovers() -> Result<(), Error> {
    let limits = Limits {
        fuel: 100,
        ..Limits::default()
    };
    let mut vm = Runtime::new(limits);
    assert!(matches!(
        vm.run(&compile("while(true){}", limits)?, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    ));
    assert_eq!(
        vm.run(&compile("42", limits)?, &mut SilentHost)?,
        Value::Number(42.0)
    );
    for (source, limited) in [
        (
            "123",
            Limits {
                source_bytes: 2,
                ..limits
            },
        ),
        (
            "1+2",
            Limits {
                tokens: 2,
                ..limits
            },
        ),
        (
            "((1))",
            Limits {
                nesting: 2,
                ..limits
            },
        ),
        (
            "1+2",
            Limits {
                instructions: 1,
                ..limits
            },
        ),
        (
            "'abc'",
            Limits {
                string_units: 2,
                ..limits
            },
        ),
    ] {
        assert!(
            matches!(compile(source, limited), Err(Error::Limit { .. })),
            "{source}"
        );
    }
    let program = compile("'a'+'b'+'c'", limits)?;
    assert!(matches!(
        Runtime::new(Limits {
            string_units: 2,
            ..limits
        })
        .run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    let program = compile("1+2", limits)?;
    assert!(matches!(
        Runtime::new(Limits { stack: 1, ..limits }).run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    assert!(matches!(
        Runtime::new(Limits {
            instructions: 1,
            ..limits
        })
        .run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn long_flat_and_deep_inputs_are_bounded() {
    for source in [
        format!("{}0{}", "(".repeat(1000), ")".repeat(1000)),
        format!("{}0", "1+".repeat(1000)),
        format!("{}0", "!".repeat(1000)),
    ] {
        assert!(matches!(
            compile(&source, Limits::default()),
            Err(Error::Limit { .. })
        ));
    }
}

#[test]
fn host_failure_propagates_without_poisoning_vm() -> Result<(), Error> {
    struct FailingHost;
    impl Host for FailingHost {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            Err(Error::Host)
        }
    }
    let limits = Limits::default();
    let program = compile("print(1)", limits)?;
    let mut vm = Runtime::new(limits);
    assert_eq!(vm.run(&program, &mut FailingHost), Err(Error::Host));
    assert_eq!(vm.run(&program, &mut SilentHost), Ok(Value::Undefined));
    Ok(())
}

#[test]
fn parentheses_preserve_references_for_assignment_update_and_typeof() {
    for (source, expected) in [
        ("let x=1; (x)=3; x", Value::Number(3.0)),
        ("let x=1; ++(x)", Value::Number(2.0)),
        ("let x=1; (x)++", Value::Number(1.0)),
        ("typeof ((absent))", Value::string("undefined")),
        ("(Number)('42')", Value::Number(42.0)),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    assert!(matches!(
        eval("typeof (x); let x"),
        Err(Error::Reference { .. })
    ));
    assert!(compile("let x=1; ++x++", Limits::default()).is_err());
}

#[test]
fn resource_limits_exact_boundaries_are_accepted() -> Result<(), Error> {
    let limits = Limits {
        source_bytes: 1,
        tokens: 2,
        nesting: 2,
        instructions: 2,
        fuel: 2,
        stack: 1,
        string_units: 1,
        heap_entries: 32,
        call_frames: 8,
        binding_slots: 32,
        properties: 32,
        jobs: 32,
        weak_entries: 32,
    };
    let program = compile("1", limits)?;
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost)?,
        Value::Number(1.0)
    );
    for limited in [
        Limits {
            source_bytes: 0,
            ..limits
        },
        Limits {
            tokens: 1,
            ..limits
        },
        Limits {
            nesting: 1,
            ..limits
        },
        Limits {
            instructions: 1,
            ..limits
        },
    ] {
        assert!(matches!(compile("1", limited), Err(Error::Limit { .. })));
    }
    for limited in [Limits { fuel: 1, ..limits }, Limits { stack: 0, ..limits }] {
        assert!(matches!(
            Runtime::new(limited).run(&program, &mut SilentHost),
            Err(Error::Limit { .. })
        ));
    }
    let program = compile("'a'", Limits::default())?;
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost)?,
        Value::string("a")
    );
    assert!(matches!(
        Runtime::new(Limits {
            string_units: 0,
            ..limits
        })
        .run(&program, &mut SilentHost),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn error_variants_render_useful_messages() {
    for error in [
        Error::Syntax {
            offset: 4,
            message: "bad token",
        },
        Error::Reference {
            name: "x".to_owned(),
        },
        Error::Type {
            message: "constant",
        },
        Error::Limit { resource: "fuel" },
        Error::InvalidBytecode,
        Error::Host,
    ] {
        assert!(!error.to_string().is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
}

#[test]
fn bitwise_conversion_shift_masks_and_precedence() {
    for (source, expected) in [
        ("~0", -1.0),
        ("~NaN", -1.0),
        ("Infinity | 0", 0.0),
        ("-Infinity | 0", 0.0),
        ("-0.9 | 0", 0.0),
        ("4294967297 | 0", 1.0),
        ("-4294967297 | 0", -1.0),
        ("2147483648 | 0", -2_147_483_648.0),
        ("-1 >>> 0", 4_294_967_295.0),
        ("1 << 31", -2_147_483_648.0),
        ("1 << 32", 1.0),
        ("1 << -1", -2_147_483_648.0),
        ("-8 >> 2", -2.0),
        ("-8 >>> 2", 1_073_741_822.0),
        ("1 << 2 + 1", 8.0),
        ("7 & 3 ^ 1 | 8", 10.0),
        ("4 | 1 == 1", 5.0),
        ("~1 + 3 * 4", 10.0),
        ("let x=7; x&=3; x|=8; x^=1; x<<=2; x>>=1; x>>>=1; x", 10.0),
        ("'3.9' & 7", 3.0),
        ("9007199254740991 >>> 0", 4_294_967_295.0),
        ("9007199254740992 >>> 0", 0.0),
        ("1e100 | 0", 0.0),
    ] {
        assert_eq!(eval(source), Ok(Value::Number(expected)), "{source}");
    }
}

#[test]
fn decimal_format_obeys_ecmascript_notation_boundaries() {
    for (source, expected) in [
        ("String(1e21)", "1e+21"),
        ("String(1e20)", "100000000000000000000"),
        ("String(1e-6)", "0.000001"),
        ("String(1e-7)", "1e-7"),
        ("String(-1.23e25)", "-1.23e+25"),
        ("String(5e-324)", "5e-324"),
        ("String(1.7976931348623157e308)", "1.7976931348623157e+308"),
        ("'x'+1e21", "x1e+21"),
        ("String(-2164670438985643.25)", "-2164670438985643.2"),
        ("String(2164670438985643.25)", "2164670438985643.2"),
    ] {
        assert_eq!(eval(source), Ok(Value::string(expected)), "{source}");
    }
}
