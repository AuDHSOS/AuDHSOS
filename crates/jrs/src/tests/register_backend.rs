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
        "true ? 1 : false",
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
    Ok(())
}

#[test]
fn register_loop_lowering_rejects_unstable_or_abrupt_bodies() -> Result<(), Error> {
    for source in [
        "let x=1;while(false)x=true;x",
        "let x=1;while(x=true)x",
        "while(true)break",
        "while(true)continue",
        "while(false){let x=1;x}",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}
