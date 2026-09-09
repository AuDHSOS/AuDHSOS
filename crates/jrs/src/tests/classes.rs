// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn classes_construct_inherit_methods_and_preserve_new_target() {
    for (source, expected) in [
        (
            "class A{constructor(x){this.x=x}m(){return this.x}static s(){return 7}}let a=new A(3);a.m()+A.s()",
            Value::Number(10.0),
        ),
        (
            "class A{constructor(x){this.x=x}}class B extends A{}new B(4).x",
            Value::Number(4.0),
        ),
        (
            "class A{m(){return this.x}}class B extends A{constructor(x){super();this.x=x}m(){return super.m()+1}}new B(4).m()",
            Value::Number(5.0),
        ),
        (
            "class A{static f(){return this.x}}class B extends A{static f(){return super.f()+1}}B.x=5;B.f()",
            Value::Number(6.0),
        ),
        (
            "class A{get x(){return this.n}set x(v){this.n=v}}class B extends A{f(){super.x=7;return super.x}}new B().f()",
            Value::Number(7.0),
        ),
        (
            "class A{constructor(){this.t=new.target}}class B extends A{}new B().t===B",
            Value::Boolean(true),
        ),
        ("function F(){return new.target}F()", Value::Undefined),
        (
            "class A{}let B=class Named{m(){return Named}};new B().m()===B",
            Value::Boolean(true),
        ),
        (
            "class A{}let a=new A();a instanceof A&&Object.getPrototypeOf(A)===Function.prototype",
            Value::Boolean(true),
        ),
        (
            "class A{}class B extends A{}new B() instanceof A&&new B() instanceof B",
            Value::Boolean(true),
        ),
        (
            "class A extends Array{}let a=new A(1,2);Array.isArray(a)&&a.length===2&&a instanceof A",
            Value::Boolean(true),
        ),
        (
            "class A extends Promise{}let p=new A(r=>r(7));p instanceof A&&p instanceof Promise",
            Value::Boolean(true),
        ),
        (
            "class A extends null{constructor(){return Object.create(null)}}Object.getPrototypeOf(new A())===null",
            Value::Boolean(true),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
}

#[test]
fn derived_this_tdz_arrows_and_constructor_return_rules() {
    for source in [
        "class A{}class B extends A{constructor(){let f=()=>this;super();this.f=f}}let b=new B();b.f()===b",
        "class A{}class B extends A{constructor(){return {x:7}}}new B().x===7",
        "class A{constructor(){return 4}}new A() instanceof A",
        "class A{constructor(){return {x:4}}}class B extends A{constructor(){super();this.y=5}}let b=new B();b.x+b.y===9",
        "class A{m(){return 7}}class B extends A{m(){return ()=>super.m()}}new B().m()()===7",
        "class A{m(){return this===undefined}}let m=A.prototype.m;m()",
        "class A{}class B extends A{constructor(){let f=()=>super();f();this.x=7}}new B().x===7",
        "class A{constructor(x=new.target){this.x=x}}class B extends A{}new B().x===B",
        "let p={};let o=Object.create(p);new Object(o)===o&&Object.getPrototypeOf(o)===p",
        "class A{get x(){return this.n}set x(v){this.n=v}}class B extends A{f(){this.n=3;let old=super.x++;return old===3&&this.n===4&&++super.x===5}}new B().f()",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "class A{}A()",
        "class A{}class B extends A{constructor(){this.x=1;super()}}new B()",
        "class A{}class B extends A{constructor(){}}new B()",
        "class A{}class B extends A{constructor(){super();super()}}new B()",
        "class A extends null{}new A()",
        "class A{}class B extends A{constructor(){return 1}}new B()",
        "class A extends 1{}",
    ] {
        assert!(eval(source).is_err(), "{source}");
    }
}

#[test]
fn class_early_errors_keys_and_tdz() {
    for source in [
        "super.x",
        "super()",
        "new.target",
        "class A{constructor(){super()}}",
        "class A{get constructor(){}}",
        "class A{static prototype(){}}",
        "class A{constructor(){}constructor(){}}",
        "class A{m(a,a){}}",
        "class A{m(){function f(){return super.x}}}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    for source in [
        "new A();class A{}",
        "class A extends A{}",
        "class A{[A](){}}",
    ] {
        assert!(
            matches!(eval(source), Err(Error::Reference { .. })),
            "{source}"
        );
    }
    for source in [
        "class A{m(){}static s(){}}Object.keys(A).length===0&&Object.keys(A.prototype).length===0",
        "class A{}let d=Object.getOwnPropertyDescriptor(A,'prototype');!d.writable&&!d.configurable&&!d.enumerable",
        "class A{m(){}get x(){return 1}}A.prototype.m.name==='m'&&Object.getOwnPropertyDescriptor(A.prototype,'x').get.name==='get x'",
        "let k=Symbol();class A{[k](){return 7}}new A()[k]()===7",
        "class A{static() {return 7}}new A().static()===7",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn class_home_objects_and_constructor_state_survive_gc() -> Result<(), Error> {
    let limits = Limits {
        heap_entries: 120,
        ..Limits::default()
    };
    let source = "class A{m(){return this.x}}class B extends A{constructor(){let f=()=>this;super();this.f=f;this.x=7}m(){return ()=>super.m()}}let b=new B();let f=b.m();for(let i=0;i<200;i++){let t={}}b.f()===b&&f()===7";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Boolean(true)
    );
    Ok(())
}

#[test]
fn class_contexts_computed_keys_and_super_lookup_follow_home_object() {
    for (source, expected) in [
        (
            "let trace='';class A{[(trace+='a')](){}static [(trace+='b')](){}}trace",
            Value::string("ab"),
        ),
        (
            "class A{}let C=A;class B{m(){return B}}let old=B;B=1;new old().m()===old",
            Value::Boolean(true),
        ),
        (
            "let o={__proto__:{m(){return this.x}},x:7,m(){return super.m()}};o.m()",
            Value::Number(7.0),
        ),
        (
            "class A{m(){return 1}}class B extends A{m(){return super.m()}}let f=B.prototype.m;Object.setPrototypeOf(B.prototype,{m(){return this.x}});f.call({x:9})",
            Value::Number(9.0),
        ),
        (
            "class A{set x(v){this.y=v}}class B extends A{m(){super.x=7}}let b=new B();b.m();b.y",
            Value::Number(7.0),
        ),
        (
            "class A{}A.prototype.x=2;class B extends A{m(){super.x+=3;return this.x}}new B().m()",
            Value::Number(5.0),
        ),
        (
            "let calls=0;class A{constructor(){calls++}}class B extends A{constructor(){super();try{super()}catch(e){}}}new B();calls",
            Value::Number(2.0),
        ),
        (
            "class A{constructor(){throw 7}}class B extends A{constructor(){try{super()}finally{}}}try{new B()}catch(e){e}",
            Value::Number(7.0),
        ),
        (
            "class A extends WeakMap{set(k,v){return super.set(k,v+1)}}let k={};new A([[k,7]]).get(k)",
            Value::Number(8.0),
        ),
        (
            "class A extends Number{}new A(7).valueOf()",
            Value::Number(7.0),
        ),
        (
            "class A extends TypeError{}let e=new A('bad');e instanceof A&&e instanceof Error&&e.message==='bad'",
            Value::Boolean(true),
        ),
        (
            "class A{m(){return new.target}}new A().m()",
            Value::Undefined,
        ),
        (
            "class A{}class B extends A{constructor(f=()=>this){super();this.f=f}}let b=new B();b.f()===b",
            Value::Boolean(true),
        ),
        (
            "class A{constructor(){this.x=1}}class B extends A{}Array.prototype[Symbol.iterator]=()=>{throw 9};new B().x",
            Value::Number(1.0),
        ),
    ] {
        assert_eq!(eval(source), Ok(expected), "{source}");
    }
    for source in [
        "if(true)class A{}",
        "class A{constructor=1}",
        "class A{#x}",
        "class A{static {}}",
        "class A{async constructor(){}}",
        "class A{set x(){}}",
    ] {
        assert!(compile(source, Limits::default()).is_err(), "{source}");
    }
    for source in [
        "class A{m(){}}new A.prototype.m()",
        "class A{}class B extends A{constructor(){let f=()=>this;f();super()}}new B()",
        "class A{}class B extends A{m(){super.x=1}}let b=new B();Object.freeze(b);b.m()",
        "class A{get x(){return 1}}class B extends A{m(){super.x=3}}new B().m()",
    ] {
        assert!(eval(source).is_err(), "{source}");
    }
}

#[test]
fn constructor_frames_are_explicit_and_limits_still_terminate() -> Result<(), Error> {
    assert_eq!(
        eval("class A{constructor(n){if(n>0)this.child=new A(n-1);this.n=n}}new A(100).n"),
        Ok(Value::Number(100.0))
    );
    let limits = Limits {
        call_frames: 10,
        ..Limits::default()
    };
    assert!(matches!(
        Runtime::new(limits).run(
            &compile("class A{constructor(){new A()}}new A()", limits)?,
            &mut SilentHost
        ),
        Err(Error::Limit { .. })
    ));
    let limits = Limits {
        heap_entries: 100,
        ..Limits::default()
    };
    let source = "class A{constructor(f){this.f=f}}class B extends A{constructor(){let f=()=>this;for(let i=0;i<200;i++){let t={}}super(f)}}let b=new B();b.f()===b";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Boolean(true)
    );
    Ok(())
}
