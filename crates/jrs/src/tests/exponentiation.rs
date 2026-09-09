// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Value, compile, eval};
#[test]
fn exponentiation_associativity_unary_and_compound_assignment() {
    for (source, expected) in [
        ("2**3**2", 512.0),
        ("(2**3)**2", 64.0),
        ("2*3**2+1", 19.0),
        ("2**-2", 0.25),
        ("(-2)**3", -8.0),
        ("let a=2;++a**2", 9.0),
        ("let a=2;a++**2", 4.0),
        ("let a=2;a**=3**2;a", 512.0),
        ("let a=[2];a[0]**=3;a[0]", 8.0),
        ("Math.pow(2,32)", 4_294_967_296.0),
        ("Math.pow('3',3)", 27.0),
        ("2**null", 1.0),
        ("true**false", 1.0),
    ] {
        assert_eq!(eval(source), Ok(Value::Number(expected)), "{source}");
    }
    for source in [
        "-2**2",
        "+2**2",
        "!2**2",
        "~2**2",
        "typeof 2**2",
        "void 2**2",
        "delete x**2",
        "2**",
        "**2",
        "async function f(){return await 2**2}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    for source in [
        "2**NaN",
        "NaN**1",
        "(-2)**0.5",
        "Math.pow()",
        "Math.pow(1,Infinity)",
    ] {
        assert!(
            matches!(eval(source),Ok(Value::Number(n)) if n.is_nan()),
            "{source}"
        );
    }
    for (source, bits) in [
        ("(-0)**3", (-0.0f64).to_bits()),
        ("(-0)**-3", f64::NEG_INFINITY.to_bits()),
        ("NaN**0", 1.0f64.to_bits()),
        ("(-Infinity)**-3", (-0.0f64).to_bits()),
    ] {
        assert!(
            matches!(eval(source),Ok(Value::Number(n)) if n.to_bits()==bits),
            "{source}"
        );
    }
}
#[test]
fn exponentiation_evaluates_operands_and_hooks_in_spec_order() {
    for source in [
        "let log='';function a(){log+='a';return {valueOf(){log+='x';return 2}}}function b(){log+='b';return {valueOf(){log+='y';return 3}}}a()**b()===8&&log==='abxy'",
        "let log='';let o={get x(){log+='g';return {valueOf(){log+='v';return 2}}},set x(v){log+='s'+v}};let k={toString(){log+='k';return 'x'}};o[k]**={valueOf(){log+='e';return 3}};log==='kgves8'",
        "let log='';let x={valueOf(){log+='a';return NaN}},y={valueOf(){log+='b';return 0}};Math.pow(x,y)===1&&log==='ab'",
        "let x={};let yes=false;try{Math.pow({valueOf(){throw x}},{valueOf(){throw 9}})}catch(e){yes=e===x}yes",
        "let yes=false;try{Math.pow(Symbol(),0)}catch(e){yes=e instanceof TypeError}yes",
        "let yes=false;try{Symbol()**0}catch(e){yes=e instanceof TypeError}yes",
        "let yes=false;try{2**Symbol()}catch(e){yes=e instanceof TypeError}yes",
        "async function f(){let n=2**(await 3);if(n!==8)throw 7}f();true",
        "class A{get x(){return 2}set x(v){this.n=v}}class B extends A{m(){super.x**=3}}let b=new B();b.m();b.n===8",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn numeric_left_conversion_finishes_before_right_hooks() {
    for operator in ["**", "-", "*", "/", "%", "&", "|", "^", "<<", ">>", ">>>"] {
        let source = format!(
            "let n=0;let yes=false;try{{({{valueOf(){{return Symbol()}}}}){operator}({{valueOf(){{n++;throw 7}}}})}}catch(e){{yes=e instanceof TypeError}}yes&&n===0"
        );
        assert_eq!(eval(&source), Ok(Value::Boolean(true)), "{operator}");
    }
}
#[test]
fn math_pow_intrinsic_is_mutable_and_nonconstructible() {
    for source in [
        "Math.pow.length===2&&Math.pow.name==='pow'&&Object.isExtensible(Math.pow)",
        "let d=Object.getOwnPropertyDescriptor(Math,'pow');d.writable&&!d.enumerable&&d.configurable",
        "Math.pow.prototype===undefined&&Object.getPrototypeOf(Math.pow)===Function.prototype",
        "let f=Math.pow;f.x=7;delete f.name;f.x===7&&f(2,3)===8",
        "Object.getOwnPropertyNames(Math.pow).join()==='length,name'",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    assert!(matches!(eval("new Math.pow(2,3)"), Err(Error::Type { .. })));
}

#[test]
fn comma_expressions_get_values_without_preserving_references() {
    for source in [
        "let log='';let o={get x(){log+='x';return 2}};(log+='a',o.x)**(log+='b',3)===8&&log==='axb'",
        "let o={m(){'use strict';return this}};(0,o.m)()===undefined&&o.m()===o",
        "let n=0;for(n=0,n++;n++,n<5;n++,n++){}n===5",
        "function f(){return 1,2}f()===2",
        "let x=0;if(x++,true)x++;x===2",
        "let a=[7];a[1,0]===7",
        "let a=1,b=2;a===1&&b===2",
        "function f(a,b){return a+b}f((1,2),3)===5",
        "let o={get x(){throw 7}};let n=0;try{(o.x,n++)}catch(e){}n===0",
        "let x=1;delete (0,x);x===1",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in ["(1,)", "(0,x)=2", "true?1,2:3", "[...1,2,] = 3"] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
}
