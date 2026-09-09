// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Limits, Realm, SilentHost, Value};
fn check(source: &str) -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    Ok(())
}
#[test]
fn dynamic_parameters_bodies_global_environment_and_identity() -> Result<(), Error> {
    for source in [
        "Function()()===undefined&&new Function()()===undefined&&Function('return 7')()===7",
        "Function('a','b','return a+b')(2,3)===5&&Function('a,b','return a*b')(2,3)===6",
        "let f=Function('a=2','...rest','return a+rest.length');f.length===0&&f(undefined,1,2)===4",
        "let x=7;function f(){let x=9;return Function('return x')()}f()===7",
        "let f=Function('return later');let later=3;f()===3",
        "function f(){'use strict';return Function('return this')()}f()===globalThis&&Function('\"use strict\";return this')()===undefined",
        "let f=Function('a','a','return a');f(1,2)===2&&f.length===2",
        "let f=Function('a','arguments[0]=7;return a');f(1)===7",
        "let f=Function('a','\"use strict\";arguments[0]=7;return a');f(1)===1",
        "let anonymous=7;Function('return anonymous')()===7",
        "let yes=false;try{Function('return anonymous')()}catch(e){yes=e instanceof ReferenceError}yes",
        "Function('var privateName=7;return privateName')()===7&&typeof privateName==='undefined'",
        "let a=Function('return 1'),b=Function('return 1');a!==b&&a.prototype!==b.prototype&&a.prototype.constructor===a",
        "Function('a,// comment','b','return a')(4)===4",
        "Function('return 7 // comment')()===7",
        "Function('a /* comment */','return a')(7)===7",
    ] {
        check(source)?;
    }
    Ok(())
}
#[test]
fn dynamic_syntax_validates_fragments_and_cross_boundary_early_errors() -> Result<(), Error> {
    for (p, b) in [
        ("/*", "*/ ) {"),
        ("a)", "return 1"),
        ("a=", "return 1"),
        ("...a,b", ""),
        ("a,a", "'use strict';return a"),
        ("eval", "'use strict'"),
        ("arguments", "'use strict'"),
        ("a=1", "'use strict'"),
        ("a", "let a"),
        ("a", "const a=1"),
        ("", "}"),
        ("", "return super.x"),
        ("", "return super()"),
        ("", "/*"),
        ("#!param", "return 1"),
        ("", "#!body"),
    ] {
        let source = format!(
            "let yes=false;try{{Function({p:?},{b:?})}}catch(e){{yes=e instanceof SyntaxError}}yes"
        );
        check(&source)?;
    }
    check(
        "let n=0;let a={toString(){n++;return 'a)'}},b={toString(){n++;return 'return 1'}};try{Function(a,b)}catch(e){}n===2",
    )?;
    check(
        "let log='';try{Function({toString(){log+='p';throw 7}},{toString(){log+='b'}})}catch(e){}log==='p'",
    )?;
    check("let yes=false;try{Function(Symbol(),'')}catch(e){yes=e instanceof TypeError}yes")?;
    check("let yes=false;try{Function('',Symbol())}catch(e){yes=e instanceof TypeError}yes")?;
    Ok(())
}

#[test]
fn shared_restricted_accessors_and_lazy_function_properties() -> Result<(), Error> {
    for source in [
        "let a=Object.getOwnPropertyDescriptor(Function.prototype,'caller'),b=Object.getOwnPropertyDescriptor(Function.prototype,'arguments');a.get===a.set&&a.get===b.get&&a.get===b.set&&!a.enumerable&&a.configurable",
        "let t=Object.getOwnPropertyDescriptor(Function.prototype,'caller').get;!Object.isExtensible(t)&&t.name===''&&t.length===0&&!Object.getOwnPropertyDescriptor(t,'name').configurable&&!Object.getOwnPropertyDescriptor(t,'length').configurable",
        "function f(){'use strict';return Object.getOwnPropertyDescriptor(arguments,'callee').get}f()===Object.getOwnPropertyDescriptor(Function.prototype,'caller').get",
        "let f=Function('\"use strict\";');let ok=0;try{f.caller}catch(e){if(e instanceof TypeError)ok++}try{f.arguments=1}catch(e){if(e instanceof TypeError)ok++}ok===2&&!Object.hasOwn(f,'caller')",
        "function f(a){}delete f.length;let p=f.prototype;!Object.hasOwn(f,'length')",
    ] {
        check(source)?;
    }
    Ok(())
}

#[test]
fn dynamic_compilation_policy_order_and_quotas() -> Result<(), Error> {
    use crate::Host;
    struct Deny {
        prints: usize,
        checks: usize,
    }
    impl Host for Deny {
        fn print(&mut self, _: &[Value]) -> Result<(), Error> {
            self.prints += 1;
            Ok(())
        }
        fn ensure_can_compile_strings(&mut self) -> Result<(), Error> {
            self.checks += 1;
            Err(Error::Host)
        }
    }
    let mut host = Deny {
        prints: 0,
        checks: 0,
    };
    {
        let mut r = Realm::new(Limits::default(), &mut host)?;
        assert_eq!(r.evaluate("Function({toString(){print(1);return 'a)'}},{toString(){print(2);return 'invalid!'}})"),Err(Error::Host));
    }
    assert_eq!((host.prints, host.checks), (2, 1));
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            fuel: 2000,
            ..Limits::default()
        },
        &mut host,
    )?;
    assert!(matches!(
        r.evaluate("while(true){Function('return 1')}"),
        Err(Error::Limit { .. })
    ));
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert!(matches!(
        r.evaluate("let o={toString(){return Function(o)}};Function(o)"),
        Err(Error::Limit { .. })
    ));
    Ok(())
}

