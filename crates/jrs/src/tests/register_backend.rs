// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Error, Limits, Realm, Runtime, SilentHost, Value, compile, compile_script};

fn same_value(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits()
        }
        _ => left == right,
    }
}

#[test]
fn primitive_expressions_run_through_register_bytecode_and_match_legacy() -> Result<(), Error> {
    for source in [
        "1 + 2 * 3",
        "(1 + 2) * 3",
        "20 / 2 / 2",
        "7 % 3",
        "0 / 0",
        "1 / -0",
        "- -2",
        "!0",
        "void 1",
        "~1",
        "1 < 2",
        "2 <= 2",
        "3 > 2",
        "3 >= 3",
        "true === true",
        "true === false",
        "null === null",
        "null === false",
        "'1'+2",
        "1+'2'",
        "true+2",
        "null+2",
        "undefined+2",
        "'x'+true",
        "'x'+null",
        "'x'+undefined",
        "'4'-2",
        "true*7",
        "'a'<'b'",
        "'a'<='a'",
        "'b'>'a'",
        "'b'>='b'",
        "'2'<10",
        "true>=1",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn primitive_unary_numeric_conversion_runs_through_register_bytecode() -> Result<(), Error> {
    for source in [
        "+true",
        "+false",
        "+null",
        "+undefined",
        "+''",
        "+'  42 '",
        "+'0x10'",
        "+'x'",
        "+'-0'",
        "-true",
        "-null",
        "-undefined",
        "-'2'",
        "~true",
        "~null",
        "~undefined",
        "~'2'",
        "function f(x){return +x}f('42')",
        "function f(x){return -x}f(true)",
        "function f(x){return ~x}f('2')",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn string_expressions_run_through_heap_independent_register_bytecode() -> Result<(), Error> {
    for source in [
        "'hello'",
        "'hello' + ' world'",
        "'Grüße' + ' 世界'",
        "'ab' === 'a' + 'b'",
        "'ab' !== 'a' + 'c'",
        "!''",
        "!'x'",
        "let x='a';x+='b';x",
        "let x='a';while(x==='a'){x+='b'}x",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn typeof_runs_through_register_bytecode_and_matches_legacy() -> Result<(), Error> {
    for source in [
        "typeof undefined",
        "typeof null",
        "typeof 1",
        "typeof -0",
        "typeof NaN",
        "typeof true",
        "typeof ''",
        "typeof 'hello world'",
        "typeof ({})",
        "typeof []",
        "typeof function(){}",
        "let x=1;typeof x",
        "let x='a';typeof (x+='b')",
        "let f=function(){};typeof f",
        "function f(a){return typeof a}f(null)",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert_eq!(actual, expected, "{source}");
    }

    let limits = Limits {
        string_units: 5,
        ..Limits::default()
    };
    let program = compile("typeof 1", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "string units"
        })
    );
    Ok(())
}

#[test]
fn string_constants_are_reusable_across_independent_agent_heaps() -> Result<(), Error> {
    let mut program = compile("'hello' + ' 世界'", Limits::default())?;
    assert!(program.uses_register_backend());
    program.code.clear();

    let expected = Value::string("hello 世界");
    assert_eq!(
        Runtime::new(Limits::default()).run(&program, &mut SilentHost)?,
        expected
    );
    assert_eq!(
        Runtime::new(Limits::default()).run(&program, &mut SilentHost)?,
        expected
    );
    Ok(())
}

#[test]
fn ordinary_named_properties_run_through_shapes_and_inline_caches() -> Result<(), Error> {
    for source in [
        "let o={x:42};o.x",
        "let o={x:1};o.x=42;o.x",
        "let o={x:1,x:2};o.x",
        "let o={true:'yes',null:'no'};o.true",
        "let o={x:'a'};o['x']='ab';o.x",
        "let o={x:1};o.x===1",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn array_literals_and_indices_run_through_dense_elements() -> Result<(), Error> {
    for source in [
        "[1,2,3][1]",
        "let a=[1,,3];a.length===3",
        "let a=[];a[2]=7;a.length===3",
        "let a=[1];a['0']",
        "let a=[];a[2147483648]=9;a[2147483648]",
        "let a=[];a[4294967294]=9;a.length",
        "let a=[1];a[-1]===undefined",
        "let a=[1,,3];a[1]===undefined",
        "let a=[2,3,5];let i=1;a[i]",
        "let a=[2,3,5];let i=-0;a[i]",
        "let a=[2,3,5];let i=-1;a[i]===undefined",
        "let a=[2,3,5];let i=1.5;a[i]===undefined",
        "let a=[2,3,5];let i=4294967295;a[i]===undefined",
        "let a=[2,3,5];let i='1';a[i]",
        "let a=[2,3,5];let i='length';a[i]",
        "let a=[2,3,5];let i='01';a[i]===undefined",
        "let a=[2,3,5];let i=true;a[i]===undefined",
        "let a=[2,3,5];let i=null;a[i]===undefined",
        "let a=[2,3,5];let i=9;a[i]+1",
        "let a=[2,3,5];let sum=0;for(let i=0;i<a.length;i++){sum+=a[i]}sum",
        "let a=[];for(let i=0;i<3;i++){a[i]=i+1}a[2]",
        "let a=[1,2];let i=0;while(i<2){a[i]=a[i]+1;i++}a[0]+a[1]",
        "let a=[];let i=-1;a[i]=7;a[i]",
        "let a=[];let i=4294967295;a[i]=9;a[i]",
        "let a=[];let i=4294967295;a[i]=9;a.length",
        "let a=[];a[4294967295]=9;a.length",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn numeric_array_indices_do_not_enter_the_property_name_pool() -> Result<(), Error> {
    let program = compile("let a=[1,2];a[1]", Limits::default())?;
    let code = program
        .register_code
        .as_ref()
        .ok_or(Error::InvalidBytecode)?;
    assert!(code.string_constants.is_empty());
    assert!(code.instructions.iter().any(|instruction| matches!(
        instruction,
        crate::engine::bytecode::Instruction::SetByValue { .. }
    )));
    assert!(code.instructions.iter().any(|instruction| matches!(
        instruction,
        crate::engine::bytecode::Instruction::GetByValue { .. }
    )));
    assert!(
        code.instructions.iter().any(|instruction| matches!(
            instruction,
            crate::engine::bytecode::Instruction::LdaSmi(1)
        ))
    );
    Ok(())
}

#[test]
fn non_indices_and_object_results_remain_on_the_full_property_path() -> Result<(), Error> {
    for source in ["({0:1})[0]", "let o={x:1};o.missing===undefined", "[1,2]"] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn register_array_elements_preserve_property_limits() -> Result<(), Error> {
    let limits = Limits {
        properties: 1,
        ..Limits::default()
    };
    assert!(!compile("[1,2][0]", limits)?.uses_register_backend());
    let program = compile("let a=[];a.length", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost)?,
        Value::Number(0.0)
    );
    assert!(
        !compile("let a=[];a[0]=1;0", limits)?.uses_register_backend(),
        "the compiler must retain the full property-limit semantics"
    );

    let limits = Limits {
        properties: 2,
        ..Limits::default()
    };
    let program = compile("let a=[];let i=0;while(i<2){a[i]=i;i++}0", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "object properties"
        })
    );
    Ok(())
}

#[test]
fn named_property_bytecode_is_reusable_across_independent_agent_heaps() -> Result<(), Error> {
    let mut program = compile("let o={answer:42};o.answer", Limits::default())?;
    assert!(program.uses_register_backend());
    program.code.clear();

    for _ in 0..2 {
        assert_eq!(
            Runtime::new(Limits::default()).run(&program, &mut SilentHost)?,
            Value::Number(42.0)
        );
    }
    Ok(())
}

#[test]
fn feedback_vectors_persist_per_code_identity_and_obey_the_agent_quota() -> Result<(), Error> {
    let limits = Limits {
        feedback_vectors: 2,
        ..Limits::default()
    };
    let first = compile("let o={x:1};o.x", limits)?;
    let second = compile("let o={y:2};o.y", limits)?;
    let third = compile("let o={z:3};o.z", limits)?;
    let mut runtime = Runtime::new(limits);

    assert_eq!(runtime.run(&first, &mut SilentHost)?, Value::Number(1.0));
    assert_eq!(runtime.run(&first, &mut SilentHost)?, Value::Number(1.0));
    assert_eq!(runtime.register_feedback_invocations(&first), Some(2));
    assert_eq!(runtime.run(&second, &mut SilentHost)?, Value::Number(2.0));
    assert_eq!(runtime.register_feedback_invocations(&first), Some(2));
    assert_eq!(runtime.register_feedback_invocations(&second), Some(1));
    assert_eq!(runtime.register_feedback_count(), 2);
    assert_eq!(
        runtime.run(&third, &mut SilentHost),
        Err(Error::Limit {
            resource: "feedback vectors"
        })
    );
    Ok(())
}

#[test]
fn object_completion_values_stay_on_the_legacy_backend_until_handles_are_public()
-> Result<(), Error> {
    for source in ["({x:1})", "let o={x:1};o", "true?({x:1}):({x:2})"] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    assert!(compile("let o={x:1};42", Limits::default())?.uses_register_backend());
    Ok(())
}

#[test]
fn register_string_concatenation_preserves_string_unit_limit() -> Result<(), Error> {
    let limits = Limits {
        string_units: 3,
        ..Limits::default()
    };
    let program = compile("'ab' + 'cd'", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "string units"
        })
    );
    Ok(())
}

#[test]
fn register_backend_is_selected_statically_without_runtime_fallback() -> Result<(), Error> {
    for source in [
        "typeof absent",
        "{let x=1;x+2}",
        "Number(1)",
        "({valueOf(){return 1}})+2",
        "+({valueOf(){return 1}})",
        "-function(){}",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }

    let mut program = compile("1+2", Limits::default())?;
    let register = alloc::rc::Rc::get_mut(
        program
            .register_code
            .as_mut()
            .ok_or(Error::InvalidBytecode)?,
    )
    .ok_or(Error::InvalidBytecode)?;
    register.instructions[0] = crate::engine::bytecode::Instruction::Ldar(
        crate::engine::bytecode::Reg(register.register_count),
    );
    assert_eq!(
        Runtime::new(Limits::default()).run(&program, &mut SilentHost),
        Err(Error::InvalidBytecode)
    );
    Ok(())
}

#[test]
fn simple_functions_use_contiguous_register_call_frames() -> Result<(), Error> {
    for source in [
        "(function(){return 42})()",
        "let f=function(){return 42};f()",
        "function f(){return 42}f()",
        "let x=f();function f(){return 42}x",
        "function f(){let x=40;return x+2}f()",
        "function f(){if(true)return 42;return 0}f()",
        "function f(){42}f()",
        "function f(){return}f()",
        "function f(){return 'a'+'b'}f()",
        "function f(a){return a}f(42)",
        "function f(a){return a}f()",
        "function f(a,b){return b}f(1,42)",
        "function f(a){return a}f('value')",
        "function f(a){return a}f(null)",
        "function f(a){return a}f(true)",
        "function f(a){return a}f(42,7)",
        "function f(a){if(a)return 1;return 2}f(true)",
        "function f(a){return a===42}f(42)",
        "function add(a,b){return a+b}add(20,22)",
        "function add(a,b){return a+b}add('a','b')",
        "function add(a,b){return a+b}add('a',2)",
        "function sub(a,b){return a-b}sub('44',2)",
        "function mul(a,b){return a*b}mul(true,42)",
        "function div(a,b){return a/b}div(null,0)",
        "function rem(a,b){return a%b}rem(7,3)",
        "function compare(a,b){return a<=b}compare('a','b')",
        "function compare(a,b){return a>b}compare('2',1)",
        "let f=function fact(x){return x<2?1:x*fact(x-1)};f(6)",
        "let f=function fib(x){return x<2?x:fib(x-1)+fib(x-2)};f(8)",
        "let f=function inner(){return inner===inner};f()",
        "function f(x){return x<2?1:x*f(x-1)}f(6)",
        "function f(x){return x<2?x:f(x-1)+f(x-2)}f(8)",
        "let x=f(6);function f(x){return x<2?1:x*f(x-1)}x",
        "function f(){return typeof f}f()",
        "function f(x){let a=[];return x<2?1:x*f(x-1)}let i=0;while(i<300){f(6);i++}f(6)",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn recursive_function_declarations_capture_the_hoisted_binding() -> Result<(), Error> {
    let program = compile(
        "function f(x){return x<2?1:x*f(x-1)}f(6)",
        Limits::default(),
    )?;
    let code = program
        .register_code
        .as_ref()
        .ok_or(Error::InvalidBytecode)?;
    assert_eq!(code.own_context_slot_count, Some(1));
    let function = code.functions.first().ok_or(Error::InvalidBytecode)?;
    assert_eq!(function.self_register, None);
    assert_eq!(function.outer_context_slot_counts, [1]);
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        crate::engine::bytecode::Instruction::LoadContext { depth: 0, slot: 0 }
    )));
    Ok(())
}

#[test]
fn closures_share_captured_context_bindings_in_register_bytecode() -> Result<(), Error> {
    for source in [
        "let x=40;let f=function(){return x+2};f()",
        "let x=1;let f=function(){return x};x=2;f()",
        "let x=42;function f(){return x}f()",
        "let x=undefined;function f(){return x}f()",
        "let x=NaN;function f(){return x}f()",
        "let x=Infinity;function f(){return x}f()",
        "let x=(1,42);function f(){return x}f()",
        "let y=1;let x=(y=42);function f(){return x}f()",
        "let x=!0;function f(){return x}f()",
        "let x=void 0;function f(){return x}f()",
        "let x=-1;function f(){return x}f()",
        "let x=+1;function f(){return x}f()",
        "let x=~1;function f(){return x}f()",
        "let x='a'+'b';function f(){return x}f()",
        "let x=40+2;function f(){return x}f()",
        "let x=44-2;function f(){return x}f()",
        "let x=21*2;function f(){return x}f()",
        "let x=84/2;function f(){return x}f()",
        "let x=86%44;function f(){return x}f()",
        "let x=1<2;function f(){return x}f()",
        "let x=1<=2;function f(){return x}f()",
        "let x=2>1;function f(){return x}f()",
        "let x=2>=1;function f(){return x}f()",
        "let x=1===1;function f(){return x}f()",
        "let x=1!==2;function f(){return x}f()",
        "let x=true?42:0;function f(){return x}f()",
        "let flag=true;let x=flag?42:0;function f(){return x}f()",
        "let x=40;let f=function(){x++;return x};f();f()",
        "let x='a';let f=function(){x+='b';return x};f();f()",
        "let x=1;let f=function(){return x};let g=function(){x++;return x};f()+g()+f()",
        "let x=1;let f=function(){x='a';return x};f();x+1",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let code = program
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        assert!(code.own_context_slot_count.is_some(), "{source}");
        assert!(code.functions.iter().any(|function| {
            !function.outer_context_slot_counts.is_empty()
                && function.instructions.iter().any(|instruction| {
                    matches!(
                        instruction,
                        crate::engine::bytecode::Instruction::LoadContext { .. }
                            | crate::engine::bytecode::Instruction::StoreContext { .. }
                    )
                })
        }));

        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn nested_functions_use_flat_code_and_lexical_context_tables() -> Result<(), Error> {
    for source in [
        "function f(){function g(){return 1}return g()}f()",
        "function f(){let x=40;function g(){return x+2}return g()}f()",
        "let x=40;function f(){let y=1;function g(){return x+y+1}return g()}f()",
        "function f(){let x=1;function g(){x++;return x}return g()+g()}f()",
        "function f(a){let x=40;if(a){x++}else{x+=2}function g(){return x}return g()}f(true)",
        "function f(){function g(){return 40}function h(){return g()+2}return h()}f()",
        "let x=39;function f(){let y=1;function g(){let z=1;function h(){return x+y+z+1}return h()}return g()}f()",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let code = program
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        assert!(code.functions.len() >= 2, "{source}");
        assert!(
            code.functions
                .iter()
                .all(|function| function.functions.is_empty())
        );

        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn returned_closures_outlive_register_frames_and_keep_distinct_contexts() -> Result<(), Error> {
    for source in [
        "function f(){function g(){return 42}return g}let g=f();g()",
        "function counter(){let n=0;function next(){n++;return n}return next}let c=counter();c();c()",
        "function counter(){let n=0;function next(){n++;return n}return next}let a=counter();let b=counter();a();a();b()",
        "function make(x){function get(){return x}return get}let a=make(20);let b=make(22);a()+b()",
        "function outer(x){function middle(){function inner(){return x}return inner}return middle}outer(42)()()",
        "function make(x){function get(){return x}return get}let keep=make(42);let i=0;while(i<3000){make(i);i++}keep()",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn register_function_calls_preserve_limits_and_reject_unlowered_semantics() -> Result<(), Error> {
    for source in [
        "function f(){return this}f()",
        "function f(){return arguments.length}f()",
        "function f(){return {x:1}}f()",
        "async function f(){return 1}f()",
        "function f(a,a){return a}f(1,2)",
        "function f(a={}){return a}f()",
        "function f(...a){return a.length}f(1)",
        "let f=function inner(){return inner===f};f()",
        "let f=function inner(inner){return inner};f(42)",
        "function outer(){function f(){return x}f();let x=1}outer()",
        "function f(){return f()}let g=f;f=0;g()",
        "function f(){f=0;return typeof f}f()",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }

    let limits = Limits {
        call_frames: 0,
        ..Limits::default()
    };
    let program = compile("function f(){return 42}f()", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "call frames"
        })
    );

    let limits = Limits {
        call_frames: 3,
        ..Limits::default()
    };
    let program = compile(
        "let f=function fact(x){return x<2?1:x*fact(x-1)};f(6)",
        limits,
    )?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "call frames"
        })
    );

    let limits = Limits {
        feedback_vectors: 1,
        ..Limits::default()
    };
    let program = compile("function f(){return 42}f()", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "feedback vectors"
        })
    );

    let limits = Limits {
        binding_slots: 1,
        ..Limits::default()
    };
    let program = compile("function f(a,b){return b}f(1,2)", limits)?;
    assert!(program.uses_register_backend());
    let mut runtime = Runtime::new(limits);
    assert_eq!(
        runtime.run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "binding slots"
        })
    );
    let recovery = compile("42", limits)?;
    assert_eq!(
        runtime.run(&recovery, &mut SilentHost)?,
        Value::Number(42.0)
    );

    let base = Limits::default();
    let program = compile("function f(){return 42}f()", base)?;
    let exact = u64::try_from(program.instruction_count()).unwrap();
    assert_eq!(
        Runtime::new(Limits {
            fuel: exact,
            ..base
        })
        .run(&program, &mut SilentHost)?,
        Value::Number(42.0)
    );
    assert_eq!(
        Runtime::new(Limits {
            fuel: exact.saturating_sub(1),
            ..base
        })
        .run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    );
    Ok(())
}

