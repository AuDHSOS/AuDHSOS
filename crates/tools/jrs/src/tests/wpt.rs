// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::*;

#[test]
fn report_requires_real_completion_and_all_results() -> Result<(), Error> {
    let mut report = Report::default();
    assert!(report.validate().is_err());
    report.print(&[
        Value::string("__jrs_wpt_result__"),
        Value::Number(0.0),
        Value::string("case"),
        Value::Null,
    ])?;
    assert!(report.validate().is_err());
    report.print(&[
        Value::string("__jrs_wpt_complete__"),
        Value::Number(0.0),
        Value::Number(1.0),
        Value::Null,
    ])?;
    assert_eq!(report.validate(), Ok(1));
    report.print(&[
        Value::string("__jrs_wpt_complete__"),
        Value::Number(0.0),
        Value::Number(1.0),
        Value::Null,
    ])?;
    assert!(report.validate().is_err());
    Ok(())
}
#[test]
fn report_fails_on_failures_mismatch_and_malformed_callbacks() -> Result<(), Error> {
    for (status, total, harness) in [(1.0, 1.0, 0.0), (0.0, 2.0, 0.0), (0.0, 1.0, 1.0)] {
        let mut report = Report::default();
        report.print(&[
            Value::string("__jrs_wpt_result__"),
            Value::Number(status),
            Value::string("case"),
            Value::string("reason"),
        ])?;
        report.print(&[
            Value::string("__jrs_wpt_complete__"),
            Value::Number(harness),
            Value::Number(total),
            Value::Null,
        ])?;
        assert!(report.validate().is_err());
    }
    assert_eq!(
        Report::default().print(&[Value::string("__jrs_wpt_result__")]),
        Err(Error::Host)
    );
    assert_eq!(
        Report::default().print(&[Value::string("__jrs_wpt_complete__")]),
        Err(Error::Host)
    );
    for args in [
        vec![Value::string("__jrs_wpt_start__")],
        vec![Value::string("__jrs_wpt_start__"), Value::Number(1.0)],
    ] {
        assert_eq!(Report::default().print(&args), Err(Error::Host));
    }
    Ok(())
}

#[test]
fn extraction_preserves_scripts_and_enforces_resource_boundaries() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let source = scripts(&root, "src/tests/fixtures/scripts.html", 8192)?;
    assert_eq!(source.sources.len(), 3);
    let source = source.sources.join("\n");
    assert!(source.contains("let helper = 7"));
    assert!(source.contains("'&amp;'"));
    assert!(source.contains("3 < 4"));
    assert!(source.contains("42;"));
    assert!(!source.contains("not executable"));
    let meta = scripts(&root, "src/tests/fixtures/meta.js", 8192)?;
    assert_eq!(meta.sources.len(), 2);
    let meta = meta.sources.join("\n");
    assert!(meta.find("let helper").unwrap_or(usize::MAX) < meta.find("META:").unwrap_or(0));
    assert!(path(&root, "../Cargo.toml").is_err());
    assert!(path(&root, "missing-file").is_err());
    assert!(scripts(&root, "src/tests/fixtures/helper.js", 1).is_err());
    for file in ["module.html", "async.html", "empty.html", "variant.js"] {
        assert!(scripts(&root, &format!("src/tests/fixtures/{file}"), 8192).is_err());
    }
    let mut out = Scripts::default();
    for resource in [
        "https://example.invalid/test.js",
        "x.js?q=1",
        "x.js#fragment",
    ] {
        assert!(append_resource(&mut out, &root, Path::new(""), resource, 8192).is_err());
    }
    append_resource(
        &mut out,
        &root,
        Path::new(""),
        "/src/tests/fixtures/helper.js",
        8192,
    )?;
    assert!(out.sources[0].contains("helper"));
    assert!(append(&mut Scripts::default(), "12345", 4).is_err());
    Ok(())
}

#[test]
fn shell_runner_rejects_absent_checkout_and_missing_test_selection() {
    for args in [
        vec![],
        vec![env!("CARGO_MANIFEST_DIR")],
        vec!["/missing-wpt-root", "test.js"],
        vec![env!("CARGO_MANIFEST_DIR"), "test.js"],
    ] {
        assert!(run(args.into_iter().map(str::to_owned), &mut Vec::new()).is_err());
    }
    let mut report = Report::default();
    assert_eq!(
        report.print(&[Value::string("other"), Value::Number(1.0)]),
        Ok(())
    );
    assert_eq!(report.lines, ["other 1"]);
    assert_eq!(
        report.print(&[
            Value::string("__jrs_wpt_complete__"),
            Value::Number(0.0),
            Value::string("bad"),
            Value::Null
        ]),
        Err(Error::Host)
    );
}

#[test]
fn runner_transport_reports_success_errors_and_incomplete_runs() -> Result<(), String> {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("test"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // Project-owned transport fixture, not the WPT harness or WPT evidence.
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tests/fixtures");
    let mut output = Vec::new();
    run(
        [root, "report.js"].into_iter().map(str::to_owned),
        &mut output,
    )?;
    assert!(String::from_utf8_lossy(&output).contains("PASS (1 subtests)"));
    let mut output = Vec::new();
    run(
        [root, "microtask-order.js"].into_iter().map(str::to_owned),
        &mut output,
    )?;
    assert!(String::from_utf8_lossy(&output).contains("PASS mixed FIFO: a,b,c,d,e"));
    let mut output = Vec::new();
    run(
        [root, "timers.js"].into_iter().map(str::to_owned),
        &mut output,
    )?;
    assert!(String::from_utf8_lossy(&output).contains("PASS timer checkpoint: am"));
    let mut output = Vec::new();
    run(
        [root, "manual-timer.js"].into_iter().map(str::to_owned),
        &mut output,
    )?;
    assert!(String::from_utf8_lossy(&output).contains("PASS manual completion: true"));
    assert!(
        run(
            [root, "manual-missing.js"].into_iter().map(str::to_owned),
            &mut Vec::new()
        )
        .is_err()
    );
    let mut output = Vec::new();
    assert!(
        run(
            [root, "manual-late-failure.js"]
                .into_iter()
                .map(str::to_owned),
            &mut output
        )
        .is_err()
    );
    assert!(String::from_utf8_lossy(&output).contains("FAIL late failure"));
    let mut output = Vec::new();
    assert!(
        run(
            [root, "microtask-throw.js"].into_iter().map(str::to_owned),
            &mut output
        )
        .is_err()
    );
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("PASS after exception"));
    assert!(output.contains("uncaught JavaScript exception: 7"));
    let mut output = Vec::new();
    run(
        [root, "script-boundaries.html"]
            .into_iter()
            .map(str::to_owned),
        &mut output,
    )?;
    assert!(String::from_utf8_lossy(&output).contains("PASS (1 subtests)"));
    for file in ["throw.js", "syntax.js", "helper.js", "missing.js"] {
        let mut output = Vec::new();
        assert!(run([root, file].into_iter().map(str::to_owned), &mut output).is_err());
        assert!(String::from_utf8_lossy(&output).contains("FAIL:"));
    }
    for file in ["report.js", "helper.js", "throw.js"] {
        assert!(run([root, file].into_iter().map(str::to_owned), &mut Broken).is_err());
    }
    Ok(())
}
