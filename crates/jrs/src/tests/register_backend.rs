// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{Backend, Error, Limits, Realm, Runtime, SilentHost, Value, compile, compile_script};

#[test]
fn a_realm_on_the_engine_backend_refuses_what_it_cannot_lower() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;

    // An expression Script lowers, so the engine evaluates it.
    assert_eq!(realm.evaluate("1+1")?, Value::Number(2.0));

    // The two paths hold separate object models, so a Script the lowering does
    // not take is refused instead of running on the stack path.
    let source = "{ let z = {} }";
    assert!(
        matches!(realm.evaluate(source), Err(Error::Unsupported { .. })),
        "{source}"
    );

    // The refusal is not a language error, so it leaves the realm usable.
    assert_eq!(realm.evaluate("2*3")?, Value::Number(6.0));

    // A name clause 19 gives every Realm but this one has not built is a gap,
    // never an answer. It shows only once the Script has run, which is fatal
    // like every other unsupported feature.
    assert!(matches!(
        realm.evaluate("typeof JSON"),
        Err(Error::Unsupported { .. })
    ));
    assert!(realm.evaluate("1").is_err());
    Ok(())
}

#[test]
fn an_operation_converts_an_object_operand_by_calling_its_methods() -> Result<(), Error> {
    for source in [
        // 7.1.1.1 asks valueOf first and toString after it.
        "let o={valueOf(){return 2}};let n=1;n+o",
        "let o={valueOf(){return 2}};let n=1;o+n",
        "let o={toString(){return 'x'}};let s='a';s+o",
        "let o={valueOf(){return 3}};let n=2;o*n",
        "let o={valueOf(){return 2}};let n=1;n<o",
        "({valueOf(){return 1}})+2",
        "({valueOf(){return 2}})**3",
        // An ordinary object reaches %Object.prototype%.toString.
        "let o={};let n=1;n+o",
        // A valueOf that answers an Object is not the answer, so toString is
        // asked next; when neither answers a primitive it is a TypeError.
        "let o={valueOf(){return {}},toString(){return 'T'}};1+o",
        "let o={valueOf(){return {}}};1+o",
        // Both operands are converted, in the order the operation reads them.
        "let a={valueOf(){return 1}};let b={valueOf(){return 2}};a+b",
        // A conversion that itself converts, and one that throws.
        "let i={valueOf(){return 5}};let o={valueOf(){return 1+i}};2+o",
        "let o={valueOf(){throw 7}};try{1+o}catch(e){e}",
        "let o={x:40,y:2},key=true?'x':'y';o[key]+2",
        // 23.1.3.37 answers the join of the receiver.
        "let a=[1];a.toString()",
        "[!{}]+'x'",
        "[1,2]+'x'",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_completion_of_an_unknown_type_lowers_and_answers_the_same() -> Result<(), Error> {
    // A completion whose type the lowering does not know is carried to the
    // boundary, which refuses only what it cannot represent.
    for source in [
        "let o={x:42},key='x';o[key]",
        "let k='x';let {[k]:x}={x:42};x",
        "let a=[2,3,5];let i='1';a[i]",
        "let a=[2,3,5];let i='length';a[i]",
        "function f(){return 1}try{f()}catch(e){e}",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn an_object_completion_of_the_engine_is_refused_at_the_boundary() -> Result<(), Error> {
    // An Object of the engine has no identity in this API. A completion that
    // is one is reported as the gap it is, never approximated.
    let program = compile("let x=1;if(true)x=function(){};x", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn a_realm_on_the_engine_backend_evaluates_and_refuses_without_poisoning() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;

    // A Script the lowering does not take is refused before anything runs, so
    // the realm stays usable.
    assert!(matches!(
        realm.evaluate("{ let z = {} }"),
        Err(Error::Unsupported { .. })
    ));
    assert_eq!(realm.evaluate("1+1")?, Value::Number(2.0));

    // 9.1.1.2.7 throws for a name nothing binds, and 13.5.3 answers undefined
    // for the same name under `typeof`.
    assert!(matches!(
        realm.evaluate("notDefined"),
        Err(Error::Reference { .. })
    ));
    assert_eq!(realm.evaluate("typeof absent")?, Value::string("undefined"));

    // A completion the boundary cannot carry is refused before it runs too,
    // because the lowering knows the type is an Array.
    assert!(matches!(
        realm.evaluate("[1,2]"),
        Err(Error::Unsupported { .. })
    ));
    assert_eq!(realm.evaluate("2*3")?, Value::Number(6.0));
    Ok(())
}

#[test]
fn a_var_of_a_realm_script_outlives_it() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut engine = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    let mut stack_host = SilentHost;
    let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;

    // 16.1.7 creates the binding on the Global Environment Record, so a later
    // Script of the same Realm reads and writes the same one.
    for realm in [&mut engine, &mut stack] {
        realm.evaluate("var x=1")?;
        assert_eq!(realm.evaluate("x")?, Value::Number(1.0));
        realm.evaluate("x=x+41")?;
        assert_eq!(realm.evaluate("x")?, Value::Number(42.0));
        // 9.1.1.4.16 leaves an existing binding alone.
        realm.evaluate("var x")?;
        assert_eq!(realm.evaluate("x")?, Value::Number(42.0));
        // The property it created is enumerable on the global object, and a
        // name nothing declared is still unresolvable.
        assert_eq!(realm.evaluate("typeof x")?, Value::string("number"));
        assert_eq!(realm.evaluate("typeof other")?, Value::string("undefined"));
        // 9.1.1.2.5 creates the property for an assignment that is not strict.
        realm.evaluate("created=7")?;
        assert_eq!(realm.evaluate("created")?, Value::Number(7.0));
    }
    Ok(())
}

#[test]
fn a_callee_the_lowering_cannot_type_is_dispatched_at_run_time() -> Result<(), Error> {
    // 7.3.14 dispatches on the callee, so a name resolved on the Global
    // Environment Record is still callable. A name of clause 19 this Realm has
    // not built is a gap, and it is reported as one rather than answered.
    let program = compile("Number(1)", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    assert_eq!(
        Runtime::new(Limits::default()).run(&program.legacy_only(), &mut SilentHost)?,
        Value::Number(1.0)
    );
    Ok(())
}

#[test]
fn a_catch_parameter_widens_when_the_range_can_throw_an_error_object() -> Result<(), Error> {
    // Only `throw` carries a value the lowering saw. Every other instruction
    // that throws raises an error object of the Realm, whose type it does not
    // know, so the parameter is a value the lowering cannot name and `|`
    // reaches the conversion that names the gap (7.1.6 over 7.1.4).
    let program = compile("try{f}catch(e){e|5}", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    // A member of a value that is not an object is not lowered at all, so
    // these never reach the conversion.
    for source in [
        "try{null.x}catch(e){e|5}",
        "let o={};try{o.x.y}catch(e){e|5}",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    // A range that can only throw what it was given keeps the typed parameter.
    differential("let s=0;for(let i=0;i<3;i++){try{if(i===1)throw i;s+=10}catch(e){s+=e}}s")?;
    Ok(())
}

#[test]
fn a_global_function_is_callable_in_the_script_that_declared_it() -> Result<(), Error> {
    // 16.1.7 binds the function on the Global Environment Record, and 7.3.14
    // dispatches on it at the call.
    for source in [
        "function f(){return 1}f()",
        "function f(){return 1}function f(){return 2}f()",
        "var x=2;function f(){return x}f()",
        "function f(a,b){return a+b}f(20,22)",
    ] {
        differential(source)?;
    }

    Ok(())
}

#[test]
fn a_global_function_outlives_the_script_that_declared_it() -> Result<(), Error> {
    // The Realm holds the code of every Script it has run, and a function
    // object names its unit, so the call of the next Script resolves in the
    // table the function was compiled into.
    for scripts in [
        &["function f(a){return a+1}", "f(41)"][..],
        // A function of the third Script calls one of the first.
        &[
            "function f(a){return a+1}",
            "function g(){return f(1)+1}",
            "g()",
        ][..],
        // A call of another unit unwinds to a handler of this one.
        &["function f(){throw 7}", "try{f()}catch(e){e+1}"][..],
        // Recursion across two units.
        &[
            "function down(n){return n===0?0:up(n-1)+1}",
            "function up(n){return down(n)}down(6)",
        ][..],
        // A conversion of an operand calls a method of another unit.
        &[
            "function two(){return 2}",
            "var o={valueOf(){return two()}};1+o",
        ][..],
        // A call of another unit in the body of a loop.
        &[
            "function inc(x){return x+1}",
            "function sum(n){let s=0;for(let i=0;i<n;i++){s=inc(s)}return s}sum(20)",
        ][..],
        // Enough allocation in a frame of another unit for the Nursery to fill.
        &[
            "function rep(n){let s=\"\";let i=0;while(i<n){s=s+\"x\";i=i+1}return s}",
            "rep(400)+\"!\"",
        ][..],
    ] {
        differential_scripts(scripts)?;
    }
    Ok(())
}

/// Runs the Scripts in order in one Realm on each backend and compares the
/// completion of the last one.
fn differential_scripts(scripts: &[&str]) -> Result<(), Error> {
    let outcome = |backend| -> Result<Result<Value, Error>, Error> {
        let mut host = SilentHost;
        let mut realm = Realm::with_backend(Limits::default(), &mut host, backend)?;
        let mut last = Ok(Value::Undefined);
        for source in scripts {
            last = realm.evaluate(source);
            if last.is_err() {
                break;
            }
        }
        Ok(last)
    };
    let actual = outcome(Backend::Engine)?;
    let expected = outcome(Backend::Stack)?;
    match (&actual, &expected) {
        (Ok(actual), Ok(expected))
        | (Err(Error::Thrown { value: actual }), Err(Error::Thrown { value: expected })) => {
            assert!(
                same_value(actual, expected),
                "{scripts:?}: {actual:?} != {expected:?}"
            );
        }
        _ => panic!("{scripts:?}: {actual:?} != {expected:?}"),
    }
    Ok(())
}

#[test]
fn a_call_the_lowering_could_not_type_is_returnable() -> Result<(), Error> {
    // `Return` carries the accumulator whatever it holds, so a call whose
    // result the lowering could not name leaves a function like any other
    // value. The call site receives it as the same unknown a call it could not
    // name already produces.
    for source in [
        "function id(x){return x}id(41)+1",
        "function id(x){return x}id('a')+'b'",
        "function f(key){let o={[key]:42};return o[key]}f('answer')",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn arguments_inside_a_function_is_never_a_global() -> Result<(), Error> {
    // 10.4.4 binds `arguments` in every ordinary function. Resolving it on the
    // Global Environment Record answered a ReferenceError where the stack
    // backend answers the arguments object.
    differential_scripts(&["function f(){return arguments.length}f()"])?;
    differential_scripts(&["function f(a){return arguments.length}f(1,2,3)"])?;
    differential_scripts(&["function f(a){return arguments[1]}f(1,2)"])?;
    // The object is only read as the base of a property access, because the
    // mapping of 10.4.4.7 is not built.
    for source in [
        "function f(){return typeof arguments}f()",
        "function f(){arguments=1;return 2}f()",
        "function f(a){arguments[0]=2;return a}f(1)",
        "function f(a){a=2;return arguments[0]}f(1)",
        "function g(x){return x}function f(a){return g(arguments)}f(1)",
        "function f(a){return arguments}f(1)",
        "function f(a){return (()=>arguments.length)()}f(1)",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }

    // A binding of that name is an ordinary binding, inside a function and out.
    for source in [
        "var arguments=1;typeof arguments",
        "function f(){var arguments=2;return arguments}f()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn this_is_the_receiver_of_the_call() -> Result<(), Error> {
    // 10.2.1.2 binds `this` to the receiver, which 13.3.6.1 takes from the
    // base of a method call.
    for source in [
        "let o={g:function(){return 1}};o.g()",
        "let o={a:1,g:function(){return this.a}};o.g()",
        "let o={a:'x',g:function(){return this.a+'y'}};o.g()",
        "let o={a:2,g:function(n){return this.a*n}};o.g(21)",
        "let o={g:function(){return this}};typeof o.g()",
        // The same function answers the receiver it was called on.
        "let o={a:1,g:function(){return this.a}};let p={a:2,g:o.g};p.g()",
        // A name the receiver does not have is undefined, as on any object.
        "let o={g:function(){return this.nope}};typeof o.g()",
    ] {
        differential(source)?;
    }

    // 10.2.1.2 gives a call without a receiver the global object when the
    // function is not strict, and leaves it undefined when it is.
    for source in [
        "function f(){return typeof this}f()",
        "function f(){'use strict';return typeof this}f()",
        "function f(){return typeof this.nope}f()",
        // A primitive receiver of a non-strict function is boxed by ToObject.
        "let o={g:function(){return typeof this}};o.g()",
    ] {
        differential(source)?;
    }

    // An arrow function has no `this` of its own (10.2.1.1).
    assert!(!compile("let o={g:()=>this};o.g()", Limits::default())?.uses_register_backend());
    Ok(())
}

#[test]
fn instanceof_walks_the_prototype_chain_of_the_value() -> Result<(), Error> {
    // 13.10.2 reaches 7.3.22 because no object of this Realm carries an
    // `@@hasInstance`, and 7.3.22 looks for the constructor's `prototype` on
    // the chain of the value.
    for source in [
        "function F(){}var f=new F();f instanceof F",
        "function F(){}function G(){}var f=new F();f instanceof G",
        "function F(){}1 instanceof F",
        "function F(){}'a' instanceof F",
        "function F(){}var f=new F();typeof (f instanceof F)",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_value_the_embedding_cannot_hold_leaves_the_realm_usable() -> Result<(), Error> {
    // The Script reaches a defined end and only its value cannot cross, so this
    // is the one unsupported feature that does not poison the Realm.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    for source in ["({x:1})", "[1,2]", "throw {}"] {
        assert!(
            matches!(realm.evaluate(source), Err(Error::Unsupported { .. })),
            "{source}"
        );
        assert_eq!(realm.evaluate("2*3")?, Value::Number(6.0), "{source}");
    }
    Ok(())
}

#[test]
fn a_property_of_a_primitive_names_the_object_it_would_need() -> Result<(), Error> {
    // 7.3.2 sends the base of a property access through ToObject, which 7.1.18
    // refuses for undefined and null alone. Every other primitive gets a
    // wrapper Object this engine has not built, so the access names that
    // instead of the TypeError only the first two deserve.
    for source in [
        "[x=>h=>x,3,3,22,5][-3,3,2][-2]+1",
        "let f=function(o){return o.x};f(3)",
        "let f=function(o){return o[0]};f(true)",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    // 10.4.3 answers a String without producing the Object, and undefined and
    // null keep the TypeError.
    differential("let f=function(o){return o.length};f('ab')")?;
    differential("let f=function(o){return o[0]};f('ab')")?;
    for source in [
        "let f=function(o){return o.x};f(null)",
        "let f=function(o){return o.x};f(undefined)",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let expected = Runtime::new(Limits::default()).run(&program.legacy_only(), &mut SilentHost);
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost);
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"), "{source}");
    }
    Ok(())
}

#[test]
fn an_error_constructor_makes_what_the_engine_throws() -> Result<(), Error> {
    // 20.5.1.1 and 20.5.6.1.1 make an error under the Prototype of the
    // constructor that was called, with an own `message` when one was passed,
    // and 20.5.1.2 and 20.5.6.2 tie each constructor to its prototype.
    for source in [
        "typeof TypeError",
        "typeof Error",
        "TypeError('x').message",
        "new TypeError('x').message",
        "new TypeError('x') instanceof TypeError",
        "new TypeError('x') instanceof Error",
        "new RangeError().message",
        "new Error('e').name",
        "new SyntaxError('s').name",
        "new ReferenceError('r').name",
        "new EvalError('v').name",
        "new URIError('u').name",
        "new SyntaxError('s').constructor===SyntaxError",
        // An error this engine throws is one of these, so a Script can tell
        // which it is.
        "var r=0;function f(o){return o.x}try{f(null)}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{undefinedName}catch(e){r=e instanceof ReferenceError}r",
        "var r=0;try{throw new RangeError('z')}catch(e){r=e.message}r",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_math_namespace_answers_what_its_realm_built() -> Result<(), Error> {
    // 21.3 is an ordinary object that 19.1 gives the global object, and
    // 21.3.2.26 is Number::exponentiate on two arguments 7.1.4 has made
    // numbers of.
    for source in [
        "typeof Math",
        "Math.pow(2,32)-1",
        "Math.pow(2,10)",
        "Math.pow('3',2)",
        "Math.pow(2)",
        "typeof Math.pow",
        // A name 21.3 does not give it is undefined like any other miss.
        "typeof Math.notAName",
    ] {
        differential(source)?;
    }
    // A name 21.3 gives it and this Realm has not built is a gap, because
    // answering undefined would say the namespace does not have it.
    for source in ["Math.sqrt", "Math.log", "Math.hypot"] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_call_reaches_the_function_it_was_found_on() -> Result<(), Error> {
    // 20.2.3.3 calls the function it was reached through, with the first
    // argument as the `this` value and the rest shifted down by one, and
    // 10.2.1.2 still binds `this` for a callee that is not strict.
    for source in [
        "function f(a){return this.v+a}var o={v:1};f.call(o,2)",
        "function f(){return typeof this}f.call()",
        "function f(a,b){return a+b}f.call(null,3,4)",
        "function f(){return 7}f.call(null)",
        // Forwarding again drops one more argument, which is why a chain ends.
        "function f(a){return this.v+a}var o={v:1};f.call.call(f,o,2)",
        "var a=[3,1,2];Array.prototype.join.call(a,'-')",
        "typeof Function",
        "typeof Function.prototype.call",
        "Function.name",
        "Function.length",
    ] {
        differential(source)?;
    }
    // 17 counts the arguments of the heading of 20.2.3.3 without its rest
    // parameter, so `call` has a length of one. The stack backend answers
    // zero, which is why this is not compared against it.
    let program = compile("function f(){}f.call.length", Limits::default())?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost)?,
        Value::Number(1.0)
    );
    // 20.2.3.2 answers the bound function exotic object of 10.4.1, which
    // 10.4.1.1 calls with the `this` value the bind gave it, and 7.2.3 counts
    // as callable.
    for source in [
        "function f(){return this.v}var g=f.bind({v:5});g()",
        "function f(a){return this.v+a}var g=f.bind({v:1});g(2)",
        "function f(){return 1}typeof f.bind({})",
        "typeof Function.prototype.bind",
        // A second bind cannot replace the `this` the first one fixed.
        "function f(){return this.v}f.bind({v:1}).bind({v:2})()",
        // The idiom that takes a method off its receiver.
        "var j=Function.prototype.call.bind(Array.prototype.join);j([1,2,3],'-')",
    ] {
        differential(source)?;
    }
    // `[[BoundArguments]]` would have to go in front of the arguments the call
    // passes, which lie in the registers of the caller, so a bound argument is
    // named rather than dropped.
    let program = compile("function f(){}f.bind({},1)", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    // 20.2.1.1 compiles a body at run time, which this engine names as a gap
    // rather than answering a function that would not be the one asked for.
    let program = compile("Function('return 1')", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn a_lexical_declaration_of_a_script_binds_on_the_realm() -> Result<(), Error> {
    // 16.1.7 puts a `let` or a `const` of a Script on the
    // [[DeclarativeRecord]] of the Global Environment Record, not on the
    // global object, and the binding outlives the Script that made it.
    differential_scripts(&["let x = 1;", "x + 1"])?;
    differential_scripts(&["const y = 2;", "y * 3"])?;
    differential_scripts(&["let a = 1; let b = 2;", "a + b"])?;
    differential_scripts(&["let u;", "typeof u"])?;
    differential_scripts(&["let s = 'a';", "s + 'b'"])?;
    // 16.1.7 refuses a name the Realm already binds. Both paths answer the
    // same SyntaxError beside the value rather than throwing it, which the
    // comparison above does not cover.
    for scripts in [
        ["let d = 1;", "let d = 2;"],
        ["let e = 1;", "var e = 2;"],
        ["var f = 1;", "let f = 2;"],
    ] {
        let outcome = |backend| -> Result<String, Error> {
            let mut host = SilentHost;
            let mut realm = Realm::with_backend(Limits::default(), &mut host, backend)?;
            let mut last = Ok(Value::Undefined);
            for source in scripts {
                last = realm.evaluate(source);
            }
            Ok(format!("{last:?}"))
        };
        assert_eq!(
            outcome(Backend::Engine)?,
            outcome(Backend::Stack)?,
            "{scripts:?}"
        );
    }
    // A `var` still becomes a property of the global object, and the two
    // kinds of binding live beside one another.
    differential_scripts(&["var v = 1; let w = 2;", "v + w"])?;
    // 9.1.1.4.5 refuses to write a binding `const` made immutable. Both paths
    // answer a TypeError, and they answer it differently: the engine throws
    // the Object the specification throws, which the boundary cannot carry to
    // the embedding, and the stack backend reports the error beside the value
    // instead. So this compares what each one does, not how it is carried.
    let mut host = SilentHost;
    let mut engine = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    engine.evaluate("const c = 1;")?;
    assert!(matches!(
        engine.evaluate("c = 2"),
        Err(Error::Thrown { .. })
    ));
    let mut host = SilentHost;
    let mut stack = Realm::with_backend(Limits::default(), &mut host, Backend::Stack)?;
    stack.evaluate("const c = 1;")?;
    assert!(matches!(stack.evaluate("c = 2"), Err(Error::Type { .. })));
    Ok(())
}

#[test]
fn a_compound_assignment_reaches_a_property() -> Result<(), Error> {
    // 13.15.2 evaluates the Reference once, reads through it, evaluates the
    // right side, applies the operator of 13.15.3 and writes back. The base
    // and the key are evaluated before the right side and not again after it.
    for source in [
        "var o={a:1};o.a+=2;o.a",
        "var o={a:'x'};o.a+='y';o.a",
        "var o={a:8};o.a-=3;o.a",
        "var o={a:8};o.a*=3;o.a",
        "var o={a:9};o.a/=2;o.a",
        "var o={a:7};o.a%=4;o.a",
        "var o={a:6};o.a<<=2;o.a",
        "var o={a:6};o.a>>=1;o.a",
        "var o={a:6};o.a&=3;o.a",
        "var o={a:6};o.a|=1;o.a",
        "var o={a:6};o.a^=3;o.a",
        "var a=[1,2];a[0]+=5;a[0]",
        "var o={a:1};var k='a';o[k]+=2;o.a",
        // The right side runs after the read, so it sees the old value and
        // what it writes is overwritten by the sum.
        "var o={a:1};o.a+=(o.a=10);o.a",
        // The key is evaluated once, whatever evaluating it does.
        "var n=0;var o={a:1};o[(n++,'a')]+=2;''+n+','+o.a",
        "var a=[1,2,3];var i=0;a[i++]+=10;''+i+','+a[0]+','+a[1]",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_property_descriptor_crosses_between_the_object_and_the_engine() -> Result<(), Error> {
    // 20.1.2.10 answers the own String keys in the order 10.1.11 gives them,
    // 20.1.2.8 answers the descriptor 6.2.6.4 makes of an own property, and
    // 20.1.2.4 defines the one 6.2.6.5 reads. A field a descriptor does not
    // carry is absent, which is false for an attribute.
    for source in [
        "Object.getOwnPropertyNames({a:1,b:2}).join(',')",
        "Object.getOwnPropertyNames({}).length",
        "Object.getOwnPropertyNames([1,2]).join(',')",
        "var d=Object.getOwnPropertyDescriptor({a:7},'a');''+d.value+d.writable+d.enumerable+d.configurable",
        "typeof Object.getOwnPropertyDescriptor({},'zz')",
        "var o={};Object.defineProperty(o,'x',{value:5});''+o.x",
        "var o={};Object.defineProperty(o,'x',{value:5});var d=Object.getOwnPropertyDescriptor(o,'x');''+d.writable+d.enumerable+d.configurable",
        "var o={};Object.defineProperty(o,'x',{value:5,enumerable:true});var s='';for(var k in o){s+=k}s",
        "var o={};Object.defineProperty(o,'x',{value:5});var s='';for(var k in o){s+=k}'['+s+']'",
        "var o={};''+(Object.defineProperty(o,'x',{value:1})===o)",
    ] {
        differential(source)?;
    }
    // This engine has no accessor property, so a descriptor that names one is
    // a gap and not a descriptor with the accessor quietly dropped.
    for source in [
        "Object.defineProperty({},'x',{get:function(){return 1}})",
        "Object.defineProperty({},'x',{set:function(v){}})",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_object_constructor_answers_what_its_realm_built() -> Result<(), Error> {
    // 17 gives `%Object.prototype%` its `constructor`, so a read of that name
    // is no longer a gap but the constructor it belongs to.
    differential("let o={a:1};let k='constructor';typeof o[k]")?;
    // 20.1.1.1 makes an ordinary object of undefined and null and answers
    // every Object unchanged, and 17 ties `%Object%` and `%Object.prototype%`
    // to one another.
    for source in [
        "typeof Object",
        "Object.name",
        "Object.length",
        "typeof Object()",
        "typeof Object(undefined)",
        "typeof Object(null)",
        "let o={a:1};Object(o).a",
        "typeof new Object()",
        "let f=function(o){return Object(o)===o};f({})",
    ] {
        differential(source)?;
    }
    // 20.1.2 gives `%Object%` names this Realm has not built, and `ToObject`
    // of a primitive needs a wrapper it has not built either.
    for source in ["Object.keys", "Object.create", "typeof Object(1)"] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_array_constructor_answers_what_its_realm_built() -> Result<(), Error> {
    // 23.1.1.1 makes an Array of one length or of many elements, 23.1.2.3
    // answers IsArray, and 23.1.2.5 and 23.1.3.2 tie the constructor and its
    // prototype to one another.
    for source in [
        "typeof Array",
        "Array.name",
        "Array.length",
        "''+Array.isArray([1])+Array.isArray({})+Array.isArray(1)+Array.isArray('a')",
        "Array().length",
        "Array(3).length",
        "Array('a').length",
        "''+Array(1,2,3).length+Array(1,2,3)[2]",
    ] {
        differential(source)?;
    }
    // 23.1.2 gives `%Array%` names this Realm has not built, and a read of one
    // of them is a gap and not the undefined of a constructor without it.
    for source in ["Array.from", "Array.of", "typeof Array.fromAsync"] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_constructor_path_survives_a_collection() -> Result<(), Error> {
    // The Nursery fills after some hundreds of allocating iterations, so only
    // a long loop reaches the scavenge. 10.2.5 allocates the `prototype` of a
    // function and 10.1.13 allocates the object `new` creates; a reference
    // read before either allocation does not survive it, and the collector
    // follows the accumulator and the registers, not a local of the
    // interpreter.
    for source in [
        "var n=0;for(var i=0;i<2000;i++){var f=function q(){};n+=typeof f==='function'?1:0}n",
        "function F(){this.x=1}var n=0;for(var i=0;i<2000;i++){n+=new F().x}n",
        "function F(){this.x=1}var n=0;for(var i=0;i<2000;i++){var a=[i];n+=new F().x+a[0]-i}n",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn an_object_only_one_branch_made_survives_the_join() -> Result<(), Error> {
    // 14.6.2 joins the two states of an `if`. An Object only one branch made
    // exists only where that branch ran, so what its layout says still holds;
    // one the two branches shaped differently has no single shape to name and
    // is read through the instruction instead.
    for source in [
        "let f=function(d){let o={w:0};if(d){o={w:1}}return o.w};''+f(1)+f(0)",
        "let f=function(d){let o={w:0};if(d){o.w=9}return o.w};''+f(1)+f(0)",
        "let f=function(d){let o={w:0,x:0};if(d){o={w:1}}else{o={w:2,x:3}}return ''+o.w+o.x};''+f(1)+f(0)",
        "let f=function(d){let o={w:0};if(d){o={a:[1,2]}}return typeof o};''+f(1)+f(0)",
        "let f=function(d){let a=[1];if(d){a=[1,2,3]}return a.length};''+f(1)+f(0)",
        // One branch gives the Object a property the other never gives it, so
        // it has no single shape after the join and is read through the
        // instruction on both paths.
        "let f=function(d){let r={};if(d){r.e=1}return ''+r.e};''+f(1)+f(0)",
        "let f=function(d){let r={a:7};if(d){r.e=1}return ''+r.a+r.e};''+f(1)+f(0)",
        "let f=function(d){let r={a:7};if(d){r.a=1}return r.a};''+f(1)+f(0)",
        // A binding one branch makes an Object and the other a Number keeps
        // no single type, so the join names what the value is at run time.
        "let x=1;if(true){x={}}else{x=2}typeof x",
        "let x=1;if(false){x={}}else{x=2}typeof x",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_property_is_written_under_a_key_only_the_run_time_knows() -> Result<(), Error> {
    // 13.15.2 writes through PutValue, which 10.1.9.2 sends along the
    // Prototype Chain when the receiver has no own property of that name.
    for source in [
        "let f=function(o,k){o[k]=1;return o[k]};f({a:0},'a')",
        "let f=function(o,k){o[k]=1;return o.b};f({a:0},'b')",
        "let f=function(o,k){o[k]=1;return o[0]};f([7,8],0)",
        "let f=function(o,k){o[k]='x';return typeof o[k]};f({},'a')",
        "let f=function(o,k){o[k]=1;return o.length};f({},'length')",
        "let f=function(o,k){o[k]=1;return o.name};f({},'name')",
    ] {
        differential(source)?;
    }
    // 6.2.5.5 sends the base of the Reference through ToObject before it
    // writes, and 7.1.18 refuses undefined and null there.
    for source in [
        "let o={};o[o.t].r=1",
        "let f=function(o,k){o[k]=1};f(null,'a')",
        "let f=function(o){o.a=1};f(undefined)",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let expected = Runtime::new(Limits::default()).run(&program.legacy_only(), &mut SilentHost);
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost);
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"), "{source}");
    }
    // `__proto__`, `length` and `name` belong to a Prototype or an exotic
    // object this Realm has not built, so a store that reaches one names that
    // where it happens instead of shadowing it.
    for source in [
        "let f=function(o,k){o[k]=1};f({},'__proto__')",
        "let f=function(o,k){o[k]=1};f([1,2],'length')",
        "let f=function(o,k){o[k]=1};f(function(){},'name')",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_delete_takes_the_property_off_the_object() -> Result<(), Error> {
    // 13.5.1.2 sends the base through ToObject and the name through
    // [[Delete]], which 10.1.10.1 refuses for a property that is not
    // configurable and answers true for one that is not there at all.
    for source in [
        "let o={a:1,b:2};let f=function(p){return delete p.a};f(o);''+o.a+o.b",
        "let f=function(p){return delete p.a};f({a:1})",
        "let f=function(p){return delete p.zz};f({a:1})",
        "let f=function(p){return delete p['a']};f({a:1})",
        "let f=function(p,k){return delete p[k]};f({a:1},'a')",
        "let a=[1,2,3];let f=function(p){return delete p[1]};''+f(a)+a[1]+a.length",
        "let f=function(p){return delete p.length};f([1,2,3])",
        "let f=function(){return delete 5};f()",
        "let f=function(){let v=1;return delete v};f()",
    ] {
        differential(source)?;
    }
    // 10.4.4 gives the arguments object ordinary properties, so an index of it
    // is deletable like any other.
    differential("let f=function(){return ''+(delete arguments[0])+arguments[0]};f(7)")?;
    Ok(())
}

#[test]
fn a_delete_of_a_global_name_names_the_gap() -> Result<(), Error> {
    // 13.5.1.2 sends an unresolvable Reference to true and a resolvable one to
    // the Environment Record it belongs to. A free name belongs to the global
    // object, which this lowering does not reach.
    let program = compile("delete zz", Limits::default())?;
    assert!(!program.uses_register_backend());
    // A `var` of a Script is a property of the global object that 16.1.7 makes
    // non-configurable, and every binding of a declaration is one too.
    differential("var q=1;delete q")?;
    differential("let f=function(){var v=1;return delete v};f()")?;
    Ok(())
}

#[test]
fn an_array_answers_an_index_and_length_under_a_static_name() -> Result<(), Error> {
    // 10.4.2 keeps the indices of an Array in an element store and its length
    // in a field of its own. A name that asks for one of them reaches them
    // whichever instruction asks, so a base the lowering could not name
    // answers what an Array holds and not what its Shape carries.
    differential("let f=function(o){return o[0]};f([7,8])")?;
    differential("let f=function(o){return o.length};f([7,8])")?;
    differential("let f=function(o){return o['length']};f([1,2,3])")?;
    differential("let f=function(o){return o[5]};f([7,8])")?;
    differential("let f=function(o){return o['00']};f([7,8])")?;
    differential("let f=function(o){o[0]=9};let a=[7];f(a);a[0]")?;
    differential("let f=function(o){o[3]=9;return o.length};f([7])")?;
    differential("let f=function(o){o.x=1;return o.x};f([7])")?;
    // An ordinary object keeps both under its Shape.
    differential("let f=function(o){return o.length};f({length:4})")?;
    differential("let f=function(o){return o[0]};f({0:4})")?;
    Ok(())
}

#[test]
fn an_object_that_escaped_in_one_branch_escaped_after_the_join() -> Result<(), Error> {
    // 14.6.2 joins the two Blocks. An Object one of them handed to user code
    // is no longer this lowering's after the join, whichever Block ran.
    differential(
        "let p=function(a,v){a[0]=v};let f=function(d){let a=[1];if(d){p(a,9)}return a[0]};\
         f(true)+','+f(false)",
    )?;
    differential(
        "let p=function(a,v){a.b=v};let f=function(d){let a={x:1};if(d){p(a,9)}else{}return a.b};\
         typeof f(false)",
    )?;
    Ok(())
}

#[test]
fn a_parameter_is_any_value() -> Result<(), Error> {
    // 10.2.11 binds the argument itself, so a parameter is not a primitive and
    // an argument is not restricted to one.
    differential_scripts(&["function f(o){return o.a}var o={a:42};f(o)"])?;
    differential_scripts(&["function f(o){o.b=2}var o={a:1};f(o);o.a+o.b"])?;
    differential_scripts(&["function f(o){return typeof o}f({})"])?;
    differential_scripts(&["function f(o){return o.hasOwnProperty('a')}f({a:1})"])?;
    Ok(())
}

#[test]
fn an_object_that_reached_a_call_keeps_no_layout() -> Result<(), Error> {
    // The callee can change what the Object holds, so a read after the call
    // asks the Object, not the layout the lowering had before it.
    differential("let f=function(o){o.a=2};let o={a:1};f(o);o.a")?;
    differential("let f=function(o){o.b=2};let o={a:1};f(o);o.b")?;
    differential("let f=function(o){o.a.b=2};let o={a:{b:1}};f(o);o.a.b")?;
    differential("let f=function(o){return o.a};let o={a:1};let r=f(o);o.a=2;r+o.a")?;
    // A function carries its closure with it, so what it captures leaves too.
    differential("let o={a:1};let f=function(){o.a=5};let g=function(h){h()};g(f);o.a")?;
    differential("let o={a:1};let f=function(){o.b=5};let g=function(h){h()};g(f);o.b")?;
    Ok(())
}

#[test]
fn a_conversion_that_cannot_run_valueof_names_the_gap() -> Result<(), Error> {
    // 7.1.4 sends an Object through ToPrimitive. A conversion that cannot open
    // a frame for a `valueOf` of the Script names that instead of answering.
    for source in [
        "function f(p){return p&1}f({})",
        "function f(p){return +p}f({})",
        "function f(p){return p==1}f({})",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    // The same code answers for every argument that is a primitive.
    differential_scripts(&["function f(p){return p&1}f(3)"])?;
    differential_scripts(&["function f(p){return +p}f('42')"])?;
    differential_scripts(&["function f(p){return p==1}f('1')"])?;
    Ok(())
}

#[test]
fn a_script_run_for_effect_ends_on_a_value_that_cannot_cross() -> Result<(), Error> {
    // `run_compiled` drops the completion value, so a Script whose value has no
    // identity outside the engine still completes. A thrown value stays a
    // failure.
    let limits = Limits::default();
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(limits, &mut host, Backend::Engine)?;
    for source in ["({x:1})", "[1,2]", "2*3"] {
        realm.run_compiled(&compile_script(source, limits)?)?;
    }
    assert!(matches!(
        realm.run_compiled(&compile_script("throw {}", limits)?),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn a_constructor_and_a_method_of_a_global_are_reached_at_run_time() -> Result<(), Error> {
    // In a Realm a function declaration binds a name of the Global Environment
    // Record, so neither the constructor of `new` nor the callee of a method
    // call has a type the lowering can give; 7.3.15 and 7.3.14 resolve both.
    for scripts in [
        &[
            "function T(m){this.message=m}",
            "var t=new T('x');t.message",
        ][..],
        &[
            "function T(m){this.message=m}",
            "T.prototype.describe=function(){return 'E: '+this.message};0",
            "var t=new T('x');t.describe()",
        ][..],
        // A constructor that constructs itself, which sta.js of Test262 opens
        // with.
        &[
            "function T(m){if(!(this instanceof T))return new T(m);this.message=m||''}",
            "var t=new T('x');t.message",
        ][..],
        // The same constructor called without `new`, which its guard turns
        // into a construction.
        &[
            "function T(m){if(!(this instanceof T))return new T(m);this.message=m||''}",
            "var t=T('x');t.message",
        ][..],
        // A thrown Object a handler of the same Script catches never reaches
        // the boundary.
        &[
            "function T(m){this.message=m}",
            "var r='';try{throw new T('x')}catch(e){r=e.message}r",
        ][..],
    ] {
        differential_scripts(scripts)?;
    }
    Ok(())
}

#[test]
fn new_constructs_from_the_prototype_of_the_constructor() -> Result<(), Error> {
    // 10.2.5 gives an ordinary function a `prototype`, 10.1.13 creates the
    // object from it, and 10.2.2 answers that object unless the constructor
    // answers one of its own.
    for source in [
        "function F(){};typeof F.prototype",
        "function F(){};F.prototype===F.prototype",
        "function F(){this.x=1}var f=new F();f.x",
        "function F(a){this.x=a}var f=new F(41);f.x+1",
        "function F(){this.x='a'}var f=new F();f.x+'b'",
        "function F(){}var f=new F();typeof f",
        "function F(){}var f=new F();typeof f.nope",
        // A primitive completion is discarded for the created object.
        "function F(){return 7}var f=new F();typeof f",
        // Two constructions of the same constructor are distinct objects.
        "function F(){this.x=1}var a=new F();var b=new F();b.x=2;a.x+b.x",
    ] {
        differential(source)?;
    }

    // An arrow has no [[Construct]] (10.2.5), so the lowering does not take it.
    assert!(!compile("var g=()=>1;var f=new g();1", Limits::default())?.uses_register_backend());
    Ok(())
}

#[test]
fn a_property_of_a_function_object_is_read_and_written() -> Result<(), Error> {
    // A function object is an ordinary object with a Prototype, so it carries
    // own properties like any other. The lowering tracks no layout for it, so
    // the generic instruction reads and writes them.
    for source in [
        "var f=function(){};f.z=1;f.z",
        "var f=function(){};f.z=1;typeof f.z",
        "var f=function(){};f.a=1;f.b=2;f.a+f.b",
        "var f=function(){};typeof f.nope",
        // The same instruction serves a base the lowering could not name.
        "let o={a:1,g:function(){this.b=2;return this.b}};o.g()",
        "let o={g:function(){this.b='x';return this.b+'y'}};o.g()",
    ] {
        differential(source)?;
    }

    // 10.2 and 20.2.3 name the properties a function object and
    // %Function.prototype% own. Neither exists yet, so a read of one is a gap
    // and a write of one is refused: it would shadow what is not writable.
    let program = compile("var f=function(){};f.name", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    for source in [
        "var f=function(){};f.name=1;1",
        "var f=function(){};f.__proto__=1;1",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_read_that_reaches_an_unbuilt_prototype_is_a_gap() -> Result<(), Error> {
    // 10.1.8.1 answers undefined for a name no object of the Prototype Chain
    // has. That is the answer only when the chain is complete: a name of
    // 20.1.3, 22.1.3 or 23.1.3 this Realm has not built would have been found,
    // so the miss is a gap. A static read is refused by the lowering; a
    // computed one reaches the engine and used to answer undefined.
    for source in [
        "let o={a:1};let k='valueOf';typeof o[k]",
        "let a=[1];let k='flat';typeof a[k]",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }

    // A name no Prototype of the chain would own is absent on both paths.
    for source in [
        "let o={a:1};let k='nope';typeof o[k]",
        // A name of 20.1.3 the Realm has built is answered, not refused.
        "let o={a:1};let k='propertyIsEnumerable';typeof o[k]",
        "let o={a:1};let k='hasOwnProperty';typeof o[k]",
        "let o={a:1};let k='isPrototypeOf';typeof o[k]",
        "let o={a:1};let k='a';o[k]",
        "let a=[1,2];let k='length';a[k]",
        "let a=[1,2];let k=1;a[k]",
        // A computed read of an own name is answered, and returnable.
        "function f(key){let o={[key]:42};return o[key]}f('answer')",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_realm_on_the_stack_backend_takes_the_same_scripts() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Stack)?;
    for source in [
        "1+1",
        "let x=1",
        "var y=2",
        "function f(){}",
        "{ let z = 3 }",
    ] {
        assert!(realm.evaluate(source).is_ok(), "{source}");
    }
    Ok(())
}

fn same_value(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits()
        }
        _ => left == right,
    }
}

#[test]
fn iterator_and_dynamic_binding_patterns_stay_on_legacy_backend() -> Result<(), Error> {
    for source in [
        "let input={next(){return {done:true}},[Symbol.iterator](){return this}};let [x]=input;x",
        "let a=[1];a[Symbol.iterator]=function(){return {next(){return {value:42}}}};let [x]=a;x",
        "let input={next(){return {done:true}},[Symbol.iterator](){return this}};let [...x]=input;x",
        "let source={get x(){return 1},y:2};let {x,...rest}=source;rest.y",
        "let key={toString(){return 'x'}};let {[key]:x,...rest}={x:1,y:2};rest.y",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
    }
    Ok(())
}

#[test]
fn fresh_array_binding_patterns_use_dense_elements() -> Result<(), Error> {
    for source in [
        "let [x]=[42];x",
        "var [x,y]=[20,22];x+y",
        "let [,x,,]=[1,42,3];x",
        "let [x]=[];x===undefined",
        "let [x=42]=[];x",
        "let [x=42]=[,];x",
        "let [x=1]=[2];x",
        "let [x,y=x+2]=[40];x+y",
        "let [{x}]=[{x:42}];x",
        "let [[x]]=[[42]];x",
        "let [x,...rest]=[1,2,3];x+rest[0]+rest[1]",
        "let [,,...rest]=[1,2,20,22];rest[0]+rest[1]",
        "let [x,...rest]=[42];x+rest.length",
        "let [...rest]=[];rest.length",
        "let [...[x,y]]=[20,22];x+y",
        "let [...{0:x,1:y,length:n}]=[20,22];x+y+n",
        "let input=[1,2];let [...rest]=input;rest[0]=40;input[0]+rest[0]",
        "{const [x]=[42];x}",
        "for(let [x]=[1];x<2;x++){}",
        "function f(){let [x]=[42];return x}f()",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let code = program
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        if source != "let [...rest]=[];rest.length" {
            assert!(
                core::iter::once(code.as_ref())
                    .chain(code.functions.iter())
                    .flat_map(|function| &function.instructions)
                    .any(|instruction| matches!(
                        instruction,
                        crate::engine::bytecode::Instruction::GetByValue { .. }
                    )),
                "{source}"
            );
        }
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn static_object_binding_patterns_and_defaults_use_register_property_caches() -> Result<(), Error> {
    for source in [
        "let {x}={x:42};x",
        "var {x:y}={x:42};y",
        "let {x,y}={x:20,y:22};x+y",
        "let {x:{y}}={x:{y:42}};y",
        "let {[('x')]:x}={x:42};x",
        "let {['x']:x}={x:42};x",
        "let {[true]:x}={'true':42};x",
        "let {[false]:x}={'false':42};x",
        "let {[null]:x}={'null':42};x",
        "let {[undefined]:x}={'undefined':42};x",
        "let {[0]:x,['length']:n}=[41];x+n",
        "let {[1]:x=42}=[];x",
        "let {0:x,length:n}=[41];x+n",
        "let {}={x:1};42",
        "let {x=42}={x:undefined};x",
        "let {x=42}={};x",
        "let {x=1}={x:2};x",
        "var {x:y=40}={x:undefined};y+2",
        "let {x,y=x+2}={x:40,y:undefined};x+y",
        "let {x:{y=42}}={x:{y:undefined}};y",
        "let n=0;let {x=(n=1)}={x:2};x+n",
        "let n=0;let {x=(n=1)}={x:undefined};x+n",
        "let flag=true;let {x=42}={x:flag?undefined:1};x",
        "let flag=true;let {x=42}={x:flag?undefined:'a'};x",
        "let flag=false;let {x=42}={x:flag?undefined:'a'};x",
        "function f(){let {x}={x:42};return x}f()",
        "function f(){let {x=42}={x:undefined};return x}f()",
        "function f(){let n=0;let {x=(n=1)}={x:undefined};return x+n}f()",
        "function f(){let x=1;function g(){return x}let {y=(x=2)}={};return g()+y}f()",
        "function f(){var {x:y}={x:42};return y}f()",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let code = program
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        if source.contains("flag?undefined") {
            assert!(code.instructions.iter().any(|instruction| matches!(
                instruction,
                crate::engine::bytecode::Instruction::JumpIfNotUndefined(_)
            )));
        }
        if source.contains("{[") {
            assert!(
                core::iter::once(code.as_ref())
                    .chain(code.functions.iter())
                    .flat_map(|function| &function.instructions)
                    .any(|instruction| matches!(
                        instruction,
                        crate::engine::bytecode::Instruction::GetByValue { .. }
                    )),
                "{source}"
            );
        }
        if !source.contains("let {}") {
            assert!(
                core::iter::once(code.as_ref())
                    .chain(code.functions.iter())
                    .flat_map(|function| &function.instructions)
                    .any(|instruction| matches!(
                        instruction,
                        crate::engine::bytecode::Instruction::GetNamed { .. }
                            | crate::engine::bytecode::Instruction::GetByValue { .. }
                            | crate::engine::bytecode::Instruction::GetArrayLength { .. }
                    )),
                "{source}"
            );
        }
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn observable_object_binding_defaults_stay_on_legacy_backend() -> Result<(), Error> {
    for source in [
        "let {x=({})}={};42",
        "let o={};let {x=(o.y=1)}={};42",
        "let key={toString(){return 'x'}};let {[key]:x}={x:42};x",
        "let key='x';let {[key='y']:x}={x:42};x",
        "let {['x'+'']:x}={x:42};x",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
    }
    Ok(())
}

#[test]
fn fresh_object_rest_bindings_copy_shape_slots() -> Result<(), Error> {
    for source in [
        "let {...rest}={x:20,y:22};rest.x+rest.y",
        "let {x,...rest}={x:1,y:42};x+rest.y",
        "let {x,...rest}={x:1,y:42};let {x:copy=7,y}=rest;copy+y",
        "let {['x']:x,...rest}={x:1,y:42};x+rest.y",
        "let {nested:{x,...rest}}={nested:{x:1,y:42}};x+rest.y",
        "let {...rest}={};42",
        "let {x,...rest}={x:42};x",
        "let {x,...rest}={x:42,x:1,y:41};x+rest.y",
        "let input={x:1,y:2};let {x,...rest}=input;rest.y=40;input.y+rest.y",
        "function f(){let {x,...rest}={x:1,y:41};return x+rest.y}f()",
        "let total=0;for(let {x,...rest}={x:1,y:2};total<1;total++)rest.y",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let code = program
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        assert!(
            core::iter::once(code.as_ref())
                .chain(code.functions.iter())
                .flat_map(|function| &function.instructions)
                .any(|instruction| matches!(
                    instruction,
                    crate::engine::bytecode::Instruction::CreateObject
                )),
            "{source}"
        );
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn identifier_destructuring_assignments_use_register_storage() -> Result<(), Error> {
    for source in [
        "let x=0,y=0;[x,y]=[20,22];x+y",
        "let x=0,y=0;[x,,y]=[20,0,22];x+y",
        "let x=0,y=0;[x=20,y=x+2]=[];x+y",
        "let x=0,y=0;[[x],{value:y}]=[[20],{value:22}];x+y",
        "let x=0,rest=[];[x,...rest]=[1,20,22];x+rest[0]+rest[1]",
        "let x=0,y=0;({x,y}={x:20,y:22});x+y",
        "let x=0,y=0;({x:{y}}={x:{y:42}});y",
        "let x=0,y=0;({x=20,y=x+2}={});x+y",
        "let x=0,rest={};({x,...rest}={x:1,y:41});x+rest.y",
        "let x=0;let input=[42];let same=([x]=input)===input;same&&x===42",
        "let x=0;let input={x:42};let same=({x}=input)===input;same&&x===42",
        "function f(){let x=0;[x]=[42];return x}f()",
        "let i=0,x=0;while(i<1){[x]=[42];i++}x",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn member_destructuring_targets_use_shape_and_elements_storage() -> Result<(), Error> {
    for source in [
        "let target={};[target.x]=[42];target.x",
        "let target={};({x:target.x}={x:42});target.x",
        "let target=[];[target[0]]=[42];target[0]",
        "let target=[0],i=0;[target[i]=(i=1)]=[];target[0]===1&&i===1",
        "let target=[0],i=0;({x:target[i]=(i=1)}={});target[0]===1&&i===1",
        "let target={};[[target.x]]=[[42]];target.x",
        "let target={};({x:{y:target.z}}={x:{y:42}});target.z",
        "let target={};[...target.x]=[20,22];target.x[0]+target.x[1]",
        "let target={},x=0;({x,...target.rest}={x:1,y:41});x+target.rest.y",
        "let target={},key='x';[target[key]]=[42];target.x",
        "let target={},key='x';({value:target[key]}={value:42});target.x",
        "let target={x:'old'},key='x';[target[key]]=[42];target.x",
        "let target={},value={x:42};target.value=value;target.value.x",
        "let target=[],value=[42];target[0]=value;target[0][0]",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn dynamic_object_writes_keep_subsequent_reads_conservative() -> Result<(), Error> {
    for source in [
        "let target={x:1},key='x';target[key]='a';target.x+'b'",
        "let target={x:1},key='y';target[key]=2;target.x+target.y",
        "let target={x:1},key='x';[target[key]]=['a'];target.x+'b'",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }

    let limits = Limits {
        properties: 1,
        ..Limits::default()
    };
    let program = compile("let target={x:1},key='y';target[key]=42;target.x", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "object properties"
        })
    );
    Ok(())
}

#[test]
fn observable_destructuring_assignments_stay_on_legacy_backend() -> Result<(), Error> {
    for source in [
        "const x=0;[x]=[1]",
        "let x=0;[x]={0:42,length:1}",
        "let x=0;let input=[1];input[Symbol.iterator]=function(){return {next(){return {value:42}}}};[x]=input;x",
        "let target={},key={toString(){return 'x'}};[target[key]]=[42];target.x",
        "let target={x:1},key='y',x=0,rest={};target[key]=2;({x,...rest}=target);rest.y",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        let _ = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost);
    }
    Ok(())
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn primitive_loose_equality_runs_through_register_bytecode() -> Result<(), Error> {
    for source in [
        "null == undefined",
        "undefined == null",
        "null != undefined",
        "null == false",
        "'2' == 2",
        "2 == '2'",
        "'x' == 0",
        "false == ''",
        "true == 1",
        "1 == true",
        "NaN == NaN",
        "0 == -0",
        "1 != 2",
        "function f(a,b){return a==b}f('42',42)",
        "function f(a,b){return a!=b}f(null,undefined)",
        "let i=0;while(i!=3)i++;i",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert_eq!(actual, expected, "{source}");
    }
    Ok(())
}

#[test]
fn primitive_bitwise_operations_run_through_register_bytecode() -> Result<(), Error> {
    for source in [
        "7 & 3",
        "4 | 1",
        "7 ^ 3",
        "1 << 31",
        "1 << 32",
        "1 << -1",
        "-8 >> 2",
        "-1 >>> 0",
        "-8 >>> 2",
        "'3.9' & 7",
        "true | null",
        "undefined ^ 7",
        "4294967297 | 0",
        "9007199254740991 >>> 0",
        "9007199254740992 >>> 0",
        "let x=7;x&=3;x|=8;x^=1;x<<=2;x>>=1;x>>>=1;x",
        "function f(a,b){return a>>>b}f(-1,'1')",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn short_circuit_expressions_run_through_register_bytecode() -> Result<(), Error> {
    for source in [
        "0 || 4",
        "0 && 4",
        "true || 4",
        "true && 4",
        "null ?? 3",
        "undefined ?? 4",
        "0 ?? 4",
        "false ?? 4",
        "let x=0;false&&(x=1);x",
        "let x=0;true||(x=1);x",
        "let x=0;null??(x=1);x",
        "let x=0;0??(x=1);x",
        "let x=1;true||(x='a');x+1",
        "let x=1;false&&(x='a');x+1",
        "(null??false)||4",
        "null??(false||4)",
        "function f(a,b){return a&&b}f('left','right')",
        "function f(a,b){return a??b}f(undefined,'right')",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn primitive_exponentiation_runs_through_register_bytecode() -> Result<(), Error> {
    for source in [
        "2**3**2",
        "(2**3)**2",
        "2**-2",
        "(-2)**3",
        "2**null",
        "true**false",
        "'3'**3",
        "2**undefined",
        "(-0)**3",
        "(-0)**-3",
        "NaN**0",
        "(-Infinity)**-3",
        "let a=2;a**=3**2;a",
        "let a='2';a**=3;a",
        "function f(a,b){return a**b}f('2',3)",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert_eq!(actual, expected, "{source}");
    }

    let limits = Limits {
        string_units: 5,
        ..Limits::default()
    };
    let program = compile("typeof 1", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
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
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost)?,
        expected
    );
    assert_eq!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost)?,
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn computed_object_data_properties_use_keyed_shape_storage() -> Result<(), Error> {
    for source in [
        "let key='x';let o={[key]:40,y:2};o.x+o.y",
        "let o={['x']:40,[true]:2};o.x+o.true",
        "let o={[0]:40,['0']:42};o['0']",
        "let o={['__proto__']:42};o.__proto__",
        "let key='x',value='v';let o={[(key='k')]:(value=key+'!')};key+value+o.k",
        "let key='x';let o={x:1,[key]:'a'};o.x+'b'",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let code = program
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        assert!(
            core::iter::once(code.as_ref())
                .chain(code.functions.iter())
                .flat_map(|function| &function.instructions)
                .any(|instruction| matches!(
                    instruction,
                    crate::engine::bytecode::Instruction::SetByValue { .. }
                )),
            "{source}"
        );
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }

    let fallback = "let key={toString(){return 'x'}};let o={[key]:42};o.x";
    assert!(!compile(fallback, Limits::default())?.uses_register_backend());
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
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
    // The Array is built; only its crossing to the embedding is refused.
    let source = "[1,2]";
    let program = compile(source, Limits::default())?;
    assert!(program.uses_register_backend(), "{source}");
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn missing_ordinary_properties_produce_undefined_in_register_bytecode() -> Result<(), Error> {
    for source in [
        "let o={x:1};o.missing===undefined",
        "let o={};o.missing",
        "let o={x:1};o['missing']===undefined",
        "let o={x:1},key='missing';o[key]===undefined",
        "let o={x:1},key='y';o[key]=2;o.missing===undefined",
        "function f(){let o={x:1};return o.missing}f()===undefined",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn ordinary_objects_read_primitive_bracket_keys_through_keyed_caches() -> Result<(), Error> {
    for source in [
        "({0:1})[0]",
        "let o={[0]:42};o[0]",
        "let o={x:1},key='y';o[key]===undefined",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let code = program
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        assert!(code.instructions.iter().any(|instruction| matches!(
            instruction,
            crate::engine::bytecode::Instruction::GetByValue { .. }
        )));
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }

    let observable = "let key={toString(){return 'x'}};let o={x:42};o[key]";
    assert!(!compile(observable, Limits::default())?.uses_register_backend());
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
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost)?,
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
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
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
            Runtime::with_backend(Limits::default(), Backend::Engine)
                .run(&program, &mut SilentHost)?,
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
    let mut runtime = Runtime::with_backend(limits, Backend::Engine);

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
    // The Script is lowered; the Object it completes with is refused where the
    // missing part is, at the boundary, by the name of what it needs.
    for source in ["({x:1})", "let o={x:1};o"] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    // Two Object literals of one conditional are two layouts the lowering
    // cannot merge, which refuses the Script before the completion matters.
    assert!(!compile("true?({x:1}):({x:2})", Limits::default())?.uses_register_backend());
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
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "string units"
        })
    );
    Ok(())
}

#[test]
fn register_backend_is_selected_statically_without_runtime_fallback() -> Result<(), Error> {
    for source in [
        "+({valueOf(){return 1}})",
        "-function(){}",
        "({}) & 1",
        "1 | ({})",
        "let o={};let x=o^1;0",
        "let x={};x<<=1;0",
        "let x=1;x>>={};0",
        "let o={};false&&(o.x=1);0",
        "let x=1;false&&(function(){return x});x",
        "let x={};x**=2;0",
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
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn register_functions_return_gc_owned_objects_and_layouts() -> Result<(), Error> {
    for source in [
        "function f(){return {x:42}}f().x",
        "function f(){return [20,22]}let a=f();a[0]+a[1]",
        "function f(){return {x:{y:42}}}f().x.y",
        "function f(){let o={x:1};o.x=42;return o}f().x",
        "function f(){let a=[1];a[0]=42;return a}f()[0]",
        "function make(x){return {x}}make(42).x",
        "function f(){let i=0;while(i<300){[i];i++}return {x:42}}f().x",
        "let o={x:42};let f=function(){return o};f().x",
        "let o={x:42};let f=function(){return o};f()===o",
        "let o={x:40};let f=function(){o.y=2;return o};f().x+o.y",
        "let o={x:1};let f=function(){o.y=2};o.y===undefined",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    assert!(
        !compile(
            "let o={x:42};function f(){return o}f().x",
            Limits::default()
        )?
        .uses_register_backend()
    );
    assert!(
        !compile(
            "let o={x:40};function f(){o.y=2;return o}f().x+o.y",
            Limits::default()
        )?
        .uses_register_backend()
    );
    Ok(())
}

#[test]
fn object_identity_equality_runs_without_coercion() -> Result<(), Error> {
    for source in [
        "let o={};o===o",
        "let o={};o!==o",
        "let a={},b={};a===b",
        "let a={},b={};a!==b",
        "let o={};o==o",
        "let o={};o!=o",
        "let a={},b={};a==b",
        "let a={},b={};a!=b",
        "let a=[],b=a;a===b&&a==b",
        "function f(){return {}}let a=f(),b=f();a!==b&&a!=b",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert_eq!(actual, expected, "{source}");
    }

    for source in ["let o={};o==0", "let o={};0!=o"] {
        assert!(!compile(source, Limits::default())?.uses_register_backend());
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one function keeps the refused forms beside the limits they share"
)]
fn register_function_calls_preserve_limits_and_reject_unlowered_semantics() -> Result<(), Error> {
    for source in [
        "async function f(){return 1}f()",
        "function f(a,a){return a}f(1,2)",
        "function f(...a){return a.length}f(1)",
        "let f=function inner(){return inner===f};f()",
        "let f=function inner(inner){return inner};f(42)",
        "typeof (function inner(){var inner;return inner})()",
        "typeof (function inner(){function inner(){return 42}return inner})()",
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
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
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
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
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
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
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
        Runtime::with_backend(
            Limits {
                fuel: exact,
                ..base
            },
            Backend::Engine,
        )
        .run(&program, &mut SilentHost)?,
        Value::Number(42.0)
    );
    assert_eq!(
        Runtime::with_backend(
            Limits {
                fuel: exact.saturating_sub(1),
                ..base
            },
            Backend::Engine,
        )
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
        Runtime::with_backend(Limits { stack: 1, ..base }, Backend::Engine)
            .run(&program, &mut SilentHost),
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
    let mut realm = Realm::with_backend(limits, &mut host, Backend::Engine)?;
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
        "{let x=1;x+2}",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn local_binding_lowering_preserves_tdz_and_const_guards_by_staying_legacy() -> Result<(), Error> {
    for source in ["let x=x;x", "let x=y,y=1;x", "const x=1;x=2"] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn block_lexical_bindings_use_scoped_registers() -> Result<(), Error> {
    for source in [
        "{let x=1;x}",
        "let x=1;{let x=2;x}x",
        "let x=1;{let x=2;x+1}",
        "let x=1;{const {x,y}={x:20,y:22};x+y}x",
        "let x=1;{{let x=2;x}x}",
        "function f(){let x=1;{let {x=2}={};x}return x}f()",
        "let x=1;if(true){let x=2;x}else{x}x",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn block_registers_count_towards_the_binding_budget() -> Result<(), Error> {
    let two = Limits {
        binding_slots: 2,
        ..Limits::default()
    };
    let nested = compile("{let x=20;{let y=22;x+y}}", two)?;
    assert!(nested.uses_register_backend());
    assert_eq!(
        nested
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?
            .binding_count,
        2
    );
    assert_eq!(
        Runtime::with_backend(two, Backend::Engine).run(&nested, &mut SilentHost)?,
        Value::Number(42.0)
    );
    assert_eq!(
        Runtime::with_backend(
            Limits {
                binding_slots: 1,
                ..Limits::default()
            },
            Backend::Engine
        )
        .run(&nested, &mut SilentHost),
        Err(Error::Limit {
            resource: "binding slots"
        })
    );

    let sequential = compile("{let x=1;x}{let y=2;y}", Limits::default())?;
    assert_eq!(
        sequential
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?
            .binding_count,
        1
    );
    assert_eq!(
        Runtime::with_backend(
            Limits {
                binding_slots: 1,
                ..Limits::default()
            },
            Backend::Engine
        )
        .run(&sequential, &mut SilentHost)?,
        Value::Number(2.0)
    );

    let function = compile(
        "function f(a){let x=1;{let y=2;return a+x+y}}f(39)",
        Limits::default(),
    )?;
    let body = function
        .register_code
        .as_ref()
        .and_then(|code| code.functions.first())
        .ok_or(Error::InvalidBytecode)?;
    assert_eq!(body.binding_count, 3);
    assert_eq!(
        Runtime::with_backend(
            Limits {
                binding_slots: 2,
                ..Limits::default()
            },
            Backend::Engine
        )
        .run(&function, &mut SilentHost),
        Err(Error::Limit {
            resource: "binding slots"
        })
    );
    Ok(())
}

#[test]
fn a_block_scope_is_taken_or_names_what_stops_it() -> Result<(), Error> {
    // A block of `let` and `const` binds in registers of the frame, which the
    // lowering has always done. What it does not take is named where it
    // stands, and no longer refuses the whole Script around it.
    for source in [
        "{let x=1;x}",
        "var r=0;{let x=1;r=x}r",
        "{let x=1};typeof x",
        "let y=1;{let y=2};y",
        "{let x=1;let y=2;x+y}",
    ] {
        differential(source)?;
    }
    for source in [
        "{let x=x;x}",
        "{let x=y,y=1;x}",
        "{const x=1;x=2}",
        "let f;{let x=42;f=()=>x}f()",
        "{function f(){return 42}f()}",
        "{let x={};42}",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        let _ = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost);
    }
    Ok(())
}

#[test]
fn var_bindings_are_hoisted_in_register_frames() -> Result<(), Error> {
    for source in [
        "x;var x",
        "var x=42;var x;x",
        "var a=b,b=2;a===undefined&&b===2",
        "var x=1;{var x=2}x",
        "if(false){var x=3}x",
        "if(true){var x=1}else{var x=2}x",
        "var i=0;while(i<3){var x=i;i++}x",
        "var i=0;while(i<1){i++;var x=42;continue}x",
        "for(var i=0;i<3;i++){}i",
        "for(var i=0;i<3;i++){var x=i}x",
        "function f(x){var x;return x}f(42)",
        "function f(x){var x=2;return x}f(1)",
        "function f(x){function x(){return 42}var x;return x()}f(1)",
        "function f(x){function x(n){return n<2?1:n*x(n-1)}var x;return x(4)}f(0)",
        "function f(){if(true){var x=42}return x}f()",
        "function f(){function g(){return x}let before=g();var x=42;return before===undefined&&g()===42}f()",
        "function f(){var g=function(){return x};var before=g();var x=42;return before===undefined&&g()===42}f()",
        "function f(){var x=42;return function(){return x}}f()()",
        "function f(){var x=40;function g(){return x+2}return g()}f()",
        "function outer(){var x=1;function read(){return x}function shadow(x){x='a';return x}return read()+shadow('b')}outer()",
        "function outer(){var x=1;function read(){return x}function shadow(){var x=2;x='a';return x}return read()+shadow()}outer()",
        "function outer(){var x=1;function read(){return x}function shadow(){function x(){return 2}x='a';return x}return read()+shadow()}outer()",
        "var f=1;function f(){return 42}f",
        "var f;function f(){return 42}f()",
        "function f(){return 1}function f(){return 42}f()",
        "function make(x){var y=x;return function(){return y}}let keep=make(42);let i=0;while(i<3000){make(i);i++}keep()",
        "var x",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn sibling_function_capture_requires_an_available_runtime_type() -> Result<(), Error> {
    let safe = "function g(){return 41}function f(){return g()+1}f()";
    let program = compile(safe, Limits::default())?;
    assert!(program.uses_register_backend());
    let mut legacy = program.clone();
    legacy.register_code = None;
    let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
    let actual =
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost)?;
    assert!(same_value(&actual, &expected));

    // The var initializer gives `g` a primitive type hint while declaration
    // instantiation replaces it with the later FunctionDeclaration before
    // `f` can run. Lowering `f` from that stale hint would select numeric Add
    // for a Function object.
    let forward = "var g=1;function f(){return g+1}function g(){return 42}f()";
    let program = compile(forward, Limits::default())?;
    assert!(!program.uses_register_backend());
    Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
    Ok(())
}

#[test]
fn var_initializers_infer_anonymous_function_and_class_names() -> Result<(), Error> {
    for source in [
        "var f=function(){},a=()=>{},c=class{},p=(function(){}),s=(0,function(){});f.name==='f'&&a.name==='a'&&c.name==='c'&&p.name==='p'&&s.name===''",
        "var f=function inner(){},c=class Inner{};f.name==='inner'&&c.name==='Inner'",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        assert_eq!(
            Runtime::new(Limits::default()).run(&program, &mut SilentHost)?,
            Value::Boolean(true),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn captured_var_uses_a_hoisted_heap_context() -> Result<(), Error> {
    let source = "function f(){function g(){return x}let before=g();var x=42;return before===undefined&&g()===42}f()";
    let program = compile(source, Limits::default())?;
    let code = program
        .register_code
        .as_ref()
        .ok_or(Error::InvalidBytecode)?;
    let outer = code.functions.first().ok_or(Error::InvalidBytecode)?;
    let inner = code.functions.get(1).ok_or(Error::InvalidBytecode)?;
    assert_eq!(outer.own_context_slot_count, Some(1));
    assert_eq!(inner.outer_context_slot_counts, [1]);
    assert!(inner.instructions.iter().any(|instruction| matches!(
        instruction,
        crate::engine::bytecode::Instruction::LoadContext { depth: 0, slot: 0 }
    )));

    let source = "let y=40;function f(){var x=2;return function(){return y+x}}f()()";
    let program = compile(source, Limits::default())?;
    let code = program
        .register_code
        .as_ref()
        .ok_or(Error::InvalidBytecode)?;
    let outer = code.functions.first().ok_or(Error::InvalidBytecode)?;
    let inner = code.functions.get(1).ok_or(Error::InvalidBytecode)?;
    assert_eq!(outer.own_context_slot_count, Some(1));
    assert_eq!(outer.outer_context_slot_counts, [1]);
    assert_eq!(inner.outer_context_slot_counts, [1, 1]);
    assert!(inner.instructions.iter().any(|instruction| matches!(
        instruction,
        crate::engine::bytecode::Instruction::LoadContext { depth: 0, slot: 0 }
    )));
    assert!(inner.instructions.iter().any(|instruction| matches!(
        instruction,
        crate::engine::bytecode::Instruction::LoadContext { depth: 1, slot: 0 }
    )));
    let mut legacy = program.clone();
    legacy.register_code = None;
    let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
    let actual =
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost)?;
    assert!(same_value(&actual, &expected));

    let source = "let a=1;function outer(){var b=2;function middle(){var c=3;return function(){return a+b+c}}return middle()}outer()()";
    let program = compile(source, Limits::default())?;
    let code = program
        .register_code
        .as_ref()
        .ok_or(Error::InvalidBytecode)?;
    let outer = code.functions.first().ok_or(Error::InvalidBytecode)?;
    let middle = code.functions.get(1).ok_or(Error::InvalidBytecode)?;
    let inner = code.functions.get(2).ok_or(Error::InvalidBytecode)?;
    assert_eq!(outer.own_context_slot_count, Some(1));
    assert_eq!(outer.outer_context_slot_counts, [1]);
    assert_eq!(middle.own_context_slot_count, Some(1));
    assert_eq!(middle.outer_context_slot_counts, [1, 1]);
    assert_eq!(inner.own_context_slot_count, None);
    assert_eq!(inner.outer_context_slot_counts, [1, 1, 1]);
    for depth in 0..=2 {
        assert!(inner.instructions.iter().any(|instruction| matches!(
            instruction,
            crate::engine::bytecode::Instruction::LoadContext { depth: found, slot: 0 }
                if *found == depth
        )));
    }
    let mut legacy = program.clone();
    legacy.register_code = None;
    let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
    let actual =
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost)?;
    assert!(same_value(&actual, &expected));

    let limits = Limits {
        binding_slots: 1,
        ..Limits::default()
    };
    let program = compile("var x=1,y=2;x+y", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "binding slots"
        })
    );
    // 16.1.7 puts a top-level `var` of a Realm Script on the Global
    // Environment Record, which the lowering now takes.
    assert!(
        compile_script("var x=1;x", Limits::default())?
            .program
            .uses_register_backend()
    );
    Ok(())
}

#[test]
fn var_frame_slots_exist_before_source_order_initializers() -> Result<(), Error> {
    use crate::engine::bytecode::{Instruction, Reg};

    let program = compile("x;var x=42;x", Limits::default())?;
    let code = program
        .register_code
        .as_ref()
        .ok_or(Error::InvalidBytecode)?;
    assert_eq!(code.binding_count, 1);
    let read = code
        .instructions
        .iter()
        .position(|instruction| *instruction == Instruction::Ldar(Reg(0)))
        .ok_or(Error::InvalidBytecode)?;
    let initialize = code
        .instructions
        .iter()
        .position(|instruction| *instruction == Instruction::Star(Reg(0)))
        .ok_or(Error::InvalidBytecode)?;
    assert!(read < initialize);
    Ok(())
}

#[test]
fn captured_var_mutations_wait_for_deoptimization() -> Result<(), Error> {
    for source in [
        "let x=1;function g(){return x}let {y=(x='a')}={};g()+1",
        "var x=1;function g(){return x}var {y=(x='a')}={};g()",
        "var x=1,y=0;function g(){return x}[x]=[2];g()",
        "var x=1,y=0;function g(){return x}[[x]]=[[2]];g()",
        "var x=1;function g(){return x}[...x]=[2];g()",
        "var x=1,y=0;function g(){return x}[y=(x=2)]=[];g()",
        "var x=1,y=0;function g(){return x}[y]=(x=[2]);g()",
        "var x=1,y=0;function g(){return x}({a:x}={a:2});g()",
        "var x=1,y=0;function g(){return x}({a:{b:x}}={a:{b:2}});g()",
        "var x=1,y=0;function g(){return x}({[x='key']:y}={key:2});g()",
        "var x=1,y=0;function g(){return x}({a:y=(x=2)}={});g()",
        "var x=1;function g(){return x}({...x}={a:2});g()",
        "var x=1;function g(){return x}x='a';g()",
        "var x=1;var g=function(){return x};x='a';g()",
        "var f=1;function f(){return f}typeof f",
        "function f(){var x=1;function g(){return x}{x='a'}return g()}f()",
        "function f(){var x=1;var g=function(){return x};x='a';return g()}f()",
        "function f(){var x=1;function g(){return x}if(false)x='a';return g()}f()",
        "function f(){var x=1;function g(){return x}while(false)x='a';return g()}f()",
        "function f(){var x=1;function g(){return x}for(;false;x='a'){}return g()}f()",
        "function f(){var x=1;function g(){return x}let y=(x='a');return g()}f()",
        "function f(){var x=1;function g(){return x}var y=(x='a');return g()}f()",
        "function f(){var x=1;function g(){return x}if(false)return x='a';return g()}f()",
        "function f(){var x=1;function g(){return x}(0,x='a');return g()}f()",
        "function f(){var x=1;function g(){return x}!(x='a');return g()}f()",
        "function f(){var x=1;function g(){return x}0+(x='a');return g()}f()",
        "function f(){var x=1;function g(){return x}false?0:(x='a');return g()}f()",
        "function f(){var x=1;function g(){return x}g(x='a');return g()}f()",
        "function f(){var x=1;function g(){return x}({a:(x='a')});return g()}f()",
        "function f(){var x=1;function g(){return x}[x='a'];return g()}f()",
        "function f(){var x=1,o={};function g(){return x}o.x=(x='a');return g()}f()",
        "function f(){var x=1;function g(){return x}let h=function(){return x='a'};return g()}f()",
        "function f(){var x=1;function g(){return x}if(false)0;else x='a';return g()}f()",
        "function f(){var x=1;function g(){return x}do{x='a'}while(false);return g()}f()",
        "function f(){var x=1;function g(){return x}switch(0){case 0:x='a'}return g()}f()",
        "function f(){var x=1;function g(){return x}switch(0){case (x='a'):break}return g()}f()",
        "function f(){var x=1;function g(){return x}try{x='a'}catch(e){}return g()}f()",
        "function f(){var x=1;function g(){return x}for(const k in {a:1}){x='a'}return g()}f()",
        "function f(){var x=1;function g(){return x}for(const k in (x='a')){}return g()}f()",
        "function f(){var x=1;function g(){return x}for(const v of [1]){x='a'}return g()}f()",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
    }
    Ok(())
}

#[test]
fn destructuring_other_bindings_preserves_captured_register_contexts() -> Result<(), Error> {
    for source in [
        "function f(){var x=40,y=0;function g(){return x}[y]=[2];return g()+y}f()",
        "function f(){var x=40,y=0;function g(){return x}[,y]=[0,2];return g()+y}f()",
        "function f(){var x=40,y=0;function g(){return x}[y=2]=[];return g()+y}f()",
        "function f(){var x=40,y=[];function g(){return x}[...y]=[2];return g()+y[0]}f()",
        "function f(){var x=40,y=0;function g(){return x}({a:y}={a:2});return g()+y}f()",
        "function f(){var x=40,y=0;function g(){return x}({a:y=2}={});return g()+y}f()",
        "function f(){var x=40,y={};function g(){return x}({...y}={a:2});return g()+y.a}f()",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn loop_var_type_inference_reaches_a_fixed_point() -> Result<(), Error> {
    for source in [
        "var i=0;while(i<2){var x=y;var y=1;i++}x",
        "for(var i=0;i<2;i++){var x=y;var y=1}x",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
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
        "let x=1;if(true)x=true;else x=2;x",
        "let x=1;if(false)x=true;x",
        "if(true) if(false) 3; else 4;",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
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
        "function f(){function g(){return x}var x=1;x='a';return g()}f()",
        "function f(){var x=1;function g(){x++;return x}return g()}f()",
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn do_while_loops_match_legacy_completion_and_control_flow() -> Result<(), Error> {
    for source in [
        "let i=0;do{i++}while(i<3);i",
        "let i=0;do i++;while(false);i",
        "let i=0;do{i++;if(i<3)continue;break}while(true);i",
        "let i=0,s=0;do{s+=i;i++}while(i<4);s",
        "let i=0;do{if(i===2)break;i++}while(true);i",
        "function f(){let i=0;do{if(i===2)return i;i++}while(true)}f()",
        "let i=0;do{let x=i;i=x+1}while(i<3);i",
        "let i=0;do{i++}while(false)\ni",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let mut legacy = program.clone();
        legacy.register_code = None;
        let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost)?;
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

#[test]
fn register_do_while_checks_fuel_at_the_condition_back_edge() -> Result<(), Error> {
    let limits = Limits {
        fuel: 4,
        ..Limits::default()
    };
    let program = compile("do{}while(true)", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    );
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
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    );
    let program = compile("while(true)continue", limits)?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::with_backend(limits, Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Limit {
            resource: "execution fuel"
        })
    );
    Ok(())
}

#[test]
fn register_loop_lowering_rejects_unstable_or_abrupt_bodies() -> Result<(), Error> {
    // The condition of this one assigns, so the loop never leaves the head.
    let source = "let x=1;while(x=true)x";
    assert!(
        !compile(source, Limits::default())?.uses_register_backend(),
        "{source}"
    );
    Ok(())
}

#[test]
fn a_loop_whose_body_changes_the_type_of_a_binding_lowers() -> Result<(), Error> {
    // The head starts from the types the body's assignments produce, so a
    // binding the body widens holds its type across the back edge. The fit
    // check is unchanged: it is what makes either set of types sound.
    for source in [
        "let x=1;while(false)x=true;x",
        "let x=1;while(true){x=true;break}x",
        "let x=1;let i=0;while(i<3){x=(i===1)?'s':x;i++}x",
        "let x=1;for(let i=0;i<3;i++){x=i<2?x:'done'}x",
        // A loop whose bindings do hold their types keeps them.
        "let x=1;for(let i=0;i<3;i++){x=x+1}x",
        "let s=0;for(let i=0;i<3;i++){s+=i}s",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn classic_for_loops_match_legacy_execution() -> Result<(), Error> {
    for source in [
        "let sum=0;for(let i=0;i<10;i++){sum+=i;}sum",
        "let sum=0;for(let {i,limit}={i:0,limit:4};i<limit;i++){sum+=i}sum",
        "let sum=0;for(let {i=0,step=2}={};i<5;i+=step){sum+=i}sum",
        "let count=0;for(const {x}={x:42};count<1;count++)x",
        "function f(){let sum=0;for(let {i,n}={i:0,n:3};i<n;i++){sum+=i}return sum}f()",
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost);
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
        "let x=1;for(let {x}={x:2};x<3;x++){}x",
        "for(const i=0;i<2;i++){}",
        "for(let i=0;i<2;i++){(()=>i)}",
        "for(let {i}={i:0};i<2;i++){(()=>i)}",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    assert!(matches!(
        compile("for(let {x}={x:1},x=2;x<3;x++){}", Limits::default()),
        Err(Error::Syntax { .. })
    ));
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
        let actual = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)?;
        assert!(
            same_value(&actual, &expected),
            "{source}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

fn differential(source: &str) -> Result<(), Error> {
    let program = compile(source, Limits::default())?;
    assert!(program.uses_register_backend(), "{source}");
    let mut legacy = program.clone();
    legacy.register_code = None;
    let expected = Runtime::new(Limits::default()).run(&legacy, &mut SilentHost);
    let actual =
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost);
    match (&actual, &expected) {
        (Ok(actual), Ok(expected)) => assert!(
            same_value(actual, expected),
            "{source}: {actual:?} != {expected:?}"
        ),
        (Err(Error::Thrown { value: actual }), Err(Error::Thrown { value: expected })) => {
            assert!(
                same_value(actual, expected),
                "{source}: {actual:?} != {expected:?}"
            );
        }
        _ => panic!("{source}: {actual:?} != {expected:?}"),
    }
    Ok(())
}

#[test]
fn register_throw_leaves_the_script_with_the_thrown_value() -> Result<(), Error> {
    for source in [
        "throw 42",
        "throw 'boom'",
        "throw true",
        "throw null",
        "throw undefined",
        "let x=41;throw x+1",
        "if(true){throw 1}else{throw 2}",
        "let i=0;while(true){i++;if(i===3)throw i}",
        "1;throw 2;3",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_try_catch_transfers_the_thrown_value_to_the_parameter() -> Result<(), Error> {
    for source in [
        "try{throw 42}catch(e){e}",
        "try{throw 'boom'}catch(e){e}",
        "try{1}catch(e){2}",
        "try{throw 1}catch(e){e+41}",
        "let x=0;try{throw 2}catch(e){x=e}x",
        "try{throw 1}catch{42}",
        "try{try{throw 1}catch(e){throw e+1}}catch(e){e}",
        "try{throw 1}catch(e){try{throw 2}catch(f){e+f}}",
        "let s=0;for(let i=0;i<3;i++){try{if(i===1)throw i;s+=10}catch(e){s+=e}}s",
        "let e=1;try{throw 2}catch(e){e}",
        "let e=1;try{throw 2}catch(f){f}e",
        "try{}catch(e){e}",
        // 14.15.3: an empty Block completes with undefined, not with the value
        // of the statement before the `try`.
        "1;try{}catch(e){}",
        "2;try{3}catch(e){}",
        "4;try{}catch(e){5}",
        "6;try{7}catch(e){8}",
        "1;try{throw null}catch(e){}",
        "2;try{throw null}catch(e){3}",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_try_catch_rethrows_when_the_handler_throws() -> Result<(), Error> {
    for source in [
        "try{throw 1}catch(e){throw e+1}",
        "try{throw 'a'}catch(e){throw e}",
        // A call inside the protected range unwinds through its frame.
        "function f(){throw 1}let x=0;try{f()}catch(e){x=2}x",
        "function f(){throw 1}let x=0;try{f()}finally{x=3}",
        "function f(){return 1}let x=0;try{f()}catch(e){x=2}x",
        "function g(){throw 5}function f(){g()}let x=0;try{f()}catch(e){x=1}x",
        "function f(){try{throw 1}catch(e){return e+1}return 0}f()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_lowering_rejects_exception_shapes_it_cannot_type() -> Result<(), Error> {
    for source in [
        // A destructuring catch parameter is not lowered.
        "try{throw [1]}catch([e]){e}",
        // The Block must not change a tracked binding type.
        "let x=1;try{x='a'}catch(e){e}x",
        // A Finally Block cannot run before a control transfer leaves it.
        "let i=0;while(i<2){try{i++;break}finally{i+=10}}i",
        "let i=0;while(i<2){try{i++}finally{continue}}i",
        "function f(){try{return 1}finally{2}}f()",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn register_try_finally_runs_the_finally_block_on_both_paths() -> Result<(), Error> {
    for source in [
        "try{1}finally{2}",
        "let x=0;try{x=1}finally{x+=10}x",
        "let x=0;try{throw 1}catch(e){x=e}finally{x+=10}x",
        "let x=0;try{x=1}catch(e){x=2}finally{x+=10}x",
        "try{throw 1}catch(e){e}finally{2}",
        "let x=0;try{try{throw 1}finally{x=5}}catch(e){x+e}",
        "let x=0;try{try{throw 1}catch(e){throw e+1}finally{x=5}}catch(e){x+e}",
        "let x=0;try{1}finally{x=2}x",
        "let x=0;try{throw 7}finally{x=1}",
        "let x=0;try{throw 7}catch(e){throw e+1}finally{x=1}",
        "1;try{}finally{}",
        "2;try{3}finally{}",
        "4;try{}finally{5}",
        "6;try{7}finally{8}",
        "1;try{}catch(e){}finally{}",
        "2;try{}catch(e){3}finally{}",
        "4;try{}catch(e){}finally{5}",
        "6;try{}catch(e){7}finally{8}",
        "9;try{10}catch(e){}finally{}",
        "11;try{12}catch(e){13}finally{}",
        "14;try{15}catch(e){}finally{16}",
        "17;try{18}catch(e){19}finally{20}",
        "1;try{throw null}catch(e){}finally{}",
        "2;try{throw null}catch(e){3}finally{}",
        "4;try{throw null}catch(e){}finally{5}",
        "6;try{throw null}catch(e){7}finally{8}",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_switch_falls_through_clauses_in_source_order() -> Result<(), Error> {
    for source in [
        "let x=0;switch(1){case 1:x=1;break;case 2:x=2;break}x",
        "let x=0;switch(2){case 1:x=1;break;case 2:x=2;break}x",
        "let x=0;switch(3){case 1:x=1;break;default:x=9;break}x",
        "let x=0;switch(1){case 1:x+=1;case 2:x+=2;case 3:x+=4}x",
        "let x=0;switch(9){case 1:x+=1;default:x+=2;case 3:x+=4}x",
        "let x=0;switch('a'){case 'a':x=1;break;case 'b':x=2}x",
        "let x=0;switch(1){}x",
        "let x=0;switch(1){default:x=5}x",
        "switch(1){case 1:42}",
        "switch(2){case 1:42}",
        "0;switch(2){case 1:42}",
        "switch(1){case 1:1;case 2:2}",
        "let n=0;let k=()=>{n++;return 1};switch(1){case k():n+=10;break;case k():n+=20}n",
        "let n=0;switch(3){case (n+=1):break;case (n+=2):break;default:n+=100}n",
        "let s=0;for(let i=0;i<4;i++){switch(i){case 1:continue;case 2:s+=10;break;default:s+=1}s+=100}s",
        "let s=0;let i=0;while(i<3){i++;switch(i){case 2:continue;default:s+=i}}s",
        // 14.12.4 accumulates V across clauses; a clause is entered without the
        // discriminant in the completion.
        "1;switch('a'){default:break}",
        "2;switch('a'){default:{3;break}}",
        "4;do{switch('a'){default:{continue}}}while(false)",
        "5;do{switch('a'){default:{6;continue}}}while(false)",
        "7;switch('a'){case 'a':break}",
        "8;do{switch('a'){case 'a':{continue}}}while(false)",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_lowering_rejects_case_blocks_with_lexical_declarations() -> Result<(), Error> {
    for source in [
        "switch(1){case 1:let x=1;break}",
        "switch(1){default:function f(){}}",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn register_for_in_walks_own_keys_before_the_prototype_chain() -> Result<(), Error> {
    for source in [
        "let s='';for(const k in {a:1,b:2}){s+=k}s",
        "let s='';for(let k in {a:1,b:2,c:3}){s+=k}s",
        "let s='';for(const k in {}){s+=k}s",
        "let n=0;for(const k in {a:1,b:2}){n++}n",
        "let s='';for(const k in [10,20,30]){s+=k}s",
        "let s='';for(const k in {2:'c',1:'b',0:'a'}){s+=k}s",
        "let s='';for(const k in {b:1,a:2,1:3,0:4}){s+=k}s",
        "let s='';for(const k in {a:1,b:2}){if(k==='a')continue;s+=k}s",
        "let s='';for(const k in {a:1,b:2}){s+=k;break}s",
        "let s='';for(const k in {a:1}){for(const j in {b:2}){s+=k+j}}s",
        "for(const k in {a:1}){k}",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_for_of_iterates_an_array_through_its_iterator() -> Result<(), Error> {
    for source in [
        "let s=0;for(const x of [1,2,3]){s+=x}s",
        "let s='';for(const x of ['a','b']){s+=x}s",
        "let n=0;for(const x of ['a','bb']){n+=x.length}n",
        // A hole leaves undefined in the element type.
        "let n=0;for(const x of [1,,3]){n+=x}n",
        "let n=0;for(const x of []){n++}n",
        "let s=0;for(let x of [1,2,3]){s+=x;if(x===2)break}s",
        "let s=0;for(const x of [1,2,3]){if(x===2)continue;s+=x}s",
        "let a=[1,2];let s=0;for(const x of a){s+=x}s",
        "for(const x of [7]){x}",
        "let n=0;for(const x of [1,2]){}n",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_array_search_methods_answer_on_the_new_engine() -> Result<(), Error> {
    for source in [
        // 23.1.3.17: IsStrictlyEqual, ascending, and -1 for no match.
        "[1,2,3].indexOf(2)",
        "[1,2,3].indexOf(4)",
        "[1,2,3].indexOf(2,2)",
        "[1,2,3].indexOf(3,-1)",
        "[1,2,3].indexOf(1,-99)",
        "[NaN].indexOf(NaN)",
        "['a','b'].indexOf('b')",
        "[].indexOf(undefined)",
        "[undefined].indexOf(undefined)",
        "[,1].indexOf(undefined)",
        // 23.1.3.20: the same, descending, from the last index.
        "[1,2,1].lastIndexOf(1)",
        "[1,2,1].lastIndexOf(1,1)",
        "[1,2,1].lastIndexOf(1,-1)",
        "[1,2,1].lastIndexOf(1,-99)",
        "[1,2,1].lastIndexOf(9)",
        "[].lastIndexOf(1)",
        // 23.1.3.16: SameValueZero, and a hole reads as undefined.
        "[1,2,3].includes(2)",
        "[1,2,3].includes(4)",
        "[NaN].includes(NaN)",
        "[0].includes(-0)",
        "[1,2,3].includes(1,-2)",
        "[,1].includes(undefined)",
        "[].includes(undefined)",
        // 23.1.3.1: a negative index counts from the end.
        "[1,2,3].at(0)",
        "[1,2,3].at(-1)",
        "[1,2,3].at(1.7)",
        "[1,2,3].at(-4)",
        "[].at(0)",
        "[1,2,3].at('1')",
        // The receiver keeps its layout, so a later read still lowers.
        "let a=[1,2,3];let i=a.indexOf(3);a[i]",
        "let a=['x'];a.includes('x')?a.length:0",
        // A written index leaves the length unknown but not the method.
        "let a=[1,2];let i=1;a[i]=3;a.indexOf(3)",
        "let a=[1,2];let i=1;a[i]=3;a.includes(3)",
        "let a=[1,2];let i=1;a[i]=3;a.lastIndexOf(2)",
        "let a=[1,2];let i=5;a[i]=3;a.at(1)",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_array_join_concatenates_the_elements() -> Result<(), Error> {
    for source in [
        // 23.1.3.18: the separator defaults to a comma.
        "[1,2,3].join()",
        "[1,2,3].join('-')",
        "[1,2].join('')",
        "[1].join('-')",
        "[].join('-')",
        // undefined, null and a hole contribute the empty String.
        "[null,undefined,1].join()",
        "[1,,3].join('-')",
        "[,].join('-')",
        // Every other element is converted with ToString.
        "[true,false].join('|')",
        "['a','b'].join('')",
        "[1.5,-0,NaN].join(',')",
        "[1,2].join(3)",
        "let a=[1,2];let i=1;a[i]=3;a.join('-')",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_array_push_and_pop_move_the_last_element() -> Result<(), Error> {
    for source in [
        // 23.1.3.23 answers the new length and 23.1.3.22 the element taken.
        "let a=[1,2];a.push(3)",
        "let a=[1,2];a.push(3);a.join('-')",
        "let a=[1];a.push(2,3)",
        "let a=[1];a.push(2,3);a.length",
        "let a=[];a.push('x');a.join()",
        "let a=[1];a.push()",
        "let a=[1,2];a.pop()",
        "let a=[1,2];a.pop();a.length",
        "let a=[1,2];a.pop();a.join('-')",
        "let a=[];a.pop()",
        "let a=[];a.pop();a.length",
        "let a=[1,2,3];a.pop();a.pop();a.join()",
        "let a=[1];a.push(2);a.pop()",
        "let a=[1];a.pop();a.push(9);a.at(0)",
        // A hole at the end is taken as undefined and shortens the Array.
        "let a=[1,,];a.pop();a.length",
        // The element keeps its own type, not the merged one.
        "let a=[1,'b'];a.pop()",
        "let a=[1,'b'];a.pop();a.pop()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_array_reverse_mirrors_the_indices() -> Result<(), Error> {
    for source in [
        // 23.1.3.26 mirrors the indices and answers the receiver.
        "let a=[1,2,3];a.reverse();a.join('-')",
        "let a=[1,2];a.reverse().join()",
        "let a=[1];a.reverse().join()",
        "let a=[];a.reverse().join()",
        "let a=[1,,3];a.reverse().join('-')",
        "let a=[1,2,'c'];a.reverse();a.at(0)",
        // The layout follows the mirroring, so a later read still lowers.
        "let a=[1,2,3];a.reverse();a.pop();a.push(9);a.join('-')",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_iteration_survives_a_collection() -> Result<(), Error> {
    for source in [
        // Every step of 7.4.8 allocates a result object and every level of
        // 14.7.5.9 an Array of keys, so these loops cross a scavenge.
        "let n=0;let i=0;while(i<400){for(const x of [1,2,3]){n+=x}i=i+1}n",
        "let n=0;let i=0;while(i<400){for(const x of ['a','bb']){n+=x.length}i=i+1}n",
        "let n=0;let i=0;while(i<400){for(const k in {a:1,b:2}){n=n+1}i=i+1}n",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_array_slice_copies_a_range_into_a_new_array() -> Result<(), Error> {
    for source in [
        // 23.1.3.28: both ends are clamped and a negative one counts from
        // the end.
        "[1,2,3].slice(1).join('-')",
        "[1,2,3].slice().join('-')",
        "[1,2,3].slice(0,2).join('-')",
        "[1,2,3].slice(-2).join('-')",
        "[1,2,3].slice(1,-1).join('-')",
        "[1,2,3].slice(5).join('-')",
        "[1,2,3].slice(2,1).join('-')",
        "[1,2,3].slice(-99,99).join('-')",
        "[].slice(0).join('-')",
        // A hole stays a hole in the copy.
        "[1,,3].slice(0).join('-')",
        "[1,,3].slice(1).length",
        // The copy is an Array of the same Realm, so its own methods answer.
        "[1,2,3].slice(1).indexOf(3)",
        "let a=[1,2,3];let b=a.slice(1);b.length",
        "let a=[1,2,3];let b=a.slice(1);a.join('-')",
        "let a=['x','y'];a.slice(0).join('')",
        // The elements are read, not shared: a later change to the receiver
        // leaves the copy alone.
        "let a=[1,2];let b=a.slice(0);a.pop();b.join('-')",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn an_if_branch_starts_its_completion_at_undefined() -> Result<(), Error> {
    for source in [
        // 14.6.2 answers UpdateEmpty(stmtCompletion, undefined), so a break
        // that leaves an if carries undefined and not the value the enclosing
        // statement list reached.
        "let i=4;while(true){i++;if(i)break}",
        "let i=4;while(true){i++;if(i){break}}",
        "let i=4;while(true){i++;if(!i)0;else break}",
        "let i=4;while(true){i++;if(i)if(i)break}",
        // A value inside the branch is the one it carries.
        "let i=4;while(true){i++;if(i){5;break}}",
        "let i=4;while(true){i++;if(!i)0;else{5;break}}",
        // A break that is not inside an if still carries the statement list.
        "let i=4;while(true){i++;break}",
        "let i=4;while(true){i++;{break}}",
        // The same for continue, which the condition then ends.
        "let i=0;while(i<3){i++;if(i)continue}",
        "let i=0;while(i<3){i++;continue}",
        // A branch that completes normally is unchanged.
        "2;if(true){}",
        "2;if(true){3}",
        "2;if(false){3}",
        "2;if(false){3}else{4}",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_compound_assignment_of_two_strings_only_concatenates_for_plus() -> Result<(), Error> {
    for source in [
        // 13.15.3: only + concatenates; every other operator applies
        // ToNumeric to both operands first.
        "let s;s='x';s+='x'",
        "let s;s='x';s*='x'",
        "let s;s='x';s-='x'",
        "let s;s='x';s/='x'",
        "let s;s='x';s%='x'",
        "let s='2';s*='3'",
        "let s='6';s%='4'",
        "let s='6';s-='4'",
        "let s='6';s/='4'",
        "let s='2';s**='3'",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_lowering_rejects_intrinsic_arguments_it_cannot_coerce() -> Result<(), Error> {
    for source in [
        // An intrinsic runs without a call frame, so a user valueOf in a
        // coerced argument position keeps the call on the legacy backend.
        "let n=0;'abc'.charAt({valueOf(){n++;return 1}})",
        "let n=0;[1,2].at({valueOf(){n++;return 0}})",
        "let n=0;[1,2].indexOf(1,{valueOf(){n++;return 0}})",
        "let n=0;({}).hasOwnProperty({toString(){n++;return 'a'}})",
        "let n=0;[1].join({toString(){n++;return '-'}})",
        // 23.1.3.18 applies ToString to every element, which needs a frame.
        "[{}].join('-')",
        "[[1]].join('-')",
        // 23.1.3.22 and 23.1.3.23 need the length the layout starts from.
        "let a=[1];let i=0;a[i]=2;a.pop()",
        "let a=[1];let i=0;a[i]=2;a.reverse()",
        "let a=[1];let i=0;a[i]=2;a.push(3)",
        // A key that is not a Number is not lowered on an Array at all.
        "let a=[1];let k='indexOf';a[k]=1;a.indexOf",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
    }
    Ok(())
}

#[test]
fn a_for_in_head_reaches_the_binding_the_declaration_made() -> Result<(), Error> {
    // 8.2.7 makes a `var` head a var name of the body, and 14.7.5.5 gives it no
    // binding of its own: the loop writes the one the declaration made.
    differential("let o={a:1,b:2};let s='';for(var k in o){s+=k}s")?;
    differential("let f=function(o){var s='';for(var k in o){s+=k}return s};f({a:1,b:2})")?;
    differential("var k=1;let o={a:1};let s='';for(var k in o){s+=k}s+k")?;
    // An enumeration that produced nothing leaves the declaration's value.
    differential("let s='';for(var k in {}){s+=k}typeof k")?;
    differential("let a=[1,2];let s='';for(var i in a){s+=i}s")?;
    // A head the lowering could not name is enumerated at run time.
    differential("let f=function(o){var s='';for(var k in o){s+=k}return s};f([1,2])")?;
    // A closure holds the binding the head writes, so the body is not lowered.
    for source in [
        "var k=9;let g=function(){return k};let o={a:1};for(var k in o){}g()",
        "var k=9;let g=function(){return k};let o={a:1};for(var k in o){}k",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn register_lowering_rejects_for_in_heads_it_cannot_model() -> Result<(), Error> {
    // 14.7.5.6 decides the head at run time: null and undefined enumerate
    // nothing, and a primitive needs a ToObject this engine has no wrapper
    // Object for, which the instruction names where it happens.
    differential("let s='';for(const k in null){s+=k}s")?;
    differential("let s='';for(const k in undefined){s+=k}s")?;
    let program = compile("for(const k in 'ab'){}", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    for source in [
        // An assignment head writes an existing reference.
        "let k;for(k in {a:1}){}",
        // A destructuring head is not lowered.
        "for(const [k] in {a:1}){}",
        // A captured per-iteration binding needs a context of its own.
        "for(const k in {a:1}){(()=>k)}",
        "let k;for(k of [1]){}",
        // A destructuring head is not lowered.
        "for(const [a] of [[1]]){}",
        // A per-iteration binding captured by a closure needs a context.
        "for(const x of [1]){(()=>x)}",
        // A head that shadows a binding of the enclosing scope is not lowered.
        "let x=1;for(const x of [2]){}x",
        // The body must not change the Array's tracked layout.
        "let a=[1];for(const x of a){a[1]='s'}",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn register_object_prototype_methods_run_as_native_intrinsics() -> Result<(), Error> {
    for source in [
        "let o={a:1};o.hasOwnProperty('a')",
        "let o={a:1};o.hasOwnProperty('b')",
        "let o={};o.hasOwnProperty('a')",
        "let o={a:1,b:2};o.hasOwnProperty('b')",
        "let o={a:1};o.hasOwnProperty()",
        "let o={undefined:1};o.hasOwnProperty(undefined)",
        "let o={a:1};o.hasOwnProperty('a')&&o.hasOwnProperty('a')",
        "let o={a:1};o.propertyIsEnumerable('a')",
        "let o={};o.toString()",
        "let o={};o.toString()==='[object Object]'",
        "let o={a:1};o.propertyIsEnumerable('b')",
        "let o={};o.propertyIsEnumerable('toString')",
        "let o={a:1};o.isPrototypeOf({})",
        "let o={a:1};o.isPrototypeOf(1)",
        "let o={a:1};o.isPrototypeOf('a')",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        let code = program
            .register_code
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        assert!(
            core::iter::once(code.as_ref())
                .chain(code.functions.iter())
                .flat_map(|function| &function.instructions)
                .any(|instruction| matches!(
                    instruction,
                    crate::engine::bytecode::Instruction::CallMethod { .. }
                )),
            "{source}"
        );
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_lowering_rejects_reads_the_prototype_chain_cannot_answer() -> Result<(), Error> {
    for source in [
        // 20.1.3 names %Object.prototype% owns whose intrinsic does not exist yet.
        "let o={};o.valueOf",
        // 23.1.3 names %Array.prototype% owns, none of which exists yet.
        "let a=[1];a.concat",
        "let a=[1];a.indexOf",
        "let a=[1];a['push']",
        "let a=[1];a.constructor",
        "let o={};o['toString']",
        // An intrinsic is only lowered at a call site.
        "let o={a:1};o.hasOwnProperty",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn an_engine_error_reaches_the_embedding_as_the_same_error_type() -> Result<(), Error> {
    use crate::engine::bytecode::{BytecodeFunction, FeedbackKind, Instruction, Reg};

    // A Program whose register bytecode calls a value that is not callable.
    // 13.3.6.1 makes that a TypeError, and the embedding has to see one.
    let mut code = BytecodeFunction::new(2, 0);
    let callee = Reg(0);
    let argument = Reg(1);
    let slot = code.allocate_feedback_slot(FeedbackKind::Call);
    code.emit(Instruction::LdaSmi(1));
    code.emit(Instruction::Star(callee));
    code.emit(Instruction::LdaUndefined);
    code.emit(Instruction::Star(argument));
    code.emit(Instruction::Call {
        func: callee,
        arg_start: argument,
        arg_count: 0,
        slot,
    });
    code.emit(Instruction::Return);

    let mut script = compile_script("1", Limits::default())?;
    script.program.register_code = Some(alloc::rc::Rc::new(code));
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    let error = realm
        .evaluate_compiled(&script)
        .expect_err("a Smi is not callable");
    assert!(
        matches!(error, Error::Type { .. }),
        "expected a TypeError, got {error:?}"
    );
    Ok(())
}

#[test]
fn register_string_members_answer_length_and_indices() -> Result<(), Error> {
    for source in [
        "'abc'.length",
        "''.length",
        "'abc'[0]",
        "'abc'[2]",
        "'abc'[3]",
        "'abc'[-1]",
        "let s='hello';s.length",
        "let s='hello';let i=1;s[i]",
        "let s='hello';s['length']",
        "let s='hello';s[0]+s[1]",
        "let s='ab';let n=0;for(let i=0;i<s.length;i++){n++}n",
        "'abc'.missing",
        "let s='abc';s['missing']",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn register_string_methods_run_as_native_intrinsics() -> Result<(), Error> {
    for source in [
        "'abc'.charAt(0)",
        "'abc'.charAt(2)",
        "'abc'.charAt(3)",
        "'abc'.charAt(-1)",
        "'abc'.charAt()",
        "'abc'.charCodeAt(0)",
        "'abc'.charCodeAt(9)",
        "'abc'.indexOf('b')",
        "'abc'.indexOf('z')",
        "'abcabc'.indexOf('b',2)",
        "'abc'.indexOf('')",
        "''.indexOf('a')",
        "let s='hello';s.charAt(1)+s.charAt(0)",
        "let s='hello';s.indexOf('l')",
        "'abc'.at(-1)",
        "'abc'.at(0)",
        "'abc'.at(9)",
        "'ab'.concat('c','d')",
        "'abc'.endsWith('c')",
        "'abc'.endsWith('a')",
        "'abc'.endsWith('a',1)",
        "'abc'.includes('b')",
        "'abc'.includes('z')",
        "'abcabc'.lastIndexOf('b')",
        "'abcabc'.lastIndexOf('b',2)",
        "'ab'.repeat(3)",
        "'ab'.repeat(0)",
        "''.repeat(5)",
        "'abcdef'.slice(1,3)",
        "'abcdef'.slice(-2)",
        "'abcdef'.slice(4,2)",
        "'abc'.startsWith('ab')",
        "'abc'.startsWith('b',1)",
        "'abcdef'.substring(4,2)",
        "'abcdef'.substring(1)",
        "let s='hello world';s.slice(s.indexOf(' ')+1)",
        "'abc'.codePointAt(0)",
        "'abc'.codePointAt(9)",
        "'\\u{1f600}x'.codePointAt(0)",
        "'\\u{1f600}x'.codePointAt(1)",
        "'5'.padStart(3,'0')",
        "'5'.padStart(3)",
        "'5'.padStart(1,'0')",
        "'5'.padEnd(4,'ab')",
        "'5'.padEnd(4,'')",
        "'  ab  '.trim()",
        "'  ab  '.trimStart()",
        "'  ab  '.trimEnd()",
        "'\\n\\tab'.trim()",
        "'   '.trim()",
        "''.trim()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_string_constructor_answers_the_primitive_a_call_makes() -> Result<(), Error> {
    // 22.1.1.1: a call with no argument is the empty String, and every other
    // value goes through ToString. A method read off a String resolves on
    // %String.prototype% (10.4.3), which the instruction walks.
    for source in [
        "typeof String",
        "String(42)",
        "String()",
        "String(true)",
        "String(null)",
        "String.length",
        "String('a')+String(1)",
        "\"\".constructor===String",
        "String.prototype.constructor===String",
        "typeof ''.charAt",
        "String.prototype.charAt===''.charAt",
        "var m='abc'.charAt; typeof m",
        "'abc'['charAt'](1)",
        "typeof ''.notAName",
        // 22.1.3 names %String.prototype% owns, read rather than called, and
        // a String key that names one of them.
        "typeof 'abc'.slice",
        "typeof 'abc'['slice']",
        "typeof 'abc'.toString",
        "let s='abc';let k='length';s[k]",
        // 22.1.3 begins every method with RequireObjectCoercible and
        // ToString, so the receiver need not already be a String.
        "String.prototype.charAt.call(42,0)",
        "String.prototype.charAt.call(true,1)",
        "var r=0;try{String.prototype.charAt.call(null,0)}catch(e){r=e instanceof TypeError}r",
    ] {
        differential(source)?;
    }
    // `new` makes the String exotic object of 10.4.3, and 22.1.2 gives
    // `%String%` statics this Realm has not built. Both are gaps, because an
    // answer here would be the wrong one.
    for source in [
        "new String('x')",
        "String.fromCharCode",
        "String.raw",
        "''.valueOf",
        // A name %String.prototype% owns and this Realm has not built is the
        // same gap read on the Prototype itself as read on a String.
        "String.prototype.anchor",
        "String.prototype.trimLeft",
        "'abc'.split",
        // ToString of an Object would run a method of the Script.
        "String({})",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_script_the_lowering_refuses_says_what_it_holds() -> Result<(), Error> {
    // A refusal that names nothing cannot be prioritised, and it reads as if
    // the Script were at fault. Each one names the construct it stopped at.
    // A refusal the passes before the lowering raise still has no name.
    for (source, feature) in [
        ("var o={get x(){return 1}}; o.x", "an object literal"),
        ("class C{}", "a declaration of the Script"),
        // 10.2.11 does more for these parameter lists than the lowering does,
        // and the body is lowered in a unit of its own, so both name what
        // stopped them and not the expression the function was written as.
        ("var f=function(...r){return r}; f()", "a rest parameter"),
        (
            "var f=function([a]){return a}; f([1])",
            "a parameter that is a binding pattern",
        ),
        (
            "var f=function(){try{return 1}finally{}}; f()",
            "a try statement",
        ),
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { feature: named }) if named == feature
            ),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_top_level_this_of_a_script_is_the_global_object() -> Result<(), Error> {
    // 9.4.2 resolves `this` on the Environment Record that has one. At the
    // top level of a Script that is the Global Environment Record, whose
    // [[GlobalThisValue]] (9.1.1.4.11) is the global object.
    for source in [
        "typeof this",
        "this.Array === Array",
        "var g = this; typeof g.String",
        "var self = this; typeof self",
        "function f(){ return this === undefined } f()",
    ] {
        differential_scripts(&[source])?;
    }
    // 19.1 gives the global object properties this Realm reaches through the
    // Global Environment Record, so reading one off the object is a gap and
    // not the undefined of an object that does not have it.
    for source in ["this.undefined", "this.Infinity", "this.NaN"] {
        let mut host = SilentHost;
        let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
        assert!(
            matches!(realm.evaluate(source), Err(Error::Unsupported { .. })),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_object_constructor_answers_what_20_1_2_asks_of_it() -> Result<(), Error> {
    // 20.1.2.2 makes an object under a Prototype and 20.1.2.3 defines the
    // descriptors of a source on it; 20.1.2.12 answers the [[Prototype]],
    // 20.1.2.19 the own enumerable String keys, 20.1.2.14 SameValue and
    // 20.1.2.13 HasOwnProperty without the Prototype Chain.
    for source in [
        "Object.is(NaN,NaN)",
        "Object.is(0,-0)",
        "Object.is(-0,-0)",
        "Object.is(1,1)",
        "Object.is('a','a')",
        "Object.hasOwn({a:1},'a')",
        "Object.hasOwn({a:1},'b')",
        "Object.hasOwn([1],'length')",
        "var o=Object.create(null); typeof o",
        "var o=Object.create({x:1}); o.x",
        "var o=Object.create({},{a:{value:5}}); o.a",
        "var o={}; Object.defineProperties(o,{a:{value:1},b:{value:2}}); o.a+o.b",
        "Object.getPrototypeOf([])===Array.prototype",
        "Object.getPrototypeOf({})===Object.prototype",
        "Object.keys({a:1,b:2}).join(',')",
        "Object.keys({}).length",
        "var o={}; Object.defineProperty(o,'h',{value:1}); Object.keys(o).length",
        "Object.getOwnPropertyNames({a:1}).join(',')",
        "Object.create(Object.prototype).toString()",
        "Object.length",
    ] {
        differential(source)?;
    }
    // 17 counts the one argument of the heading of 20.1.2.19, so `keys` has a
    // length of one. The stack backend answers zero, as it does for `call`,
    // which is why this is not compared against it.
    let program = compile("Object.keys.length", Limits::default())?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost)?,
        Value::Number(1.0)
    );
    // 20.1.2.2 and 20.1.2.3 refuse what is not an object.
    for source in [
        "var r=0;try{Object.create(1)}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{Object.defineProperties(1,{})}catch(e){r=e instanceof TypeError}r",
    ] {
        differential(source)?;
    }
    // 20.1.2.3.2 reads every descriptor before it defines any, so a source
    // whose second entry is no descriptor leaves the object untouched. The
    // lowering does not take a `try` that holds an object literal, so the two
    // Scripts run in one realm instead.
    differential_scripts(&[
        "var o={};var s={a:{value:1},b:2};var caught=0;",
        "try{Object.defineProperties(o,s)}catch(e){caught=1}",
        "caught+(o.a===undefined)",
    ])?;
    Ok(())
}

#[test]
fn the_array_methods_that_move_elements_answer_what_23_1_3_asks() -> Result<(), Error> {
    // 23.1.3.31 answers the removed elements and closes the distance the
    // arguments leave, 23.1.3.7 fills a range, 23.1.3.4 copies one range of
    // the Array over another, 23.1.3.2 flattens one level, and 23.1.3.39 and
    // 23.1.3.33 copy instead of moving.
    for source in [
        "var a=[1,2,3,4]; a.splice(1,2).join(',')+'|'+a.join(',')",
        "var a=[1,2,3]; a.splice(1,0,9).length+'|'+a.join(',')",
        "var a=[1,2,3,4]; a.splice(1,1,7,8).join(',')+'|'+a.join(',')",
        "var a=[1,2,3]; a.splice(1).join(',')+'|'+a.join(',')",
        "var a=[1,2,3]; a.splice(-1).join(',')+'|'+a.join(',')",
        "var a=[1,2,3]; a.splice(0,0).length+'|'+a.join(',')",
        "[1,2,3].fill(0,1).join(',')",
        "[1,2,3].fill(7).join(',')",
        "[1,2,3].fill(7,-2,-1).join(',')",
        "[1,2,3,4,5].copyWithin(0,3).join(',')",
        "[1,2,3,4,5].copyWithin(1,3,4).join(',')",
        "[1,2].concat([3,4],5).join(',')",
        "[1].concat().length",
        "[1,2,3].with(1,9).join(',')",
        "[1,2,3].with(-1,9).join(',')",
        "[1,2,3].toReversed().join(',')",
        "[].toReversed().length",
    ] {
        differential(source)?;
    }
    // 23.1.3.39 step 5 refuses an index outside the Array. The lowering does
    // not take a `try` that holds an array literal, so this runs as Scripts
    // of one realm instead.
    differential_scripts(&[
        "var a=[1,2];var r=0;",
        "try{a.with(5,0)}catch(e){r=e instanceof RangeError}",
        "r",
    ])?;
    // 23.1.3.27 and 23.1.3.37 move every element of the Array, and the stack
    // backend has neither, so these are asserted against the clause instead.
    for (source, expected) in [
        ("var a=[1,2,3]; a.shift()", Value::Number(1.0)),
        ("var a=[1,2,3]; a.shift(); a.length", Value::Number(2.0)),
        ("var a=[]; a.shift(); a.length", Value::Number(0.0)),
        ("var a=[1]; a.unshift(9,8)", Value::Number(3.0)),
        ("var a=[1]; a.unshift(); a.length", Value::Number(1.0)),
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert_eq!(
            Runtime::with_backend(Limits::default(), Backend::Engine)
                .run(&program, &mut SilentHost)?,
            expected,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_math_functions_of_21_3_2_that_need_no_library() -> Result<(), Error> {
    // 21.3.1 gives %Math% eight values, and 21.3.2 the functions that need
    // nothing transcendental: the magnitude, the three roundings, the sign,
    // the two extrema and the three that work on 32-bit integers.
    for source in [
        "Math.PI",
        "Math.E",
        "Math.LN10",
        "Math.LN2",
        "Math.LOG10E",
        "Math.LOG2E",
        "Math.SQRT1_2",
        "Math.SQRT2",
        "Math.abs(-3)",
        "Math.floor(-1.5)",
        "Math.ceil(-1.5)",
        "Math.trunc(-1.7)",
        "Math.round(-0.5)",
        "Math.round(0.5)",
        "Math.round(2.5)",
        "Math.round(-2.5)",
        "Math.sign(-3)",
        "Math.max(1,2,3)",
        "Math.max()",
        "Math.min()",
        "Math.max(1,NaN)",
        "Math.min('2',3)",
        "Math.clz32(1)",
        "Math.clz32(0)",
        "Math.sin(0)",
        "Math.abs.length",
        "Math.max.length",
    ] {
        differential(source)?;
    }
    // 6.1.6.1 tells the two zeroes apart, and 21.3.2 says which one each of
    // these answers. The printed form does not, so 20.1.2.14 is asked.
    for source in [
        "Object.is(Math.ceil(-0.5),-0)",
        "Object.is(Math.round(-0.2),-0)",
        "Object.is(Math.min(0,-0),-0)",
        "Object.is(Math.max(-0,0),0)",
        "Object.is(Math.abs(-0),0)",
        "Object.is(Math.trunc(-0.5),-0)",
        "Object.is(Math.sign(-0),-0)",
    ] {
        differential(source)?;
    }
    // 21.3.2.19 and 21.3.2.17 the stack backend does not have, so these are
    // asserted against the clause instead of compared.
    for (source, expected) in [
        ("Math.imul(3,4)", Value::Number(12.0)),
        ("Math.imul(-5,12)", Value::Number(-60.0)),
        ("Math.fround(5.5)", Value::Number(5.5)),
        ("Math.fround(5.05)", Value::Number(5.050_000_190_734_863)),
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert_eq!(
            Runtime::with_backend(Limits::default(), Backend::Engine)
                .run(&program, &mut SilentHost)?,
            expected,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_array_methods_of_23_1_3_that_ask_the_script() -> Result<(), Error> {
    // Each element is a call, so the engine leaves the method to make it and
    // comes back through the frame the call opened. What the walk has reached
    // lives in an object of the heap, which the collector traces.
    for source in [
        "[1,2,3].map(function(x){return x*2}).join(',')",
        "[1,2,3,4].filter(function(x){return x>2}).join(',')",
        "var a=[];[1,2,3].forEach(function(x){a.push(x)});a.join(',')",
        "typeof [1,2].forEach(function(x){})",
        "[1,2,3].every(function(x){return x>0})",
        "[1,2,3].every(function(x){return x>1})",
        "[1,2,3].some(function(x){return x>2})",
        "[1,2,3].some(function(x){return x>3})",
        "[1,2,3].find(function(x){return x>1})",
        "[1,2,3].find(function(x){return x>9})",
        "[1,2,3].findIndex(function(x){return x>1})",
        "[1,2,3].findIndex(function(x){return x>9})",
        "[1,2,3].reduce(function(a,b){return a+b})",
        "[1,2,3].reduce(function(a,b){return a+b},10)",
        "[].reduce(function(a,b){return a+b},7)",
        // 23.1.3 gives the callback the element, its index and the object.
        "[1,2,3].map(function(x,i,a){return i+':'+a.length}).join(',')",
        // The second argument is the `this` of the callback.
        "var o={v:10};[1,2,3].map(function(x){return this.v+x},o).join(',')",
        // A hole is not an index the object has, so no callback sees it.
        "[1,,3].map(function(x){return x*2}).length",
        "[].map(function(x){return x}).length",
        "[1,2,3].map(function(x){return x}).constructor===Array",
        "[[1],[2]].map(function(x){return x[0]}).join(',')",
        // A walk inside a walk keeps its own state.
        "[1,2].map(function(x){return [10,20].map(function(y){return x*y}).join('+')}).join(',')",
    ] {
        differential(source)?;
    }
    // 23.1.3.25 the stack backend does not have, and 23.1.3.24 step 6 and the
    // callback check of step 3 are raised where a `try` reaches them.
    let program = compile(
        "[1,2,3].reduceRight(function(a,b){return a+'-'+b})",
        Limits::default(),
    )?;
    assert!(program.uses_register_backend());
    assert_eq!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost)?,
        Value::string("3-2-1")
    );
    differential_scripts(&[
        "var one=[1];var none=[];var caught=0;",
        "try{one.map(1)}catch(e){caught=e instanceof TypeError}",
        "caught",
    ])?;
    differential_scripts(&[
        "var none=[];var caught=0;var keep=function(a,b){return a};",
        "try{none.reduce(keep)}catch(e){caught=e instanceof TypeError}",
        "caught",
    ])?;
    Ok(())
}

#[test]
fn a_parameter_takes_its_initializer_where_the_call_passed_none() -> Result<(), Error> {
    // 8.6.2 runs the Initializer of a parameter only where the argument is
    // undefined, which is what a call that passed too few leaves. They run
    // left to right, so a later one reads what an earlier one bound, and
    // 15.1.5 stops counting the `length` of 10.2.9 at the first of them.
    for source in [
        "function f(a=1){return a};f()",
        "function f(a=1){return a};f(5)",
        "function f(a=1){return a};f(undefined)",
        "function f(a=1){return a};f(null)",
        "function f(a=1){return a};f(0)",
        "function f(a,b=a+1){return b};f(2)",
        "function f(a=1,b=2){return a+b};f()",
        "function f(a=1,b=2){return a+b};f(10)",
        "var g=function(a=7){return a};g()",
        "function f(a=1){return arguments.length};f()",
        "function f(a=1){return arguments.length};f(1,2)",
        "function f(a){return a};f.length",
        "function f(a=1){return a};f.length",
        "function f(a,b=2){return a};f.length",
        "function f(a,b,c){return a};f.length",
        "function f(){return 1};f.length",
        // The Initializer is an expression, so it can call.
        "function one(){return 1};function f(a=one()){return a};f()",
        "function f(a={}){return typeof a};f()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_for_of_takes_a_var_head_as_a_for_in_does() -> Result<(), Error> {
    // 14.7.5 makes one binding per iteration for a lexical head and writes the
    // one the declaration made for a `var` head, which is the same difference
    // a `for`-`in` has. What the loop left is what the binding holds after it.
    for source in [
        "var s=0;for(var x of [1,2,3]){s+=x}s",
        "var a=[1,2];var s=0;for(var x of a){s+=x}s",
        "var s=0;for(var x of [1,2,3]){if(x==2)continue;s+=x}s",
        "var s=0;for(var x of [1,2,3]){if(x==2)break;s+=x}s",
        "var x=9;for(var x of []){}x",
        "var x=9;for(var x of [1,2]){}x",
        "var s='';for(var x of ['a','b']){s+=x}s",
        "var s=0;for(let x of [1,2,3]){s+=x}s",
        "var n=0;for(var x of []){n+=1}n",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_for_of_walks_an_iterable_by_the_protocol_of_7_4() -> Result<(), Error> {
    // An Array is stepped by an instruction that needs no call. Every other
    // iterable is walked by 7.4 itself: 7.4.2 reads @@iterator and calls it,
    // 7.4.6 calls `next` and asks the result whether it is `done`, and 7.4.7
    // reads `value` only where it is not.
    for source in [
        "var s=0;for(var x of [1,2,3].values()){s+=x}s",
        "var s=0;for(let x of [1,2].values()){s+=x}s",
        "var n=0;for(var x of [].values()){n+=1}n",
        "var s=0;var a=[1,2,3];for(var x of a){s+=x}s",
    ] {
        differential(source)?;
    }
    // 22.1.3.34 gives a String an iterator this Realm has not built, and
    // answering undefined would say a String is not iterable.
    for source in ["for(var c of 'ab'){}", "for(const c of 'ab'){}"] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    // A body that leaves by `break` or `return` would have to close the
    // iterator (7.4.9), which the lowering does not emit.
    for source in [
        "for(var x of [1].values()){break}",
        "function f(a){for(var x of a.values()){return x}}f([1])",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_descriptor_changes_what_a_property_is_and_not_only_what_it_holds() -> Result<(), Error> {
    // A Shape holds the attributes with the names, so a property redefined
    // with different ones belongs to a different Shape. 10.1.6.3 leaves a
    // field the descriptor does not name as it was, and 6.2.6.6 fills it in
    // as false only for a property that did not exist.
    for source in [
        "var o={x:1};Object.defineProperty(o,'x',{enumerable:false});Object.keys(o).length",
        "var o={x:1,y:2};Object.defineProperty(o,'x',{enumerable:false});o.x+o.y",
        "var o={x:1,y:2};Object.defineProperty(o,'x',{enumerable:false});Object.keys(o).join(',')",
        "var o={x:1,y:2,z:3};Object.defineProperty(o,'y',{enumerable:false});o.x+o.y+o.z",
        "var o={x:1};Object.defineProperty(o,'x',{value:2});o.x",
        "var o={x:1};Object.defineProperty(o,'x',{value:2});Object.keys(o).length",
        "var o={x:1};Object.defineProperty(o,'x',{value:2});Object.getOwnPropertyDescriptor(o,'x').enumerable",
        "var o={};Object.defineProperty(o,'x',{value:1});Object.getOwnPropertyDescriptor(o,'x').enumerable",
        "var o={x:1};Object.defineProperty(o,'x',{});o.x",
        "var o={x:1};Object.defineProperty(o,'x',{configurable:true});Object.getOwnPropertyDescriptor(o,'x').configurable",
        "var o={x:1};Object.defineProperties(o,{x:{enumerable:false}});Object.keys(o).length",
        "var o={x:1};Object.defineProperties(o,{x:{enumerable:false}});o.x",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_integrity_levels_of_20_1_2_hold_on_the_new_engine() -> Result<(), Error> {
    // 20.1.2.20 and 20.1.2.16 are [[PreventExtensions]] and [[IsExtensible]];
    // 20.1.2.22 and 20.1.2.6 set the level of 7.3.14 and 20.1.2.18 and
    // 20.1.2.17 test it with 7.3.15. Step 1 of each answers a value that is
    // not an Object, because there is nothing on it to configure.
    for source in [
        "var o={};Object.preventExtensions(o);Object.isExtensible(o)",
        "Object.isExtensible({})",
        "Object.isExtensible(1)",
        "var o={a:1};Object.seal(o);Object.isSealed(o)",
        "var o={a:1};Object.seal(o);Object.isFrozen(o)",
        "var o={a:1};Object.freeze(o);Object.isFrozen(o)",
        "var o={a:1};Object.freeze(o);Object.isSealed(o)",
        "var o={a:1};Object.freeze(o);o.a",
        "var o={a:1};Object.freeze(o);Object.getOwnPropertyDescriptor(o,'a').writable",
        "var o={a:1};Object.seal(o);Object.getOwnPropertyDescriptor(o,'a').configurable",
        "var o={a:1};Object.seal(o);Object.getOwnPropertyDescriptor(o,'a').writable",
        "Object.isSealed({})",
        "Object.isFrozen({})",
        "var o={};Object.preventExtensions(o);Object.isSealed(o)",
        "var o={};Object.preventExtensions(o);Object.isFrozen(o)",
        "Object.isFrozen(1)",
        "Object.isSealed('a')",
        "Object.seal(1)",
        "Object.preventExtensions(1)",
        // 20.1.2.24 and 20.1.2.5 answer the enumerable values and the pairs.
        "Object.values({a:1,b:2}).join(',')",
        "Object.values({}).length",
        "Object.entries({a:1}).length",
        "Object.entries({a:1})[0].join(':')",
        "Object.entries({a:1,b:2})[1].join(':')",
        "var o={a:1};Object.defineProperty(o,'h',{value:2});Object.values(o).join(',')",
    ] {
        differential(source)?;
    }
    // 7.3.14 speaks of every own property, and this engine keeps an index in
    // a store that carries no attributes of its own.
    for source in ["Object.freeze([1])", "Object.isFrozen([1])"] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert!(
            matches!(
                Runtime::with_backend(Limits::default(), Backend::Engine)
                    .run(&program, &mut SilentHost),
                Err(Error::Unsupported { .. })
            ),
            "{source}"
        );
    }
    Ok(())
}
