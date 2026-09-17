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
    let source = "{ let z = function(...rest){} }";
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
        realm.evaluate("typeof Proxy"),
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
        realm.evaluate("{ let z = function(...rest){} }"),
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
    let program = compile("Symbol(1)", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    // The stack backend has the name, so the two paths differ in what they
    // have and not in what they do with a callee.
    assert!(matches!(
        Runtime::new(Limits::default()).run(&program.legacy_only(), &mut SilentHost),
        Ok(_) | Err(Error::Thrown { .. })
    ));
    Ok(())
}

#[test]
fn a_catch_parameter_widens_when_the_range_can_throw_an_error_object() -> Result<(), Error> {
    // Only `throw` carries a value the lowering saw. Every other instruction
    // that throws raises an error object of the Realm, whose type it does not
    // know, so the parameter is a value the lowering cannot name and `|`
    // reaches the conversion of 7.1.6 over 7.1.4, which 20.5.3.4 answers.
    differential("try{f}catch(e){e|5}")?;
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
#[test]
fn object_prototype_to_string_answers_the_tag_of_20_1_3_6() -> Result<(), Error> {
    for source in [
        // Steps 1 and 2 answer before the receiver is boxed.
        "Object.prototype.toString.call(null)",
        "Object.prototype.toString.call(undefined)",
        "Object.prototype.toString.call()",
        // The builtin tags of steps 4 through 13.
        "Object.prototype.toString.call([])",
        "Object.prototype.toString.call(function(){})",
        "Object.prototype.toString.call(new Error('x'))",
        "Object.prototype.toString.call(new Boolean(true))",
        "Object.prototype.toString.call(new Number(1))",
        "Object.prototype.toString.call(new String('a'))",
        "Object.prototype.toString.call(/a/)",
        "Object.prototype.toString.call({})",
        "(function(){return Object.prototype.toString.call(arguments)})()",
        // Step 14 reads @@toStringTag out of the boxed object, so a primitive
        // receiver answers the tag of its own prototype.
        "Object.prototype.toString.call(Math)",
        "Object.prototype.toString.call(JSON)",
        "Object.prototype.toString.call(Reflect)",
        "Object.prototype.toString.call([].values())",
        "Object.prototype.toString.call(Symbol())",
        "Object.prototype.toString.call(Object(Symbol()))",
        "Object.prototype.toString.call(1)",
        "Object.prototype.toString.call('s')",
        "Object.prototype.toString.call(true)",
        // An own tag wins over the builtin one, and a tag that is not a String
        // is not read at all.
        "let o={};o[Symbol.toStringTag]='Q';Object.prototype.toString.call(o)",
        "let o={};o[Symbol.toStringTag]=1;Object.prototype.toString.call(o)",
        // 20.4.3.5 gives the tag, so it is readable as a property.
        "Symbol.prototype[Symbol.toStringTag]",
        "Math[Symbol.toStringTag]",
        "JSON[Symbol.toStringTag]",
        "Object.getOwnPropertyDescriptor(Math,Symbol.toStringTag).configurable",
        "Object.getOwnPropertyDescriptor(Math,Symbol.toStringTag).writable",
        "Object.getOwnPropertyDescriptor(Math,Symbol.toStringTag).enumerable",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_symbol_boxes_into_the_wrapper_of_20_4_3() -> Result<(), Error> {
    for source in [
        // 7.1.18 gives the wrapper %Symbol.prototype%.
        "typeof Object(Symbol())",
        "Object(Symbol('z')).toString()",
        "typeof Object(Symbol('z')).valueOf()",
        "Object(Symbol('z')).valueOf()===Symbol.prototype.valueOf.call(Object(Symbol('z')))",
        // Each wrapper is its own object, and 20.4.3.4 answers the Symbol it
        // holds rather than the wrapper.
        "Object(Symbol())===Object(Symbol())",
        "let s=Symbol('q');Object(s).valueOf()===s",
        "let s=Symbol('q');Symbol.prototype.toString.call(Object(s))",
        // 20.4.3 refuses a receiver that is neither.
        "try{Symbol.prototype.toString.call(1)}catch(e){e instanceof TypeError}",
        "try{Symbol.prototype.valueOf.call({})}catch(e){e instanceof TypeError}",
        // The wrapper owns nothing of its own.
        "Object.keys(Object(Symbol('a'))).length",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_write_adds_a_property_only_while_the_object_is_extensible() -> Result<(), Error> {
    for source in [
        // 10.1.6.3 step 2 refuses a name the object does not own yet.
        "let o={};Object.preventExtensions(o);o.x=1;o.x",
        "let o={};Object.preventExtensions(o);o['y'+'']=1;o.y",
        "let o={};Object.preventExtensions(o);o[Symbol.iterator]=1;o[Symbol.iterator]",
        "let a=[1];Object.preventExtensions(a);a[3]=9;a.length+':'+a[3]",
        "let o=Object.seal({a:1});o.b=2;o.a=3;o.a+':'+o.b",
        "let o=Object.freeze({a:1});o.b=2;o.a+':'+o.b",
        // A name it owns is written as before, and the object stays writable.
        "let o={a:1};Object.preventExtensions(o);o.a=5;o.a",
        "let a=[1,2];Object.preventExtensions(a);a[0]=9;a.join()",
        // 13.15.2 turns the refusal of a strict Reference into a TypeError.
        "(function(){'use strict';let o={};Object.preventExtensions(o);try{o.x=1}catch(e){return e instanceof TypeError}return false})()",
        "(function(){'use strict';let a=[];Object.preventExtensions(a);try{a[0]=1}catch(e){return e instanceof TypeError}return false})()",
        // An extensible object takes every write it took before.
        "let o={};o.x=1;o['y']=2;o.x+o.y",
        "let a=[];a[0]=1;a.push(2);a.join()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_string_constructor_tells_a_missing_argument_from_undefined() -> Result<(), Error> {
    for source in [
        // 22.1.1.1 step 1 answers the empty String for an argument that is
        // not there, and step 2.b the text of one that is `undefined`.
        "'['+String()+']'",
        "'['+String(undefined)+']'",
        "'['+new String()+']'",
        "'['+new String(undefined)+']'",
        "new String().length",
        "'['+String(null)+']'",
        "'['+String(Symbol('k'))+']'",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_restricted_properties_of_20_2_3_throw_on_every_function() -> Result<(), Error> {
    for source in [
        // Both accessors are %ThrowTypeError% on get and on set.
        "(function(){try{(function(){}).caller}catch(e){return e instanceof TypeError}return false})()",
        "(function(){try{(function(){}).arguments}catch(e){return e instanceof TypeError}return false})()",
        "(function(){try{(function(){}).bind({}).caller}catch(e){return e instanceof TypeError}return false})()",
        "(function(){try{Function.prototype.caller}catch(e){return e instanceof TypeError}return false})()",
        "(function(){try{(function(){}).caller=1}catch(e){return e instanceof TypeError}return false})()",
        // 20.2.3 puts them on the prototype with the attributes of 17.
        "let d=Object.getOwnPropertyDescriptor(Function.prototype,'caller');typeof d.get+':'+typeof d.set+':'+(d.get===d.set)",
        "let d=Object.getOwnPropertyDescriptor(Function.prototype,'arguments');d.enumerable+':'+d.configurable+':'+d.writable",
        "Object.getOwnPropertyDescriptor(Function.prototype,'caller').get===Object.getOwnPropertyDescriptor(Function.prototype,'arguments').set",
        "(function(){}).hasOwnProperty('caller')",
        // 10.2.4.1 is the same function the strict `callee` of 10.4.4 uses.
        "(function(){'use strict';try{arguments.callee}catch(e){return e instanceof TypeError}return false})()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_wrapper_prototypes_carry_the_data_slot_of_their_clause() -> Result<(), Error> {
    for source in [
        // 20.3.3 gives %Boolean.prototype% [[BooleanData]] false, 21.1.3 gives
        // %Number.prototype% [[NumberData]] +0 and 22.1.3 gives
        // %String.prototype% [[StringData]] the empty String.
        "Boolean.prototype.toString()",
        "Boolean.prototype.valueOf()",
        "Number.prototype.toString()",
        "Number.prototype.toString(2)",
        "Number.prototype.valueOf()",
        "String.prototype.toString()",
        "String.prototype.valueOf()",
        "String.prototype.length",
        "String.prototype[0]",
        "Object.prototype.toString.call(Boolean.prototype)",
        "Object.prototype.toString.call(Number.prototype)",
        "Object.prototype.toString.call(String.prototype)",
        // The methods each prototype carries are reachable as before.
        "typeof String.prototype.charAt",
        "'abc'.charAt(1)",
        "(5).toString(2)",
        "Object.keys(Boolean.prototype).length",
        "Object.getOwnPropertyNames(String.prototype).indexOf('length')>=0",
        // 10.4.3.1 gives a String exotic object its `length` and its indices;
        // every other own property of one is ordinary.
        "let s=new String('ab');s.x=1;delete s.x",
        "let s=new String('ab');s.x=1;s.x=2;s.x",
        "let s=new String('ab');s[0]=9;s[0]+':'+(delete s[0])+':'+(delete s.length)",
        "delete String.prototype.constructor",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_accessors_of_22_2_6_answer_the_flags_of_the_receiver() -> Result<(), Error> {
    for source in [
        // Each flag accessor reads [[OriginalFlags]] of the receiver.
        "/ab/g.global",
        "/ab/g.ignoreCase",
        "/ab/g.sticky",
        "/ab/g.dotAll",
        "/ab/g.hasIndices",
        "/ab/g.multiline",
        "/ab/g.unicode",
        "/ab/g.unicodeSets",
        "/ab/.flags",
        "/ab/g.flags",
        "/a\\/b/.source",
        "new RegExp('').source",
        // Step 3 answers undefined for %RegExp.prototype%, which carries no
        // [[OriginalFlags]], and "(?:)" for its source.
        "RegExp.prototype.global",
        "RegExp.prototype.sticky",
        "RegExp.prototype.flags",
        "RegExp.prototype.source",
        // Every other receiver without the slot is a TypeError.
        "let g=Object.getOwnPropertyDescriptor(RegExp.prototype,'global').get;try{g.call({})}catch(e){e instanceof TypeError}",
        "let g=Object.getOwnPropertyDescriptor(RegExp.prototype,'source').get;try{g.call({})}catch(e){e instanceof TypeError}",
        // 22.2.6.4 reads the eight names off the receiver, whatever it is.
        "let f=Object.getOwnPropertyDescriptor(RegExp.prototype,'flags').get;f.call({global:true,sticky:true})",
        "let f=Object.getOwnPropertyDescriptor(RegExp.prototype,'flags').get;f.call({})",
        // 17 gives each accessor a getter and no setter.
        "let d=Object.getOwnPropertyDescriptor(RegExp.prototype,'global');typeof d.get+':'+typeof d.set",
        "let d=Object.getOwnPropertyDescriptor(RegExp.prototype,'global');d.enumerable+':'+d.configurable",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_function_object_owns_the_name_and_the_prototype_10_2_gives_it() -> Result<(), Error> {
    for source in [
        // 10.2.5 step 8 gives every function object a `name`, which is the
        // empty String where nothing named it.
        "(function(){}).name===''",
        "(function(){}).hasOwnProperty('name')",
        "Object.getOwnPropertyDescriptor((function(){}),'name').value===''",
        "let d=Object.getOwnPropertyDescriptor((()=>{}),'name');d.writable+':'+d.enumerable+':'+d.configurable",
        // 10.2.5 gives `prototype` to a constructor and to nothing else, so a
        // function without one answers undefined rather than a gap.
        "String(Array.prototype.join.prototype)",
        "String(Math.max.prototype)",
        "String(({m(){}}).m.prototype)",
        "String(Function.prototype.prototype)",
        "String((()=>{}).prototype)",
        "typeof (function(){}).prototype",
        // 10.1.5 stops at the object, so a name a Prototype owns has no own
        // descriptor and no gap either.
        "String(Object.getOwnPropertyDescriptor((function(){}),'bind'))",
        "String(Object.getOwnPropertyDescriptor([],'map'))",
        "String(Object.getOwnPropertyDescriptor(new Number(1),'toFixed'))",
        "String(Object.getOwnPropertyDescriptor(/a/,'test'))",
        "Object.getOwnPropertyDescriptor(Array.prototype,'map').writable",
        "String(Object.getOwnPropertyDescriptor(Math,'zzz'))",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_return_out_of_a_for_of_closes_its_iterator() -> Result<(), Error> {
    for source in [
        // 14.7.5.6 leaves the loop with a return completion, which 7.4.9
        // closes the iterator for.
        "(function(){for(var x of [1,2,3]){if(x===2)return x}return -1})()",
        "(function(){for(var x of [1,2,3]){}return 'end'})()",
        "(function(){for(var x of [1,2,3]){return x}})()",
        "(function(){for(var x of []){return 1}return 2})()",
        // The `return` method of the iterator runs, and the value the return
        // answers is built before it does.
        "var log=[];function f(){var it={};it[Symbol.iterator]=function(){var i=0;return {next:function(){i++;return {value:i,done:i>5}},return:function(){log.push('closed');return {}}}};for(var v of it){if(v===2)return v}}var r=f();log.join()+':'+r",
        "var log=[];function f(){var it={};it[Symbol.iterator]=function(){var i=0;return {next:function(){i++;return {value:i,done:i>5}},return:function(){log.push('closed');return {}}}};for(var v of it){}return 'end'}var r=f();'['+log.join()+']:'+r",
        // An iterator with no `return` is closed by doing nothing.
        "function f(){var it={};it[Symbol.iterator]=function(){var i=0;return {next:function(){i++;return {value:i,done:i>5}}}};for(var v of it){if(v===2)return v}}f()",
        // A `break` still reaches the close after the body.
        "var log=[];function f(){var it={};it[Symbol.iterator]=function(){var i=0;return {next:function(){i++;return {value:i,done:i>5}},return:function(){log.push('c');return {}}}};for(var v of it){if(v===2)break}return log.join()}f()",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_var_head_of_a_nested_iteration_carries_the_top_of_the_lattice() -> Result<(), Error> {
    for source in [
        // 14.7.5.6 step 7.g writes the head of each step, so the declared type
        // of a `var` head says only that the name exists.
        "var n=0;for(var a of [1,2]){for(var b of [3,4]){n+=b}}''+n",
        "var n=0;for(var a of [1,2]){for(var b in {p:1,q:2}){n+=1}}''+n",
        "var n=0;for(var a in {x:1}){for(var b of [3]){n+=b}}''+n",
        "var n=0;for(var a of [1,2]){for(var b of [3,4]){for(var c of [5]){n+=c}}}''+n",
        "var n='';for(var a of ['x','y']){for(var b of ['p']){n+=a+b}}n",
        "(function(){var m=0;for(var a of [1,2]){for(var b of [3,4]){m+=b}}return m})()",
        // 14.7.5.6 step 7.g writes every name a `var` pattern head binds.
        "var n=0;for(var [a,b] of [[1,2],[3,4]]){n+=a+b}''+n",
        "var n='';for(var {x} of [{x:'a'},{x:'b'}]){n+=x}n",
        "var n=0;for(var [a=5] of [[],[7]]){n+=a}''+n",
        "var n='';for(var [a,...t] of [[1,2,3]]){n=a+':'+t.join()}n",
        "var n=0;for(let [a,b] of [[1,2]]){n+=a+b}''+n",
        // A single `var` head still carries what the loop writes.
        "var n=0;for(var a of [1,2]){n+=a}''+n",
        "var n='';for(var a of ['x','y']){n+=a}n",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn the_positions_a_clause_converts_take_an_object() -> Result<(), Error> {
    let number = "{valueOf:function(){return 1}}";
    let text = "{toString:function(){return 'b'}}";
    for source in [
        // 21.1.1.1, 23.1.3.29, 23.1.3.6, 23.1.3.39 and 23.1.3.4 convert the
        // positions they read, which leaves the clause to run 7.1.1.
        &alloc::format!("Number({number})"),
        &alloc::format!("[1,2,3].splice({number},1).join()"),
        &alloc::format!("[1,2,3].fill(9,{number}).join()"),
        &alloc::format!("[1,2,3].with({number},7).join()"),
        &alloc::format!("[1,2,3,4].copyWithin(0,{number}).join()"),
        &alloc::format!("'a,b,c'.split(',',{number}).join('|')"),
        // The element each of them stores is taken as it is.
        &alloc::format!("[1,2,3].fill({number}).length"),
        &alloc::format!("[1,2].splice(0,1,{number}).length"),
        &alloc::format!("[1,2].with(0,{number})[0].valueOf()"),
        // 20.1.2.4, 20.1.2.8, 20.1.3.2, 20.1.3.4 and 28.1 apply ToPropertyKey.
        &alloc::format!("Object.defineProperty({{}},{text},{{value:5}}).b"),
        &alloc::format!("({{b:1}}).hasOwnProperty({text})"),
        &alloc::format!("({{b:1}}).propertyIsEnumerable({text})"),
        &alloc::format!("Object.getOwnPropertyDescriptor({{b:2}},{text}).value"),
        &alloc::format!("Reflect.has({{b:1}},{text})"),
        &alloc::format!("Reflect.get({{b:3}},{text})"),
        // Step 1 of each reads the target before the name is converted.
        "(function(){var n=0;try{Object.hasOwn(undefined,{toString:function(){n=1;return 'x'}})}catch(e){return (e instanceof TypeError)+':'+n}})()",
        "(function(){var n=0;try{Reflect.has(1,{toString:function(){n=1;return 'x'}})}catch(e){return (e instanceof TypeError)+':'+n}})()",
        "(function(){var n=0;try{Object.defineProperty(1,{toString:function(){n=1;return 'x'}},{})}catch(e){return (e instanceof TypeError)+':'+n}})()",
        "(function(){var n=0;try{Object.getOwnPropertyDescriptor(null,{toString:function(){n=1;return 'x'}})}catch(e){return (e instanceof TypeError)+':'+n}})()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_method_carries_the_home_object_super_reads() -> Result<(), Error> {
    for source in [
        // 13.3.7.3 reads the Prototype of the [[HomeObject]] 10.2.11 gave the
        // method, and 13.3.7 reads the property through `this`.
        "let o={m:function(){return 'base'}};let p={m(){return super.m()+'!'}};Object.setPrototypeOf(p,o);p.m()",
        "let p={go(){return typeof super.toString}};p.go()",
        "let p={go(){return super.toString()}};p.go()",
        "let p={go(){return String(super.nope)}};p.go()",
        "let k='toString';let p={go(){return typeof super[k]}};p.go()",
        // A getter of the chain is called with `this` and not with the base.
        "let o={get g(){return this.v}};let p={v:7,go(){return super.g}};Object.setPrototypeOf(p,o);p.go()",
        // 13.2.5.1 gives each half of an accessor the same home.
        "let p={get g(){return typeof super.toString}};p.g",
        "let o={m:function(){return 1}};let p={set s(v){this.seen=super.m()+v}};Object.setPrototypeOf(p,o);p.s=1;p.seen",
        // 15.7.14 gives a class method the prototype and a static one the
        // constructor.
        "(function(){class C{go(){return typeof super.toString}}return new C().go()})()",
        "(function(){class C{static go(){return typeof super.toString}}return C.go()})()",
        // 15.7.14 step 16 makes the constructor a method of the prototype.
        "(function(){class C{constructor(){this.t=typeof super.toString}}return new C().t})()",
        // The home travels with the function, not with the call.
        "let p={go(){return typeof super.toString}};let q=p.go;q()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn new_target_is_the_constructor_the_call_named() -> Result<(), Error> {
    for source in [
        // 9.4.3 answers the constructor `new` named, and undefined for every
        // call `new` did not make.
        "(function(){function F(){this.t=new.target===F}return new F().t})()",
        "(function(){function F(){return typeof new.target}return F()})()",
        "(function(){function F(){return new.target===undefined}return F()})()",
        "(function(){function F(){this.t=typeof new.target}return F.call({}).t})()",
        // 7.3.15 passes the `newTarget` 28.1.2 was given.
        "(function(){function F(){this.t=new.target!==undefined}return Reflect.construct(F,[]).t})()",
        "(function(){function F(){}function G(){this.same=new.target!==undefined}return Reflect.construct(G,[],F).same})()",
        // The value travels into the frame, so a nested call sees its own.
        "(function(){function I(){this.inner=typeof new.target}function O(){this.t=new I().inner}return new O().t})()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_function_that_captures_itself_leaves_an_intrinsic_once() -> Result<(), Error> {
    // The lowering follows everything a function that leaves can reach. One
    // that captures itself was followed again on every pass, so the lowering
    // never ended and no limit of the embedding could stop it.
    for source in [
        "function G(){return typeof G}function H(){return Reflect.construct(G,[])}typeof H",
        "(function(){function G(){return typeof G}return Object.keys(G).length})()",
        "(function(){function G(){return G}return typeof Object.keys(G)})()",
        "(function(){function G(){return typeof G}return Reflect.construct(G,[],G)===undefined})()",
        "(function(){function G(){return new.target===G}return Reflect.construct(G,[],G)!==undefined})()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_class_derives_from_another_and_super_binds_its_this() -> Result<(), Error> {
    for source in [
        // 15.7.14 steps 6 through 8 tie the class and its prototype to the
        // heritage.
        "(function(){class A{}class B extends A{constructor(){super()}}return new B() instanceof A})()",
        "(function(){class A{}class B extends A{}return Object.getPrototypeOf(B)===A})()",
        "(function(){class A{}class B extends A{}return Object.getPrototypeOf(B.prototype)===A.prototype})()",
        "(function(){class B extends null{constructor(){return {}}}return Object.getPrototypeOf(B.prototype)===null})()",
        "(function(){try{class B extends 1{}}catch(e){return e instanceof TypeError}return false})()",
        // 13.3.7.1 constructs the Prototype with the `[[NewTarget]]` of the
        // call and binds the answer as `this`.
        "(function(){class A{constructor(){this.a=1}}class B extends A{constructor(){super();this.b=2}}let o=new B();return o.a+o.b})()",
        "(function(){class A{constructor(x){this.x=x}}class B extends A{constructor(){super(5)}}return new B().x})()",
        "(function(){class A{}class B extends A{constructor(){super()}}class C extends B{constructor(){super()}}return new C() instanceof A})()",
        // 15.7.14 step 10: the default constructor passes what it was given.
        "(function(){class A{constructor(x){this.x=x}}class B extends A{}return new B(7).x})()",
        "(function(){class A{constructor(a,b){this.s=a+b}}class B extends A{}return new B(2,3).s})()",
        // 9.4.5 refuses the binding until the super call has made it, and
        // 13.3.7.1 makes it once.
        "(function(){class A{}class B extends A{constructor(){var t=this;super()}}try{new B()}catch(e){return e instanceof ReferenceError}})()",
        "(function(){class A{}class B extends A{constructor(){this.e=1;super()}}try{new B()}catch(e){return e instanceof ReferenceError}})()",
        "(function(){class A{}class B extends A{constructor(){}}try{new B()}catch(e){return e instanceof ReferenceError}})()",
        "(function(){class A{}class B extends A{constructor(){super();super()}}try{new B()}catch(e){return e instanceof ReferenceError}})()",
        // 10.2.2 step 13: the body answers an Object or the binding.
        "(function(){class A{}class B extends A{constructor(){super();return {z:1}}}return new B().z})()",
        "(function(){class A{}class B extends A{constructor(){super();return}}return new B() instanceof B})()",
        // 13.3.7 reads a property of the chain the heritage made.
        "(function(){class A{m(){return 'A'}}class B extends A{constructor(){super()}m(){return super.m()+'B'}}return new B().m()})()",
        // 15.7.14 gives the class a `prototype` no ordinary function has.
        "(function(){class A{}class B extends A{}let d=Object.getOwnPropertyDescriptor(B,'prototype');return d.writable+':'+d.enumerable+':'+d.configurable})()",
        // 10.1.13: a constructor of this Realm written in Rust makes its
        // object with the Prototype the `newTarget` names.
        "(function(){class E extends Error{}return new E('m').message})()",
        "(function(){class E extends Error{}return new E() instanceof E})()",
        "(function(){class E extends Error{}return new E() instanceof Error})()",
        "(function(){class E extends TypeError{}return new E('x').message})()",
        "(function(){class E extends Error{constructor(m){super(m);this.t=1}}let e=new E('q');return e.message+e.t})()",
        "(function(){class A extends Array{}return Array.isArray(new A())})()",
        "(function(){class A extends Array{}return new A(3).length})()",
        "(function(){class A extends Array{}return Object.getPrototypeOf(new A())===A.prototype})()",
        "(function(){class O extends Object{}return typeof new O()})()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_string_method_converts_the_this_value_it_was_given() -> Result<(), Error> {
    for source in [
        // 22.1.3 sends the `this` value through ToString, which for an Object
        // is a method of the Script the native leaves to run.
        "let o={toString:function(){return ' ab '}};String.prototype.trim.call(o)",
        "let o={toString:function(){return 'abc'}};String.prototype.charAt.call(o,1)",
        "let o={toString:function(){return 'a,b'}};String.prototype.split.call(o,',').join('|')",
        "let o={toString:function(){return 'abc'}};String.prototype.replace.call(o,'b','Z')",
        "let o={toString:function(){return 'abc'}};String.prototype.padStart.call(o,5,'-')",
        "let o={valueOf:function(){return 1},toString:function(){return 'xy'}};String.prototype.indexOf.call(o,'y')",
        // The receiver is converted before any argument is.
        "let n=[];let o={toString:function(){n.push('t');return 'ab'}};let a={toString:function(){n.push('a');return 'b'}};String.prototype.indexOf.call(o,a);n.join()",
        // A throw of the conversion leaves the clause.
        "let o={toString:function(){throw 7}};try{String.prototype.trim.call(o)}catch(e){e}",
        // An object with no method of its own reaches %Object.prototype%.
        "String.prototype.trim.call({})",
        "String.prototype.trim.call(new String('  x  '))",
        "(function(){try{String.prototype.trim.call(null)}catch(e){return e instanceof TypeError}})()",
        // A primitive receiver is unchanged.
        "'abc'.charAt(1)",
        "'  a '.trim()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn an_integrity_level_reaches_the_indices_of_an_array() -> Result<(), Error> {
    for source in [
        // 7.3.15 makes every own property of the object the level asks for,
        // and an index of an Array is one of them.
        "let a=[1,2,3];Object.freeze(a);a[0]=9;a.join()",
        "let a=[1,2,3];Object.freeze(a);''+Object.isFrozen(a)",
        "let a=[1,2];Object.seal(a);a[0]=9;a.join()+':'+Object.isSealed(a)",
        "let a=[1,2];Object.seal(a);''+Object.isFrozen(a)",
        "let a=[1,2];Object.freeze(a);''+Object.getOwnPropertyDescriptor(a,'0').writable",
        "let a=[1,2];Object.seal(a);''+Object.getOwnPropertyDescriptor(a,'0').configurable",
        "let a=[1,2];Object.freeze(a);''+Object.getOwnPropertyDescriptor(a,'length').writable",
        "let a=[1,2];Object.seal(a);delete a[0];a.join()+':'+a.length",
        "''+Object.isFrozen([])",
        "let a=[];Object.freeze(a);''+Object.isFrozen(a)",
        // 7.3.4 and 7.3.9 write and delete with `Throw` true, so a clause of
        // 23.1.3 that edits a frozen Array raises the TypeError.
        "(function(){let a=[1,2,3];Object.freeze(a);try{a.push(4)}catch(e){return e instanceof TypeError}return false})()",
        "(function(){let a=[1,2,3];Object.freeze(a);try{a.pop()}catch(e){return e instanceof TypeError}return false})()",
        "(function(){let a=[1,2,3];Object.seal(a);try{a.pop()}catch(e){return e instanceof TypeError}return false})()",
        "(function(){let a=[1,2,3];Object.preventExtensions(a);try{a.push(4)}catch(e){return e instanceof TypeError}return false})()",
        // An Array nothing froze takes every edit it took before.
        "let a=[1,2,3];a.push(4)+':'+a.join()",
        "let a=[1,2,3];a.pop()+':'+a.join()",
        "let a=[1,2,3];Object.preventExtensions(a);a.pop()+':'+a.join()",
        "let a=[1,2,3];a.splice(1,1).join()+':'+a.join()",
        "let a=[1,2,3];a.reverse().join()",
        "let a=[1,2,3];a.fill(0).join()",
        "let a=[];a.push(1,2);a.join()",
        // 10.4.3.1 gives a String exotic object own names that both levels
        // already hold of.
        "let s=new String('ab');Object.freeze(s);''+Object.isFrozen(s)",
        "let s=new String('ab');Object.seal(s);''+Object.isSealed(s)",
        "(function(){let s=new String('abc');s.foo=10;Object.freeze(s);let d=Object.getOwnPropertyDescriptor(s,'foo');return d.value+':'+d.writable+':'+d.configurable})()",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_descriptor_field_that_is_a_getter_runs_once_and_in_order() -> Result<(), Error> {
    for source in [
        // 6.2.6.5 reads six fields, and a getter among them is a method of
        // the Script the clause leaves to run.
        "let d={get value(){return 5}};let o={};Object.defineProperty(o,'x',d);o.x",
        "let d={get get(){return function(){return 7}}};let o={};Object.defineProperty(o,'x',d);o.x",
        "let d={get value(){return 1},get writable(){return true}};let o={};Object.defineProperty(o,'x',d);o.x=2;o.x",
        "let d={get value(){return 1}};let o={};Reflect.defineProperty(o,'x',d);o.x",
        // Each getter runs once, in the order 6.2.6.5 reads the fields.
        "let n=[];let d={get enumerable(){n.push('e');return true},get configurable(){n.push('c');return true},get value(){n.push('v');return 1},get writable(){n.push('w');return true}};Object.defineProperty({},'x',d);n.join()",
        // The field is read off the chain, as 7.3.2 reads it.
        "let p={get value(){return 4}};let d=Object.create(p);let o={};Object.defineProperty(o,'x',d);o.x",
        // A throw of a getter leaves the clause.
        "let d={get value(){throw 9}};try{Object.defineProperty({},'x',d)}catch(e){e}",
        // Steps 7.b and 8.b refuse a half that is not callable, and step 9 a
        // descriptor that is both kinds.
        "(function(){let d={get set(){return 1}};try{Object.defineProperty({},'x',d)}catch(e){return e instanceof TypeError}})()",
        "(function(){let d={get value(){return 1},get get(){return function(){}}};try{Object.defineProperty({},'x',d)}catch(e){return e instanceof TypeError}})()",
        // A descriptor of plain fields is read as before.
        "let o={};Object.defineProperty(o,'x',{value:3});o.x",
        "let o={};Object.defineProperty(o,'x',{get:function(){return 6}});o.x",
        // 7.3.25 reads one descriptor per key, and the key itself may be a
        // getter of the Script too.
        "let o=Object.create(null,{x:{get value(){return 5},enumerable:true}});o.x",
        "let o={};Object.defineProperties(o,{x:{get value(){return 1}},y:{value:2}});o.x+':'+o.y",
        "(function(){let n=[];let p={};Object.defineProperty(p,'x',{get:function(){n.push('g');return {value:1}},enumerable:true});let o={};Object.defineProperties(o,p);return n.join()+':'+o.x})()",
        "let n=[];let props={a:{get value(){n.push('a');return 1}},b:{get value(){n.push('b');return 2}}};let o={};Object.defineProperties(o,props);n.join()+':'+o.a+o.b",
        "let props={x:{get value(){throw 3}}};try{Object.defineProperties({},props)}catch(e){e}",
        "(function(){let props={x:1};try{Object.defineProperties({},props)}catch(e){return e instanceof TypeError}})()",
        // A key the properties object does not enumerate is not read.
        "(function(){let props={};Object.defineProperty(props,'x',{value:{get value(){return 9}},enumerable:false});let o={};Object.defineProperties(o,props);return typeof o.x})()",
        "let o=Object.create(Object.prototype,{x:{get value(){return 7},enumerable:true}});o.x+':'+(Object.getPrototypeOf(o)===Object.prototype)",
        // The plain form is read as before.
        "let o={};Object.defineProperties(o,{x:{value:1},y:{value:2}});''+(o.x+o.y)",
        "let o=Object.create(null,{x:{value:1,enumerable:true}});o.x",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_string_constructor_carries_the_three_functions_of_22_1_2() -> Result<(), Error> {
    // 22.1.2.1 is the only one of the three the stack backend built, so the
    // rest are checked against the clause rather than against it.
    for source in ["String.fromCharCode(97,98,99)", "String.fromCharCode()"] {
        differential(source)?;
    }
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    for (source, expected) in [
        // 22.1.2.1 step 2.a takes each argument as a code unit.
        ("String.fromCharCode(65536+97)", "a"),
        // 22.1.2.2 takes each as a code point.
        ("String.fromCodePoint(97,98)", "ab"),
        ("String.fromCodePoint()", ""),
        ("String.fromCodePoint(0x1F600).length+''", "2"),
        ("String.fromCodePoint(0xD800).length+''", "1"),
        // 22.1.2.4 puts one substitution between each pair of literals.
        ("String.raw({raw:['a','b','c']},1,2)", "a1b2c"),
        ("String.raw({raw:['a']})", "a"),
        ("String.raw({raw:['a','b']})", "ab"),
        ("String.raw({raw:[]})", ""),
        // 17 gives each the name and the length its clause names.
        ("String.fromCharCode.name", "fromCharCode"),
        ("String.fromCodePoint.name", "fromCodePoint"),
        ("String.raw.name", "raw"),
        (
            "String.fromCharCode.length+':'+String.fromCodePoint.length+':'+String.raw.length",
            "1:1:1",
        ),
    ] {
        assert_eq!(
            realm.evaluate(source)?,
            Value::String(
                expected
                    .encode_utf16()
                    .collect::<alloc::vec::Vec<u16>>()
                    .into()
            ),
            "{source}"
        );
    }
    // 22.1.2.2 step 2.c refuses a code point that is not an integer of the
    // Unicode range.
    for source in [
        "String.fromCodePoint(-1)",
        "String.fromCodePoint(1.5)",
        "String.fromCodePoint(0x110000)",
    ] {
        assert!(realm.evaluate(source).is_err(), "{source}");
    }
    Ok(())
}

#[test]
fn the_string_methods_22_1_3_and_annex_b_add() -> Result<(), Error> {
    // 22.1.3.9 and 22.1.3.29 are the two the stack backend built.
    for source in [
        "'abc'.isWellFormed()",
        "String.fromCharCode(0xD800).isWellFormed()",
        "'ab\u{1F600}'.isWellFormed()",
        "String.fromCharCode(0xD800,97).toWellFormed().charCodeAt(0)+''",
    ] {
        differential(source)?;
    }
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    for (source, expected) in [
        // B.2.2.1 takes a length and not an end, and counts a negative start
        // from the end of the text.
        ("'abcdef'.substr(1,3)", "bcd"),
        ("'abcdef'.substr(-2)", "ef"),
        ("'abcdef'.substr(2)", "cdef"),
        ("'abcdef'.substr(0,-1)", ""),
        ("'abcdef'.substr(-99,2)", "ab"),
        ("'abcdef'.substr(99)", ""),
        // 17 gives each the length its clause names.
        (
            "'abc'.substr.length+':'+'abc'.isWellFormed.length+':'+'a'.localeCompare.length",
            "2:0:1",
        ),
    ] {
        assert_eq!(
            realm.evaluate(source)?,
            Value::String(
                expected
                    .encode_utf16()
                    .collect::<alloc::vec::Vec<u16>>()
                    .into()
            ),
            "{source}"
        );
    }
    // 22.1.3.12 orders by the code units.
    for (source, expected) in [
        ("'a'.localeCompare('b')", -1.0),
        ("'b'.localeCompare('a')", 1.0),
        ("'a'.localeCompare('a')", 0.0),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::Number(expected), "{source}");
    }
    Ok(())
}

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
        // An error the engine raised itself carries its message, and both
        // backends must name the same one.
        (Err(Error::Type { message: actual }), Err(Error::Type { message: expected })) => {
            assert_eq!(actual, expected, "{scripts:?}");
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
    // A function with no formal parameter has an empty mapping, so its object
    // goes where any other value goes.
    differential("function f(){return typeof arguments}f()")?;
    // 10.4.4.7 maps the indices onto the parameters, so a write to either is
    // read through the other.
    for source in [
        "function f(a){arguments[0]=2;return a}f(1)",
        "function f(a){a=2;return arguments[0]}f(1)",
        "function g(x){return x}function f(a){return g(arguments)[0]}f(1)",
        "function f(a){return arguments[0]}f(1)",
    ] {
        differential(source)?;
    }
    // A binding of that name and an arrow that reads the object of the frame
    // it was made in are both refused.
    for source in [
        "function f(){arguments=1;return 2}f()",
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
    // is the one unsupported feature that does not poison the Realm. A Script
    // that throws such a value reached its end too, and that it threw is what
    // crosses.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    for source in ["({x:1})", "[1,2]"] {
        assert!(
            matches!(realm.evaluate(source), Err(Error::Unsupported { .. })),
            "{source}"
        );
        assert_eq!(realm.evaluate("2*3")?, Value::Number(6.0), "{source}");
    }
    assert!(matches!(
        realm.evaluate("throw {}"),
        Err(Error::ThrownUnrepresentable { .. })
    ));
    assert_eq!(realm.evaluate("2*3")?, Value::Number(6.0));
    Ok(())
}

#[test]
fn a_property_of_a_primitive_names_the_object_it_would_need() -> Result<(), Error> {
    // 7.3.2 sends the base of a property access through ToObject, which 7.1.18
    // refuses for undefined and null alone. A Number and a Boolean answer from
    // the Prototype 21.1.3 and 20.3.3 name, without producing the wrapper.
    differential("let f=function(o){return o.x};f(3)")?;
    differential("let f=function(o){return o[0]};f(true)")?;
    differential("let f=function(o){return typeof o.toString};f(3)")?;
    // Every other primitive gets a wrapper Object this engine has not built,
    // so the access names that instead of the TypeError only undefined and
    // null deserve.
    let source = "[x=>h=>x,3,3,22,5][-3,3,2][-2]+1";
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
        // 7.1.18 wraps a primitive in the object of its own constructor.
        "typeof Object(1)",
        "Object(1).valueOf()",
    ] {
        differential(source)?;
    }
    // 20.1.2 gives `%Object%` names this Realm has not built.
    for source in ["Object.keys", "Object.create"] {
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
    // `__proto__` belongs to a Prototype this Realm has not built, and an
    // Array's `length` has the `[[DefineOwnProperty]]` of 10.4.2.1, so a store
    // that reaches one names that where it happens instead of shadowing it. A
    // name the object owns is written on the object itself.
    differential("var f=function(o,k){o[k]='x'};var g=function(){};f(g,'name');g.name")?;
    for source in [
        "let f=function(o,k){o[k]=1};f({},'__proto__')",
        "let f=function(o,k){o[k]=1};f([1,2],'length')",
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
fn a_conversion_of_an_object_operand_runs_its_valueof() -> Result<(), Error> {
    // 7.1.4 sends an Object through ToPrimitive. 13.5.4, 13.12 and 13.11.1
    // each reach the instruction that opens a frame for a `valueOf` of the
    // Script.
    differential_scripts(&["function f(p){return +p}f({})"])?;
    differential_scripts(&["function f(p){return p&1}f({})"])?;
    differential_scripts(&["function f(p){return p==1}f({})"])?;
    // The same code answers for every argument that is a primitive.
    differential_scripts(&["function f(p){return p&1}f(3)"])?;
    differential_scripts(&["function f(p){return +p}f('42')"])?;
    differential_scripts(&["function f(p){return p==1}f('1')"])?;
    Ok(())
}

#[test]
fn a_script_run_for_effect_ends_on_a_value_that_cannot_cross() -> Result<(), Error> {
    // `run_compiled` drops the completion value, so a Script whose value has no
    // identity outside the engine still completes.
    let limits = Limits::default();
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(limits, &mut host, Backend::Engine)?;
    for source in ["({x:1})", "[1,2]", "2*3"] {
        realm.run_compiled(&compile_script(source, limits)?)?;
    }
    // A Script that throws reached its end and threw, which is a completion of
    // the language. That what it threw has no identity outside the engine says
    // nothing about the engine missing a feature, so it is not reported as one.
    assert!(matches!(
        realm.run_compiled(&compile_script("throw {}", limits)?),
        Err(Error::ThrownUnrepresentable { .. })
    ));
    assert!(matches!(
        realm.run_compiled(&compile_script("throw new TypeError('x')", limits)?),
        Err(Error::Thrown { .. })
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

    // 10.2.10 gives the function its `name`, and 8.5.2 gives an anonymous one
    // the name of the binding it is for.
    differential("var f=function(){};f.name")?;
    // 10.2.10 makes `name` a property that is not writable, and 10.1.9.1
    // refuses the write where it runs.
    differential("var f=function(){};f.name=1;f.name")?;
    // B.2.2.1 makes `__proto__` an accessor of a Prototype this Realm has not
    // built, so a write of that name is refused rather than guessed.
    let source = "var f=function(){};f.__proto__=1;1";
    assert!(
        !compile(source, Limits::default())?.uses_register_backend(),
        "{source}"
    );
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
        "let o={a:1};let k='toLocaleString';typeof o[k]",
        "let a=[1];let k='toLocaleString';typeof a[k]",
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
    // An iterator the Script wrote reaches 8.6.2 on the engine now, with the
    // Symbol of table 1 as the key it is.
    differential(
        "let input={next(){return {done:true}},[Symbol.iterator](){return this}};let [x]=input;typeof x",
    )?;
    // 8.6.2 collects a rest element of such an iterator too.
    differential(
        "let input={next(){return {done:true}},[Symbol.iterator](){return this}};let [...x]=input;x.length",
    )?;
    // A computed key of a pattern is a named gap, and an Array whose
    // `@@iterator` the Script replaced is not a layout the lowering keeps.
    for source in [
        "let a=[1];a[Symbol.iterator]=function(){return {next(){return {value:42}}}};let [x]=a;x",
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
    // An Initializer the lowering runs on one path only answers a value it
    // cannot name, and one it runs on every path keeps what it changed.
    differential("let {x=({})}={};42")?;
    differential("let o={};let {x=(o.y=1)}={};42")?;
    differential("let o={};let {x=(o.y=1)}={x:0};o.y")?;
    for source in [
        "let key='x';let {[key='y']:x}={x:42};x",
        "let {['x'+'']:x}={x:42};x",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
    }
    // 7.1.19 sends the key through 7.1.1, and the binding opens a frame for a
    // `toString` of the Script.
    differential("let key={toString(){return 'x'}};let {[key]:x}={x:42};x")?;
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
    // 13.15.5.5 takes the elements from the iterator of the value, so a
    // pattern over an object without one throws where it used to be refused.
    differential("let x=0;[x]={0:42,length:1}")?;
    for source in [
        "const x=0;[x]=[1]",
        "let target={x:1},key='y',x=0,rest={};target[key]=2;({x,...rest}=target);rest.y",
    ] {
        let program = compile(source, Limits::default())?;
        assert!(!program.uses_register_backend(), "{source}");
        let _ = Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost);
    }
    // 7.1.19 sends the key through 7.1.1, and the access opens a frame for a
    // `toString` of the Script.
    differential("let target={},key={toString(){return 'x'}};[target[key]]=[42];target.x")?;
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

    // 7.1.19 sends the key through 7.1.1, and the read opens a frame for a
    // `toString` of the Script.
    differential("let key={toString(){return 'x'}};let o={x:42};o[key]")?;
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
        "let o={};false&&(o.x=1);0",
        "let x=1;false&&(function(){return x});x",
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

    // 13.11.1 is 7.2.14, which sends the Object operand through 7.1.1.
    for source in [
        "let o={};o==0",
        "let o={};0!=o",
        "let o={valueOf(){return 0}};o==0",
        "let o={valueOf(){return 1}};o!=0",
        "let o={toString(){return '2'}};o=='2'",
        // 7.2.14 step 1 and step 12 answer without converting either.
        "let a={},b={};a==b",
        "let o={};o==o",
        "let o={};o==null",
        "let o={};o!=undefined",
        "let o={valueOf(){return 1}};o==true",
    ] {
        differential(source)?;
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
        // 14.2.3 leaves the block with the binding, so an Object one is no
        // different from a primitive one.
        "{let x={};42}",
        "var r=0;{let o={a:2};r=o.a}r",
        "var r=0;{let a=[1,2];r=a.length}r",
        "function f(){ {let o={a:3}; return o.a} }f()",
    ] {
        differential(source)?;
    }
    for source in [
        "{let x=x;x}",
        "{let x=y,y=1;x}",
        "{const x=1;x=2}",
        "let f;{let x=42;f=()=>x}f()",
        "{function f(){return 42}f()}",
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
    // 8.5.2 gives an anonymous function or class the name of the binding it
    // is for, and one that carries its own keeps it.
    differential("var f=function inner(){},c=class Inner{};f.name==='inner'&&c.name==='Inner'")?;
    // 8.5.2 names a sequence expression's function nothing at all, and a
    // function with no own `name` reads the empty String 20.2.3 gives
    // %Function.prototype%, so both paths answer the same.
    differential(
        "var f=function(){},a=()=>{},c=class{},p=(function(){}),s=(0,function(){});\
         f.name==='f'&&a.name==='a'&&c.name==='c'&&p.name==='p'&&s.name===''",
    )?;
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
fn a_capture_a_write_reaches_takes_the_answer_of_the_run() -> Result<(), Error> {
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
        // A binding a write reaches carries no type, so every read of it takes
        // the generic path. Each of these either answers what the stack
        // backend answers or names a feature it has not got; none of them
        // answers something else.
        let program = compile(source, Limits::default())?;
        let expected = Runtime::new(Limits::default()).run(&program, &mut SilentHost)?;
        if !program.uses_register_backend() {
            assert!(program.register_refusal.is_some(), "{source}");
            continue;
        }
        match Runtime::with_backend(Limits::default(), Backend::Engine)
            .run(&program, &mut SilentHost)
        {
            Ok(actual) => assert!(
                same_value(&actual, &expected),
                "{source}: {actual:?} != {expected:?}"
            ),
            Err(Error::Unsupported { .. }) => {}
            other => panic!("{source}: {other:?}"),
        }
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
fn a_capture_a_write_reaches_inside_a_function_takes_the_same_answer() -> Result<(), Error> {
    for source in [
        "function f(){function g(){return x}var x=1;x='a';return g()}f()",
        "function f(){var x=1;function g(){x++;return x}return g()}f()",
    ] {
        differential_scripts(&[source])?;
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
        // An error the engine raised itself carries its message, and both
        // backends must name the same one.
        (Err(Error::Type { message: actual }), Err(Error::Type { message: expected }))
        | (Err(Error::Range { message: actual }), Err(Error::Range { message: expected })) => {
            assert_eq!(actual, expected, "{source}");
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
    // 8.6.2 binds a destructuring catch parameter, and a binding the Block
    // writes carries the top of the lattice in the handler.
    for source in [
        "try{throw [1]}catch([e]){e}",
        "var x=1;var r='';try{x='a';throw 0}catch(e){r=typeof x}r",
        "var r='';try{throw {a:1,b:2}}catch({a,b}){r=a+','+b}r",
    ] {
        differential(source)?;
    }
    for source in [
        // A Block beside a Finally Block must not change a tracked type.
        "let x=1;try{x='a'}finally{}x",
        // A Finally Block cannot run before a `break` or a `continue` leaves
        // it; a `return` takes the path 14.15.3 gives it.
        "let i=0;while(i<2){try{i++;break}finally{i+=10}}i",
        "let i=0;while(i<2){try{i++}finally{continue}}i",
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
        // An intrinsic runs without a call frame, so a user valueOf in an
        // argument position whose conversion the native does not leave for
        // keeps the call on the legacy backend. 22.1.3.6 converts every
        // argument it reads and has no position that leaves for one.
        "let n=0;''.concat({toString(){n++;return 'a'}})",
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
        // A captured per-iteration binding needs a context of its own.
        "for(const k in {a:1}){(()=>k)}",
        // A per-iteration binding captured by a closure needs a context.
        "for(const x of [1]){(()=>x)}",
        // A head that shadows a binding of the enclosing scope is not lowered.
        "let x=1;for(const x of [2]){}x",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    // 8.6.2 binds the names a pattern head names out of the value of each
    // step, in a `for`-`in` as in a `for`-`of`. A `for`-`in` key is a String,
    // whose iterator 22.1.3.34 gives is not built, so that one names the gap.
    differential("for(const [a] of [[1]]){}")?;
    let source = "for(const [k] in {a:1}){}";
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
        // 22.1.4 and 10.4.3: the wrapper holds the String, and its indices
        // and `length` are its own properties.
        "typeof new String('abc')",
        "new String('abc').length",
        "new String('abc')[1]",
        "typeof new String('abc')[9]",
        "new String('abc').valueOf()",
        "new String('abc').toString()",
        "String(new String('abc'))",
        "new String('abc')+''",
        "new String('abc') instanceof String",
        "Object.getPrototypeOf(new String('a'))===String.prototype",
        "Object.keys(new String('abc')).join(',')",
        "Object.getOwnPropertyNames(new String('ab')).join(',')",
        "Object.getOwnPropertyDescriptor(new String('ab'),'0').value",
        "Object.getOwnPropertyDescriptor(new String('ab'),'0').writable",
        "Object.getOwnPropertyDescriptor(new String('ab'),'length').value",
        "Object.prototype.toString.call(new String('a'))",
        "var s=new String('ab');s[0]='z';s[0]",
        "var s=new String('ab');delete s[0]",
        "var r=0;try{String.prototype.valueOf.call({})}catch(e){r=e instanceof TypeError}r",
        // 22.1.1.1 step 2 is 7.1.17, which for an Object asks the object.
        "String({})",
        "String([1,2])",
        "String({toString:function(){return 'made'}})",
        "String({valueOf:function(){return 'wrong'},toString:function(){return 'right'}})",
        "String({toString:function(){return {}},valueOf:function(){return 'fallback'}})",
        "var e=new Error({toString:function(){return 'why'}});e.message",
        "var e=new TypeError({toString:function(){return 'why'}});e.message",
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
        ("class C{}", "a declaration of the Script"),
        // 10.2.11 does more for these parameter lists than the lowering does,
        // and the body is lowered in a unit of its own, so both name what
        // stopped them and not the expression the function was written as.
        ("var f=function(...r){return r}; f()", "a rest parameter"),
        (
            "var f=function(){while(1){try{break}finally{}}}; f()",
            "a jump out of a try with a Finally Block",
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
    // A body that leaves by `return` closes the iterator where it leaves.
    let source = "function f(a){for(var x of a.values()){return x}}f([1])";
    assert!(
        compile(source, Limits::default())?.uses_register_backend(),
        "{source}"
    );
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
    // 7.3.14 and 7.3.15 speak of every own property, and an index of an Array
    // is one of them.
    for source in [
        "''+Object.isFrozen([1])",
        "let a=[1];Object.freeze(a);''+Object.isFrozen(a)",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn an_update_takes_tonumeric_of_what_the_binding_held() -> Result<(), Error> {
    // 13.4.4.1 takes ToNumeric of the old value first, so the operand of the
    // addition is a Number whatever the binding held, and the answer a
    // postfix update gives is that Number and not what was there before.
    for source in [
        "var x='1';x++;x",
        "var x='1';var y=x++;y",
        "var x='1';var y=++x;y",
        "var x=1;x++;x",
        "var x=1;var y=x++;y",
        "var x=true;x++;x",
        "var x=null;x++;x",
        "var x='a';x++;x",
        "var x='3';x--;x",
        "var x=1;x--;x",
        "let x='2';x++;x",
    ] {
        differential(source)?;
    }
    // An Object reaches the ToPrimitive of 7.1.1, which the instruction opens
    // a frame for.
    for source in ["var x=[];x++;x", "var x=[2];var y=x++;y", "var x={};x--;x"] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn the_number_constructor_and_what_21_1_2_gives_it() -> Result<(), Error> {
    // 21.1.1.1 answers +0 for no argument and the Number ToNumber makes of
    // every other, 21.1.2 gives eight values and four questions, and none of
    // the four coerces: 21.1.2.2 step 1 answers false for anything that is
    // not a Number, where 19.2.2 takes ToNumber first.
    for source in [
        "Number('42')",
        "Number()",
        "Number(true)",
        "Number(null)",
        "Number('')",
        "typeof Number",
        "Number.length",
        "Number.prototype.constructor===Number",
        "Number.NaN",
        "Number.EPSILON",
        "Number.MAX_SAFE_INTEGER",
        "Number.MIN_SAFE_INTEGER",
        "Number.MAX_VALUE",
        "Number.MIN_VALUE",
        "Number.POSITIVE_INFINITY",
        "Number.NEGATIVE_INFINITY",
        "Number.isNaN(NaN)",
        "Number.isNaN('NaN')",
        "Number.isFinite(1)",
        "Number.isFinite('1')",
        "Number.isFinite(Infinity)",
        "Number.isInteger(1.5)",
        "Number.isInteger(-3)",
        "Number.isInteger('3')",
        "Number.isSafeInteger(9007199254740991)",
        "Number.isSafeInteger(9007199254740992)",
        "Number.isSafeInteger(1.5)",
    ] {
        differential(source)?;
    }
    // `new` makes the Number exotic object of 21.1.3, and 21.1.3 gives its
    // Prototype methods, neither of which this engine has built.
    for source in ["new Number(1)", "Number.prototype.toFixed"] {
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
fn the_boolean_constructor_answers_toboolean() -> Result<(), Error> {
    // 20.3.1.1 is ToBoolean of the argument, which is false for no argument
    // at all, and 17 ties %Boolean% to %Boolean.prototype%.
    for source in [
        "Boolean(1)",
        "Boolean()",
        "Boolean(0)",
        "Boolean('')",
        "Boolean('a')",
        "Boolean(null)",
        "Boolean(undefined)",
        "Boolean(NaN)",
        "Boolean(-0)",
        "typeof Boolean",
        "Boolean.length",
        "Boolean.prototype.constructor===Boolean",
    ] {
        differential(source)?;
    }
    // `new` makes the Boolean exotic object of 20.3.3, and 20.3.3 gives its
    // Prototype methods, neither of which this engine has built.
    for source in ["new Boolean(1)", "Boolean.prototype.valueOf"] {
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
fn the_reflect_namespace_does_what_20_1_2_does_without_coercing() -> Result<(), Error> {
    // 28.1 is the operations of clause 20.1.2 without their coercion: step 1
    // of each refuses a target that is not an Object, and the answer says
    // whether the operation worked where 20.1.2 throws.
    for source in [
        "typeof Reflect",
        "Reflect.has({a:1},'a')",
        "Reflect.has({a:1},'b')",
        "Reflect.has({a:1},'toString')",
        "Reflect.get({a:5},'a')",
        "Reflect.getPrototypeOf([])===Array.prototype",
        "Reflect.getPrototypeOf({})===Object.prototype",
        "Reflect.ownKeys({a:1,b:2}).join(',')",
        "Reflect.ownKeys({}).length",
        "Reflect.isExtensible({})",
        "var o={};Reflect.preventExtensions(o);Reflect.isExtensible(o)",
        "var o={a:1};Reflect.deleteProperty(o,'a')",
        "var o={};Reflect.defineProperty(o,'x',{value:3});o.x",
        "var o={};Reflect.defineProperty(o,'x',{value:3})",
        "Reflect.getOwnPropertyDescriptor({a:1},'a').value",
        "typeof Reflect.getOwnPropertyDescriptor({a:1},'b')",
        // A name 28.1 does not give it is undefined like any other miss.
        "typeof Reflect.notAName",
    ] {
        differential(source)?;
    }
    // 28.1.1, 28.1.2, 28.1.12 and 28.1.13 are names 28.1 gives it that this
    // Realm has not built, so a read of one is a gap.
    for source in ["Reflect.apply", "Reflect.construct", "Reflect.set"] {
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
fn an_accessor_property_is_read_and_written_by_calling_it() -> Result<(), Error> {
    // 6.1.7.1 gives a property a getter and a setter instead of a value, and
    // 10.1.8.1 and 10.1.9.2 call them where a data property is read or
    // written. 6.2.6.4 and 6.2.6.5 turn the pair into and out of a descriptor.
    for source in [
        "var o={};Object.defineProperty(o,'x',{get:function(){return 42}});o.x",
        "var o={};Object.defineProperty(o,'x',{get:function(){return this.n},configurable:true});o.n=7;o.x",
        // A property with no getter reads undefined, one with no setter takes
        // nothing, and an assignment answers the value however it was taken.
        "var o={};Object.defineProperty(o,'x',{set:function(v){}});typeof o.x",
        "var o={};Object.defineProperty(o,'x',{set:function(v){this.n=v*2}});o.x=21;o.n",
        "var o={};Object.defineProperty(o,'x',{set:function(v){}});o.x=5",
        "var o={};Object.defineProperty(o,'x',{get:function(){return 1}});o.x=5",
        // The getter of a Prototype answers for the object that was read.
        "var p={};Object.defineProperty(p,'x',{get:function(){return this.n}});var o=Object.create(p);o.n=3;o.x",
        "var p={};Object.defineProperty(p,'x',{set:function(v){this.n=v}});var o=Object.create(p);o.x=9;o.n",
        // A computed name reaches the same property.
        "var o={};var k='x';Object.defineProperty(o,'x',{get:function(){return 8}});o[k]",
        "var o={};var k='x';Object.defineProperty(o,'x',{set:function(v){this.n=v}});o[k]=4;o.n",
        // 6.2.6.4 answers get and set, and neither value nor writable.
        "var o={};Object.defineProperty(o,'x',{get:function(){return 1}});typeof Object.getOwnPropertyDescriptor(o,'x').get",
        "var o={};Object.defineProperty(o,'x',{get:function(){return 1}});Object.getOwnPropertyDescriptor(o,'x').set",
        "var o={};Object.defineProperty(o,'x',{get:function(){return 1}});typeof Object.getOwnPropertyDescriptor(o,'x').value",
        "var o={};Object.defineProperty(o,'x',{get:function(){return 1},enumerable:true});Object.getOwnPropertyDescriptor(o,'x').enumerable",
        // 6.2.6.5 step 9: a descriptor is one kind or the other.
        "var o={};Object.defineProperty(o,'x',{get:function(){return 1},value:2})",
        // 6.2.6.5 steps 7.b and 8.b: a half is callable or undefined.
        "var o={};Object.defineProperty(o,'x',{get:1})",
        "var o={};Object.defineProperty(o,'x',{get:undefined});typeof o.x",
        // 10.1.6.3 step 5: what a non-configurable property does not allow.
        "var o={};Object.defineProperty(o,'x',{value:1});Reflect.defineProperty(o,'x',{value:2})",
        "var o={};Object.defineProperty(o,'x',{value:1});Reflect.defineProperty(o,'x',{value:1})",
        "var o={};Object.defineProperty(o,'x',{value:1});Reflect.defineProperty(o,'x',{configurable:true})",
        "var o={};Object.defineProperty(o,'x',{value:1});Reflect.defineProperty(o,'x',{get:function(){return 1}})",
        "var o={};Object.defineProperty(o,'x',{get:function(){return 1}});Reflect.defineProperty(o,'x',{get:function(){return 1}})",
        "var o={};Object.defineProperty(o,'x',{value:1,configurable:true});Reflect.defineProperty(o,'x',{value:2})",
        "var o={};Object.defineProperty(o,'x',{value:1});Object.defineProperty(o,'x',{value:2})",
        // 10.1.6.3 step 6: a property changes from one kind to the other.
        "var o={};Object.defineProperty(o,'x',{get:function(){return 1},configurable:true});Object.defineProperty(o,'x',{value:5});o.x",
        "var o={};Object.defineProperty(o,'x',{value:5,configurable:true});Object.defineProperty(o,'x',{get:function(){return 6}});o.x",
        // 10.1.6.3 step 2: a property is not made on an object that is not
        // extensible.
        "var o={};Object.preventExtensions(o);Reflect.defineProperty(o,'x',{get:function(){return 1}})",
        // 20.1.2.3 and 20.1.2.2 take the same descriptors.
        "var o=Object.create(null,{x:{get:function(){return 11}}});o.x",
        "var o={};Object.defineProperties(o,{x:{get:function(){return 12}}});o.x",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_descriptor_for_an_index_reaches_the_element_store() -> Result<(), Error> {
    // 10.4.2.1 defines an index against the element store, which holds a
    // value and nothing else. An index that is anything but an ordinary data
    // property leaves the store and becomes a property of the Shape, where a
    // read finds it once the store answers a hole.
    for source in [
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9});a[1]",
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9});a.length",
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9,writable:false});a[1]",
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9,writable:false});a.length",
        "var a=[1,2,3];Object.defineProperty(a,'1',{enumerable:false});Object.keys(a).join(',')",
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9,enumerable:false,configurable:false,writable:false});a[1]",
        "var a=[1,2,3];Object.getOwnPropertyDescriptor(a,'1').value",
        "var a=[1,2,3];Object.getOwnPropertyDescriptor(a,'1').writable",
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9,writable:false});Object.getOwnPropertyDescriptor(a,'1').writable",
        "var a=[1];Object.defineProperty(a,'0',{get:function(){return 7}});a[0]",
        "var a=[1];Object.defineProperty(a,'0',{get:function(){return 7}});Object.getOwnPropertyDescriptor(a,'0').set",
        // 10.4.2.1 step 3.g grows the Array to hold the index it defined.
        "var a=[];Object.defineProperty(a,'3',{value:1});a.length",
        "var a=[];Object.defineProperty(a,'3',{value:1,writable:true,enumerable:true,configurable:true});a.length",
        // A read of a hole goes on over the Prototype Chain.
        "var a=[1,2,3];delete a[1];typeof a[1]",
        "var a=[1,2,3];delete a[1];a.length",
        // A write finds the index the Shape took over.
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9,configurable:true});a[1]=4;a[1]",
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9,configurable:true});a[1]=4;a.length",
        "var a=[1,2,3];Object.defineProperty(a,'1',{value:9,configurable:true});a[4]=5;a.length",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn new_number_and_new_boolean_make_the_wrapper_objects() -> Result<(), Error> {
    // 21.1.3 and 20.3.3 wrap one primitive, and 21.1.3.7, 21.1.3.6, 20.3.3.3
    // and 20.3.3.2 answer it. 7.1.18 gives each wrapper the Prototype of the
    // constructor it belongs to.
    for source in [
        "typeof new Number(1)",
        "new Number(1).valueOf()",
        "new Number(1)+1",
        "new Number(1.5).toString()",
        "String(new Number(42))",
        "new Number(1) instanceof Number",
        "Object.getPrototypeOf(new Number(1))===Number.prototype",
        "typeof new Boolean(1)",
        "new Boolean(1).valueOf()",
        "new Boolean(0).valueOf()",
        "new Boolean(1).toString()",
        "String(new Boolean(0))",
        "new Boolean(1) instanceof Boolean",
        "Object.getPrototypeOf(new Boolean(1))===Boolean.prototype",
        // A primitive receiver reaches the same method through 7.1.18.
        "Object(5).valueOf()",
        "Object(true).valueOf()",
        "Object.getPrototypeOf(Object(5))===Number.prototype",
        // 20.1.3.6 tags each by its kind.
        "Object.prototype.toString.call(new Number(1))",
        "Object.prototype.toString.call(new Boolean(1))",
        // The methods belong to their own kind.
        "var r=0;try{Number.prototype.valueOf.call({})}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{Boolean.prototype.valueOf.call({})}catch(e){r=e instanceof TypeError}r",
    ] {
        differential(source)?;
    }
    // 21.1.3.6 takes a radix other than 10 as well.
    differential("new Number(255).toString(16)")?;
    Ok(())
}

#[test]
fn the_integer_operators_convert_an_object_operand() -> Result<(), Error> {
    // 13.12 and 13.9 read 6.1.6.1.2 of each operand, which for an Object is
    // 7.1.1 and therefore a call. The instruction that converts takes them.
    for source in [
        "({}) & 1",
        "1 | ({})",
        "let o={};let x=o^1;x",
        "let x={valueOf(){return 3}};x<<1",
        "let x=8;x>>{valueOf(){return 1}}",
        "let o={valueOf(){return -1}};o>>>28",
        "let x={valueOf(){return 6}};x&=3;x",
        "let x=1;x<<={valueOf(){return 4}};x",
        "let o={toString(){return '5'}};o|0",
        "let x={valueOf(){return 3}};x**=2;x",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn an_object_literal_defines_accessor_properties() -> Result<(), Error> {
    // 13.2.5.1 gives the property a getter or a setter, and the two clauses
    // of one name meet on the object.
    for source in [
        "var o={get x(){return 5}};o.x",
        "var o={set x(v){this.n=v*2}};o.x=4;o.n",
        "var o={get x(){return this.n},set x(v){this.n=v+1}};o.x=1;o.x",
        "var o={get x(){return 1}};typeof Object.getOwnPropertyDescriptor(o,'x').get",
        "var o={get x(){return 1}};Object.getOwnPropertyDescriptor(o,'x').enumerable",
        "var o={get x(){return 1}};Object.getOwnPropertyDescriptor(o,'x').configurable",
        "var o={a:1,get x(){return 2},b:3};o.a+o.x+o.b",
        "var o={get x(){return 1}};Object.keys(o).join(',')",
        "var o={get x(){return 1},y:2};Object.keys(o).join(',')",
        // A getter that is not there reads undefined, and a setter that is
        // not there takes nothing.
        "var o={set x(v){}};typeof o.x",
        "var o={get x(){return 1}};o.x=9;o.x",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_class_body_builds_its_constructor_and_its_prototype() -> Result<(), Error> {
    // 15.7.14 makes the constructor and the object it carries, and puts every
    // method the body defines on one of the two.
    for source in [
        "class C{};typeof C",
        "class C{constructor(a){this.a=a}};new C(3).a",
        "class C{m(){return 7}};new C().m()",
        "class C{m(){return this.a}constructor(){this.a=2}};new C().m()",
        "class C{static m(){return 9}};C.m()",
        "class C{get x(){return 4}};new C().x",
        "class C{set x(v){this.n=v+1}};var o=new C();o.x=1;o.n",
        "class C{static get x(){return 6}};C.x",
        // 15.7.14 step 12 and 7.3.5: neither the prototype nor a method is
        // enumerable, and the prototype cannot be replaced.
        "class C{m(){}};Object.keys(C.prototype).length",
        "class C{m(){}};Object.getOwnPropertyDescriptor(C.prototype,'m').enumerable",
        "class C{m(){}};Object.getOwnPropertyDescriptor(C.prototype,'m').writable",
        "class C{};Object.getOwnPropertyDescriptor(C,'prototype').writable",
        "class C{};Object.getOwnPropertyDescriptor(C,'prototype').configurable",
        "class C{};C.prototype.constructor===C",
        "class C{};new C() instanceof C",
        "class C{};Object.getPrototypeOf(new C())===C.prototype",
        // An expression form binds nothing.
        "var K=class{m(){return 1}};new K().m()",
    ] {
        differential(source)?;
    }
    // 15.7.14 gives the constructor a `[[Call]]` that throws.
    differential_scripts(&["class C{};var r=0;try{C()}catch(e){r=e instanceof TypeError};r"])?;
    // 15.7 derives a class through a native constructor, which 13.3.7.1 has
    // no frame to enter.
    differential_scripts(&["class C extends Object{};0"])?;
    Ok(())
}

/// 15.7.14 and 13.2.5.5 define a method under a key only the run time knows,
/// and 10.2.10 names it after that key.
#[test]
fn a_computed_key_defines_a_method_and_an_accessor() -> Result<(), Error> {
    for source in [
        "var k='m';class C{[k](){return 1}};(new C()).m()",
        "var k='s';class C{static [k](){return 2}};C.s()",
        "var k='g';class C{get [k](){return 3}};(new C()).g",
        "var k='g';class C{get [k](){return 3};set [k](v){}};(new C()).g",
        "var k='m';class C{[k](){}};Object.keys(C.prototype).length",
        "var k='m';class C{[k](){}};(new C()).m.name",
        "var k='m';var o={[k](){return 1}};o.m()",
        "var k='m';var o={[k](){}};Object.keys(o).length",
        "var k='p';var o={get [k](){return 1}};o.p",
        "var k='p';var o={get [k](){return 1},set [k](v){}};o.p",
        "var s=Symbol('d');var o={[s](){return 4}};o[s]()",
        "var o={a:1};var k='b';o[k]=function(){return 7};o.b()",
        "var k='m';var o={[k]:1};o.m",
    ] {
        differential_scripts(&[source])?;
    }
    // 10.2.10 names such a function after its key, which the stack backend
    // leaves empty; these check the engine against the specification.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    for (source, expected) in [
        ("var k='m';var o={[k](){return 1}};o.m.name", "m"),
        ("var k='m';var o={[k]:function(){}};o.m.name", "m"),
        ("var s=Symbol('d');var o={[s](){}};o[s].name", "[d]"),
        (
            "var k='p';var o={get [k](){return 1}};             Object.getOwnPropertyDescriptor(o,'p').get.name",
            "get p",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(expected), "{source}");
    }
    Ok(())
}

#[test]
fn an_object_pattern_reads_a_value_with_no_known_layout() -> Result<(), Error> {
    // 14.3.3.3 reads each property of the source, which for a value the
    // lowering cannot name is a read at run time.
    for source in [
        "function f(o){let {a}=o;return a}f({a:1})",
        "function f(o){let {a,b}=o;return a+b}f({a:1,b:2})",
        "function f(o){let {a:x}=o;return x}f({a:5})",
        "function f(o){let {a=7}=o;return a}f({})",
        "function f(o){let {a:{b}}=o;return b}f({a:{b:3}})",
        "function f(o,k){let {[k]:v}=o;return v}f({x:4},'x')",
        "function f(o){let {a}=o;return a}f({get a(){return 8}})",
    ] {
        differential_scripts(&[source])?;
    }
    // 14.3.3.3 refuses a source that is not coercible to an Object, which the
    // read of its first property is what finds out.
    for source in [
        "function f(o){let {a}=o;return a}f(null)",
        "function f(o){let {a}=o;return a}f(undefined)",
    ] {
        differential_scripts(&[source])?;
    }
    // 14.3.3.3 requires the source to be coercible to an Object, which a
    // pattern that reads no property still checks.
    differential("function f(o){let {}=o;return 1}f({})")?;
    differential("function f(o){let {}=o;return 1}f(null)")?;
    Ok(())
}

#[test]
fn a_parameter_that_is_an_object_pattern_binds_its_names() -> Result<(), Error> {
    // 10.2.11 binds the argument, and 8.6.2 then binds the names the pattern
    // names out of it.
    for source in [
        "function f({a}){return a};f({a:1})",
        "function f({a,b}){return a+b};f({a:1,b:2})",
        "function f({a:x}){return x};f({a:5})",
        "function f({a=3}){return a};f({})",
        "function f({a}={a:9}){return a};f()",
        "function f({a:{b}}){return b};f({a:{b:4}})",
        "function f(p,{a}){return p+a};f(1,{a:2})",
        "function f({a},q){return a+q};f({a:1},2)",
        "function f({a}){return function(){return a}()};f({a:6})",
        "function f({a}){return a};f({get a(){return 7}})",
        // 7.2.1 refuses a source that is neither undefined nor null only
        // after the Initializer has had its turn.
        "function f({a}){return a};f(null)",
        "function f({a}){return a};f()",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn an_array_pattern_takes_its_elements_from_the_iterator() -> Result<(), Error> {
    // 8.6.2 opens the iterator of the value (7.4.2), takes one step of 7.4.6
    // per element, and closes what it did not exhaust (7.4.9).
    for source in [
        "function f(v){let [a]=v;return a}f([1])",
        "function f(v){let [a,b]=v;return a+b}f([1,2])",
        "function f(v){let [,b]=v;return b}f([1,2])",
        "function f(v){let [a,b]=v;return b}f([1])",
        "function f(v){let [a=5]=v;return a}f([])",
        "function f(v){let [[a]]=v;return a}f([[3]])",
        "function f(v){let [{a}]=v;return a}f([{a:4}])",
        "function f([a]){return a}f([7])",
        "function f([a,b=2]){return a+b}f([1])",
        "function f([a]=[8]){return a}f()",
        // A value with no iterator is a TypeError.
        // 7.4.9 closes an iterator the pattern left unfinished; the Array
        // iterator of 23.1.5 has no `return`, so closing it calls nothing.
        "function f(v){let [a]=v;return a}f([1,2,3])",
        "function f(v){let [a]=v;return a}f({})",
    ] {
        differential_scripts(&[source])?;
    }
    // 8.6.2 collects a rest element by walking the iterator to its end into
    // an Array of its own.
    for source in [
        "function f(v){let [a,...r]=v;return r.join()}f([1,2,3])",
        "function f(v){let [...r]=v;return r.length}f([])",
        "function f(v){let [a,b,...r]=v;return r.length}f([1])",
        "function f(v){let [a,...r]=v;return r[0]}f([1,2])",
        "function f(v){let [a,...[b,c]]=v;return b+','+c}f([1,2,3])",
        "function f(v){let [a,...r]=v;return typeof r}f([1])",
        "function f([a,...r]){return r.join()}f([1,2,3])",
        "function f(v){let [a,...r]=v;return r.join()}f({})",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_captured_var_a_closure_writes_carries_no_type() -> Result<(), Error> {
    // A captured reader is compiled against the type its binding carries, and
    // an assignment anywhere can make that type wrong. A captured `var` that
    // is written carries the type the lowering cannot name instead.
    for source in [
        "var c=0;var f=function(){c=c+1};f();c",
        "var c=0;function f(){c=c+1}f();c",
        "var c=0;var f=function(){c=1};f();c",
        "var c=0;var f=function(){c='s'};f();typeof c",
        "var c=0;var f=function(){c=c+1};f();f();c",
        "var c=1;var f=function(){c=c*2};var g=function(){return c};f();g()",
        "var c=0;var f=function(){var g=function(){c=7};g()};f();c",
        "var c=0;var f=function(){c=c+1;return c};f()+f()",
        // The same binding read and never written keeps the type it had.
        "var c=2;var f=function(){return c+1};f()",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn an_initializer_that_makes_an_object_is_taken() -> Result<(), Error> {
    // 8.6.2 and 14.3.3.3 run an Initializer only where the value is
    // undefined, so a layout it made is there on one path only and what it
    // answers is a value the lowering cannot name.
    for source in [
        "var o={method([{x,y}={x:1,y:2}]=[{x:3,y:4}]){return x+y}};o.method()",
        "var o={method([{x,y}={x:1,y:2}]){return x+y}};o.method([])",
        "function f(a={b:1}){return a.b}f()",
        "function f(a={b:1}){return a.b}f({b:2})",
        "function f(a=[1,2]){return a.length}f()",
        "function f(a=[1,2]){return a.length}f([1])",
        "function f({a={b:3}}){return a.b}f({})",
        "function f({a={b:3}}){return a.b}f({a:{b:4}})",
        "function f(v){let [a=[7]]=v;return a.length}f([])",
        "function f(v){let {a={c:5}}=v;return a.c}f({})",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_function_carries_the_name_it_was_given() -> Result<(), Error> {
    // 10.2.10 gives the function its `name`, and 8.5.2 gives an anonymous one
    // the name of the binding, the key or the Initializer it stands in.
    for source in [
        "function f(){};f.name",
        "var f=function(){};f.name",
        "var f=function inner(){};f.name",
        "let g=function(){};g.name",
        "var o={m:function named(){}};o.m.name",
        "class C{};C.name",
        "var K=class{};K.name",
        "var K=class Inner{};K.name",
        "class C{m(){}};C.prototype.m.name",
        "class C{static s(){}};C.s.name",
        "class C{get x(){return 1}};Object.getOwnPropertyDescriptor(C.prototype,'x').get.name",
        "function f({fn=function(){}}){return fn.name}f({})",
        "function f(v){let [a=function(){}]=v;return a.name}f([])",
        // 10.2.10 gives the property the attributes it names.
        "function f(){};Object.getOwnPropertyDescriptor(f,'name').writable",
        "function f(){};Object.getOwnPropertyDescriptor(f,'name').enumerable",
        "function f(){};Object.getOwnPropertyDescriptor(f,'name').configurable",
    ] {
        differential(source)?;
    }
    // 13.2.5.5 names the function a property definition holds after its key,
    // and 10.2.10 names an accessor "get x" or "set x". The stack backend
    // leaves each of those empty, so these are the answers of the engine
    // alone, checked against the specification.
    for (source, expected) in [
        ("var o={m(){}};o.m.name", "m"),
        ("var o={m:function(){}};o.m.name", "m"),
        (
            "var o={get x(){return 1}};Object.getOwnPropertyDescriptor(o,'x').get.name",
            "get x",
        ),
        (
            "var o={set x(v){}};Object.getOwnPropertyDescriptor(o,'x').set.name",
            "set x",
        ),
        (
            "function f({fn=function(){}}){return fn.name}f({fn:function(){}})",
            "fn",
        ),
    ] {
        let program = compile(source, Limits::default())?;
        assert!(program.uses_register_backend(), "{source}");
        assert_eq!(
            Runtime::with_backend(Limits::default(), Backend::Engine)
                .run(&program, &mut SilentHost)?,
            Value::String(expected.encode_utf16().collect()),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_arguments_object_of_a_strict_function_is_a_value() -> Result<(), Error> {
    // 10.4.4 makes an unmapped arguments object for a strict function, so
    // nothing of it can be observed that this engine does not build. It is a
    // value there and not only the base of a property access.
    for source in [
        "'use strict';function f(){return arguments}f(1,2).length",
        "'use strict';function f(){var a=arguments;return a[0]+a[1]}f(1,2)",
        "'use strict';function f(){return typeof arguments}f()",
        "'use strict';function f(){var a=arguments;return a.length}f()",
        "'use strict';function f(){var r=0;for(var i=0;i<arguments.length;i++)r=r+arguments[i];return r}f(1,2,3)",
        // 10.4.4 step 7: reading `callee` is the accessor of 10.2.4.1.
        "'use strict';function f(){var r=0;try{arguments.callee}catch(e){r=e instanceof TypeError}return r}f()",
        "'use strict';function f(){return Object.getOwnPropertyDescriptor(arguments,'callee').configurable}f()",
        // 10.4.4 gives it the iterator of 23.1.3.33.
        "'use strict';function f(){var r=0;for(var v of arguments)r=r+v;return r}f(1,2,3)",
    ] {
        differential_scripts(&[source])?;
    }
    // 10.4.4.7 maps the indices of a sloppy function's object onto its
    // parameters, so a write to one is read through the other.
    differential_scripts(&["function f(p){p=2;var a=arguments;return a[0]}f(1)"])?;
    // 23.1.5.2.1 reads the length of the array-like again at every step, so
    // an object that is no Array is walked the same way.
    for source in [
        "var o={length:2,0:'a',1:'b'};var r='';for(var v of Array.prototype.values.call(o))r=r+v;r",
        "var a=[1,2,3];var r=0;for(var v of Array.prototype.values.call(a))r=r+v;r",
    ] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn a_property_write_reaches_a_base_the_lowering_cannot_name() -> Result<(), Error> {
    // 13.15.2 writes through `[[Set]]`, which needs no layout. And 10.4.2.4
    // sets an Array's own length, deleting every index at or above it.
    for source in [
        "function f(o){o.x=1;return o.x}f({})",
        "function f(o){o.x=1;return o.x}f({x:0})",
        "function f(o,k){o[k]=2;return o.y}f({},'y')",
        "function f(o){o.x=1;return o}f({}).x",
        "var a=[1,2,3];a.length=2;a.length",
        "var a=[1,2,3];a.length=2;a[2]",
        "var a=[1,2,3];a.length=2;a.join(',')",
        "var a=[1];a.length=3;a.length",
        "var a=[1];a.length=3;typeof a[2]",
        "var a=[1,2,3];a.length=0;a.length",
        "var a=[1,2,3];a.length='2';a.length",
    ] {
        differential(source)?;
    }
    // 10.4.2.4 refuses a length that is not the Number `ToUint32` gives.
    for source in ["var a=[1,2,3];a.length=-1", "var a=[1,2,3];a.length=1.5"] {
        differential(source)?;
    }
    Ok(())
}

#[test]
fn an_update_reaches_a_global_and_a_property() -> Result<(), Error> {
    // 13.4.4.1 reads the Reference, takes `ToNumeric` of it and writes back.
    // A name this Script does not bind is resolved on the Global Environment
    // Record (9.1.1.4), and a property Reference is evaluated once.
    for source in [
        "var i=0;i++;i",
        "var i=0;++i",
        "var i=0;i++",
        "var i=0;i--;i",
        "var i='3';i++;i",
        "var c=0;function f(){c++}f();f();c",
        "var o={n:0};o.n++;o.n",
        "var o={n:0};o.n++",
        "var o={n:0};++o.n",
        "var o={n:'2'};o.n++;o.n",
        "var a=[1,2];a[0]++;a[0]",
        "var a=[1,2];var i=1;a[i]--;a[1]",
        "var o={n:0};function f(){o.n++}f();f();o.n",
        "var o={};var r=o.n++;typeof r",
        "function f(p){p.n++;return p.n}f({n:5})",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_var_head_of_a_realm_script_writes_the_global() -> Result<(), Error> {
    // 16.1.7 makes a `var` of a Realm Script a binding of the Global
    // Environment Record, and 14.7.5.6 writes the head's binding wherever it
    // lives.
    for source in [
        "var r='';for(var k in {a:1,b:2}){r=r+k};r",
        "var r=0;for(var v of [1,2,3]){r=r+v};r",
        "var r=0;for(var v of [1,2]){}v",
        "var r='';for(var k in {a:1}){r=r+k};k",
        "var r=0;for(var v of []){r=1};typeof v",
        "var r=0;var o={a:1,b:2};for(var k in o){r=r+o[k]};r",
        "var r=0;for(var v of [1,2,3]){if(v===2)continue;r=r+v};r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn the_in_operator_asks_the_prototype_chain() -> Result<(), Error> {
    // 13.10.2 is `HasProperty` of 7.3.11 on the key 7.1.19 makes, and the
    // right side has to be an Object.
    for source in [
        "var o={a:1};'a' in o",
        "var o={a:1};'b' in o",
        "var o={a:1};'toString' in o",
        "var a=[1,2];0 in a",
        "var a=[1,2];5 in a",
        "var a=[1,2];'length' in a",
        "var o=Object.create({p:1});'p' in o",
        "var o={};var k='x';k in o",
        "var o={x:1};var k='x';k in o",
        "function f(o){return 'a' in o}f({a:1})",
        "var o={};var r=0;try{'a' in 1}catch(e){r=e instanceof TypeError};r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn the_well_known_symbols_are_keys_of_their_own() -> Result<(), Error> {
    // 20.4.2 gives `%Symbol%` the thirteen Symbols of table 1, and 7.1.19
    // keeps a Symbol as the key it is.
    for source in [
        "typeof Symbol",
        "typeof Symbol.iterator",
        "typeof Symbol.toPrimitive",
        "Symbol.iterator===Symbol.iterator",
        "Symbol.iterator===Symbol.asyncIterator",
        "var o={};o[Symbol.iterator]=1;o[Symbol.iterator]",
        "var o={};typeof o[Symbol.iterator]",
        "var o={[Symbol.iterator](){return this}};typeof o[Symbol.iterator]",
        "var o={};o[Symbol.iterator]=1;Object.keys(o).length",
        "var o={};o[Symbol.iterator]=1;delete o[Symbol.iterator];typeof o[Symbol.iterator]",
        "var r='';var o={a:1};o[Symbol.iterator]=2;for(var k in o){r=r+k};r",
        "'use strict';function f(){return typeof arguments[Symbol.iterator]}f(1)",
        "var o={};o[Symbol.iterator]=1;Object.getOwnPropertyNames(o).length",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 20.4.1.1 makes a Symbol no other value is, 20.4.2.2 shares one under a key,
/// and 20.4.3 answers about one without a wrapper object.
#[test]
fn the_symbol_constructor_makes_a_symbol_of_its_own() -> Result<(), Error> {
    for source in [
        "typeof Symbol()",
        "typeof Symbol.for",
        "Symbol('a').toString()",
        "Symbol().toString()",
        "typeof Symbol('a').description",
        "Symbol('a').description",
        "typeof Symbol().description",
        "Symbol()===Symbol()",
        "var s=Symbol('x');s===s",
        "Symbol.for('k')===Symbol.for('k')",
        "Symbol.keyFor(Symbol.for('k'))",
        "typeof Symbol.keyFor(Symbol('q'))",
        "Symbol.iterator.toString()",
        "Symbol.iterator.description",
        "var o={};var s=Symbol('p');o[s]=1;o[s]",
        "typeof Symbol.prototype",
        "Symbol.prototype.toString.call(Symbol('z'))",
        "var s=Symbol('a');s.valueOf()===s",
        "Symbol.prototype.constructor===Symbol",
        "Symbol.length",
        "String(Symbol('a'))",
        "var o={};o[Symbol('a')]=1;Object.keys(o).length",
        // 7.1.17 of a Symbol is a TypeError, whichever operation asks.
        "var r=0;try{''+Symbol()}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{`${Symbol()}`}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{new Symbol()}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{Symbol.keyFor('x')}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{Symbol.prototype.toString.call(1)}catch(e){r=e instanceof TypeError}r",
    ] {
        differential_scripts(&[source])?;
    }
    // 20.4.2.2 gives `Symbol.for` one formal parameter; the stack backend
    // gives it none, and this checks the engine against the specification.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert_eq!(realm.evaluate("Symbol.for.length")?, Value::Number(1.0));
    Ok(())
}

#[test]
fn a_regular_expression_literal_makes_the_object_of_its_pattern() -> Result<(), Error> {
    // 22.2.4.1 makes an object of a pattern the Script was compiled with, and
    // 22.2.7.2 matches with it; 22.2.6.16 answers whether it matched and
    // 22.2.6.17 the text of the literal.
    for source in [
        "typeof /a/",
        "/a/.test('bab')",
        "/a/.test('bbb')",
        "/(a)(b)/.exec('xabz')[0]",
        "/(a)(b)/.exec('xabz')[1]",
        "/(a)(b)/.exec('xabz')[2]",
        "/(a)(b)/.exec('xabz').index",
        "/(a)(b)/.exec('xabz').input",
        "/(a)(b)/.exec('xabz').length",
        "/(a)|(b)/.exec('b')[1]",
        "/z/.exec('ab')",
        "String(/ab/g)",
        "String(/ab/)",
        "var r=/a/g;r.test('aa');r.lastIndex",
        "var r=/a/g;r.test('aa');r.test('aa');r.lastIndex",
        "var r=/a/;r.test('aa');r.lastIndex",
        "var r=/a/g;r.lastIndex",
        "var r=/b/g;r.test('aa');r.lastIndex",
        "/a/ instanceof RegExp",
        "Object.getPrototypeOf(/a/)===RegExp.prototype",
        "Object.prototype.toString.call(/a/)",
        "var r=/a/;r.exec('a')[0]",
    ] {
        differential_scripts(&[source])?;
    }
    // 22.2.6 gives `%RegExp.prototype%` more than this Realm builds.
    let source = "/a/.compile";
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert!(
        matches!(realm.evaluate(source), Err(Error::Unsupported { .. })),
        "{source}"
    );
    Ok(())
}

#[test]
fn a_write_a_property_refuses_says_so_where_it_runs() -> Result<(), Error> {
    // 10.1.9.1 refuses a write to a property that is not writable, and a
    // strict Reference turns that refusal into a TypeError.
    for source in [
        "var a=[1,2,3];function f(){a.length=2}f();a.length",
        "var a=[1,2,3];var f=function(){a.length=2};f();a.length",
        "var f=function(){};f.length=9;f.length",
        "var f=function(){};f.name='x';f.name",
        "var f=function(){};f.x=1;f.x",
        "var o={};Object.defineProperty(o,'x',{value:1,writable:false});o.x=2;o.x",
        "var o={};Object.defineProperty(o,'x',{value:1,writable:true});o.x=2;o.x",
        "var p={};Object.defineProperty(p,'x',{value:1,writable:false});var o=Object.create(p);o.x=2;o.x",
        "var o={};o.x=1;o.x=2;o.x",
        "'use strict';var f=function(){};var r=0;try{f.length=9}catch(e){r=1};r",
        "'use strict';var o={};Object.defineProperty(o,'x',{value:1});var r=0;try{o.x=2}catch(e){r=1};r",
        // 13.2.5.5 defines the properties of a literal, so a Prototype that
        // holds the name and refuses a write does not reach them.
        "Object.defineProperty(Object.prototype,'p',{value:1,writable:false,configurable:true});var o={p:2};o.p",
        "Object.defineProperty(Object.prototype,'p',{value:1,writable:false,configurable:true});var o={p:2};o.hasOwnProperty('p')",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_head_that_is_a_pattern_binds_each_step() -> Result<(), Error> {
    // 8.6.2 binds the names a pattern head names out of the value of each
    // step, and 14.7.5.5 makes one binding per iteration for a lexical head.
    for source in [
        "var r=0;for(var [a,b] of [[1,2],[3,4]]){r=r+a+b};r",
        "var r=0;for(const [a,b] of [[1,2],[3,4]]){r=r+a+b};r",
        "var r=0;for(let [a] of [[5]]){r=r+a};r",
        "var r=0;for(var {x} of [{x:1},{x:2}]){r=r+x};r",
        "var r=0;for(const {x,y} of [{x:1,y:2}]){r=r+x+y};r",
        "var r=0;for(const {x:{y}} of [{x:{y:7}}]){r=r+y};r",
        "var r=0;for(const [a=9] of [[]]){r=r+a};r",
        "var r=0;for(const {x=3} of [{}]){r=r+x};r",
        "var r=0;for(var [a] of [[1]]){r=r+a};r+a",
        "var r='';for(const [a,b] of [[1,2]]){r=r+a+'-'+b};r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn json_parses_and_quotes_what_it_reaches() -> Result<(), Error> {
    // 25.5.1 parses one JSON text, and 25.5.2 answers the text of a value.
    for source in [
        "typeof JSON",
        "JSON.parse('1')",
        "JSON.parse('\"a\"')",
        "JSON.parse('true')",
        "JSON.parse('null')",
        "JSON.parse('[1,2,3]').length",
        "JSON.parse('[1,2,3]')[1]",
        "JSON.parse('{\"a\":1}').a",
        "JSON.parse('{\"a\":{\"b\":2}}').a.b",
        "JSON.parse('[{\"a\":1}]')[0].a",
        "var r=0;try{JSON.parse('{')}catch(e){r=e instanceof SyntaxError};r",
        "JSON.stringify(1)",
        "JSON.stringify('a')",
        "JSON.stringify(true)",
        "JSON.stringify(null)",
        "typeof JSON.stringify(undefined)",
        "JSON.stringify([1,2])",
        "JSON.stringify({a:1})",
        "JSON.stringify({a:1,b:'x'})",
        "JSON.stringify({a:[1,{b:2}]})",
        "JSON.stringify({a:undefined,b:1})",
        "JSON.stringify([undefined])",
        "JSON.stringify(1/0)",
        "JSON.stringify('a\"b')",
        "JSON.parse(JSON.stringify({a:[1,2],b:'c'})).b",
    ] {
        differential_scripts(&[source])?;
    }
    // 25.5.1 and 25.5.2 take a reviver, a replacer and a space, each of
    // which this engine has not built.
    for source in [
        "JSON.parse('1',function(k,v){return v})",
        "JSON.stringify({},null,2)",
        "JSON.stringify({toJSON(){return 1}})",
    ] {
        let mut host = SilentHost;
        let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
        assert!(
            matches!(realm.evaluate(source), Err(Error::Unsupported { .. })),
            "{source}"
        );
    }
    Ok(())
}

/// 23.1.3 reads each element with 7.3.2, so an accessor's getter runs where a
/// data property would simply be read.
#[test]
fn walks_over_accessor_elements() -> Result<(), Error> {
    for source in [
        "var a=[0,0,0];var n=0;Object.defineProperty(a,1,{get:function(){n++;return 7},\
         enumerable:true,configurable:true});var s=0;a.forEach(function(v){s+=v});''+s+','+n",
        "var a=[1,2,3];Object.defineProperty(a,0,{get:function(){return 10},enumerable:true});\
         ''+a.map(function(v){return v*2})",
        "var a=[1,2,3];Object.defineProperty(a,2,{get:function(){return 0},enumerable:true});\
         ''+a.filter(function(v){return v>0})",
        "var a=[1,2,3];Object.defineProperty(a,1,{get:function(){return 9},enumerable:true});\
         a.reduce(function(p,v){return p+v})",
        "var a=[1,2,3];Object.defineProperty(a,0,{get:function(){return 4},enumerable:true});\
         a.reduce(function(p,v){return p+v})",
        "var a=[1,2,3];Object.defineProperty(a,1,{get:function(){return 0},enumerable:true});\
         a.every(function(v){return v>0})",
        "var a=[1,2,3];Object.defineProperty(a,1,{get:function(){return 0},enumerable:true});\
         a.some(function(v){return v===0})",
        // 10.1.8.1 step 3.b: an accessor with no getter reads undefined.
        "var a=[1,2];Object.defineProperty(a,0,{set:function(v){},enumerable:true});\
         var s='';a.forEach(function(v){s+=typeof v});s",
        // The getter may write the array the walk is in the middle of.
        "var a=[1,2,3];Object.defineProperty(a,0,{get:function(){a.length=1;return 5},\
         enumerable:true});var s=0;a.forEach(function(v){s+=v});s",
        // A getter that throws leaves the walk through the same path a
        // throwing callback does.
        "var a=[1,2];Object.defineProperty(a,0,{get:function(){throw 1},enumerable:true});\
         try{a.forEach(function(){});0}catch(e){e}",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 13.15.5.5 over a value whose layout the lowering does not know: an array
/// pattern walks the iterator of 7.4.2, an object pattern reads properties
/// after 7.3.5 refused undefined and null.
#[test]
fn destructuring_assignment_over_an_unknown_value() -> Result<(), Error> {
    for source in [
        "function f(v){var a,b;[a,b]=v;return a+','+b}f([1,2])",
        "function f(v){var a,b;[a,...b]=v;return a+':'+b.join()}f([1,2,3])",
        "function f(v){var a;[,a]=v;return a}f([1,2])",
        "function f(v){var a;[a=7]=v;return a}f([])",
        "function f(v){var o={};[o.p]=v;return o.p}f([9])",
        "function f(v){var a,b;[[a,b]]=v;return a+b}f([[1,2]])",
        "function f(v){var a;[a]=v;return a}f({})",
        "function f(v){var a;({a}=v);return a}f({a:5})",
        "function f(v){var a;({a=7}=v);return a}f({})",
        "function f(v){var a;({a}=v);return a}f(null)",
        "function f(v){var a;({a}=v);return a}f(undefined)",
        "function f(v){var k='a',a;({[k]:a}=v);return a}f({a:3})",
        "function f(v){var a,b;({a,b=a}=v);return a+','+b}f({a:1})",
        "function f(v){var o={};({p:o.q}=v);return o.q}f({p:4})",
        // 13.15.5.5 writes each target as the iterator answers it.
        "function f(v){var s='';var t={set p(x){s+=x}};[t.p,t.p]=v;return s}f([1,2])",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 22.1.3.23 splits at every occurrence of a separator that is not an Object.
#[test]
fn a_string_splits_at_a_string_separator() -> Result<(), Error> {
    for source in [
        "'a,b,c'.split(',').join('|')",
        "'abc'.split('').join('|')",
        "'abc'.split('').length",
        "''.split(',').length",
        "''.split('').length",
        "'a,b'.split(',',1).join('|')",
        "'a,b'.split(',',0).length",
        "'abc'.split(undefined).length",
        "'abc'.split(undefined)[0]",
        "'a,,b'.split(',').length",
        "',a,'.split(',').length",
        "'abc'.split('b').join('|')",
        "'aaa'.split('aa').join('|')",
        "'abc'.split('',2).join('|')",
        "'abc'.split('x').join('|')",
        "'a1b'.split(1).join('|')",
        "'atrueb'.split(true).join('|')",
        "typeof ''.split",
        "''.split.length",
        "'a,b'.split(',',-1).length",
    ] {
        differential_scripts(&[source])?;
    }
    // 22.2.6.14 splits at a RegExp, appending the captures of each match.
    for source in [
        "'a1b2c'.split(/[0-9]/).join('|')",
        "'ab'.split(/x/).join('|')",
        "''.split(/x/).length",
        "''.split(/(?:)/).length",
        "'a1b'.split(/([0-9])/).join('|')",
        "'a1b'.split(/([0-9])/).length",
        "'abc'.split(/(?:)/).join('|')",
        "'A<B>b</B>c'.split(/<(\\/)?([^<>]+)>/).length",
        "typeof 'A<B>b</B>c'.split(/<(\\/)?([^<>]+)>/)[1]",
        "'a,b'.split(/,/,1).join('|')",
        "'ab'.split(/a*?/).join('|')",
        "'ab'.split(/a*/).join('|')",
        "var r=/,/g;r.lastIndex=5;'a,b'.split(r).join('|')+','+r.lastIndex",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 22.1.3.14 and 22.1.3.20 go through the methods 22.2.6 gives a `RegExp`.
#[test]
fn a_string_matches_and_searches_a_regexp() -> Result<(), Error> {
    for source in [
        "'a1b2c'.search(/[0-9]/)",
        "'abc'.search(/x/)",
        "'abc'.search(/^/)",
        "'a1b2'.match(/[0-9]/)[0]",
        "'a1b2'.match(/[0-9]/).index",
        "'a1b2'.match(/[0-9]/g).join('|')",
        "'abc'.match(/x/g)===null",
        "'abc'.match(/x/)===null",
        "'aaa'.match(/a*?/g).length",
        "'a1b'.match(/(a)(1)/)[1]",
        "'abc'.match(/b/).input",
        "var r=/a/g;r.lastIndex=2;''+'aaa'.search(r)+','+r.lastIndex",
        "var r=/a/g;''+'aaa'.match(r).length+','+r.lastIndex",
        "typeof ''.match",
        "''.match.length",
        "''.search.length",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// A native leaves to run 7.1.1 on each argument its clause converts, with the
/// hint that clause gives, and runs again from the beginning.
#[test]
fn a_native_converts_every_argument_its_clause_converts() -> Result<(), Error> {
    for source in [
        "'abcdef'.substring({valueOf:function(){return 1}},3)",
        "'abcdef'.slice({valueOf:function(){return 1}},{valueOf:function(){return 3}})",
        "'abc'.charAt({valueOf:function(){return 1}})",
        "'abc'.indexOf({toString:function(){return 'b'}})",
        "'abc'.indexOf('c',{valueOf:function(){return 0}})",
        "'abc'.padStart({valueOf:function(){return 5}},{toString:function(){return '-'}})",
        "[1,2,3].join({toString:function(){return '-'}})",
        "[1,2,3].indexOf(2,{valueOf:function(){return 0}})",
        "'ab'.repeat({valueOf:function(){return 2}})",
        "''+Math.pow({valueOf:function(){return 2}},{valueOf:function(){return 3}})",
        "'abc'.substring({},1)",
        // 13.15.2: the conversions run in the order the clause names them.
        "var n='';'abcd'.substring({valueOf:function(){n+='a';return 1}},\
         {valueOf:function(){n+='b';return 3}})+','+n",
        "var n='';'abcd'.indexOf({toString:function(){n+='s';return 'b'}},\
         {valueOf:function(){n+='p';return 0}})+','+n",
        // 7.1.4 passes the hint `number`, 7.1.17 the hint `string`.
        "var o={};o[Symbol.toPrimitive]=function(h){return h==='number'?2:'x'};\
         'abcdef'.slice(o,4)",
        "var o={};o[Symbol.toPrimitive]=function(h){return h==='string'?'b':9};\
         'abcdef'.indexOf(o)",
        "var o={};o[Symbol.toPrimitive]=function(h){return h};'abcdef'.indexOf(o)",
        "var o={};o[Symbol.toPrimitive]=function(h){return h};[1,2].join(o)",
        "var o={};o[Symbol.toPrimitive]=function(h){return h};''+o",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 14.15: the Catch Parameter is bound by 8.6.2, and the handler is compiled
/// against the top of the lattice for everything the Block could have written.
#[test]
fn a_catch_parameter_is_bound_by_a_pattern() -> Result<(), Error> {
    for source in [
        "var r='';try{throw {a:1,b:2}}catch({a,b}){r=a+','+b}r",
        "var r='';try{throw [1,2]}catch([x,y]){r=x+','+y}r",
        "var e=9;var r='';try{throw {e:1}}catch({e}){r=''+e}r+','+e",
        "var r='';try{throw {a:1}}catch({a,c=5}){r=a+','+c}r",
        "var r='';try{throw {a:{b:3}}}catch({a:{b}}){r=''+b}r",
        "var r=0;try{throw {}}catch({}){r=1}r",
        "var r='';try{throw [1,2,3]}catch([x,...y]){r=x+':'+y.length}r",
        // The handler sees whatever the Block left behind.
        "var x=1;try{x='a';x=2}catch(e){}typeof x",
        "var x=1;var r='';try{x='a';throw 0}catch(e){r=typeof x}r",
        "var a=[1,2];var r='';try{a.push(3);throw 0}catch(e){r=''+a.length}r",
        "var o={p:1};var r='';try{o.p='x';throw 0}catch(e){r=typeof o.p}r",
        // A Finally Block still runs on both paths.
        "var r=0;try{r=1}finally{r=r+1}r",
        "var r=0;try{throw 1}catch(e){r=e}finally{r=r+1}r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 19.2.2 to 19.2.5: the four value properties of the global object that read
/// a Number out of their argument.
#[test]
fn the_global_number_functions_answer_on_the_engine() -> Result<(), Error> {
    for source in [
        "isNaN(NaN)",
        "isNaN('x')",
        "isNaN('1')",
        "isNaN()",
        "isFinite(1/0)",
        "isFinite('2')",
        "''+parseInt('42')",
        "''+parseInt('0x1f')",
        "''+parseInt('ff',16)",
        "''+parseInt('  -7px')",
        "''+parseInt('z',36)",
        "''+parseInt('10',1)",
        "''+parseInt('')",
        "''+parseInt('11',2)",
        "''+parseFloat('3.14abc')",
        "''+parseFloat('  -Infinity')",
        "''+parseFloat('.5e2')",
        "''+parseFloat('x')",
        "typeof isNaN",
        "parseInt.length",
        "parseInt.name",
        "isNaN({valueOf:function(){return NaN}})",
        "''+parseInt({toString:function(){return '12'}})",
        "''+parseInt('12',{valueOf:function(){return 8}})",
    ] {
        differential_scripts(&[source])?;
    }
    // 19.2.3 gives `isNaN` one formal parameter; the stack backend gives it
    // none, and these check the engine against the specification.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert_eq!(realm.evaluate("isNaN.length")?, Value::Number(1.0));
    assert_eq!(realm.evaluate("isFinite.length")?, Value::Number(1.0));
    assert_eq!(realm.evaluate("parseFloat.length")?, Value::Number(1.0));
    Ok(())
}

/// 13.2.8.6: each substitution of a template goes through 7.1.17, which is
/// 7.1.1 with the hint `string` and not the one `+` gives.
#[test]
fn a_template_literal_answers_on_the_engine() -> Result<(), Error> {
    for source in [
        "`abc`",
        "``",
        "var x=1;`a${x}b`",
        "var x=1,y=2;`${x}${y}`",
        "`${1+2}`",
        "var o={toString:function(){return 'T'},valueOf:function(){return 9}};`v=${o}`",
        "var a=[1,2];`${a}`",
        "`${null} ${undefined} ${true}`",
        "var s='q';`a${`b${s}c`}d`",
        "var o={};`${o}`",
        "var n=0;var o={toString:function(){n++;return 'x'}};`${o}${o}`+n",
        "function f(v){return `<${v}>`}f(7)",
        "var o={};o[Symbol.toPrimitive]=function(h){return h};`${o}`",
        "typeof `a`",
        "`a`.length",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 10.4.4.7 maps the indices of a sloppy function's arguments object onto its
/// formal parameters. A function with none has an empty mapping, so its object
/// carries nothing that could be observed where it goes.
#[test]
fn an_arguments_object_with_no_mapping_leaves_its_frame() -> Result<(), Error> {
    for source in [
        "var arg;(function fun(){arg=arguments}(1,2,3));arg.length",
        "function f(){return arguments}f(1,2).length",
        "function f(){var a=arguments;return a[0]}f(5)",
        "function f(){return arguments}typeof f()",
        "'use strict';var arg;(function fun(a){arg=arguments}(1,2,3));arg.length",
    ] {
        differential_scripts(&[source])?;
    }
    // A sloppy function with a formal parameter carries the map of 10.4.4.7,
    // which the object keeps for as long as it lives.
    for source in [
        "var arg;(function fun(a){arg=arguments}(1,2,3));arg.length",
        "var arg;(function fun(a){arg=arguments;a=9}(1,2,3));arg[0]",
        "var arg;(function fun(a){arg=arguments}(1,2,3));arg[0]=8;arg[0]",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 14.7.5.6 step 7.g: a head that declares nothing evaluates its target as a
/// Reference of its own, once per iteration, and writes the step through it.
#[test]
fn a_for_head_that_declares_nothing_writes_its_target() -> Result<(), Error> {
    for source in [
        "var a;for(a of [1,2]){}a",
        "var a;for(a of []){}typeof a",
        "var s='';var a;for(a of [1,2,3]){s+=a}s",
        "var a;for([a] of [[1]]){}a",
        "var a;for([a=2] of [[]]){}a",
        "var a,b;for([a,b] of [[1,2],[3,4]]){}a+','+b",
        "var a;for({a} of [{a:1}]){}a",
        "var o={};for([o.p] of [[1]]){}o.p",
        "var a;for(a in {x:1}){}a",
        "var o={};for(o.k in {x:1,y:2}){}o.k",
        "var a;var n=0;for(a of [1,2]){n+=a}''+a+','+n",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 21.1.3 and 20.3.3: a method call on a Number or a Boolean resolves on the
/// Prototype 7.1.18 would give the wrapper, and 21.1.3.6 takes a radix.
#[test]
fn a_number_or_boolean_answers_its_prototype() -> Result<(), Error> {
    for source in [
        "(5).toString()",
        "(255).toString(16)",
        "(5).toString(2)",
        "(-5).toString(2)",
        "(0.5).toString(2)",
        "(5).valueOf()",
        "true.toString()",
        "false.valueOf()",
        "var n=5;n.toString(8)",
        "var x=1.5;x.toString()",
        "(0).toString(36)",
        "(1e21).toString()",
        "try{(5).toString(1)}catch(e){e instanceof RangeError}",
        "try{(5).toString(37)}catch(e){e instanceof RangeError}",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 9.1.1.1.1 leaves a lexical binding in its temporal dead zone until the
/// declaration initializes it, and the lowering has no zone to check.
#[test]
fn a_lexical_initializer_that_reads_its_own_binding_is_refused() -> Result<(), Error> {
    for source in [
        "let i=i++;i++",
        "let a=b,b=1;a",
        "let a=[a];a",
        "function f(){let x=x;return x}f()",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    // A name a nested function reads is read when that function runs, and a
    // `var` has no dead zone at all.
    for source in [
        "let f=function(){return 1};f()",
        "let a=1,b=a;b",
        "var i=i++;''+i",
        "let x=1;x",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 7.1.17 of an Object is 7.1.1 with the hint `string`. An object carrying
/// neither an `@@toPrimitive` nor a `toString` of the Script answers without a
/// frame, which is what a native has.
#[test]
fn a_native_converts_a_receiver_whose_conversion_needs_no_frame() -> Result<(), Error> {
    for source in [
        "new String('abc').split('b').join('|')",
        "String.prototype.charAt.call(new String('xy'),1)",
        "String.prototype.trim.call(new String('  a  '))",
        "''+String.prototype.split.call(new String('a,b'),',').length",
        "String.prototype.charAt.call(new Boolean(true),0)",
        "String.prototype.indexOf.call({},'o')",
        "''+String.prototype.slice.call(new Number(1234),1)",
        "/b/.exec(new String('abc'))[0]",
        "/b/.test(new String('abc'))",
        "''+/[0-9]/.exec(1234).index",
    ] {
        differential_scripts(&[source])?;
    }
    // A `toString` the Script wrote runs through the frame the clause leaves
    // for, as does the one 23.1.3.37 gives an Array.
    for source in [
        "var o={toString:function(){return 'hi'}};String.prototype.charAt.call(o,0)",
        "String.prototype.indexOf.call([1,2],'2')",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 23.1.3.12 and 23.1.3.13 walk backwards, and they and 23.1.3.9 and 23.1.3.10
/// read every index with 7.3.2, so a hole reaches the callback.
#[test]
fn the_find_clauses_walk_every_index() -> Result<(), Error> {
    for source in [
        "[1,2,3].findLast(function(x){return x<3})",
        "[1,2,3].findLastIndex(function(x){return x<3})",
        "typeof [1,2,3].findLast(function(x){return false})",
        "[1,2,3].findLastIndex(function(x){return false})",
        "var n=0;[,1].find(function(x){n++;return false});n",
        "var n=0;[,1].findIndex(function(x){n++;return false});n",
        "var n=0;[1,,3].findLast(function(x){n++;return false});n",
        "var r='';[1,2,3].findLast(function(x,i){r+=i});r",
        "var r='';[1,2,3].findLastIndex(function(x,i,a){r+=a.length});r",
        "typeof [].findLast(function(){return true})",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 23.1.3.17 and 23.1.3.4 answer the Array Iterator of 23.1.5 over the index
/// of each element, and over the index and the element together.
#[test]
fn an_array_iterates_its_keys_and_its_entries() -> Result<(), Error> {
    for source in [
        "var r='';for(var k of [7,8].keys()){r+=k}r",
        "var r='';for(var e of [7,8].entries()){r+=e[0]+':'+e[1]+','}r",
        "var i=[7,8].keys();''+i.next().value",
        "var i=[7,8].entries();i.next().value.join('-')",
        "var i=[7].values();''+i.next().value",
        "var i=[].keys();i.next().done",
        "typeof [].keys",
        "var i=[7].entries();i.next();i.next().done",
        "var r='';for(var k of [,1].keys()){r+=k}r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 20.2.3.1 and 28.1.1 call what they were given with the List 7.3.18 makes
/// out of an array-like, which no frame of the caller holds.
#[test]
fn apply_calls_with_a_list_of_arguments() -> Result<(), Error> {
    for source in [
        "function f(a,b){return a+b};''+f.apply(null,[1,2])",
        "function f(){return this.x};''+f.apply({x:5})",
        "function f(a,b){return a+b};''+f.apply(null,{length:2,0:3,1:4})",
        "function f(){return arguments.length};''+f.apply(null,[1,2,3])",
        "function f(){return arguments[1]};''+f.apply(null,[7,8])",
        "function f(a,b){return a+b};''+Reflect.apply(f,null,[1,2])",
        "function f(a){return a};typeof f.apply(null)",
        "function f(a){return a};typeof f.apply(null,[])",
        "typeof Function.prototype.apply",
        "function f(){return arguments.length};''+f.apply(null,{length:2})",
        "var r=0;try{Reflect.apply(function(){},null,null)}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{Reflect.apply(1,null,[])}catch(e){r=e instanceof TypeError}r",
        "function f(){'use strict';return this};typeof f.apply(undefined)",
    ] {
        differential_scripts(&[source])?;
    }
    // A callee written in Rust reads its arguments out of registers of the
    // caller, which a List is not.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert!(matches!(
        realm.evaluate("Math.max.apply(null,[1,5,2])"),
        Err(Error::Unsupported { .. })
    ));
    // 20.2.3.1 gives `apply` two formal parameters; the stack backend gives it
    // none, and this checks the engine against the specification.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert_eq!(
        realm.evaluate("Function.prototype.apply.length")?,
        Value::Number(2.0)
    );
    Ok(())
}

/// 7.1.20 reads a `length` that is an accessor by calling its getter, which
/// runs before the walk of 23.1.3 begins.
#[test]
fn a_walk_of_23_1_3_calls_the_getter_of_its_length() -> Result<(), Error> {
    for source in [
        "var o={get length(){return 2},0:'a',1:'b'};var r='';\
         Array.prototype.forEach.call(o,function(x){r+=x});r",
        "var o={get length(){return 2},0:1,1:2};\
         Array.prototype.map.call(o,function(x){return x*2}).join()",
        "var o={get length(){return 3},0:1,1:2,2:3};\
         ''+Array.prototype.reduce.call(o,function(a,b){return a+b})",
        "var n=0;var o={get length(){n++;return 1},0:5};\
         Array.prototype.forEach.call(o,function(){});''+n",
        "var o={get length(){throw 1},0:5};var r=0;\
         try{Array.prototype.forEach.call(o,function(){})}catch(e){r=e}''+r",
        "var o={get length(){return 2},0:1,1:2};\
         Array.prototype.filter.call(o,function(x){return x>1}).join()",
        "var o={get length(){return 0}};var r=0;\
         try{Array.prototype.reduce.call(o,function(){})}catch(e){r=e instanceof TypeError}r",
        "var o={get length(){return 1},0:1};var r=0;\
         try{Array.prototype.forEach.call(o,1)}catch(e){r=e instanceof TypeError}r",
        "var o={get length(){return 2},0:1,1:2};\
         ''+Array.prototype.findLast.call(o,function(x){return x<2})",
        "var o={set length(v){},0:1};''+Array.prototype.map.call(o,function(x){return x}).length",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 23.1.3.16, 23.1.3.17 and 23.1.3.20 walk the indices the way every other
/// clause of 23.1.3 does, so an element that is an accessor runs its getter.
#[test]
fn the_scan_clauses_walk_like_the_rest_of_23_1_3() -> Result<(), Error> {
    for source in [
        "''+[1,2,3].indexOf(2)",
        "''+[1,2,3].indexOf(9)",
        "''+[1,2,3].indexOf(2,2)",
        "''+[1,2,3].indexOf(3,-1)",
        "''+[1,2,3].indexOf(1,-99)",
        "''+[1,2,3].lastIndexOf(2)",
        "''+[1,2,1].lastIndexOf(1)",
        "''+[1,2,1].lastIndexOf(1,1)",
        "''+[1,2,3].lastIndexOf(9)",
        "''+[1,2,1].lastIndexOf(1,-2)",
        "''+[1,2,3].lastIndexOf(1,-99)",
        "[1,2,3].includes(2)",
        "[NaN].includes(NaN)",
        "''+[NaN].indexOf(NaN)",
        "[,1].includes(undefined)",
        "''+[,1].indexOf(undefined)",
        "''+[].indexOf(1)",
        "''+[].lastIndexOf(1)",
        "[].includes(1)",
        "var a=[1,2];Object.defineProperty(a,0,{get:function(){return 9},enumerable:true});\
         ''+a.indexOf(9)",
        "var a=[1,2];Object.defineProperty(a,1,{get:function(){return 9},enumerable:true});\
         ''+a.lastIndexOf(9)",
        "var a=[1,2];Object.defineProperty(a,0,{get:function(){return 9},enumerable:true});\
         a.includes(9)",
        "var o={get length(){return 2},0:'a',1:'b'};''+Array.prototype.indexOf.call(o,'b')",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 10.4.2.4 as a `[[DefineOwnProperty]]`: an Array's `length` is never
/// enumerable and never configurable, and its `[[Writable]]` only ever goes
/// from true to false.
#[test]
fn a_descriptor_sets_an_array_length() -> Result<(), Error> {
    for source in [
        "var a=[1,2,3];Object.defineProperty(a,'length',{value:1});a.length+','+a[1]",
        "var a=[1,2,3];Object.defineProperty(a,'length',{value:5});''+a.length",
        "var a=[1,2,3];Object.defineProperty(a,'length',{writable:false});a.length=1;''+a.length",
        "var a=[1,2,3];Object.defineProperty(a,'length',{writable:false});var r=0;\
         try{Object.defineProperty(a,'length',{value:1})}catch(e){r=e instanceof TypeError}r",
        "var a=[1];var r=0;\
         try{Object.defineProperty(a,'length',{enumerable:true})}catch(e){r=e instanceof TypeError}r",
        "var a=[1];var r=0;\
         try{Object.defineProperty(a,'length',{configurable:true})}catch(e){r=e instanceof TypeError}r",
        "var a=[1];var r=0;\
         try{Object.defineProperty(a,'length',{value:-1})}catch(e){r=e instanceof RangeError}r",
        "var a=[1];var r=0;try{Object.defineProperty(a,'length',{get:function(){return 1}})}\
         catch(e){r=e instanceof TypeError}r",
        "var a=[1,2,3];Object.getOwnPropertyDescriptor(a,'length').writable",
        "var a=[1];Object.defineProperty(a,'length',{writable:false});\
         Object.getOwnPropertyDescriptor(a,'length').writable",
        "var a=[1,2,3];a.length=1;''+a.length",
        "var a=[1];Object.defineProperty(a,'length',{value:0,writable:false});\
         ''+a.length+','+Object.getOwnPropertyDescriptor(a,'length').writable",
        "var a=[1];Object.defineProperty(a,'length',{});''+a.length",
        "var a=[1];Object.defineProperty(a,'length',{enumerable:false,configurable:false});''+a.length",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 16.1.7 makes a top-level lexical declaration a binding of the Global
/// Environment Record, which 8.6.2 initializes name by name for a pattern.
#[test]
fn a_global_lexical_declaration_binds_a_pattern() -> Result<(), Error> {
    for source in [
        "const [x]=[1];''+x",
        "let [x,y]=[1,2];''+(x+y)",
        "const {a}={a:1};''+a",
        "let [x=5]=[];''+x",
        "const [a,[b]]=[1,[2]];''+(a+b)",
        "let {a:{b}}={a:{b:7}};''+b",
        "const [x]=[1];function f(){return x};''+f()",
        "let [x,...r]=[1,2,3];''+r.length",
        "let [,x]=[1,2];''+x",
        "const {a,b}={a:1,b:2};''+(a+b)",
        "let {a='d'}={};a",
        "function f(v){let [a]=v;return a}''+f([3])",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 20.1.2.22 and 28.1.14 both run `OrdinarySetPrototypeOf` of 10.1.2: the same
/// value is always taken, a different one only while the object is extensible
/// and the chain stays acyclic.
#[test]
fn a_prototype_can_be_set_after_the_object_is_made() -> Result<(), Error> {
    for source in [
        "var o={};var p={x:1};Object.setPrototypeOf(o,p);''+o.x",
        "var o={};Object.setPrototypeOf(o,null);Object.getPrototypeOf(o)===null",
        "var o={};var p={};Reflect.setPrototypeOf(o,p)",
        "var o={};Object.preventExtensions(o);Reflect.setPrototypeOf(o,{})",
        "var o={};Object.preventExtensions(o);Reflect.setPrototypeOf(o,Object.getPrototypeOf(o))",
        "var a={};var b=Object.create(a);Reflect.setPrototypeOf(a,b)",
        "var r=0;try{Object.setPrototypeOf(null,{})}catch(e){r=e instanceof TypeError}r",
        "Object.setPrototypeOf(1,{})===1",
        "var r=0;try{Object.setPrototypeOf({},1)}catch(e){r=e instanceof TypeError}r",
        "var r=0;try{Reflect.setPrototypeOf(1,{})}catch(e){r=e instanceof TypeError}r",
        "var o={};var p={};Object.setPrototypeOf(o,p)===o",
        "var o={};Object.preventExtensions(o);var r=0;\
         try{Object.setPrototypeOf(o,{})}catch(e){r=e instanceof TypeError}r",
    ] {
        differential_scripts(&[source])?;
    }
    // 20.1.2.22 gives `setPrototypeOf` two formal parameters; the stack
    // backend gives it none, and this checks the engine against the
    // specification.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert_eq!(
        realm.evaluate("Object.setPrototypeOf.length")?,
        Value::Number(2.0)
    );
    Ok(())
}

/// 20.2.3.5 answers the source text of the grammar node a function was written
/// as, and the `NativeFunction` string of step 3 for one the engine wrote.
#[test]
fn a_function_answers_its_own_source_text() -> Result<(), Error> {
    for source in [
        "function f(a){return a};f.toString()",
        "var f=function(){};f.toString()",
        "var f=(x)=>x;f.toString()",
        "class C{m(){}};C.prototype.m.toString()",
        "class C{};C.toString()",
        "Math.max.toString()",
        "(function(){}).bind(null).toString()",
        "var o={m(){}};o.m.toString()",
        "String(function f(){})",
        "''+function f(){}",
        "`${function f(){}}`",
        "''+Math.max",
        "var f=function(){};typeof f.toString()",
        "Function.prototype.toString.length",
        "var r=0;try{Function.prototype.toString.call(1)}catch(e){r=e instanceof TypeError}r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 14.3.3.3 collects the rest of an object pattern with `CopyDataProperties`
/// of 7.3.25, which over a value the lowering could not name reads the own
/// enumerable keys at run time.
#[test]
fn an_object_rest_element_copies_what_the_pattern_left() -> Result<(), Error> {
    for source in [
        "function f(o){var {a,...r}=o;return a+','+r.b}f({a:1,b:2})",
        "function f(o){var {...r}=o;return ''+r.a}f({a:5})",
        "function f(o){var {a,...r}=o;return Object.keys(r).join()}f({a:1,b:2,c:3})",
        "function f(o){var {a,...r}=o;return typeof r}f(1)",
        "function f(o){var {...r}=o;return ''+Object.keys(r).length}f([1,2])",
        "function f(o){var {...r}=o;return ''+r[0]}f([7,8])",
        "function f(o){var {...r}=o;return Object.keys(r).join()}f('ab')",
        "function f(o){var {...r}=o;return r[1]}f('ab')",
        "function f(o){var {...r}=o;return ''+Object.keys(r).length}f(5)",
        "function f(o){var {...r}=o;return ''+Object.keys(r).length}f(true)",
        "function f(o){var {a,...r}=o;return Object.keys(r).length===0}f({a:1})",
        "function f(o){var a,r;({a,...r}=o);return a+','+r.b}f({a:1,b:2})",
        "var a,r;({a,...r}={a:1,b:2});''+r.b",
        "var o={a:1,b:2};var {a,...r}=o;''+r.b",
        "function f(o){var {...r}=o;return typeof r}f(null)",
        // 7.3.25 copies only the own enumerable keys.
        "function f(o){var {...r}=o;return Object.keys(r).join()}\
         f(Object.defineProperty({a:1},'b',{value:2}))",
    ] {
        differential_scripts(&[source])?;
    }
    // A property that is an accessor runs its getter, which a copy has no
    // frame for.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert!(matches!(
        realm.evaluate("function f(o){var {...r}=o;return r.a}f({get a(){return 1}})"),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

/// 7.4.9 closes an iterator a `break` left before its end; the normal exit
/// reached that end and closes nothing.
#[test]
fn a_break_out_of_a_for_of_closes_its_iterator() -> Result<(), Error> {
    for source in [
        "var n=0;var it={};it[Symbol.iterator]=function(){return {\
         next:function(){return {value:1,done:false}},return:function(){n++;return {}}}};\
         for(var x of it){break}''+n",
        "var n=0;var it={};it[Symbol.iterator]=function(){return {\
         next:function(){return {done:true}},return:function(){n++;return {}}}};\
         for(var x of it){}''+n",
        "var r=0;for(var x of [1,2,3]){if(x===2)break;r+=x}''+r",
        "var r='';for(var x of [1,2,3]){if(x===2)continue;r+=x}r",
        "var r=0;for(var x of [1,2]){r+=x}''+r",
        "var r=0;for(var x of [1,2,3]){for(var y of [1]){break}r+=x}''+r",
        "var it={};it[Symbol.iterator]=function(){return {\
         next:function(){return {value:1,done:false}}}};\
         var r=0;for(var x of it){r=x;break}''+r",
    ] {
        differential_scripts(&[source])?;
    }
    // A `return` closes the iterator of every loop it leaves.
    differential_scripts(&["function f(a){for(var x of a.values()){return x}}f([1])"])?;
    Ok(())
}

/// 20.1.3.7 answers the object `ToObject` made of the `this` value, which is
/// what 7.1.1 reaches for an object that carries no `valueOf` of its own.
#[test]
fn object_prototype_value_of_answers_the_object() -> Result<(), Error> {
    for source in [
        "var o={};o.valueOf()===o",
        "var o={};''+o",
        "var o={};''+(o*1)",
        "var o={valueOf:function(){return 7}};''+(o*2)",
        "var o={toString:function(){return 'x'}};''+o",
        "''+Object.prototype.valueOf.length",
        "var d=Object.getOwnPropertyDescriptor(Object.prototype,'valueOf');\
         ''+d.writable+d.enumerable+d.configurable",
        "var o={};o.valueOf.call(1)*2===2",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// Three answers the engine gave that the Script can tell from the right one:
/// 23.1.3.2 step 3, 20.5.6.2 and 13.10.2 step 5.
#[test]
fn the_engine_answers_concat_error_prototypes_and_instanceof_as_the_clauses_do() -> Result<(), Error>
{
    for source in [
        // 23.1.3.2.1 spreads the receiver only when it is an Array.
        "var x={};x.concat=Array.prototype.concat;var a=x.concat(1);''+a.length+(a[0]===x)",
        "var x={length:2,0:'a',1:'b'};x.concat=Array.prototype.concat;''+x.concat(1).length",
        "''+[1,2].concat([3],4)",
        "''+[].concat()",
        "''+[1].concat([2,3])",
        // 20.5.6.2 gives a native error constructor %Error% as its parent.
        "Object.getPrototypeOf(EvalError)===Error",
        "Object.getPrototypeOf(TypeError)===Error",
        "Object.getPrototypeOf(Error)===Function.prototype",
        "Object.getPrototypeOf(RangeError.prototype)===Error.prototype",
        // 13.10.2 step 5 throws for a right-hand side that is not callable.
        "var t=false;try{({}) instanceof {}}catch(e){t=e instanceof TypeError}t",
        "var t=false;try{1 instanceof 2}catch(e){t=e instanceof TypeError}t",
        "({}) instanceof Object",
        "1 instanceof Object",
        // 20.2.3 makes %Function.prototype% a built-in function of its own.
        "typeof Function.prototype",
        "Function.prototype()===undefined",
        "0 instanceof Function.prototype",
        "Function.prototype.name===''&&Function.prototype.length===0",
        "Object.prototype.toString.call(Function.prototype)",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 10.4.3.1 gives a String exotic object its `length` and its indices, which
/// is what a method of 23.1.3 walks when the `this` value is a String, and
/// 23.1.3.24 step 6 has no accumulator when it found no element at all.
#[test]
fn a_string_receiver_and_an_empty_reduce_answer_as_their_clauses_do() -> Result<(), Error> {
    for source in [
        "''+Array.prototype.map.call('abc',function(v){return v})",
        "''+Array.prototype.map.call('abc',function(v,i,o){return typeof o})",
        "''+Array.prototype.filter.call('abc',function(v){return v!=='b'})",
        "''+Array.prototype.indexOf.call('abc','b')",
        "''+Array.prototype.join.call('abc','-')",
        "''+Array.prototype.map.call('',function(v){return v}).length",
        "var a=new Array(10);var t=false;\
         try{a.reduce(function(){})}catch(e){t=e instanceof TypeError}t",
        "var t=false;try{[].reduce(function(){})}catch(e){t=e instanceof TypeError}t",
        "''+[].reduce(function(a,b){return a+b},7)",
        "''+[1,,3].reduce(function(a,b){return a+b})",
        "''+[1,2].reduce(function(a,b){return a+b})",
        // 7.3.2 reads an index of the Prototype Chain as much as an own one.
        "var a=[,,,];Array.prototype[1]='p';\
         var r=a.reduce(function(x,y){return y});delete Array.prototype[1];''+r",
        "var a=[,];Array.prototype[0]='p';var r=a.indexOf('p');delete Array.prototype[0];''+r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 20.5.3.4 joins the `name` and the `message` an Error holds, which is also
/// what 7.1.1 reaches for one.
#[test]
fn error_prototype_to_string_joins_the_name_and_the_message() -> Result<(), Error> {
    for source in [
        "var e=new Error('m');e.toString()",
        "new TypeError().toString()",
        "new Error().toString()",
        "var e=new Error('m');e.name='X';e.toString()",
        "Error.prototype.toString.call({name:'A',message:'b'})",
        "Error.prototype.toString.call({name:''})",
        "Error.prototype.toString.call({message:'only'})",
        "''+new Error('m')",
        "String(new TypeError('x'))",
        "''+Error.prototype.toString.length",
        "var t=false;try{Error.prototype.toString.call(1)}catch(e){t=e instanceof TypeError}t",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 28.1.2 calls a constructor with a List 7.3.18 makes and the object 10.1.13
/// makes for the `newTarget` it was given.
#[test]
fn reflect_construct_calls_a_constructor_with_a_list() -> Result<(), Error> {
    for source in [
        "function F(a){this.a=a}var o=Reflect.construct(F,[7]);''+o.a+(o instanceof F)",
        "function F(){}function G(){}var o=Reflect.construct(F,[],G);\
         ''+(o instanceof G)+(o instanceof F)",
        "function F(a,b){this.s=a+b}''+Reflect.construct(F,[1,2]).s",
        "function F(){return {own:1}}''+Reflect.construct(F,[]).own",
        "function isC(f){try{Reflect.construct(function(){},[],f)}catch(e){return false}\
         return true}''+isC(Array)+isC(Array.prototype.map)+isC(function(){})",
        "var t=false;try{Reflect.construct(function(){},1)}catch(e){t=e instanceof TypeError}t",
        "var t=false;try{Reflect.construct(1,[])}catch(e){t=e instanceof TypeError}t",
        "''+Reflect.construct.length+Reflect.construct.name",
        "function F(){this.n=arguments.length}''+Reflect.construct(F,{length:2,0:'a',1:'b'}).n",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 23.1.3.14, 23.1.3.30, 23.1.3.34 and 23.1.3.35: the four methods of
/// `%Array.prototype%` that need no callback.
#[test]
fn flat_sort_to_sorted_and_to_spliced_answer_on_the_new_engine() -> Result<(), Error> {
    for source in [
        "''+[3,1,2].sort()",
        "''+[3,1,2,undefined,,10].sort()",
        "var a=[3,1,,2];a.sort();''+a.length+(1 in a)+(3 in a)",
        "''+['b','a'].sort()",
        "''+[].sort()",
        "var a=[2,1];a.sort()===a",
        "''+[1,[2,[3]]].flat(2)",
        "''+[1,[2,[3]]].flat(Infinity)",
        "''+[1,[],2].flat()",
        "''+[1,[2,,3]].flat().length",
        "''+[3,1,2].toSorted()",
        "''+[,1].toSorted()",
        "''+[1,2,3].toSpliced(1,1,9)",
        "''+[1,2,3].toSpliced(1)",
        "''+[1,2,3].toSpliced()",
        "''+[1,2,3].toSpliced(-1,1,'x','y')",
        "''+Array.prototype.sort.length+Array.prototype.flat.length\
         +Array.prototype.toSpliced.length+Array.prototype.toSorted.length",
        "var t=false;try{[1].sort(1)}catch(e){t=e instanceof TypeError}t",
    ] {
        differential_scripts(&[source])?;
    }
    // A comparator of the Script needs a frame this native has none of.
    let program = compile("[2,1].sort(function(a,b){return a-b})", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

/// 7.1.20 reads the `length` of an array-like with 7.3.2 and converts it,
/// which for an Object runs a method of the Script: the walk enters it the
/// way it enters the getter of an accessor `length`.
#[test]
fn an_array_like_length_that_is_an_object_is_converted_in_a_frame() -> Result<(), Error> {
    for source in [
        "var o={0:1,1:2,length:{toString:function(){return '2'}}};\
         ''+Array.prototype.map.call(o,function(x){return x})",
        "var o={0:1,1:2,length:{valueOf:function(){return 2}}};\
         ''+Array.prototype.indexOf.call(o,2)",
        "var o={0:1,length:{}};''+Array.prototype.map.call(o,function(x){return x}).length",
        "var n=0;var o={0:1,length:{valueOf:function(){n++;return 1}}};\
         ''+Array.prototype.forEach.call(o,function(){})+n",
        "var o={0:1,1:2,length:{valueOf:function(){return {}},\
         toString:function(){return 2}}};''+Array.prototype.map.call(o,function(x){return x})",
        "var o={0:1,length:{valueOf:function(){return 1}}};\
         ''+Array.prototype.reduce.call(o,function(a,b){return a+b})",
        // 7.1.20 keeps a length past 2^31-1, which the walk starts from.
        "var a=[];a[Math.pow(2,32)-2]=null;''+a.lastIndexOf(null,Infinity)",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// A native that converts an argument runs again from the beginning, and the
/// operation it runs is the one a call of it runs: a clause of 23.1.3 that
/// opens a frame still opens one after the conversion.
#[test]
fn a_coerced_argument_leaves_the_clause_on_the_path_a_call_takes() -> Result<(), Error> {
    for source in [
        "var o={0:1};Object.defineProperty(o,'length',{get:function(){return 1},\
         configurable:true});''+Array.prototype.indexOf.call(o,1,{valueOf:function(){return 0}})",
        "''+[1,2,3].indexOf(2,{valueOf:function(){return 0}})",
        "''+[1,2,3].lastIndexOf(2,{valueOf:function(){return 2}})",
        "''+[1,2,3].includes(2,{valueOf:function(){return 0}})",
        "var n=0;var o={0:1,1:2};Object.defineProperty(o,'length',{get:function(){n++;return 2},\
         configurable:true});''+Array.prototype.indexOf.call(o,2,{valueOf:function(){return 0}})+n",
        // 7.1.5 converts that argument after the `length` is read.
        "var r='';var o={0:1};Object.defineProperty(o,'length',\
         {get:function(){r+='L';return 1},configurable:true});\
         Array.prototype.indexOf.call(o,1,{valueOf:function(){r+='F';return 0}});r",
        "''+[1,2,3].indexOf(2,1)+[1,2,3].indexOf(2,-1)",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 22.2.3.1 makes a `RegExp` of a pattern and flags 22.2.4.1 compiles where the
/// call stands, so the instance holds the automaton rather than an index into
/// the unit the Script was compiled to.
#[test]
fn the_regexp_constructor_compiles_where_the_call_stands() -> Result<(), Error> {
    for source in [
        "''+new RegExp('a').test('a')",
        "typeof new RegExp()",
        "''+Array.isArray(new RegExp())",
        "var r=/a/g;''+(RegExp(r)===r)",
        "''+new RegExp('a')",
        "''+new RegExp()",
        "''+new RegExp('')",
        "''+/a/g",
        "var t=false;try{new RegExp('(')}catch(e){t=e instanceof SyntaxError}''+t",
        "''+new RegExp('a').lastIndex",
        "''+new RegExp('a').exec('bab').index",
        "''+(new RegExp('a') instanceof RegExp)",
        "''+new RegExp('a','g').test('bab')",
        "var r=new RegExp('a','g');r.test('aa');''+r.lastIndex",
        "''+new RegExp(/a/g).test('a')",
    ] {
        differential_scripts(&[source])?;
    }
    // A pattern that is an Object and not a RegExp needs a frame for 7.1.17.
    let program = compile("new RegExp({})", Limits::default())?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

/// 23.1.2.5 and 22.2.5.2 give their constructor a `@@species` accessor whose
/// getter answers the `this` value it was read off.
#[test]
fn the_species_getter_answers_the_constructor_it_was_read_off() -> Result<(), Error> {
    for source in [
        "''+(Array[Symbol.species]===Array)",
        "''+(RegExp[Symbol.species]===RegExp)",
        "typeof Object.getOwnPropertyDescriptor(Array,Symbol.species)",
        "Object.getOwnPropertyDescriptor(Array,Symbol.species).get.name",
        "''+Object.getOwnPropertyDescriptor(Array,Symbol.species).get.length",
        "var d=Object.getOwnPropertyDescriptor(Array,Symbol.species);\
         ''+(d.set===undefined)+d.enumerable+d.configurable",
        "''+Object.getOwnPropertyDescriptor(Array,Symbol.species).get.call(7)",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 20.2.3.2 keeps every argument after the `this` value, and 10.4.1.1 puts
/// them in front of the ones the call site passes.
#[test]
fn a_bound_function_keeps_the_arguments_the_bind_gave_it() -> Result<(), Error> {
    for source in [
        "function f(a,b){}f.name",
        "function f(a,b){}var b=f.bind(null,1);''+b.length",
        "function f(a,b){return a+b}var b=f.bind(null,1);''+b(2)",
        "function f(a,b){}var b=f.bind(null,1);b.name+b.length",
        "function f(){return this.x}var b=f.bind({x:7});''+b()",
        "function f(a,b,c){return a+b+c}''+f.bind(null,1).bind(null,2)(3)",
        "var b=(function f(){}).bind(null);b.name+b.length",
        "function f(){return arguments.length}''+f.bind(null,1,2)(3)",
        "(function(){}).bind(null).bind(null).name",
        "function f(a,b,c){}''+f.bind(null,1,2,3,4).length",
        "function f(){return this===undefined}''+f.bind()()",
    ] {
        differential_scripts(&[source])?;
    }
    // 10.4.1.2 constructs the target with the arguments the bind kept and the
    // `newTarget` the call site gave, which this engine has not built.
    let program = compile(
        "function f(a){this.a=a}new (f.bind(null,1))()",
        Limits::default(),
    )?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

/// 22.1.3.13 step 5 and 22.1.3.15 step 4 make a `RegExp` of an argument that
/// is not one, which 22.2.3.1 compiles where the call stands.
#[test]
fn match_and_search_make_a_regexp_of_what_they_were_given() -> Result<(), Error> {
    for source in [
        "''+'abcbd'.match('b').index",
        "''+'abcbd'.search('c')",
        "''+'abc'.search(/b/)",
        "''+'abc'.match(/b/)[0]",
        "''+'abc'.search()",
        "''+'a.c'.search('.')",
        "''+'abc'.match('x')",
        "''+'abc'.search('x')",
        "var t=false;try{'a'.search('(')}catch(e){t=e instanceof SyntaxError}''+t",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// A method read off an instance is a callee like any other: `[].slice.call`
/// reads `call` off `%Array.prototype%.slice`, which carries the methods of
/// 20.2.3 the way a function of the Script does. 7.1.5 over 7.1.4 step 2
/// refuses a Symbol with the `TypeError`, not with a gap.
#[test]
fn an_intrinsic_read_off_an_instance_carries_the_methods_of_20_2_3() -> Result<(), Error> {
    for source in [
        "''+[].slice.length",
        "''+[].slice.call([1,2],0)",
        "''+[].concat.call([1],2)",
        "''+'abc'.charAt.call('xyz',1)",
        "typeof [].slice.call",
        "var s=[].slice;''+s.call([1,2],1)",
        "''+[1,2].slice(0)",
        "var o={};o.length=Symbol(1);var t=false;\
         try{[].copyWithin.call(o,0,0)}catch(e){t=e instanceof TypeError}''+t",
        "var t=false;try{[1].slice(Symbol())}catch(e){t=e instanceof TypeError}''+t",
        "var t=false;try{'abc'.charAt(Symbol())}catch(e){t=e instanceof TypeError}''+t",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 22.1.3.19 gives the search value its own say through `@@replace`, which
/// 22.2.6.11 answers for a `RegExp`; a search value that is a String takes the
/// first occurrence alone.
#[test]
fn replace_substitutes_the_first_match_or_every_global_one() -> Result<(), Error> {
    for source in [
        "'abcb'.replace('b','x')",
        "'abcb'.replace(/b/g,'x')",
        "'abcb'.replace(/b/,'[$&]')",
        "'abc'.replace('b','$$')",
        "typeof ''.replace",
        "''+''.replace.length",
        "'abc'.replace('x','y')",
        "'aaa'.replace(/a/g,'')",
        "'abc'.replace(/(b)/,'<$1>')",
        "'abc'.replace('','X')",
        "'abc'.replace(/x*/g,'-')",
        "'abc'.replace(/b/,\"[$`|$']\")",
        "'abc'.replace(/(a)(b)/,'$2$1')",
        "'abc'.replace(/b/,'$9')",
        "var r=/b/g;'abcb'.replace(r,'x');''+r.lastIndex",
        "'aaa'.replace(/a/,'$&$&')",
    ] {
        differential_scripts(&[source])?;
    }
    // Step 8 calls a replace value that is callable, which needs a frame.
    let program = compile(
        "'abc'.replace('b',function(){return 'x'})",
        Limits::default(),
    )?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

/// 23.1.3.18 sends every element through 7.1.17, which for an Object is 7.1.1
/// with the hint `string`. A method this Realm built answers without a frame;
/// one of the Script needs one and stays a named gap.
#[test]
fn a_join_converts_an_object_element_with_the_methods_of_the_realm() -> Result<(), Error> {
    for source in [
        "''+[{},{}]",
        "''+[[1,[2]],3]",
        "''+[new Error('x')]",
        "''+[1,2]",
        "String([{}])",
        "''+[{},1,null,undefined,'a']",
        "[{}].join('-')",
        "[[1]].join('-')",
        "''+[[]]",
        "''+[[[]]]",
    ] {
        differential_scripts(&[source])?;
    }
    // A `toString` of the Script needs a frame this native has none of.
    let program = compile(
        "var o={toString:function(){return 'T'}};''+[o]",
        Limits::default(),
    )?;
    assert!(program.uses_register_backend());
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

/// 7.1.19 step 2 sends an Object key through 7.1.1 with the hint `string`,
/// which the engine runs where the access stands.
#[test]
fn an_object_property_key_goes_through_to_property_key() -> Result<(), Error> {
    for source in [
        "var o={};o[{}]=1;''+o['[object Object]']",
        "var o={};o[[1,2]]=3;''+o['1,2']",
        "var o={};o[new Boolean(true)]=1;''+o['true']",
        "var o={'1,2':7};''+o[[1,2]]",
        "var o={a:1};''+delete o[['a']]",
        "var o={};o[1]=2;''+o[1]",
    ] {
        differential_scripts(&[source])?;
    }
    // A `toString` of the Script runs in the frame the access opens for it.
    differential_scripts(&["var o={a:1};o[{toString:function(){return 'a'}}]"])?;
    Ok(())
}

/// 7.3.18 reads the `length` before the clause does anything else, and a
/// getter there runs a method of the Script: the clause enters it and starts
/// again with the length it answered, so the getter runs exactly once.
#[test]
fn an_accessor_length_runs_its_getter_once_for_a_clause_of_23_1_3() -> Result<(), Error> {
    for source in [
        "var n=0;var o={0:'a',1:'b'};\
         Object.defineProperty(o,'length',{get:function(){n++;return 2}});\
         ''+Array.prototype.join.call(o,'-')+n",
        "var o={0:1,1:2};Object.defineProperty(o,'length',{get:function(){return 2}});\
         ''+Array.prototype.slice.call(o,0)",
        "var o={0:1};Object.defineProperty(o,'length',{get:function(){throw new TypeError()}});\
         var t=false;try{Array.prototype.copyWithin.call(o,0,0)}catch(e){t=e instanceof TypeError}\
         ''+t",
        "var o={0:1,1:2};Object.defineProperty(o,'length',{get:function(){return 2}});\
         ''+Array.prototype.indexOf.call(o,2)",
        "var o={0:3,1:1};Object.defineProperty(o,'length',{get:function(){return 2}});\
         ''+Array.prototype.toSorted.call(o)",
        "''+[1,2].join('-')",
        // 7.1.20 converts the value the read answered, and an Object there
        // runs `valueOf` and then `toString` of the Script.
        "var o={0:1,1:2,length:{valueOf:function(){return 2}}};\
         ''+Array.prototype.slice.call(o,0)",
        "var o={0:1,1:2,length:{valueOf:function(){return {}},\
         toString:function(){return 2}}};''+Array.prototype.slice.call(o,0)",
        "var o={0:1,length:{}};''+Array.prototype.slice.call(o,0).length",
        "var n=0;var o={0:'a',1:'b'};Object.defineProperty(o,'length',\
         {get:function(){n++;return {valueOf:function(){return 2}}}});\
         ''+Array.prototype.join.call(o,'-')+n",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 23.1.3.37 gives `%Array.prototype%` an `@@unscopables` object with no
/// Prototype, whose own properties are the names 13.3.1.1 keeps out of a
/// `with` binding.
#[test]
fn array_prototype_carries_the_unscopables_of_23_1_3_37() -> Result<(), Error> {
    for source in [
        "typeof Array.prototype[Symbol.unscopables]",
        "''+(Object.getPrototypeOf(Array.prototype[Symbol.unscopables])===null)",
        "Object.keys(Array.prototype[Symbol.unscopables]).join()",
        "var d=Object.getOwnPropertyDescriptor(Array.prototype,Symbol.unscopables);\
         ''+d.writable+d.enumerable+d.configurable",
        "''+Array.prototype[Symbol.unscopables].at",
        "var d=Object.getOwnPropertyDescriptor(Array.prototype[Symbol.unscopables],'at');\
         ''+d.writable+d.enumerable+d.configurable",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 23.1.2.3 makes an Array of the arguments, and 23.1.2.1 takes the elements
/// of an array-like. A mapper and an `@@iterator` each run a method of the
/// Script, which the native names.
#[test]
fn array_of_and_array_from_build_an_array_of_what_they_were_given() -> Result<(), Error> {
    for source in [
        "''+Array.of(1,2,3)",
        "''+Array.of()",
        "''+Array.of(undefined).length",
        "''+Array.from({0:'a',1:'b',length:2})",
        "''+Array.from({length:2})",
        "''+Array.from({length:2},function(x,i){return i})",
        "''+Array.from({0:1,1:2,length:2},function(x){return x*2})",
        "var t='';Array.from({0:1,length:1},function(x,i,o){t=typeof o});t",
        "var o={0:1,1:2};Object.defineProperty(o,'length',{get:function(){return 2}});\
         ''+Array.from(o,function(x){return x+1})",
        "''+Array.from({length:0}).length",
        "''+Array.of.length+Array.from.length",
        "Array.of.name+Array.from.name",
        "var t=false;try{Array.from(null)}catch(e){t=e instanceof TypeError}''+t",
        "var t=false;try{Array.from({length:1},1)}catch(e){t=e instanceof TypeError}''+t",
    ] {
        differential_scripts(&[source])?;
    }
    // An `@@iterator` decides the whole clause, and this Realm reaches one
    // only through a frame.
    for source in ["Array.from([1,2])", "Array.from('ab')"] {
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

/// 23.1.3.4 asks the `constructor` of an Array and its `@@species` before it
/// makes the answer of 23.1.3.21, 23.1.3.8, 23.1.3.28, 23.1.3.31, 23.1.3.1 and
/// 23.1.3.14.
#[test]
fn a_copying_clause_of_23_1_3_makes_its_answer_with_array_species_create() -> Result<(), Error> {
    for source in [
        "''+[1,2].map(function(x){return x*2})",
        "''+[1,2,3].filter(function(x){return x>1})",
        "''+[1,2,3].slice(1)",
        "var a=[1,2,3];''+a.splice(1,1)+a",
        "''+[1].concat([2])",
        "''+[1,[2]].flat()",
        "var a=[1];Object.defineProperty(a,'constructor',{value:Array});''+a.slice(0)",
        "var a=[1];Object.defineProperty(a,'constructor',{value:{}});''+a.slice(0)",
        "var a=[1];Object.defineProperty(a,'constructor',{value:undefined});''+a.slice(0)",
        "var a=[1];Object.defineProperty(a,'constructor',{value:null});var t=false;\
         try{a.slice(0)}catch(e){t=e instanceof TypeError}''+t",
        "var a=[1];Object.defineProperty(a,'constructor',{value:null});var t=false;\
         try{a.concat()}catch(e){t=e instanceof TypeError}''+t",
        "var a=[1];Object.defineProperty(a,'constructor',{value:null});var t=false;\
         try{a.map(function(x){return x})}catch(e){t=e instanceof TypeError}''+t",
        "var o={0:1,length:1};''+Array.prototype.slice.call(o,0)",
    ] {
        differential_scripts(&[source])?;
    }
    // A species that is a constructor of the Script is a frame this clause
    // has none of.
    let source = "var a=[1];Object.defineProperty(a,'constructor',{value:function(){}});a.slice(0)";
    let program = compile(source, Limits::default())?;
    assert!(program.uses_register_backend(), "{source}");
    assert!(matches!(
        Runtime::with_backend(Limits::default(), Backend::Engine).run(&program, &mut SilentHost),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

/// 20.2.1.1 builds the source text of a function and compiles it at run time.
/// The engine holds no units, so the run stops, the embedding makes one, and
/// the call instruction runs again with it.
#[test]
fn the_function_constructor_compiles_a_body_at_run_time() -> Result<(), Error> {
    for source in [
        "var f=Function('a','b','return a+b');''+f(1,2)",
        "var f=new Function('return 7');''+f()",
        "var f=Function('');typeof f",
        "''+Function('a','return a').length",
        "Function('a','return a').name",
        "var t=false;try{Function('(')}catch(e){t=e instanceof SyntaxError}''+t",
        "''+(Function('return 1') instanceof Function)",
        "var f=Function('return this');typeof f()",
        "''+Function()()",
        "var f=Function('a,b','return b');''+f(1,2)",
        "var f=Function('return Function(\"return 5\")()');''+f()",
        "''+(typeof Function('return 1').prototype)",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

/// 19.2.1 evaluates a Script of the same Realm, which the embedding runs on
/// the heap and Realm the caller is using: a `var` it declares is a binding of
/// the same Global Environment Record.
#[test]
fn eval_runs_a_script_of_the_same_realm() -> Result<(), Error> {
    for source in [
        "''+eval('1+1')",
        "eval('\"a\"')",
        "typeof eval(5)",
        "eval('var g=7');''+g",
        "''+eval('var h=1; h+1')",
        "var t=false;try{eval('(')}catch(e){t=e instanceof SyntaxError}''+t",
        "''+eval(\"eval('3')\")",
        "''+eval('')",
        "eval('function f(){return 4}');''+f()",
        "var t=false;try{eval('throw new TypeError()')}catch(e){t=e instanceof TypeError}''+t",
        "''+(typeof eval)",
        "''+eval.length",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_promise_settles_through_the_job_queue_of_both_backends() -> Result<(), Error> {
    // 9.5 runs every job the Script enqueued after the Script has answered, so
    // the log is read by a second Script of the same Realm.
    for source in [
        // 27.2.3.1: the executor runs at once and the reaction after it.
        "var l=[];new Promise(function(r){l.push('e');r(1)}).then(function(v){l.push('t'+v)});l.push('s');0",
        // 27.2.5.4: a chain runs one reaction per turn.
        "var l=[];Promise.resolve(1).then(function(v){l.push('a'+v);return v+1}).then(function(v){l.push('b'+v)});0",
        // 27.2.4.6 and 27.2.5.1.
        "var l=[];Promise.reject('x').catch(function(e){l.push('c'+e)});0",
        // 27.2.3.1 step 7: a throw of the executor rejects the promise.
        "var l=[];new Promise(function(){throw 'q'}).catch(function(e){l.push('x'+e)});0",
        // 27.2.2.1 step 5: a throw of a handler rejects the capability.
        "var l=[];Promise.resolve(1).then(function(){throw 'h'}).catch(function(e){l.push('h'+e)});0",
        // 27.2.1.3.2 step 8: a thenable is adopted through a job of its own.
        "var l=[];new Promise(function(r){r({then:function(a){l.push('n');a(5)}})}).then(function(v){l.push('t'+v)});0",
        // A promise resolved with a promise takes two more turns.
        "var l=[];var i=Promise.resolve('i');new Promise(function(r){r(i)}).then(function(v){l.push('p'+v)});Promise.resolve().then(function(){l.push('1')}).then(function(){l.push('2')}).then(function(){l.push('3')});0",
        // 27.2.1.3.2 step 6: a promise resolved with itself rejects.
        "var l=[];var f;var p=new Promise(function(r){f=r});f(p);p.catch(function(e){l.push('self'+(e instanceof TypeError))});0",
        // 27.2.1.3.1: the pair settles once.
        "var l=[];var f;var p=new Promise(function(r,j){f=r;j('late')});f('first');p.then(function(v){l.push('f'+v)},function(e){l.push('r'+e)});0",
        // 27.2.5.4.1 steps 3 and 4: a handler that is not callable is empty.
        "var l=[];Promise.resolve('v').then(null).then(undefined,null).then(function(v){l.push('p'+v)});Promise.reject('w').then(1).catch(function(e){l.push('q'+e)});0",
        // 27.2.4.7 step 2 answers the argument itself.
        "var l=[];var i=Promise.resolve(1);l.push('same'+(Promise.resolve(i)===i));0",
        // 10.4.4 in a handler, which takes its arguments from the job.
        "var l=[];Promise.resolve(9).then(function(a){l.push('n'+arguments.length+a)});0",
        // The two reaction lists of one promise run in the order they were
        // added.
        "var l=[];var p=Promise.resolve(0);p.then(function(){l.push('A')});p.then(function(){l.push('B')});0",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        engine.evaluate(source)?;
        stack.evaluate(source)?;
        let expected = stack.evaluate("l.join('|')")?;
        assert_eq!(engine.evaluate("l.join('|')")?, expected, "{source}");
    }
    Ok(())
}

#[test]
fn the_transcendentals_of_21_3_2_answer_the_special_values_they_name() -> Result<(), Error> {
    // The stack backend carries none of them, so only the engine answers.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    for (source, expected) in [
        // 21.3.2.32 names an exact result; every other clause approximates.
        ("''+Math.sqrt(9)", "3"),
        ("''+Math.sqrt(2)", "1.4142135623730951"),
        ("''+Math.sqrt(-1)", "NaN"),
        ("''+Math.cbrt(27)", "3"),
        ("''+(1/Math.cbrt(-0))", "-Infinity"),
        ("''+Math.cos(0)", "1"),
        ("''+Math.cos(Infinity)", "NaN"),
        ("''+(1/Math.tan(-0))", "-Infinity"),
        ("''+Math.exp(0)", "1"),
        ("''+Math.exp(-Infinity)", "0"),
        ("''+Math.expm1(-Infinity)", "-1"),
        ("''+Math.log(0)", "-Infinity"),
        ("''+Math.log(-1)", "NaN"),
        ("''+Math.log2(1024)", "10"),
        ("''+Math.log10(1e21)", "21"),
        ("''+Math.atan(Infinity)", "1.5707963267948966"),
        ("''+Math.atan2(1,1)", "0.7853981633974483"),
        ("''+Math.atan2(0,-0)", "3.141592653589793"),
        ("''+Math.atan2(Infinity,Infinity)", "0.7853981633974483"),
        ("''+Math.asin(1.5)", "NaN"),
        ("''+Math.acos(1)", "0"),
        ("''+Math.cosh(-Infinity)", "Infinity"),
        ("''+Math.tanh(Infinity)", "1"),
        ("''+Math.acosh(0.5)", "NaN"),
        ("''+Math.atanh(1)", "Infinity"),
        ("''+Math.hypot(3,4)", "5"),
        ("''+Math.hypot()", "0"),
        ("''+Math.hypot(Infinity,NaN)", "Infinity"),
        ("''+Math.f16round(1.337)", "1.3369140625"),
        ("''+Math.f16round(65520)", "Infinity"),
        // 21.3.2.27 answers a Number of the interval and nothing else.
        ("''+(Math.random()>=0&&Math.random()<1)", "true"),
        // 17 gives each one the length and the name its clause names.
        (
            "''+Math.hypot.length+Math.atan2.length+Math.random.length+Math.sqrt.length",
            "2201",
        ),
        ("Math.f16round.name", "f16round"),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(expected), "{source}");
    }
    Ok(())
}

#[test]
fn flat_map_and_from_entries_answer_what_their_clauses_ask() -> Result<(), Error> {
    // 23.1.3.13 is 23.1.3.13.1 with the depth one: an answer that is an Array
    // contributes its elements and every other answer contributes itself.
    for source in [
        "[1,2,3].flatMap(function(e){return [e*2]}).join(',')",
        "[1,2].flatMap(function(e){return e*2}).join(',')",
        "''+[1,2].flatMap(function(e){return [[e]]}).length",
        "var l=[];[1,2].flatMap(function(e,i,a){l.push(e+'/'+i+'/'+a.length)});l.join('|')",
        "''+[1,,3].flatMap(function(e){return [e]}).length",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    // 20.1.2.7 makes an object of the pairs; the stack backend has it not.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert_eq!(
        realm.evaluate("Object.keys(Object.fromEntries([['a',1],['b',2]])).join(',')")?,
        Value::string("a,b")
    );
    assert_eq!(
        realm.evaluate("''+Object.fromEntries([['a',1]]).a")?,
        Value::string("1")
    );
    Ok(())
}

#[test]
fn the_four_reflective_functions_this_realm_had_not_built() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    for (source, expected) in [
        // 20.1.2.1 copies the own enumerable properties of every source.
        (
            "var t={a:1};''+(Object.assign(t,{b:2},{c:3})===t)+'/'+t.a+t.b+t.c",
            "true/123",
        ),
        (
            "var s={};s[Symbol.iterator]=1;''+Object.assign({},s)[Symbol.iterator]",
            "1",
        ),
        (
            "var o={a:1};Object.defineProperty(o,'h',{value:2,enumerable:false});var t=Object.assign({},o);''+t.a+'/'+(t.h===undefined)",
            "1/true",
        ),
        // 20.1.2.11 answers the own Symbol keys alone.
        (
            "var s=Symbol('x'),o={};o[s]=1;''+Object.getOwnPropertySymbols(o).length+'/'+(Object.getOwnPropertySymbols(o)[0]===s)",
            "1/true",
        ),
        // 20.1.2.9 answers one descriptor per own key.
        (
            "var d=Object.getOwnPropertyDescriptors({a:1,b:2});''+d.a.value+d.b.value+'/'+d.a.writable",
            "12/true",
        ),
        // 28.1.13 answers whether it wrote.
        ("var o={};''+Reflect.set(o,'x',5)+'/'+o.x", "true/5"),
        (
            "var a=[];''+Reflect.set(a,'0',7)+'/'+a[0]+'/'+a.length",
            "true/7/1",
        ),
        // 10.1.6.3 step 2 refuses a new property of an object that is not
        // extensible, which the stack backend answers `true` for.
        (
            "var o=Object.freeze({});''+Reflect.set(o,'x',5)+'/'+(o.x===undefined)",
            "false/true",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(expected), "{source}");
    }
    Ok(())
}

#[test]
fn the_html_methods_of_b_2_2_wrap_the_text_in_a_tag() -> Result<(), Error> {
    // B.2.2.2.1 builds the tag, one attribute where the method names one, and
    // the text between the two ends. The stack backend carries none of the
    // thirteen, so only the engine answers here.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    for (source, expected) in [
        ("'x'.anchor('y')", "<a name=\"y\">x</a>"),
        ("'x'.anchor('a\"b')", "<a name=\"a&quot;b\">x</a>"),
        ("'x'.link('u')", "<a href=\"u\">x</a>"),
        ("'x'.fontcolor('c')", "<font color=\"c\">x</font>"),
        ("'x'.fontsize(3)", "<font size=\"3\">x</font>"),
        ("'x'.bold()", "<b>x</b>"),
        ("'x'.italics()", "<i>x</i>"),
        ("'x'.fixed()", "<tt>x</tt>"),
        ("'x'.big()", "<big>x</big>"),
        ("'x'.small()", "<small>x</small>"),
        ("'x'.blink()", "<blink>x</blink>"),
        ("'x'.strike()", "<strike>x</strike>"),
        ("'x'.sub()", "<sub>x</sub>"),
        ("'x'.sup()", "<sup>x</sup>"),
        // B.2.2.12 and B.2.2.13 are the function objects of 22.1.3.31 and
        // 22.1.3.32 under a second name.
        ("'  a '.trimLeft()", "a "),
        ("'  a '.trimRight()", "  a"),
        (
            "''+(String.prototype.trimLeft===String.prototype.trimStart)",
            "true",
        ),
        ("''+'x'.trimLeft.name", "trimStart"),
        // 10.3.3 gives each one the length B.2.2 names.
        (
            "''+'x'.anchor.length+'/'+'x'.bold.length+'/'+'x'.anchor.name",
            "1/0/anchor",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(expected), "{source}");
    }
    Ok(())
}

#[test]
fn a_spread_element_takes_every_value_of_its_iterator() -> Result<(), Error> {
    for source in [
        // 13.2.4.2: an Array literal.
        "[...[1,2],3].join(',')",
        "[0,...[1,2],3,...[4]].join(',')",
        "var a=[,1,...[2]];''+a.length+'/'+a.join(',')",
        "var a=[1,2],b=[...a];a.push(3);''+b.length+'/'+a.length",
        // 13.3.8: a call, a method call and a native.
        "function f(a,b,c){return ''+a+b+c}f(...[1,2,3])",
        "function f(){return arguments.length}''+f(...[1,2,3],4)",
        "var o={m:function(a,b){return this.n+a+b},n:10};''+o.m(...[1,2])",
        "''+Math.max(...[1,9,3])",
        "function f(){return Array.prototype.slice.call(arguments).join('|')}f(0,...[1,2],3)",
        // 13.3.5.1: `new`.
        "function C(a,b){this.v=a+b}''+new C(...[3,4]).v",
        "class C{constructor(a,b){this.v=a*b}}''+new C(...[3,4]).v",
        // 7.4.2 walks an iterator of the Script.
        "var it={};it[Symbol.iterator]=function(){var i=0;var o={};o.next=function(){if(i<3){var v={value:i,done:false};i=i+1;return v}return {done:true}};return o};[...it].join(',')",
        // 7.4.2 refuses a value that is not iterable, and a method that
        // throws leaves the spread with its value.
        "var r;try{[...5]}catch(e){r=e instanceof TypeError};''+r",
        "var it={};it[Symbol.iterator]=function(){throw 'bad'};var r;try{[...it]}catch(e){r=e};''+r",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    // 22.1.3.34 is the iterator of a String, which this Realm has not built.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert!(matches!(
        realm.evaluate("[...'ab']"),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn an_async_function_answers_a_promise_and_waits_in_the_job_queue() -> Result<(), Error> {
    // 27.7.5.2 answers a capability before the body runs, and 27.7.5.3 leaves
    // the frame at every wait and takes it back from a job of 9.5. The log is
    // read by a second Script of the same Realm, after the queue is drained.
    for source in [
        // The body runs to its first wait before the caller goes on.
        "var l=[];async function f(x){l.push('a'+x);return x+1}f(1).then(function(v){l.push('r'+v)});l.push('s');0",
        // Every wait is one turn of the queue.
        "var l=[];async function g(){l.push('g0');await 1;l.push('g1');await 2;l.push('g2');return 'd'}g().then(function(v){l.push(v)});0",
        // A handler of the body takes what a rejected wait throws.
        "var l=[];async function h(){try{await Promise.reject('b');l.push('no')}catch(e){l.push('c'+e)}return 1}h().then(function(v){l.push('h'+v)});0",
        // A body that throws rejects its capability.
        "var l=[];async function t(){throw 'tt'}t().catch(function(e){l.push('t'+e)});0",
        // The shapes a body takes: an expression, an arrow, a method and one
        // of a class, each with a wait in it.
        "var l=[];var a=async function(x,y){return x+await y};a(1,2).then(function(v){l.push('a'+v)});0",
        "var l=[];var a=async (x)=>{var y=await x;return y*2};a(5).then(function(v){l.push('a'+v)});0",
        "var l=[];var o={async m(x){return (await x)+1}};o.m(9).then(function(v){l.push('m'+v)});0",
        "var l=[];class C{constructor(){this.n=100}async k(x){return (await x)+this.n}}new C().k(1).then(function(v){l.push('k'+v)});0",
        // A wait inside a loop, and a capture that outlives one.
        "var l=[];async function p(){var t=0;for(var i=0;i<3;i++){t+=await i}return t}p().then(function(v){l.push('p'+v)});0",
        "var l=[];async function c(){var x=7;var g=function(){return x};await 0;return g()+x}c().then(function(v){l.push('c'+v)});0",
        // One body waiting for another.
        "var l=[];async function inner(){return 3}async function outer(){return await inner()}outer().then(function(v){l.push('o'+v)});0",
        // A wait that outlives a collection of the Nursery.
        "var l=[];async function k(){var o={v:1};await 0;for(var i=0;i<400;i++){var x={p:i}}await 0;return o.v}k().then(function(v){l.push('k'+v)});0",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        engine.evaluate(source)?;
        stack.evaluate(source)?;
        let expected = stack.evaluate("l.join('|')")?;
        assert_eq!(engine.evaluate("l.join('|')")?, expected, "{source}");
    }
    // 27.7.4 gives an async function no `[[Construct]]`.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert_eq!(
        realm.evaluate("async function f(){};''+(f.prototype===undefined)")?,
        Value::string("true")
    );
    Ok(())
}

#[test]
fn concat_spreads_what_23_1_3_1_1_says_it_spreads() -> Result<(), Error> {
    for source in [
        // `@@isConcatSpreadable` decides it where the object has one.
        "var o={length:2,0:'a',1:'b'};o[Symbol.isConcatSpreadable]=true;[0].concat(o).join(',')",
        "var a=[1,2];a[Symbol.isConcatSpreadable]=false;''+[0].concat(a).length",
        "var a=[1];a[Symbol.isConcatSpreadable]=0;''+[0].concat(a).length",
        "var a=[1];a[Symbol.isConcatSpreadable]='x';''+[0].concat(a).length",
        // 7.2.2 decides it otherwise.
        "[1,2].concat([3,4]).join(',')",
        "var o={};''+[1].concat(o).length",
        "[1].concat(2,[3]).join(',')",
        // The receiver takes the same question.
        "var o={length:1,0:'z'};o[Symbol.isConcatSpreadable]=true;Array.prototype.concat.call(o,1).join(',')",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_class_body_reaches_the_class_through_a_binding_of_its_own() -> Result<(), Error> {
    // 15.7.14 steps 3, 4 and 17 give the class body a binding for the class
    // name that every method reaches and nothing can write.
    for source in [
        "class C{m(){return C}}var cls=C;C=null;''+(cls.prototype.m()===cls)+'/'+C",
        "class C{static s(){return C}}var cls=C;C=null;''+(cls.s()===cls)",
        "var K=class C{m(){return C}};''+(new K().m()===K)",
        // The binding of the body shadows one of the same name outside it.
        "var C=1;var K=class C{m(){return C}};''+(new K().m()===K)+'/'+C",
        // A class without a name creates no binding.
        "class D{};typeof D",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    // 15.7.14 step 3 makes the binding immutable, which the lowering has no
    // instruction to refuse at run time.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert!(matches!(
        realm.evaluate("class C{m(){C=1}}"),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn a_clause_of_a_case_block_is_an_entry_point_of_its_own() -> Result<(), Error> {
    // 14.12.4 dispatches into each clause and falls through to the next, so
    // both reach it with the bindings the statement began with. The lowering
    // asked for those bindings unchanged after every statement of a clause
    // and refused a clause that narrowed a tracked type.
    for source in [
        "var p=0,r=0;switch(1){case 1:r=p;r+=1;break;default:r=2}r",
        "var r;switch(1){case 1:r='a';break;case 2:r='b';break;default:r='c'}r",
        "var r=0,x;switch(2){case 1:x=1;r+=1;case 2:x='s';r+=2;default:r+=4}''+r+x",
        "var r=0;switch(9){case 1:r=1;break}r",
        "var r='';for(var i=0;i<3;i++){switch(i){case 0:r+='a';continue;case 1:r+='b';break;default:r+='c'}r+='.'}r",
        // A clause that ends abruptly falls through to nothing, so what it
        // left behind takes no part in the comparison.
        "var s='ab',p=0;(function(){var e=p;while(e<s.length){var c=s[e];switch(c){case '_':e+=1;break;default:return 'd'+e}}return 'e'+e})()",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_jump_leaves_the_lexical_declarations_of_the_blocks_it_ends() -> Result<(), Error> {
    // 14.9 and 14.10 leave every block between the jump and its target, so a
    // lexical declaration of one of those blocks is a name the target does not
    // have. The lowering compared the two sets for equal size and refused
    // every `break` and `continue` of a body that declared one.
    for source in [
        "var i=0,r=0;while(i<3){const c=i;i+=1;if(c===0)continue;r+=c}r",
        "var i=0,r=0;while(i<3){const c=i;i+=1;if(c===1)break;r+=c}r",
        "var r=0;for(var i=0;i<4;i++){let c=i;if(c%2)continue;r+=c}r",
        "var r=0;for(var v of [1,2,3]){const c=v;if(c===2)continue;r+=c}r",
        "var r=0;for(var k in {a:1,b:2}){const c=k;if(c==='a')continue;r+=1}r",
        "var r=0,i=0;do{const c=i;i+=1;if(c===0)continue;r+=c}while(i<3);r",
        "var r=0;switch(1){case 1:{const c=5;r=c;break}}r",
        // A nested block between the jump and the loop leaves too.
        "var i=0,r=0;while(i<3){const a=i;i+=1;{const b=a;if(b===0)continue;r+=b}}r",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_loop_of_a_realm_script_takes_a_var_of_its_body() -> Result<(), Error> {
    // 16.1.7 puts a `var` of a Realm Script on the Global Environment Record.
    // The pass that widens the type of every name a loop body writes looked
    // each one up among the register bindings, so a loop whose body declared
    // one with an initializer was refused.
    for backend in [Backend::Engine, Backend::Stack] {
        let mut host = SilentHost;
        let mut realm = Realm::with_backend(Limits::default(), &mut host, backend)?;
        realm.evaluate("for (var i = 0; i < 2; i++) { var a = i; }")?;
        realm.evaluate("while (false) { var b = 1; }")?;
        realm.evaluate("for (var k in {x:1}) { var c = k; }")?;
        realm.evaluate("for (var v of [3]) { var d = v; }")?;
        realm.evaluate("do { var e = 5; } while (false);")?;
        assert_eq!(
            realm.evaluate("[a,b,c,d,e].join('|')")?,
            Value::string("1||x|3|5")
        );
    }
    Ok(())
}

#[test]
fn a_computed_key_of_the_script_is_converted_where_it_is_read() -> Result<(), Error> {
    for source in [
        // 7.1.19 step 2 sends an Object key through 7.1.1 with the hint
        // `string`, which for a `toString` of the Script is a call.
        "var k={toString(){return 'p'}},t={p:1};''+t[k]",
        "var k={toString(){return 'p'}},t={};t[k]=5;''+t.p",
        "var k={toString(){return 'p'}},t={p:1};t[k]+=1;''+t.p",
        "var k={toString(){return 'p'}},t={p:1};t[k]++;''+t.p",
        "var k={toString(){return 'p'}},t={p:1};''+(delete t[k])+'/'+('p' in t)",
        "var k={[Symbol.toPrimitive](){return 'p'}},t={p:4};''+t[k]",
        // A method that throws leaves the access with its value.
        "var k={toString(){throw 3}},t={},r;try{t[k]=1}catch(e){r=e};''+r",
        // 7.1.19 keeps a Symbol as the key it is.
        "var s=Symbol('x'),t={};t[s]=1;''+t[s]",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    // 13.3.3 note 2: `a[b] = c` reaches ToPropertyKey after `c`, which the
    // stack backend does before it; the two answer different orders and only
    // the engine answers the order of the specification.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    assert_eq!(
        realm.evaluate(
            "var l=[],k={toString(){l.push('k');return 'p'}},t={};t[k]=(l.push('v'),7);l.join('')"
        )?,
        Value::string("vk")
    );
    Ok(())
}

#[test]
fn an_update_and_a_unary_operator_convert_an_object_operand() -> Result<(), Error> {
    for source in [
        // 13.4.4.1 takes ToNumeric of the old value, which for an Object is
        // 7.1.1 with the hint `number`.
        "var o={valueOf(){return 1}},x=o;x++;''+x",
        "var o={valueOf(){return 1}},x=o;''+(x++)+'/'+x",
        "var o={valueOf(){return 1}},t={p:o};''+(t.p++)+'/'+t.p",
        "var o={valueOf(){return 1}},a=[o];''+(a[0]++)+'/'+a[0]",
        "var o={valueOf(){return '2'}},x=o;x++;''+x+'/'+typeof x",
        // The accessor of the property is read once and written once.
        "var g={get p(){return {valueOf(){return 4}}},set p(v){this.q=v}};g.p++;''+g.q",
        // 13.5.4, 13.5.5 and 13.5.6 convert the same way.
        "var o={valueOf(){return 3}};''+(-o)+'/'+(+o)+'/'+(~o)",
        "var o={[Symbol.toPrimitive](h){return h}};''+(+o)",
        // A method that throws leaves the update with its value.
        "var o={valueOf(){throw 7}},x=o,r;try{x++}catch(e){r=e};''+r",
        // 7.1.4 step 2 refuses a Symbol.
        "var s=Symbol(),r;try{-s}catch(e){r=e instanceof TypeError};''+r",
        "var s=Symbol(),x=s,r;try{x++}catch(e){r=e instanceof TypeError};''+r",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_destructuring_assignment_names_the_function_of_its_default() -> Result<(), Error> {
    // 13.15.5.2 and 13.15.5.4 name an anonymous function of an Initializer
    // after the target, where that target is a plain identifier reference.
    for source in [
        "var a;[a = function(){}] = [];a.name",
        "var a;({a = function(){}} = {});a.name",
        "var a;({x: a = function(){}} = {});a.name",
        "var a;[a = class{}] = [];a.name",
        "var a;[a = ()=>{}] = [];a.name",
        // A named function expression keeps its own name.
        "var a;[a = function q(){}] = [];a.name",
        // A target that is no identifier reference names nothing.
        "var o={};[o.p = function(){}] = [];o.p.name",
        // The iterator path of 13.15.5.5 names it the same way.
        "var a;for ([a = function(){}] of [[]]) {}a.name",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn an_array_answers_its_length_and_its_indices_as_own_properties() -> Result<(), Error> {
    for source in [
        // 10.4.2.1 keeps `length` beside the Shape, so 6.2.6.4 reads it from
        // there and not from a slot no Shape carries.
        "var d=Object.getOwnPropertyDescriptor([1,2],'length');''+d.value+d.writable+d.enumerable+d.configurable",
        "var b=[];Object.defineProperty(b,'length',{});b.length=2;''+Object.getOwnPropertyDescriptor(b,'length').value",
        // 10.1.6.3 moves an index with its own attributes into the Shape, and
        // 10.1.10.1 deletes it from there.
        "var a=[];Object.defineProperty(a,'0',{value:1,configurable:true});''+(delete a[0])+'/'+('0' in a)",
        "var a=[];Object.defineProperty(a,'0',{value:1,configurable:false});''+(delete a[0])+'/'+('0' in a)",
        // An ordinary index still leaves the store alone.
        "var a=[1,2,3];''+(delete a[1])+'/'+a.length+'/'+(1 in a)",
        "var a=Object.seal([1]);''+(delete a[0])+'/'+(0 in a)",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_lexical_declaration_of_a_realm_script_names_its_function() -> Result<(), Error> {
    // 14.3.1.2 step 4: an anonymous function takes the name the declaration
    // binds it to, and 16.1.7 puts that binding on the Global Environment
    // Record rather than in a register.
    for backend in [Backend::Engine, Backend::Stack] {
        let mut host = SilentHost;
        let mut realm = Realm::with_backend(Limits::default(), &mut host, backend)?;
        realm.evaluate("let a = function(){}, b = () => {}, c = class {};")?;
        realm.evaluate("const d = function(){}; let named = function q(){};")?;
        realm.evaluate("var v = function(){};")?;
        assert_eq!(
            realm.evaluate("[a.name,b.name,c.name,d.name,named.name,v.name].join('|')")?,
            Value::string("a|b|c|d|q|v")
        );
    }
    Ok(())
}

#[test]
fn the_symbol_keyed_methods_of_22_2_6_answer_what_the_string_methods_do() -> Result<(), Error> {
    for source in [
        // 22.2.6.8 answers the Array of 22.2.7.2 for a pattern without `g`.
        "JSON.stringify(/a(b)/[Symbol.match]('zab'))",
        "''+/q/[Symbol.match]('ab')",
        // Step 8 answers the matched substrings alone for a global pattern.
        "JSON.stringify(/a/g[Symbol.match]('aXa'))",
        // 22.2.6.12 answers the index, and leaves `lastIndex` as it found it.
        "''+/b/[Symbol.search]('abc')",
        "''+/q/[Symbol.search]('abc')",
        "var g=/a/g;g.lastIndex=1;''+g[Symbol.search]('aa')+'/'+g.lastIndex",
        // 22.2.6.14 cuts the text at every match.
        "/,/[Symbol.split]('a,b,c').join('|')",
        // 10.3.3 names each of the three.
        "''+/a/[Symbol.match].name+'/'+/a/[Symbol.search].name+'/'+/a/[Symbol.split].name",
        "''+/a/[Symbol.match].length+'/'+/a/[Symbol.search].length+'/'+/a/[Symbol.split].length",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    // 22.2.7.1 calls the `exec` of the object, and one of the Script is a call
    // these clauses have no frame to make.
    for source in [
        "var r=/a/;r.exec=function(){return null};r[Symbol.match]('a')",
        "var r=/a/;r.constructor=function(){};r[Symbol.split]('a')",
        "RegExp.prototype[Symbol.matchAll]",
    ] {
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
fn a_clause_of_23_1_3_writes_the_length_the_way_7_3_4_does() -> Result<(), Error> {
    for source in [
        // 10.4.2.1 step 3.g: an index at or above a length 7.3.15 made
        // unwritable needs the length to grow, which 10.4.2.4 refuses.
        "var a=[];Object.defineProperty(a,'length',{writable:false});var r='none';try{a.push(1)}catch(e){r=''+(e instanceof TypeError)};r",
        // 23.1.3.36 step 4.e writes the length with 7.3.4, which throws.
        "var r='none';try{Object.freeze([]).unshift(1)}catch(e){r=''+(e instanceof TypeError)};r",
        "var a=[1];Object.defineProperty(a,'length',{writable:false});var r='none';try{a.unshift(0)}catch(e){r=''+(e instanceof TypeError)};r",
        // The clauses that shorten an Array already wrote it that way.
        "var a=[1,2];Object.defineProperty(a,'length',{writable:false});var r='none';try{a.pop()}catch(e){r=''+(e instanceof TypeError)};r",
        // A push that fits under an unwritable length is no error.
        "var a=[0];a.length=2;Object.defineProperty(a,'length',{writable:false});var r='none';try{a[1]=7}catch(e){r='threw'};r+'/'+a[1]",
        // 23.1.3 gives the callback of a walk three arguments, whatever the
        // callback declares.
        "var o={0:11,length:1},t={},r='';Array.prototype.forEach.call(o,function(){r=''+(this===t)+arguments[0]+arguments[1]+(arguments[2]===o)+arguments.length},t);r",
        "var o={0:5,length:1},r='';Array.prototype.map.call(o,function(){r=''+arguments.length+arguments[0]});r",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_write_under_a_symbol_key_reads_the_property_of_the_chain_first() -> Result<(), Error> {
    // 10.1.9.2 reads the property before it writes, whatever kind its key is.
    for source in [
        // A data property that is not writable takes no value.
        "var s=Symbol('a'),o={};Object.defineProperty(o,s,{value:1,writable:false,configurable:true});o[s]=9;''+o[s]",
        // An own property keeps the attributes it was given.
        "var s=Symbol('a'),o={};Object.defineProperty(o,s,{value:1,writable:true,enumerable:false,configurable:false});o[s]=2;var d=Object.getOwnPropertyDescriptor(o,s);''+d.value+d.writable+d.enumerable+d.configurable",
        // A setter of the Prototype Chain takes the value, and the receiver
        // gains no own property.
        "var s=Symbol('b'),p={},seen;Object.defineProperty(p,s,{set:function(v){seen=v},configurable:true});var q=Object.create(p);q[s]=7;''+seen+'/'+Object.getOwnPropertyDescriptor(q,s)",
        // A data property of the Chain that is not writable refuses the write.
        "var s=Symbol('c'),r={};Object.defineProperty(r,s,{value:5,writable:false});var t=Object.create(r);t[s]=6;''+t[s]",
        // 10.1.6.3 step 2 refuses a new property of an object that is not
        // extensible.
        "var s=Symbol('d'),f=Object.freeze({});f[s]=1;''+(f[s]===undefined)",
        // Strict evaluation raises what sloppy evaluation drops.
        "var s=Symbol('e'),o={};Object.defineProperty(o,s,{value:1,writable:false});var g=function(){'use strict';o[s]=4};var r='none';try{g()}catch(e){r=''+(e instanceof TypeError)};r",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_combinators_of_27_2_4_settle_one_promise_for_many() -> Result<(), Error> {
    for source in [
        // 27.2.4.1 answers the values in the order of the iterable, whatever
        // order the elements settle in.
        "var l=[];Promise.all([1,Promise.resolve(2),3]).then(function(v){l.push(v.join(','))});0",
        // Step 8 of 27.2.4.1.3 resolves an empty iterable at once.
        "var l=[];Promise.all([]).then(function(v){l.push('e'+v.length)});0",
        // Step 6.q rejects the capability with the first element that rejects.
        "var l=[];Promise.all([1,Promise.reject('e'),3]).catch(function(e){l.push('r'+e)});0",
        // 27.2.4.5 answers the first element that settles.
        "var l=[];Promise.race([Promise.resolve('w'),Promise.reject('x')]).then(function(v){l.push('w'+v)},function(e){l.push('x'+e)});0",
        // 27.2.4.2 answers one record per element.
        "var l=[];Promise.allSettled([Promise.resolve(1),Promise.reject(2)]).then(function(v){l.push(v[0].status+v[0].value+'/'+v[1].status+v[1].reason)});0",
        // 27.2.4.9 answers the promise and the pair that settles it.
        "var l=[];var w=Promise.withResolvers();w.resolve(9);w.promise.then(function(v){l.push('w'+v)});0",
        // 7.4.2 throws for an argument that is not iterable, and the throw is
        // the rejection of step 7.
        "var l=[];Promise.all(3).catch(function(e){l.push('t'+(e instanceof TypeError))});0",
        // A hole reads through the Prototype Chain like any other index.
        "var l=[];Promise.all([1,,3]).then(function(v){l.push(v.length+':'+v[1])});0",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        engine.evaluate(source)?;
        stack.evaluate(source)?;
        let expected = stack.evaluate("l.join('|')")?;
        assert_eq!(engine.evaluate("l.join('|')")?, expected, "{source}");
    }
    Ok(())
}

#[test]
fn the_engine_names_the_parts_of_clause_27_it_has_not_built() -> Result<(), Error> {
    // 27.2.4 gives `%Promise%` five combinators and 27.2.5.3 gives the
    // prototype `finally`; a read of one of them is a gap and not undefined.
    // A gap is fatal, so each one is read in a Realm of its own.
    for source in [
        "Promise.any",
        "Promise.try",
        "Promise.prototype.finally",
        // 7.4.2 and 7.4.4 are methods of the Script, which a combinator has no
        // frame to call: only an Array of this Realm answers its elements
        // without either call.
        "Promise.all({[Symbol.iterator](){return {next(){return {done:true}}}}})",
        "Promise.all('ab')",
        // A Promise of a subclass needs the `newTarget` of 10.1.13, which this
        // engine does not carry into a constructor written in Rust.
        "class C extends Promise{}; new C(function(){})",
    ] {
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
fn the_engine_answers_the_shape_of_the_resolving_functions() -> Result<(), Error> {
    for backend in [Backend::Engine, Backend::Stack] {
        let mut host = SilentHost;
        let mut realm = Realm::with_backend(Limits::default(), &mut host, backend)?;
        // 27.2.5.5 tags the prototype, so 20.1.3.6 answers through it.
        assert_eq!(
            realm.evaluate("Object.prototype.toString.call(Promise.resolve(1))")?,
            Value::string("[object Promise]")
        );
        assert_eq!(
            realm.evaluate("Promise.prototype[Symbol.toStringTag]")?,
            Value::string("Promise")
        );
        // 27.2.4.8 answers the constructor itself.
        assert_eq!(
            realm.evaluate("Promise[Symbol.species]===Promise")?,
            Value::Boolean(true)
        );
    }
    // 27.2.1.3 gives each of the pair the `length` and `name` of 10.3.3. The
    // stack backend gives them neither, so only the engine answers here.
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    realm.evaluate("var a,b;new Promise(function(r,j){a=r;b=j});0")?;
    assert_eq!(realm.evaluate("a.length")?, Value::Number(1.0));
    assert_eq!(realm.evaluate("b.length")?, Value::Number(1.0));
    assert_eq!(realm.evaluate("a.name")?, Value::string(""));
    assert_eq!(realm.evaluate("typeof a")?, Value::string("function"));
    Ok(())
}

#[test]
fn a_read_of_an_index_reaches_the_elements_store_of_a_prototype() -> Result<(), Error> {
    for source in [
        // 10.1.8.1 reads the chain in order, and 10.4.2.1 puts an index of a
        // Prototype in that Prototype's Elements store.
        "Array.prototype[2]=-1;var y=[0,1];''+y[2]+'/'+y['2']+'/'+y[0]",
        // 10.4.2.4 deletes the index the shorter length drops, so the read
        // goes on to the Prototype.
        "Array.prototype[2]=-1;var x=[0,1,2];x.length=2;''+x[2]",
        // A named property of the Shape shadows an index of a deeper store.
        "var o=Object.create([10,20]);Object.defineProperty(o,'1',{value:'named'});''+o[1]",
        // An own index of the receiver answers before either.
        "var o=Object.create(['p','q']);o[0]='own';''+o[0]+'/'+o[1]",
        // A constructor's Prototype is an ordinary object holding an Array.
        "function A(){}A.prototype=[9,8,7];var a=new A();''+a[1]+'/'+a[3]",
    ] {
        let mut engine_host = SilentHost;
        let mut engine = Realm::with_backend(Limits::default(), &mut engine_host, Backend::Engine)?;
        let mut stack_host = SilentHost;
        let mut stack = Realm::with_backend(Limits::default(), &mut stack_host, Backend::Stack)?;
        assert_eq!(
            engine.evaluate(source)?,
            stack.evaluate(source)?,
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn the_parameter_map_of_10_4_4_7_reaches_both_ways() -> Result<(), Error> {
    for source in [
        // 10.4.4.7 maps each index onto the parameter the argument arrived in.
        "function f(a){a=2;return arguments[0]}f(1)",
        "function f(a){arguments[0]=3;return a}f(1)",
        "function f(a,b){b='B';return ''+arguments[0]+arguments[1]+arguments.length}f(1,2)",
        // 10.4.4.7 maps no index the call passed no argument for.
        "function f(a,b){b='B';return ''+arguments[1]+'/'+arguments.length}f(1)",
        // 10.4.4.5 drops the entry the delete removes, and the parameter keeps
        // what it holds.
        "function f(a){delete arguments[0];a=9;return ''+arguments[0]+'/'+a}f(1)",
        // 10.4.4.2 step 5: a descriptor that takes the writability or makes
        // the property an accessor drops the entry.
        "function f(a){Object.defineProperty(arguments,'0',{value:7});return ''+a}f(1)",
        "function f(a){Object.defineProperty(arguments,'0',{get:function(){return 5}});a=8;return ''+arguments[0]+'/'+a}f(1)",
        "function f(a){Object.freeze(arguments);a=4;return ''+arguments[0]+'/'+a}f(1)",
        // 6.2.6.4 reads the value through the map as well.
        "function f(a){a=6;return ''+Object.getOwnPropertyDescriptor(arguments,'0').value}f(1)",
        // The object outlives the frame, so the map holds the context and not
        // the registers of a call that has returned.
        "var g;function f(a){g=arguments;a=5;return 0}f(1);g[0]",
        // A nested ordinary function binds an object of its own.
        "function f(a){return (function(b){return b+arguments.length})(2)}f(1)",
        // 10.4.4.6 maps nothing for a strict function.
        "function f(a){'use strict';a=2;return arguments[0]}f(1)",
    ] {
        differential_scripts(&[source])?;
    }
    // 10.4.4.6 builds the object of a parameter list that is not simple, which
    // maps nothing; a body that writes such a parameter is refused instead.
    for source in [
        "function f(a=1){a=2;return arguments[0]}f(1)",
        "function f(...a){a=2;return arguments[0]}f(1)",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_thrown_object_the_embedding_cannot_hold_names_itself() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    // 20.5.3.4 reads the two names of the object, which an error of the Script
    // carries as data properties of its Prototype and of itself.
    let source = "function E(m){this.message=m}E.prototype.name='E';throw new E('two')";
    let Err(Error::ThrownUnrepresentable { description }) = realm.evaluate(source) else {
        panic!("{source}");
    };
    assert_eq!(description, "E: two");
    // An object that holds neither name answers the empty text.
    let Err(Error::ThrownUnrepresentable { description }) = realm.evaluate("throw {}") else {
        panic!("throw {{}}");
    };
    assert_eq!(description, "");
    Ok(())
}

#[test]
fn a_symbol_where_7_1_17_wants_a_string_is_a_type_error() -> Result<(), Error> {
    // 7.1.17 step 2 gives a Symbol no text, which is a `TypeError` of the
    // Realm and not a step the engine is missing.
    for source in [
        "var r;try{'ab'.indexOf(Symbol())}catch(e){r=e instanceof TypeError}r",
        "var r;try{'ab'+Symbol()}catch(e){r=e instanceof TypeError}r",
        "var r;try{['a'].join(Symbol())}catch(e){r=e instanceof TypeError}r",
        "var r;try{'ab'.split(Symbol())}catch(e){r=e instanceof TypeError}r",
        "var r;try{'ab'.startsWith(Symbol())}catch(e){r=e instanceof TypeError}r",
        // 7.1.19 keeps a Symbol as the key it is, so a property read of one
        // reaches no conversion at all.
        "var o={};o[Symbol.iterator]=1;typeof o[Symbol.iterator]",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_loop_body_that_edits_an_object_lets_it_leave() -> Result<(), Error> {
    for source in [
        // The body writes an element the lowering tracked, so the layout it
        // carried at the head is gone and the Array is read at run time.
        "let a=[1];for(const x of a){a[1]='s'}a.join(',')",
        "var j=[];var i=0;while(i<5){j.push(i);i++}j.join('-')",
        "var j=[];for(var i=0;i<4;i++){j.push({p:i})}j.length",
        "var o={};var i=0;while(i<3){o['k'+i]=i;i++}''+o.k0+o.k1+o.k2",
        // A body that leaves the layout alone keeps it.
        "var j=[1,2,3];var s=0;for(var i=0;i<j.length;i++){s+=j[i]}s",
        // The head's own binding still holds what each step gives it.
        "var s='';for(var k in {a:1,b:2}){s+=k}s",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_spread_element_inside_a_function_reaches_the_lowering() -> Result<(), Error> {
    // The scans that collect the names a body reads and writes stopped at a
    // spread element, so every function that held one was refused before the
    // lowering of 13.2.4.2 was reached.
    for source in [
        "function f(){return [...[1,2],3].join(',')}f()",
        "function f(){var a=[1,2];return [...a].length}f()",
        "function f(){return Math.max(...[1,9,3])}f()",
        "function f(a,b){return ''+a+b}function g(){return f(...[1,2])}g()",
        "function C(a){this.v=a}function g(){return new C(...[7]).v}g()",
        "function g(){var x=1;var h=function(){return x};[...[2]];return h()}g()",
        // The operand's own writes are seen through the spread.
        "function g(){var x=1;var a=[...[x=5]];return ''+x+a[0]}g()",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_destructuring_assignment_takes_a_primitive_value() -> Result<(), Error> {
    for source in [
        // 13.15.5.5 reaches an array pattern through 7.4.2, which throws for a
        // value that carries no `@@iterator`.
        "var r;try{[]=true}catch(e){r=e instanceof TypeError};''+r",
        "var r;try{[,]=null}catch(e){r=e instanceof TypeError};''+r",
        "var a;var r;try{[a]=7}catch(e){r=e instanceof TypeError};''+r",
        // 13.15.5.5 step 1 refuses undefined and null before it reads a
        // property, and takes every other primitive through 7.3.5.
        "var r;try{({}=null)}catch(e){r=e instanceof TypeError};''+r",
        "var r;try{({}=undefined)}catch(e){r=e instanceof TypeError};''+r",
        "var o={};({a:o.x}=true);''+o.x",
        "var n;({length:n}='abc');''+n",
        "var r={};({}=true);''+typeof r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_return_runs_the_finally_block_before_it_leaves() -> Result<(), Error> {
    for source in [
        // 14.15.3 runs the Finally Block on the path the `return` takes.
        "function f(){try{return 'a'}finally{}}f()",
        "var l='';function f(){try{l+='t';return 'r'}finally{l+='f'}}f()+l",
        // A `return` of the Finally Block replaces the one of the try Block.
        "function f(){try{return 1}finally{return 2}}f()",
        // A Catch Block's `return` takes the same path.
        "function f(){try{throw 'x'}catch(e){return 'c'}finally{}}f()",
        // Each Block of a nest runs, innermost first.
        "var o='';function f(){try{try{return 'a'}finally{o+='1'}}finally{o+='2'}}f()+o",
        // A throw still leaves the statement after the Block has run.
        "var o='';function f(){try{throw 'e'}finally{o+='f'}}var r;try{f()}catch(e){r=e}r+o",
        // A `return` inside a loop inside the try Block leaves the function.
        "function f(){for(var i=0;i<3;i++){try{if(i===1)return 'i'+i}finally{}}return 'n'}f()",
        // A Block that carries on keeps the completion of the statement.
        "function f(){try{}finally{}return 'z'}f()",
    ] {
        differential_scripts(&[source])?;
    }
    // 14.15.3 would have to run the Block before a `break` or a `continue`
    // leaves the statement, which the lowering does not do.
    for source in [
        "function f(){while(1){try{break}finally{}}}f()",
        "function f(){var i=0;while(i<2){try{i++}finally{continue}}return i}f()",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn a_clause_of_23_1_3_moves_the_elements_of_an_array_like() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    // 23.1.3 is generic over an array-like: 7.3.4 writes an index of one that
    // is no Array as an ordinary property, and 7.3.9 deletes it. The stack
    // backend carries none of these clauses, so only the engine answers here.
    for (source, answer) in [
        (
            "var o={0:'a',1:'b',length:2};''+Array.prototype.shift.call(o)+o.length+o[0]",
            "a1b",
        ),
        (
            "var o={0:'a',1:'b',length:2};Array.prototype.reverse.call(o);''+o[0]+o[1]",
            "ba",
        ),
        (
            "var o={0:'a',length:1};Array.prototype.unshift.call(o,'z');''+o[0]+o[1]+o.length",
            "za2",
        ),
        (
            "var o={0:'a',1:'b',length:2};''+Array.prototype.pop.call(o)+o.length+(1 in o)",
            "b1false",
        ),
        (
            "var o={0:'c',1:'a',2:'b',length:3};Array.prototype.sort.call(o);''+o[0]+o[1]+o[2]",
            "abc",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(answer), "{source}");
    }
    // An Array still keeps its indices in the Elements store.
    for source in [
        "var a=[1,,3];a.reverse();''+(0 in a)+(1 in a)+(2 in a)+a.join(',')",
        "var a=[1,2,3];a.splice(1,1);a.join(',')",
        "var a=Object.freeze([1,2]);var r;try{a.reverse()}catch(e){r=e instanceof TypeError};''+r",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn a_body_that_throws_after_a_wait_rejects_its_capability() -> Result<(), Error> {
    // 27.7.5.2 step 4 rejects the capability of a body that threw, and the
    // drain of 9.5 goes on with the reaction that takes the rejection. The
    // body stands on no caller once a job took it back, which ended the run
    // with the thrown value rather than with the queue.
    for source in [
        "var l=[];async function f(){await Promise.reject('o')}f().then(function(v){l.push('res'+v)},function(v){l.push('rej'+v)});0",
        "var l=[];async function f(){await 1;throw 't'}f().then(function(v){l.push('res'+v)},function(v){l.push('rej'+v)});0",
        "var l=[];async function f(){try{await Promise.reject('b')}catch(e){return 'c'+e}}f().then(function(v){l.push('res'+v)},function(v){l.push('rej'+v)});0",
        "var l=[];async function f(){try{0}finally{await Promise.reject('o')}}f().then(function(v){l.push('res'+v)},function(v){l.push('rej'+v)});0",
        "var l=[];async function f(){try{return 'e'}finally{await Promise.reject('o')}}f().then(function(v){l.push('res'+v)},function(v){l.push('rej'+v)});0",
    ] {
        differential_scripts(&[source, "l.join('|')"])?;
    }
    Ok(())
}

#[test]
fn the_throw_type_error_of_10_2_4_1_is_frozen() -> Result<(), Error> {
    // 10.2.4.1: the function is not extensible and its `length` and `name` are
    // not configurable, so 7.3.15 answers it frozen.
    for source in [
        "var T=Object.getOwnPropertyDescriptor(function(){'use strict';return arguments}(),'callee').get;''+Object.isFrozen(T)",
        "var T=Object.getOwnPropertyDescriptor(function(){'use strict';return arguments}(),'callee').get;''+Object.isExtensible(T)",
        "var T=Object.getOwnPropertyDescriptor(function(){'use strict';return arguments}(),'callee').get;var d=Object.getOwnPropertyDescriptor(T,'length');''+d.writable+d.enumerable+d.configurable",
        "var T=Object.getOwnPropertyDescriptor(function(){'use strict';return arguments}(),'callee').get;var d=Object.getOwnPropertyDescriptor(T,'name');''+d.value+d.configurable",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn an_initializer_of_8_6_2_reads_an_earlier_parameter() -> Result<(), Error> {
    // 10.2.11 makes a parameter a captured name where a later Initializer
    // reads it, so its binding lives in the context of the call rather than
    // in a register and the Initializer reaches it there.
    for source in [
        "function f(a=1,b=a+1){return ''+a+b}f()",
        "function f(a=1,b=a){return ''+a+b}f()",
        "function f(a=1,b=a+1){return ''+a+b}f(5)",
        "function f(a=1,b=a+1){return ''+a+b}f(5,7)",
        "function f([x,y]=[1,2],z=x+y){return ''+x+y+z}f()",
        // A parameter no Initializer reads keeps its register.
        "function f(a=1,b=2){return ''+a+b}f()",
        "function f(a,b=a+1){return ''+a+b}f(5)",
        // A nested function reads the parameter when it is called, not while
        // the Initializer runs.
        "function f(a=function(){return a}){return typeof a()}f()",
    ] {
        differential_scripts(&[source])?;
    }
    // 10.2.11 leaves the parameter an Initializer binds, and every one the
    // list binds after it, in a temporal dead zone. The registers of the frame
    // hold undefined there and say nothing of the two apart.
    for source in [
        "function f(x=x){return 1}f()",
        "function f(x=y,y){return 1}f()",
    ] {
        assert!(
            !compile(source, Limits::default())?.uses_register_backend(),
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn an_arrow_takes_the_this_of_the_function_it_was_made_in() -> Result<(), Error> {
    for source in [
        // 10.2.1.1 gives an arrow no Function Environment Record, so 9.4.2
        // answers its `this` out of the one the enclosing function has.
        "var o={v:7,m:function(){var f=()=>this.v;return f()}};o.m()",
        "var o={v:7,m:function(){return (()=>this.v)()}};o.m()",
        "var o={v:7,m:function(){var f=()=>this;return f().v}};o.m()",
        "function F(){this.v=3;this.g=()=>this.v}var x=new F();x.g()",
        // 10.2.1.1: the arrow's `this` is not the receiver of its own call.
        "var o={v:1,m:function(){var f=()=>this.v;return f.call({v:9})}};o.m()",
        // The arrow outlives the call it was made in and keeps that `this`.
        "var o={v:5,m:function(){return ()=>this.v}};var f=o.m();f()",
        // An arrow inside an arrow reaches the same binding.
        "var o={v:4,m:function(){return (()=>(()=>this.v)())()}};o.m()",
    ] {
        differential_scripts(&[source])?;
    }
    Ok(())
}

#[test]
fn the_map_of_24_1_and_the_set_of_24_2_hold_their_entries() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    // The stack backend carries neither clause, so only the engine answers.
    for (source, answer) in [
        ("Object.prototype.toString.call(new Map())", "[object Map]"),
        ("Object.prototype.toString.call(new Set())", "[object Set]"),
        ("''+new Map().size+new Set().size", "00"),
        // 24.1.3.9 replaces the value of the entry the key has, and 24.1.3.6
        // answers undefined for a key no entry has.
        (
            "var m=new Map();m.set('a',1);m.set('b',2);m.set('a',3);''+m.size+m.get('a')+m.get('b')+m.get('c')",
            "232undefined",
        ),
        // 24.1.3.9 answers the Map itself, so the calls chain.
        ("var m=new Map();''+(m.set(1,2)===m)", "true"),
        // 24.1.3.3 answers whether it removed an entry.
        (
            "var m=new Map();m.set('a',1);''+m.delete('a')+m.delete('a')+m.size+m.has('a')",
            "truefalse0false",
        ),
        (
            "var m=new Map();m.set('a',1);m.clear();''+m.size+m.has('a')",
            "0false",
        ),
        // 7.2.11 makes NaN the same value as NaN and -0 the same as +0.
        (
            "var s=new Set();s.add(1);s.add(1);s.add(NaN);s.add(NaN);''+s.size+s.has(1)+s.has(NaN)",
            "2truetrue",
        ),
        (
            "var s=new Set();s.add(-0);''+s.has(0)+s.has(-0)",
            "truetrue",
        ),
        (
            "var s=new Set();''+(s.add(1)===s)+s.delete(1)+s.size",
            "truetrue0",
        ),
        // 24.1.3 and 24.2.3 refuse a receiver that carries the other slot.
        (
            "var r;try{Map.prototype.get.call(new Set(),1)}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        (
            "var r;try{Set.prototype.add.call(new Map(),1)}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        // 24.1.1.1 step 1 refuses a call without `new`.
        (
            "var r;try{Map()}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        (
            "var r;try{Set()}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        // 24.1.3.10 is an accessor of the Prototype, not a property of the Map.
        (
            "var d=Object.getOwnPropertyDescriptor(Map.prototype,'size');''+(typeof d.get)+d.set+d.enumerable+d.configurable",
            "functionundefinedfalsetrue",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(answer), "{source}");
    }
    // 24.1.1.1 step 8 and 24.2.1.1 step 8 add every value of the iterable.
    for (source, answer) in [
        ("var s=new Set([1,2,2,3]);''+s.size+[...s].join('')", "3123"),
        (
            "var m=new Map([['a',1],['b',2]]);''+m.size+m.get('a')+m.get('b')",
            "212",
        ),
        (
            "var m=new Map([['a',1],['a',9]]);''+m.size+m.get('a')",
            "19",
        ),
        ("''+new Set([]).size+new Map([]).size", "00"),
        // Step 8.d.i throws for an entry of a Map that is no Object.
        (
            "var r;try{new Map([1])}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(answer), "{source}");
    }
    // Step 7 reads the `set` off the object it just made, so a Script that
    // replaced it decides what an entry is, which the clause has no frame for.
    assert!(matches!(
        realm.evaluate("var C=function(){};C.prototype=Map.prototype;Map.prototype.set=function(){};new Map([[1,2]])"),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn the_iterators_of_24_1_5_and_24_2_5_walk_the_entries() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    // The stack backend carries neither clause, so only the engine answers.
    for (source, answer) in [
        // 24.1.5.2.1 skips the `empty` a delete left behind and reaches an
        // entry added after the iterator was made.
        (
            "var m=new Map();m.set('a',1);m.set('b',2);m.delete('a');m.set('c',3);var s='';for(var e of m){s+=e[0]+e[1]+'|'}s",
            "b2|c3|",
        ),
        (
            "var m=new Map();m.set('a',1);m.set('b',2);var s='';for(var k of m.keys()){s+=k}s",
            "ab",
        ),
        (
            "var m=new Map();m.set('a',1);m.set('b',2);var s='';for(var v of m.values()){s+=v}s",
            "12",
        ),
        (
            "var s=new Set();s.add(1);s.add(2);s.delete(1);s.add(3);var t='';for(var x of s){t+=x}t",
            "23",
        ),
        // 24.2.5.1 answers the value as the key as well.
        (
            "var s=new Set();s.add(7);var t='';for(var e of s.entries()){t+=e[0]+'='+e[1]}t",
            "7=7",
        ),
        // 24.1.5.2.2 and 24.2.5.2.2 tag the iterators.
        (
            "''+Object.prototype.toString.call(new Map().keys())+Object.prototype.toString.call(new Set().values())",
            "[object Map Iterator][object Set Iterator]",
        ),
        // 24.2.3.10 and 24.2.3.17 answer the same function object as 24.2.3.16,
        // and 24.1.3.14 the same as 24.1.3.4.
        (
            "''+(Set.prototype.keys===Set.prototype.values)+(Set.prototype[Symbol.iterator]===Set.prototype.values)+(Map.prototype[Symbol.iterator]===Map.prototype.entries)",
            "truetruetrue",
        ),
        // 7.4.14 answers an ordinary object with a `value` and a `done`.
        (
            "var m=new Map();m.set('a',1);var i=m.keys();''+i.next().value+i.next().done+i.next().done",
            "atruetrue",
        ),
        // 13.2.4.2 takes every value of the iterator 24.2.3.17 answers.
        ("var s=new Set();s.add(1);s.add(2);[...s].join('-')", "1-2"),
        // 27.1.2.1 stands on %IteratorPrototype%, which every iterator of the
        // specification inherits.
        (
            "var p=Object.getPrototypeOf(Object.getPrototypeOf(new Map().keys()));''+(typeof p[Symbol.iterator])+(p===Object.getPrototypeOf(Object.getPrototypeOf([].keys())))",
            "functiontrue",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(answer), "{source}");
    }
    Ok(())
}

#[test]
fn the_set_operations_of_24_2_3_answer_over_a_set_like() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    // The stack backend carries none of these clauses, so only the engine
    // answers here.
    for (source, answer) in [
        (
            "var a=new Set([1,2,3]),b=new Set([2,3,4]);[...a.union(b)].join(',')",
            "1,2,3,4",
        ),
        (
            "var a=new Set([1,2,3]),b=new Set([2,3,4]);[...a.intersection(b)].join(',')",
            "2,3",
        ),
        (
            "var a=new Set([1,2,3]),b=new Set([2,3,4]);[...a.difference(b)].join(',')",
            "1",
        ),
        (
            "var a=new Set([1,2,3]),b=new Set([2,3,4]);[...a.symmetricDifference(b)].join(',')",
            "1,4",
        ),
        (
            "var a=new Set([1,2,3]);''+a.isSubsetOf(new Set([2,3,4]))+new Set([2]).isSubsetOf(a)",
            "falsetrue",
        ),
        (
            "var a=new Set([1,2,3]);''+a.isSupersetOf(new Set([2,3,4]))+a.isSupersetOf(new Set([1,2]))",
            "falsetrue",
        ),
        (
            "var a=new Set([1,2,3]);''+a.isDisjointFrom(new Set([2]))+a.isDisjointFrom(new Set([9]))",
            "falsetrue",
        ),
        // 24.2.1.2 step 1 refuses an argument that is no Object, step 3 one
        // with no `size`, and step 6 a negative one.
        (
            "var r;try{new Set().union(1)}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        (
            "var r;try{new Set().union({})}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        (
            "var r;try{new Set().union({size:-1,has:function(){},keys:function(){}})}catch(e){r=e instanceof RangeError};''+r",
            "true",
        ),
        (
            "var r;try{new Set().union({size:NaN,has:function(){},keys:function(){}})}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(answer), "{source}");
    }
    // A `has` or a `keys` of the Script decides what the set-like holds, which
    // these clauses have no frame to ask.
    assert!(matches!(
        realm.evaluate("new Set().union({size:1,has:function(){return true},keys:function(){}})"),
        Err(Error::Unsupported { .. })
    ));
    Ok(())
}

#[test]
fn the_for_each_of_24_1_3_5_and_24_2_3_7_calls_back_per_entry() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    // The stack backend carries neither clause, so only the engine answers.
    for (source, answer) in [
        // Step 4.b.i passes the value, the key and the collection.
        (
            "var m=new Map([['a',1],['b',2]]);var s='';m.forEach(function(v,k,c){s+=k+v+(c===m)+'|'});s",
            "a1true|b2true|",
        ),
        // 24.2.3.7 passes the value as the key as well.
        (
            "var t=new Set([1,2]);var s='';t.forEach(function(v,k,c){s+=v+'='+k+(c===t)});s",
            "1=1true2=2true",
        ),
        ("var n=0;new Map().forEach(function(){n=n+1});''+n", "0"),
        // Step 3 refuses a callback that is not callable.
        (
            "var r;try{new Map().forEach(1)}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        // The second argument is the `this` value of the callback.
        (
            "var got;new Map([['a',1]]).forEach(function(){got=this.z},{z:9});''+got",
            "9",
        ),
        // Step 4.a reads the entries again at every step: an entry deleted
        // during the walk is passed over and one added is still reached.
        (
            "var m=new Map([['a',1],['b',2]]);var s='';m.forEach(function(v,k){s+=k;if(k==='a'){m.delete('b')}});s",
            "a",
        ),
        (
            "var m=new Map([['a',1]]);var s='';m.forEach(function(v,k){s+=k;if(k==='a'){m.set('c',3)}});s",
            "ac",
        ),
        // A throw of the callback leaves the clause.
        (
            "var r;try{new Map([['a',1]]).forEach(function(){throw 'x'})}catch(e){r=e};r",
            "x",
        ),
        // The walk survives the collections a long one makes.
        (
            "var t=new Set();var i=0;while(i<500){t.add(i);i=i+1}var n=0;t.forEach(function(v){n=n+v});''+n",
            "124750",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(answer), "{source}");
    }
    Ok(())
}

#[test]
fn the_weak_collections_of_24_3_and_24_4_hold_an_entry_per_key() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    // The stack backend carries no WeakSet, so only the engine answers.
    for (source, answer) in [
        ("var m=new WeakMap();var k={};m.set(k,1);''+m.get(k)", "1"),
        ("var m=new WeakMap();''+m.get({})", "undefined"),
        (
            "var s=new WeakSet();var k={};s.add(k);''+s.has(k)+s.has({})",
            "truefalse",
        ),
        (
            "var m=new WeakMap();var k={};m.set(k,1);''+m.delete(k)+m.has(k)+m.delete(k)",
            "truefalsefalse",
        ),
        // Step 5 of both constructors reads the iterable.
        ("var k={};''+new WeakMap([[k,7]]).get(k)", "7"),
        ("var k={};''+new WeakSet([k]).has(k)", "true"),
        // 9.9.4.1: an Object and an unregistered Symbol are keys, and 24.3.3.5
        // step 4 refuses everything else.
        (
            "var y=Symbol();var m=new WeakMap();m.set(y,3);''+m.get(y)",
            "3",
        ),
        (
            "var r;try{new WeakMap().set(Symbol.for('a'),1)}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        (
            "var r;try{new WeakSet().add(1)}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        // 24.3.3.3 step 4 and 24.3.3.4 step 4 answer for such a key instead.
        (
            "var m=new WeakMap();''+m.get(1)+m.has(1)+m.delete(1)",
            "undefinedfalsefalse",
        ),
        // Step 1 of both constructors refuses a call without `new`.
        (
            "var r;try{WeakMap()}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        // Step 3 of each method refuses a receiver that carries no slot.
        (
            "var r;try{WeakMap.prototype.get.call(new WeakSet(),{})}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        (
            "''+Object.prototype.toString.call(new WeakMap())+Object.prototype.toString.call(new WeakSet())",
            "[object WeakMap][object WeakSet]",
        ),
        (
            "''+WeakMap.length+WeakMap.prototype.set.length+WeakSet.prototype.add.length",
            "021",
        ),
        // The entry of a key the Script still holds survives the collections
        // six hundred keys make.
        (
            "var m=new WeakMap();var i=0;var k;while(i<600){k={};m.set(k,i);i=i+1}''+m.get(k)",
            "599",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(answer), "{source}");
    }
    Ok(())
}

#[test]
fn the_get_or_insert_of_24_1_3_7_and_24_3_3_4_adds_only_a_missing_key() -> Result<(), Error> {
    let mut host = SilentHost;
    let mut realm = Realm::with_backend(Limits::default(), &mut host, Backend::Engine)?;
    // Neither clause stands on the stack backend, so only the engine answers.
    for (source, answer) in [
        (
            "var m=new Map();''+m.getOrInsert('a',1)+m.get('a')+m.getOrInsert('a',9)",
            "111",
        ),
        (
            "var w=new WeakMap();var k={};''+w.getOrInsert(k,3)+w.get(k)",
            "33",
        ),
        // 24.1.3.8 step 5 answers without calling the callback.
        (
            "var m=new Map([['a',1]]);var n=0;''+m.getOrInsertComputed('a',function(){n=1;return 5})+n",
            "10",
        ),
        // Step 6 passes the key alone.
        (
            "var m=new Map();''+m.getOrInsertComputed('a',function(k){return k+'!'})+m.get('a')",
            "a!a!",
        ),
        (
            "var w=new WeakMap();var k={};''+w.getOrInsertComputed(k,function(x){return x===k})+w.get(k)",
            "truetrue",
        ),
        // Step 8 reads the entries again, so the value of the callback wins
        // over the entry the callback wrote itself.
        (
            "var m=new Map();''+m.getOrInsertComputed('a',function(){m.set('a',7);return 2})+m.get('a')",
            "22",
        ),
        // Step 3 refuses a callback that is not callable.
        (
            "var r;try{new Map().getOrInsertComputed('a',1)}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
        // A throw of the callback leaves the clause and writes no entry.
        (
            "var m=new Map();var r;try{m.getOrInsertComputed('a',function(){throw 'x'})}catch(e){r=e};''+r+m.has('a')",
            "xfalse",
        ),
        // 24.3.3.5 step 3 refuses a key 9.9.4.1 refuses.
        (
            "var r;try{new WeakMap().getOrInsertComputed(1,function(){})}catch(e){r=e instanceof TypeError};''+r",
            "true",
        ),
    ] {
        assert_eq!(realm.evaluate(source)?, Value::string(answer), "{source}");
    }
    Ok(())
}

#[test]
fn the_length_of_10_4_2_4_deletes_in_descending_order() -> Result<(), Error> {
    differential_scripts(&[
        "var a=[1,2,3];a.length=1;''+a+'|'+a.length",
        "var a=[];a[5]=1;a.length=3;''+a.length+(5 in a)",
        // An index the Shape holds goes only while it is configurable, and the
        // length stops one above the first that does not.
        "var a=[0,1,2];Object.defineProperty(a,'1',{value:9,configurable:false});a.length=0;''+a.length+'|'+a[1]",
        "var a=[0,1,2,3,4];Object.defineProperty(a,'3',{value:9,configurable:false});a.length=1;''+a.length+'|'+(4 in a)+(2 in a)",
        "var a=[0,1,2];Object.defineProperty(a,'2',{value:9,configurable:true});a.length=0;''+a.length+'|'+(2 in a)",
        // 10.1.6.3 answers false for the define that stopped.
        "var a=[0,1,2];Object.defineProperty(a,'1',{value:9,configurable:false});''+Reflect.defineProperty(a,'length',{value:0})+a.length",
        "var a=[0,1,2];''+Reflect.defineProperty(a,'length',{value:0})+a.length",
    ])
}