#[test]
fn register_backend_preserves_public_fuel_and_stack_limits() -> Result<(), Error> {
    let base = Limits::default();
    let program = compile("1+2", base)?;
    let fuel = u64::try_from(program.instruction_count()).unwrap();
    assert_eq!(
        Runtime::new(Limits {
            fuel,
            stack: 2,
            ..base
        })
        .run(&program, &mut SilentHost)?,
        Value::Number(3.0)
    );
    assert_eq!(
        Runtime::new(Limits {
            fuel: fuel.saturating_sub(1),
            stack: 2,
            ..base
        })
        .run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    );
    assert_eq!(
        Runtime::new(Limits { stack: 1, ..base }).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "operand stack"
        })
    );
    Ok(())
}

#[test]
fn realm_executes_register_backend_without_legacy_bytecode() -> Result<(), Error> {
    let limits = Limits::default();
    let mut script = compile_script("6*7", limits)?;
    assert!(script.program.uses_register_backend());
    script.program.code.clear();

    let mut host = SilentHost;
    let mut realm = Realm::new(limits, &mut host)?;
    assert_eq!(realm.evaluate_compiled(&script)?, Value::Number(42.0));
    assert_eq!(realm.evaluate_compiled(&script)?, Value::Number(42.0));
    Ok(())
}

