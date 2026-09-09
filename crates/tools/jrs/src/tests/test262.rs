// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "jrs-test262-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("harness")).unwrap();
        std::fs::create_dir_all(root.join("test/sub")).unwrap();
        let f = Self(root);
        f.write(
            "harness/assert.js",
            "function assert(x){if(x!==true)throw new Error('assertion')} ",
        );
        f.write(
            "harness/sta.js",
            "function $DONOTEVALUATE(){throw new Error('evaluated')} ",
        );
        f.write("harness/doneprintHandle.js","function $DONE(e){if(e)print('Test262:AsyncTestFailure:'+e);else print('Test262:AsyncTestComplete')} ");
        f
    }
    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
    fn run(&self, args: &[&str]) -> (Result<(), String>, String) {
        let mut out = Vec::new();
        let args = std::iter::once(self.0.display().to_string())
            .chain(args.iter().map(|s| (*s).to_owned()));
        let result = run(args, Limits::default(), &mut out);
        (result, String::from_utf8(out).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn metadata_lists_variants_and_invalid_shapes() {
    assert_eq!(
        metadata::parse("/*---\rflags: [module, raw]\r---*/")
            .unwrap()
            .variants(),
        vec![Variant::Module]
    );
    for (flags, expected) in [
        ("", vec![Variant::Sloppy, Variant::Strict]),
        ("flags: [onlyStrict]", vec![Variant::Strict]),
        ("flags:\n  - noStrict", vec![Variant::Sloppy]),
        ("flags: [module, async]", vec![Variant::Module]),
        ("flags: [raw]", vec![Variant::Raw]),
    ] {
        let m = metadata::parse(&format!(
            "/*---\ndescription: |\n  flags: not metadata\n{flags}\n---*/"
        ))
        .unwrap();
        assert_eq!(m.variants(), expected);
    }
    let m=metadata::parse("/*---\nincludes: ['a.js', \"b.js\"]\nnegative:\n  phase: runtime\n  type: TypeError\n---*/").unwrap();
    assert_eq!(m.includes, ["a.js", "b.js"]);
    assert_eq!(m.negative, Some(("runtime".into(), "TypeError".into())));
    for text in [
        "",
        "/*---",
        "/*---\nflags: [raw,onlyStrict]\n---*/",
        "/*---\nflags: [foo]\n---*/",
        "/*---\nflags: [async, async]\n---*/",
        "/*---\nnegative:\n  phase: parse\n---*/",
        "/*---\nnegative:\n  phase: unknown\n  type: Error\n---*/",
        "/*---\nflags: [async]\nflags: []\n---*/",
        "/*---\nincludes: &ref\n---*/",
        "/*---\nflags: [CanBlockIsFalse,CanBlockIsTrue]\n---*/",
        "/*---\nnegative: {phase: parse}\n---*/",
        "/*---\nunsupported: true\n---*/",
    ] {
        assert!(metadata::parse(text).is_err(), "{text}");
    }
}
#[test]
fn isolated_variants_original_includes_raw_and_fixture_discovery() {
    let f = Fixture::new();
    f.write("harness/order.js", "assert(order===1);order=2");
    f.write(
        "harness/assert.js",
        "var order=1;function assert(x){if(x!==true)throw 7}",
    );
    f.write("test/a.js","/*---\nincludes: [order.js]\n---*/\nassert(order===2);assert(typeof contamination==='undefined');var contamination=1;");
    f.write(
        "test/b.js",
        "/*---\n---*/\nassert(typeof contamination==='undefined');",
    );
    f.write("test/raw.js","/*---\nflags: [raw]\nincludes: [absent.js]\n---*/\nif(typeof assert!=='undefined')throw 7;");
    f.write("test/sub/x_FIXTURE.js", "THIS MUST NOT EXECUTE");
    f.write("test/sub/data.json", "invalid standalone file");
    let (result, out) = f.run(&["--all"]);
    assert!(result.is_ok(), "{out}");
    assert!(
        out.contains("3 files, 5 variants: 5 passed, 0 failed, 0 unsupported; 1 fixtures"),
        "{out}"
    );
    let (result, out) = f.run(&["test/a.js", "test", "--summary"]);
    assert!(result.is_ok(), "{out}");
    assert!(!out.contains("[strict]"));
    assert!(out.contains("3 files"));
}
#[test]
fn negative_phase_types_cannot_mask_harness_resource_or_unsupported_errors() {
    let f = Fixture::new();
    let cases = [
        ("runtime", "TypeError", "throw new TypeError()", "PASS"),
        ("runtime", "TypeError", "throw 1", "FAIL"),
        ("runtime", "TypeError", "0", "FAIL"),
        ("runtime", "SyntaxError", "let =", "FAIL"),
        ("runtime", "SyntaxError", "JSON.parse('bad')", "PASS"),
        ("runtime", "Error", "$262.createRealm()", "UNSUPPORTED"),
        (
            "runtime",
            "SyntaxError",
            "RegExp('(a)\\\\1')",
            "UNSUPPORTED",
        ),
        ("parse", "SyntaxError", "0", "FAIL"),
        ("parse", "SyntaxError", "let =", "UNSUPPORTED"),
        ("resolution", "Error", "0", "FAIL"),
    ];
    for (phase, ty, body, expected) in cases {
        f.write(
            "test/a.js",
            &format!(
                "/*---\nflags: [noStrict]\nnegative:\n  phase: {phase}\n  type: {ty}\n---*/\n{body}"
            ),
        );
        let (_, out) = f.run(&["test/a.js"]);
        assert!(out.contains(&format!("[non-strict]: {expected}")), "{out}");
    }
    f.write(
        "test/a.js",
        "/*---\nnegative:\n  phase: runtime\n  type: TypeError\n---*/\nthrow new TypeError()",
    );
    f.write("harness/sta.js", "throw new TypeError()");
    let (r, out) = f.run(&["--all"]);
    assert!(r.is_err() && out.contains("harness execution"), "{out}");
}

#[test]
fn output_failure_and_tiny_fuel_are_not_conformance_passes() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("broken"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let f = Fixture::new();
    f.write("test/a.js", "/*---\n---*/\nwhile(true){}");
    let mut out = Vec::new();
    let args = [f.0.display().to_string(), "--all".to_owned()];
    let r = run(
        args.clone().into_iter(),
        Limits {
            fuel: 4000,
            ..Limits::default()
        },
        &mut out,
    );
    assert!(r.is_err());
    assert!(String::from_utf8(out).unwrap().contains("resource limit"));
    assert!(
        run(
            args.into_iter(),
            Limits {
                fuel: 100,
                ..Limits::default()
            },
            &mut Broken
        )
        .is_err()
    );
}
#[test]
fn async_completion_and_first_argument_print_contract() {
    let f = Fixture::new();
    for (body, pass) in [
        ("Promise.resolve().then(()=>$DONE())", true),
        ("$DONE();$DONE()", false),
        ("0", false),
        ("$DONE('bad')", false),
        ("$DONE();throw 7", false),
        (
            "print({toString(){return 'Test262:AsyncTestComplete'}},'ignored')",
            true,
        ),
    ] {
        f.write(
            "test/a.js",
            &format!("/*---\nflags: [async]\n---*/\n{body}"),
        );
        let (r, out) = f.run(&["test/a.js"]);
        assert_eq!(r.is_ok(), pass, "{out}");
    }
    f.write("test/a.js","/*---\n---*/\nassert($262.global===globalThis);assert($262.gc()===undefined);assert(!Object.getOwnPropertyDescriptor(globalThis,'print').enumerable);assert(!Object.getOwnPropertyDescriptor(globalThis,'$262').enumerable)");
    let (r, out) = f.run(&["--all"]);
    assert!(r.is_ok(), "{out}");
}

#[test]
fn eval_script_uses_the_same_realm_without_early_checkpoint() {
    let f = Fixture::new();
    f.write("test/a.js","/*---\nflags: [async]\n---*/\nlet x=7,log='';assert($262.evalScript('x')===7);$262.evalScript(\"var y=3;Promise.resolve().then(()=>log+='j')\");assert(log==='');assert(y===3);Promise.resolve().then(()=>{assert(log==='j');$DONE()})");
    let (r, out) = f.run(&["--all"]);
    assert!(r.is_ok(), "{out}");
}
#[test]
fn incomplete_features_metadata_and_missing_inputs_are_failures() {
    let f = Fixture::new();
    for meta in ["flags: [module]", "flags: [CanBlockIsTrue]"] {
        f.write("test/a.js", &format!("/*---\n{meta}\n---*/\n0"));
        let (r, out) = f.run(&["--all"]);
        assert!(r.is_err() && out.contains(": UNSUPPORTED"), "{out}");
    }
    for text in [
        "/*---\nincludes: [absent.js]\n---*/",
        "not metadata",
        "/*---\nflags: [bogus]\n---*/",
    ] {
        f.write("test/a.js", text);
        let (r, out) = f.run(&["--all"]);
        assert!(r.is_err() && out.contains("failed"), "{out}");
    }
    for args in [
        vec![],
        vec!["--unknown"],
        vec!["--all", "test/a.js"],
        vec!["missing"],
        vec!["harness/assert.js"],
        vec!["/etc/passwd"],
    ] {
        assert!(f.run(&args).0.is_err());
    }
    f.write("test/a.js", "/*---\nincludes: [../test/a.js]\n---*/");
    assert!(f.run(&["--all"]).0.is_err());
    assert!(contained(&f.0, "../").is_err());
}

#[test]
fn additional_metadata_errors_and_host_failures_are_accounted() {
    for text in [
        "/*---\nnegative:\n  phase: parse\n  phase: runtime\n  type: Error\n---*/",
        "/*---\nnegative:\n  phase: parse\n  other: Error\n---*/",
        "/*---\nflags:\n  not-a-list\n---*/",
        "/*---\nincludes: [a.js,]\n---*/",
        "/*---\nincludes: [\"bad.js]\n---*/",
        "/*---\nincludes: ['bad.js]\n---*/",
        "/*---\nno-colon\n---*/",
        "/*---\nflags: [raw, noStrict]\n---*/",
    ] {
        assert!(metadata::parse(text).is_err(), "{text}");
    }
    assert!(metadata::parse(&format!("/*---\n{}\n---*/", " ".repeat(66000))).is_err());
    assert!(
        metadata::parse(&format!(
            "/*---\nincludes: [{}]\n---*/",
            vec!["a.js"; 129].join(",")
        ))
        .is_err()
    );
    let f = Fixture::new();
    for body in [
        "$262.detachArrayBuffer({})",
        "$262.AbstractModuleSource()",
        "$262.agent.start('1')",
    ] {
        f.write("test/a.js", &format!("/*---\n---*/\n{body}"));
        let (r, out) = f.run(&["--all"]);
        assert!(r.is_err() && out.contains(": UNSUPPORTED"), "{out}");
    }
    for body in [
        "throw {constructor:{name:7}}",
        "throw {constructor:{get name(){throw 7}}}",
        "throw new RangeError('r')",
        "throw null",
        "throw {message:'x'}",
        "throw new ReferenceError('r')",
    ] {
        f.write(
            "test/a.js",
            &format!("/*---\nnegative:\n  phase: runtime\n  type: TypeError\n---*/\n{body}"),
        );
        let (r, out) = f.run(&["--all"]);
        assert!(r.is_err(), "{out}");
    }
    f.write("harness/broken.js", "let =");
    f.write("test/a.js", "/*---\nincludes: [broken.js]\n---*/");
    assert!(f.run(&["--all"]).0.is_err());
    f.write("test/sub/x_FIXTURE.js", "0");
    let (r, out) = f.run(&["test/sub"]);
    assert!(r.is_err() && out.contains("0 variants"), "{out}");
    std::fs::remove_file(f.0.join("harness/assert.js")).unwrap();
    assert!(f.run(&["--all"]).0.is_err());
    let mut out = Vec::new();
    assert!(run(std::iter::empty(), Limits::default(), &mut out).is_err());
}
