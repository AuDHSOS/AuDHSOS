// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{checksum, code, fields, property, range, records, source, unicode_data, variant};

#[test]
fn ucd_ranges_defaults_and_malformed_records() {
    assert_eq!(checksum(b""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(checksum(b"a"), 0xaf63_dc4c_8601_ec8c);
    assert!(code("110000").is_err());
    assert!(code("xyz").is_err());
    assert!(range("0020..0010").is_err());
    assert_eq!(range(" 0000..10FFFF ").expect("range"), (0, 0x10_ffff));
    let source =
        "# @missing: 0000..10FFFF; Other\n0020..0021; Letter # comment\n0022 ; InCB; Linker\n";
    let defaults = records(source, None, true).expect("defaults");
    assert_eq!(defaults, [(0, 0x10_ffff, "Other".to_owned())]);
    assert_eq!(
        records(source, Some("InCB"), false).expect("selection"),
        [(0x22, 0x22, "Linker".to_owned())]
    );
    assert_eq!(fields("0041; Letter # discarded"), ["0041", "Letter"]);
    for entries in [
        vec![(2, 1, "A".to_owned())],
        vec![(0, 0x11_0000, "A".to_owned())],
        vec![(0, 1, "A".to_owned()), (1, 2, "B".to_owned())],
    ] {
        assert!(property("Test", "test", "Other", &[], &entries).is_err());
    }
    assert_eq!(variant("Regional_Indicator"), "RegionalIndicator");
    assert_eq!(variant("ZWJ"), "Zwj");
    assert_eq!(variant(""), "");
}
fn row(code: &str, name: &str, category: &str, ccc: &str) -> String {
    format!("{code};{name};{category};{ccc};L;;;;;N;;;;;\n")
}
#[test]
fn unicode_data_first_last_ranges_are_checked() {
    let first = row("3400", "<CJK Ideograph Extension A, First>", "Lo", "0");
    let last = row("4DBF", "<CJK Ideograph Extension A, Last>", "Lo", "0");
    let parsed = unicode_data(&(first.clone() + &last)).expect("range");
    assert_eq!(parsed.categories, [(0x3400, 0x4dbf, "Lo".to_owned())]);
    for invalid in [
        "0041;SHORT".to_owned(),
        first.clone(),
        last,
        first.clone() + &first,
        first.clone() + &row("3401", "X", "Lo", "0"),
        first.clone() + &row("4DBF", "<Wrong, Last>", "Lo", "0"),
        first.clone() + &row("4DBF", "<CJK Ideograph Extension A, Last>", "Lu", "0"),
        first.clone() + &row("4DBF", "<CJK Ideograph Extension A, Last>", "Lo", "1"),
        first + &row("3300", "<CJK Ideograph Extension A, Last>", "Lo", "0"),
    ] {
        assert!(unicode_data(&invalid).is_err(), "{invalid}");
    }
}
#[test]
fn source_version_and_headerless_file_fingerprint_are_checked() {
    let path = std::env::temp_dir().join(format!("text-unicode-version-{}", std::process::id()));
    std::fs::create_dir_all(&path).expect("temp");
    let mut hashes = Vec::new();
    std::fs::write(path.join("Scripts.txt"), "# Scripts-17.0.0.txt\n").expect("write");
    assert!(source(&path, "Scripts.txt", &mut hashes).is_err());
    std::fs::write(path.join("UnicodeData.txt"), "modified").expect("write");
    assert!(source(&path, "UnicodeData.txt", &mut hashes).is_err());
    assert!(source(&path, "absent", &mut hashes).is_err());
    std::fs::remove_dir_all(&path).expect("cleanup");
}