#[test]
fn local_bindings_and_assignments_match_legacy_execution() -> Result<(), Error> {
    for source in [
        "let x=1;x+2",
        "const x=6;let y=7;x*y",
        "let x;x===undefined",
        "let x=1;x=2;x",
        "let x=1;x+=2;x*=3;x",
        "let x=1;x=2",
        "let x=1;x;2",
        "let x=1,y=x+1;y",
        "let x=1;(x=2,x+3)",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn local_binding_lowering_preserves_tdz_and_const_guards_by_staying_legacy() -> Result<(), Error> {
    for source in [
        "let x=x;x",
        "let x=y,y=1;x",
        "const x=1;x=2",
        "{let x=1;x}",
        "var x=1;x",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn conditional_expressions_match_legacy_execution() -> Result<(), Error> {
    for source in [
        "true ? 1 : 2",
        "false ? 1 : 2",
        "let x=1;(true ? (x=2) : (x=3));x",
        "let x=1;false?(x=2):(x=3);x",
        "let x=1;(x=2)?x+1:x+2",
        "true ? 1 : false",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn conditional_statements_match_legacy_execution() -> Result<(), Error> {
    for source in [
        "if(true) 3; else 4",
        "if(false) 3; else 4",
        "if(false) 3",
        "let x=1;if(true)x=2;else x=3;x",
        "let x=1;if(false)x=2;else x=3;x",
        "if(true) if(false) 3; else 4;",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn register_branch_lowering_rejects_incompatible_control_flow() -> Result<(), Error> {
    for source in [
        "let x=1;if(true)x=true;else x=2;x",
        "let x=1;if(true)x=true;x",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn blocks_updates_and_while_loops_match_legacy_execution() -> Result<(), Error> {
    for source in [
        "let x=1;{x++;x}",
        "let x=1;{x--;x}",
        "let x=1;let y=x++;x+y",
        "let x=1;let y=++x;x+y",
        "let i=0;while(i<10)i++;i",
        "let i=0;let sum=0;while(i<10){sum+=i;i++;}sum",
        "let i=0;while(i<3){i=i+1;i}",
        "let i=0;1;while(i<0){2}",
        "let x=1;{}",
        "1;{}",
        "let i=0;while(i++<2);",
        "let i=0;while(i<2){i++; ;}",
        "let i=0;while(i<3){if(i<2)i+=1;else i+=1;}i",
        "if(true){1}else{2}",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn register_while_checks_fuel_at_back_edges() -> Result<(), Error> {
    let limits = Limits {
        fuel: 100,
        ..Limits::default()
    };
    let program = compile("let i=0;while(true)i++", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    );
    let program = compile("while(true)continue", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::new(limits).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    );
    Ok(())
}

#[test]
fn register_loop_lowering_rejects_unstable_or_abrupt_bodies() -> Result<(), Error> {
    for source in [
        "let x=1;while(false)x=true;x",
        "let x=1;while(x=true)x",
        "while(false){let x=1;x}",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn classic_for_loops_match_legacy_execution() -> Result<(), Error> {
    for source in [
        "let sum=0;for(let i=0;i<10;i++){sum+=i;}sum",
        "let i=0;for(i=0;i<4;i++)i;i",
        "for(let i=0;i<3;i++)i",
        "1;for(let i=0;i<0;i++)2",
        "let i=0;for(;i<3;)i++;i",
        "let i=0;for(;;i++){if(i<2)i+=1;else i+=1}",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost);
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost);
        match (&actual, &expected) {
            (Ok(actual), Ok(expected)) => assert!(
                same_value(actual, expected),
                "{source}: {actual:?} != {expected:?}"
            ),
            _ => assert_eq!(actual, expected, "{source}"),
        }
    }
    Ok(())
}

#[test]
fn register_for_lowering_rejects_unstable_or_observable_lexical_cases() -> Result<(), Error> {
    for source in [
        "let i=1;for(let i=0;i<2;i++){}i",
        "for(const i=0;i<2;i++){}",
        "for(var i=0;i<2;i++){}i",
        "for(let i=0;i<2;i++){let x=i;}i",
        "for(let i=0;i<2;i++){(()=>i)}",
        "let x=1;while(true){x=true;break}x",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn register_loops_patch_break_and_continue_targets() -> Result<(), Error> {
    for source in [
        "let i=0;while(true){i++;if(i===3)break;}i",
        "let i=0;let sum=0;while(i<5){i++;if(i===3)continue;sum+=i;}sum",
        "let sum=0;for(let i=0;i<10;i++){if(i===4)break;sum+=i;}sum",
        "let sum=0;for(let i=0;i<5;i++){if(i===2)continue;sum+=i;}sum",
        "let i=0;while(i<3){while(true){i++;break}}i",
        "while(true)break",
        "let i=0;while(i++<3)continue;i",
        "while(true){1;break}",
        "let i=0;while(i++<2){7;continue}",
        "for(let i=0;i<1;i++){9;break}",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}