#[test]
fn fragment_boundaries_and_constructor_calls_cannot_publish_injected_code() -> Result<(), Error> {
    for source in [
        r"let n=0;try{Function('a) { n=7 } //','n=9')}catch(e){}n===0",
        r"let n=0;try{Function('a','};n=7;function x(){')}catch(e){}n===0",
        r"let n=0;try{Function('a/**/','/*')}catch(e){}n===0",
        r"Function('a,','return a')(7)===7",
        r"let f=Function('x=()=>new.target','return x()');new f()===f",
        r"let f=Function('a','return function(){return a}')(7);f()===7",
        r"let f=Function('a','return arguments.length');Reflect.apply(f,null,{length:3,0:1})===3",
        r"class F extends Function{}let f=Reflect.construct(Function,['return 7'],F);Object.getPrototypeOf(f)===F.prototype&&f()===7",
        r"let f=Function('return 7'),old=f.toString();delete f.name;delete f.length;f.toString()===old&&!Object.hasOwn(f,'length')",
    ] {
        check(source)?;
    }
    Ok(())
}

#[test]
fn isolated_runtime_dynamic_functions_have_standard_globals() {
    for source in [
        "Function('return Math.pow(2,3)')()===8",
        "Function('return Function(\"return 7\")()')()===7",
        "Function('return Reflect.apply(Number,null,[7])')()===7",
    ] {
        assert_eq!(super::eval(source), Ok(Value::Boolean(true)), "{source}");
    }
}
#[test]
fn dynamic_constructor_subclasses_new_target_and_prototypes() -> Result<(), Error> {
    for source in [
        "let C=Function('x','this.x=x;this.target=new.target');let c=new C(7);c.x===7&&c.target===C&&c instanceof C",
        "class F extends Function{}let f=new F('a','return a+1');f instanceof F&&f instanceof Function&&f(2)===3&&Object.getPrototypeOf(f)===F.prototype",
        "class F extends Function{constructor(){super('return 7');this.x=3}}let f=new F();f()===7&&f.x===3",
        "let f=Function('return ()=>new.target');new f()()===f&&f()()===undefined",
        "Function.length===1&&Function.name==='Function'&&Function.hasOwnProperty('prototype')",
        "let f=Function('return 7');f.name==='anonymous'&&Object.isExtensible(f)&&Object.getPrototypeOf(f)===Function.prototype&&f.length===0",
        "let f=Function('a,b=1','return a+b');if(f.length!==1)throw 7;delete f.length;let p=f.prototype;!Object.hasOwn(f,'length')&&f(1)===2",
        "let f=Function('return 7');delete f.name;if(f.name!=='')throw 7;Object.defineProperty(f,'name',{value:'other'});f.name==='other'&&f()===7",
        "Object.getOwnPropertyNames(Function('a','return a')).join()==='length,name,prototype'",
        "Function.prototype.name===''&&Function.prototype.length===0&&Function.prototype()===undefined&&!Object.hasOwn(Function.prototype,'prototype')",
    ] {
        check(source)?;
    }
    Ok(())
}
#[test]
fn dynamic_function_preserves_exact_synthesized_source() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert_eq!(
        r.evaluate("Function().toString()")?,
        Value::string("function anonymous(\n) {\n\n}")
    );
    assert_eq!(
        r.evaluate("Function('a','b','return a+b;').toString()")?,
        Value::string("function anonymous(a,b\n) {\nreturn a+b;\n}")
    );
    assert_eq!(
        r.evaluate("let f=Function('return 7');f.name='other';f.toString()")?,
        Value::string("function anonymous(\n) {\nreturn 7\n}")
    );
    Ok(())
}
#[test]
fn dynamic_function_gc_exceptions_and_resource_quotas() -> Result<(), Error> {
    for source in [
        "let f=Function({toString(){gc();return 'x'}},{toString(){gc();return 'return x.n'}});gc();f({n:7})===7",
        "let g=Function('let o={x:7};return ()=>o')();for(let i=0;i<200;i++){Function('return 3')}gc();g().x===7",
        "let err={};let f=Function('throw err');let yes=false;try{f()}catch(e){yes=e===err}yes",
        "let log='';let f=Function('try{throw 7}finally{log+=1}');try{f()}catch(e){log+=e}log==='17'",
    ] {
        let mut host = SilentHost;
        let mut r = Realm::new(
            Limits {
                heap_entries: 170,
                ..Limits::default()
            },
            &mut host,
        )?;
        let gc = r.gc_function()?;
        r.set_global("gc", &gc)?;
        assert_eq!(r.evaluate(source)?, Value::Boolean(true), "{source}");
    }
    let mut host = SilentHost;
    let mut r = Realm::new(
        Limits {
            source_bytes: 80,
            ..Limits::default()
        },
        &mut host,
    )?;
    let f = r.get_global("Function")?;
    assert!(matches!(
        r.call(&f, &Value::Undefined, &[Value::string(&" ".repeat(100))]),
        Err(Error::Limit { .. })
    ));
    let mut host = SilentHost;
    let mut r = Realm::new(Limits::default(), &mut host)?;
    assert!(matches!(
        r.evaluate(r"Function('\ud800')"),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}
