// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::{Error, Limits, Runtime, SilentHost, Value, compile, eval};

#[test]
fn bound_construction_ignores_this_and_preserves_argument_order() {
    for source in [
        "function C(a,b){this.sum=a+b;this.target=new.target}let ignored={},B=C.bind(ignored,2),v=new B(3);v.sum===5&&v.target===C&&v instanceof C&&v instanceof B&&ignored.sum===undefined",
        "function C(...args){this.args=args;this.target=new.target}let B=C.bind(null,1).bind({},2).bind({},3);let v=new B(4);v.args.join()==='1,2,3,4'&&v.target===C",
        "function C(){this.x=1}let B=C.bind(null);B.prototype={fake:true};let v=new B();Object.getPrototypeOf(v)===C.prototype&&!v.fake",
        "function C(){}let B=C.bind(null);Object.defineProperty(B,'prototype',{get(){throw 7}});new B() instanceof C",
        "let B=Array.bind(null,1,2);let a=new B(3);Array.isArray(a)&&a.join()==='1,2,3'&&a instanceof B",
        "let B=Function.bind(null,'x','return x+1');new B()(2)===3",
        "function C(){return {x:7}}let B=C.bind(null);new B().x===7",
        "let B=(function(){return 7}).bind(null);new B() instanceof B",
        "let B=Boolean.bind({x:1},1);new B().valueOf()===true",
        "let B=Error.bind(null,'msg');new B().message==='msg'",
        "function f(...a){return this.n+a.join()}let B=f.bind({n:7},1).bind({n:8},2);B(3)==='71,2,3'",
        "function C(a,b,c){this.sum=a+b+c}let B=C.bind(null,1);new B(...[2,3]).sum===6",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "new (()=>{}).bind(null)()",
        "Reflect.construct((()=>{}).bind(null),[])",
        "Reflect.construct(Function,[],(()=>{}).bind(null))",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn explicit_new_target_and_classes_preserve_bound_substitution() {
    for source in [
        "function C(){this.target=new.target}function D(){}let B=C.bind(null),v=Reflect.construct(B,[],D);v.target===D&&Object.getPrototypeOf(v)===D.prototype",
        "function C(){this.target=new.target}let B=C.bind(null),v=Reflect.construct(B,[],B);v.target===C&&Object.getPrototypeOf(v)===C.prototype",
        "function C(){this.target=new.target}let B=C.bind(null),D=B.bind(null),v=Reflect.construct(D,[],B);v.target===C",
        "function C(){this.target=new.target}let B=C.bind(null),D=B.bind(null);D.prototype={x:7};let v=Reflect.construct(B,[],D);v.target===D&&Object.getPrototypeOf(v)===D.prototype",
        "class C{constructor(x){this.x=x;this.target=new.target}}let B=C.bind(null,7);let v=new B();v.x===7&&v.target===C&&v instanceof C",
        "class A{constructor(a,b){this.sum=a+b}}let B=A.bind(null,3);B.prototype=A.prototype;class D extends B{constructor(){super(4)}}new D().sum===7&&new D() instanceof D",
        "function C(x){this.x=x;this.target=new.target}let B=C.bind(null,7);B.prototype=C.prototype;class D extends B{}let v=new D();v.x===7&&v.target===D&&v instanceof D",
        "function C(){}let B=C.bind(null);let yes=false;try{class D extends B{}}catch(e){yes=e instanceof TypeError}yes",
        "class A{}let B=A.bind(null);class D extends A{constructor(){return new B()}}new D() instanceof A",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}

#[test]
fn symbol_has_instance_hooks_and_ordinary_semantics() {
    for source in [
        "let seen=0,target={[Symbol.hasInstance](x){if(this===target&&x===7)seen++;return 'yes'}};7 instanceof target&&seen===1",
        "let seen=0,target={get [Symbol.hasInstance](){seen++;return function(x){return x===1}}};(1 instanceof target)&&!(2 instanceof target)&&seen===2",
        "let target={[Symbol.hasInstance](){return 0}};!({} instanceof target)",
        "let f=function(){};Object.defineProperty(f,Symbol.hasInstance,{value(){return true}});7 instanceof f",
        "let h=Function.prototype[Symbol.hasInstance];h.call({},7)===false&&h.call(null,{})===false&&h.call(7,{})===false",
        "let h=Function.prototype[Symbol.hasInstance],f=()=>{};h.call(f,7)===false",
        "let h=Function.prototype[Symbol.hasInstance];function C(){}Object.defineProperty(C,Symbol.hasInstance,{value(){throw 7}});h.call(C,new C())",
        "function C(){}let B=C.bind(null);Object.defineProperty(C,Symbol.hasInstance,{value(v){return v===7}});7 instanceof B&&Function.prototype[Symbol.hasInstance].call(B,7)",
        "function C(){}let B=C.bind(null);B.prototype={};Object.setPrototypeOf(B,null);new C() instanceof B",
        "function C(){}let B=C.bind(null);Object.defineProperty(B,Symbol.hasInstance,{value(){return false}});!(new C() instanceof B)",
        "let d=Object.getOwnPropertyDescriptor(Function.prototype,Symbol.hasInstance);!d.writable&&!d.enumerable&&!d.configurable&&d.value.name==='[Symbol.hasInstance]'&&d.value.length===1&&Object.isExtensible(d.value)",
        "let yes=false;try{({} instanceof {[Symbol.hasInstance]:1})}catch(e){yes=e instanceof TypeError}yes",
        "let yes=false;try{({} instanceof {[Symbol.hasInstance]:null})}catch(e){yes=e instanceof TypeError}yes",
        "let yes=false;try{({} instanceof (()=>{}))}catch(e){yes=e instanceof TypeError}yes",
        "function C(){}C.prototype=7;!(1 instanceof C)",
    ] {
        assert_eq!(eval(source), Ok(Value::Boolean(true)), "{source}");
    }
    for source in [
        "1 instanceof 1",
        "({}) instanceof null",
        "Function.prototype[Symbol.hasInstance].call(()=>{},{})",
        "function C(){}C.prototype=7;({}) instanceof C",
    ] {
        assert!(matches!(eval(source), Err(Error::Type { .. })), "{source}");
    }
}

#[test]
fn bind_captures_prototype_before_metadata_hooks_and_new_values_survive_gc() -> Result<(), Error> {
    for source in [
        "function C(){}let before={};Object.setPrototypeOf(C,before);Object.defineProperty(C,'length',{get(){Object.setPrototypeOf(C,{changed:true});return 1}});let B=Function.prototype.bind.call(C,null);Object.getPrototypeOf(B)===before",
        "function C(){}let log='';Object.defineProperty(C,'length',{get(){log+='l';return 2}});Object.defineProperty(C,'name',{get(){log+='n';return 'C'}});let B=C.bind(null,1);log==='ln'&&B.length===1&&B.name==='bound C'",
        "function C(x){this.x=x}let B=C.bind(null,{n:7});Object.defineProperty(C,'prototype',{value:{}});for(let i=0;i<300;i++){let x={}}new B().x.n===7",
        "function C(...args){for(let i=0;i<300;i++){let x={}}this.args=args}let B=C.bind(null,{n:1}).bind(null,{n:2});let v=new B({n:3});v.args[0].n+v.args[1].n+v.args[2].n===6",
        "let target={get [Symbol.hasInstance](){for(let i=0;i<300;i++){let x={}}return function(v){return v.n===7}}};({n:7}) instanceof target",
        "function C(){}let B=C.bind(null),value=new C();Object.defineProperty(C,Symbol.hasInstance,{get(){for(let i=0;i<300;i++){let x={}}return Function.prototype[Symbol.hasInstance]}});value instanceof B",
    ] {
        let limits = Limits {
            heap_entries: 180,
            ..Limits::default()
        };
        assert_eq!(
            Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn deep_bound_chains_use_vm_frames_not_rust_recursion() -> Result<(), Error> {
    for source in [
        "function C(){this.x=7}let B=C;for(let i=0;i<300;i++){B=B.bind(null)}new B().x===7&&new C() instanceof B",
        "function C(n){this.n=n;if(n>0)this.child=new B(n-1)}let B=C.bind(null);new B(100).n===100",
        "function C(...a){this.args=a}let B=C;for(let i=0;i<100;i++){B=B.bind(null,i)}let v=new B(100);v.args.length===101&&v.args[0]===0&&v.args[100]===100",
    ] {
        assert_eq!(eval(source)?, Value::Boolean(true), "{source}");
    }
    for (source, limits) in [
        (
            "let B=(function(){}).bind(null,...[1,2,3,4,5,6,7,8]);B(...[1,2,3,4,5,6,7,8])",
            Limits {
                stack: 16,
                ..Limits::default()
            },
        ),
        (
            "function C(){}let B=C;for(let i=0;i<20;i++){B=B.bind(null,i)}new B()",
            Limits {
                stack: 16,
                ..Limits::default()
            },
        ),
        (
            "let o={[Symbol.hasInstance](x){return x instanceof o}};1 instanceof o",
            Limits::default(),
        ),
    ] {
        assert!(
            matches!(
                Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost),
                Err(Error::Limit { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn bound_metadata_errors_and_prototype_callbacks_remain_observable() -> Result<(), Error> {
    for source in [
        "Function.prototype.bind.name==='bind'&&Function.prototype.bind.length===1&&Object.isExtensible(Function.prototype.bind)",
        "let original=Function.prototype.bind;Function.prototype.bind=function(){return 7};Number.bind()===7&&original.call(Number,null)(3)===3",
        "let b=(function f(){}).bind(null);b.toString()==='function () { [native code] }'&&!Object.hasOwn(b,'prototype')&&Object.getOwnPropertyNames(b).join()==='length,name'",
        "function C(){this.t=new.target}let B=C.bind(null),n=0;Object.defineProperty(B,'prototype',{get(){n++;return {x:7}}});let v=Reflect.construct(C,[],B);v.t===B&&v.x===7&&n===1",
        "function C(){}let B=C.bind(null),n=0;Object.defineProperty(B,'prototype',{get(){n++;throw 7}});let yes=false;try{Reflect.construct(C,[],B)}catch(e){yes=e===7}yes&&n===1",
        "let e={};function C(){throw e}let B=C.bind(null);let yes=false;try{new B()}catch(x){yes=x===e}yes",
        "let n=0;function C(){}let B=C.bind(null);Object.defineProperty(C,Symbol.hasInstance,{get(){n++;throw 7}});let yes=false;try{1 instanceof B}catch(e){yes=e===7}yes&&n===1",
        "function C(){}let B=C.bind(null);Object.defineProperty(C,Symbol.hasInstance,{value:null});new C() instanceof B",
        "let p={};function C(){}C.prototype=p;Object.defineProperty(C,Symbol.hasInstance,{value:Function.prototype[Symbol.hasInstance]});Object.create(p) instanceof C",
        "let f=()=>{};Object.defineProperty(f,'prototype',{get(){throw 7}});Function.prototype[Symbol.hasInstance].call(f,1)===false",
        "let n=0;function C(){}Object.defineProperty(C,'length',{get(){throw 7}});Object.defineProperty(C,'name',{get(){n++}});try{C.bind(null)}catch(e){}n===0",
    ] {
        assert_eq!(eval(source)?, Value::Boolean(true), "{source}");
    }
    let limits = Limits {
        heap_entries: 160,
        ..Limits::default()
    };
    let source = "function C(){}let B=C.bind(null);Object.defineProperty(B,'prototype',{get(){for(let i=0;i<300;i++){let x={}}return {n:7}}});Reflect.construct(C,[],B).n===7";
    assert_eq!(
        Runtime::new(limits).run(&compile(source, limits)?, &mut SilentHost)?,
        Value::Boolean(true)
    );
    Ok(())
}
