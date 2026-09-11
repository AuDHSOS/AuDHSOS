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
    for source in ["'1'+2", "typeof 1", "{let x=1;x+2}", "Number(1)"] {
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
