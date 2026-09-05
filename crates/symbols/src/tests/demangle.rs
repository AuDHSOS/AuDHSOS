// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::demangle`, against names taken out of the kernel image
//! of this project and against inputs no compiler produces.

use test_support::generators::{bytes, vec};
use test_support::property::check;

use crate::demangle::demangle;

/// The demangled form of `name`.
fn text(name: &str) -> String {
    format!("{}", demangle(name))
}

#[test]
fn a_path_of_a_crate_a_module_and_a_function_reads_back() {
    assert_eq!(
        text("_RNvNtCs1234_11kernel_core6memory8bring_up"),
        "kernel_core::memory::bring_up"
    );
    assert_eq!(text("_RNvCs1234_11audhsos_abi4main"), "audhsos_abi::main");
    assert_eq!(text("_RCs1234_11audhsos_abi"), "audhsos_abi");
}

#[test]
fn a_method_of_an_inherent_implementation_names_its_type() {
    // Out of the kernel image: a generic method of `PhysicalWindow`, whose
    // type comes back through a backreference into the implementation.
    assert_eq!(
        text(
            "_RINvMNtCs6T6NbRuQaM9_17kernel_hal_x86_646windowNtB3_14PhysicalWindow\
             7pointerAhj1000_EB5_"
        ),
        "kernel_hal_x86_64::window::PhysicalWindow::pointer"
    );
}

#[test]
fn a_method_of_a_primitive_and_of_a_slice_names_what_it_is_on() {
    assert_eq!(
        text("_RINvMNtCsxqOZXUF523_4core4boolb9then_someyECs225oReabtkd_9kernel_mm"),
        "bool::then_some"
    );
    assert_eq!(
        text("_RINvMNtCsxqOZXUF523_4core5sliceSAyj2_7get_mutjECs6T6NbRuQaM9_17kernel_hal_x86_64"),
        "[[u64; 2]]::get_mut"
    );
    assert_eq!(
        text("_RINvMNtCsxqOZXUF523_4core5sliceSh3getINtNtNtB5_3ops5range5RangejEECs1_3abi"),
        "[u8]::get"
    );
}

#[test]
fn every_basic_type_has_the_letter_the_standard_gives_it() {
    // One inherent method per basic type, so that a letter that moved
    // shows up as a wrong type and not as a failure to parse.
    for (letter, rendered) in [
        ('a', "i8"),
        ('b', "bool"),
        ('c', "char"),
        ('d', "f64"),
        ('e', "str"),
        ('f', "f32"),
        ('h', "u8"),
        ('i', "isize"),
        ('j', "usize"),
        ('l', "i32"),
        ('m', "u32"),
        ('n', "i128"),
        ('o', "u128"),
        ('s', "i16"),
        ('t', "u16"),
        ('u', "()"),
        ('x', "i64"),
        ('y', "u64"),
        ('z', "!"),
    ] {
        let name = format!("_RNvMCs1_4core{letter}4take");
        assert_eq!(text(&name), format!("{rendered}::take"), "letter {letter}");
    }
}

#[test]
fn the_shapes_a_type_can_have_are_written_short() {
    for (mangled, rendered) in [
        ("Rh", "&u8"),
        ("QNvCs1_1c1t", "&mut _"),
        ("Ph", "*const u8"),
        ("Oh", "*mut u8"),
        ("Sh", "[u8]"),
        ("Ahj10_", "[u8; 16]"),
        ("Thh E", "(u8, u8)"),
        ("T E", "()"),
    ] {
        let name = format!("_RNvMCs1_4core{}4take", mangled.replace(' ', ""));
        let expected = format!("{rendered}::take");
        if mangled.starts_with('Q') {
            // A path type inside a reference is a path, not a placeholder.
            assert_eq!(text(&name), "&mut c::t::take");
            continue;
        }
        assert_eq!(text(&name), expected, "{mangled}");
    }
}

#[test]
fn a_trait_implementation_names_the_type_and_the_trait() {
    assert_eq!(
        text("_RNvXCs1_3mehNtB2_3FooNtB2_3Bar3baz"),
        "<meh::Foo as meh::Bar>::baz"
    );
}

#[test]
fn a_closure_without_a_name_is_called_one() {
    assert_eq!(
        text("_RNCNvCs1_3meh4main0"),
        "meh::main::{closure}",
        "an empty identifier in the closure namespace"
    );
}

#[test]
fn a_closure_inside_a_closure_reads_both() {
    // Out of the kernel image. The two empty identifiers stand next to
    // each other, so a length that swallowed both zeros would lose the
    // name: a leading zero is the whole number.
    assert_eq!(
        text("_RNCNCNvCs2Pcg7gLbZZJ_14audhsos_kernel3run00B5_"),
        "audhsos_kernel::run::{closure}::{closure}"
    );
    assert_eq!(
        text(
            "_RINvNtCsbuXs5sHl4eN_11kernel_core6memory11with_memoryu\
             NCNCNvCs2Pcg7gLbZZJ_14audhsos_kernel3run00EBW_"
        ),
        "kernel_core::memory::with_memory"
    );
}

#[test]
fn an_identifier_of_no_length_does_not_swallow_what_follows() {
    // `0` is one identifier of no length, `10` is one of ten bytes.
    assert_eq!(text("_RNvCs1_3meh0"), "meh::{unnamed}");
    assert_eq!(text("_RNvCs1_3meh10abcdefghij"), "meh::abcdefghij");
}

#[test]
fn a_legacy_name_reads_back_without_its_hash() {
    assert_eq!(
        text("_ZN11kernel_core6memory8bring_up17h0123456789abcdefE"),
        "kernel_core::memory::bring_up"
    );
    assert_eq!(text("_ZN4main17h0123456789abcdefE"), "main");
}

#[test]
fn a_name_the_parser_does_not_read_is_written_unchanged() {
    for name in [
        "kernel_entry",
        "",
        "_R",
        "_RX",
        "_ZN",
        "_ZNE",
        "_ZN99xE",
        "_RNvNtCs1234_11kernel_core6memory",
        "_R1NvCs1_3abc",
        "_RB0_",
        "_RB_",
    ] {
        assert_eq!(text(name), name, "{name}");
    }
}

#[test]
fn a_backreference_that_points_forward_or_at_itself_is_refused() {
    // Both would make a reader loop; the name comes back unchanged.
    assert_eq!(text("_RNvB4_1a"), "_RNvB4_1a");
    assert_eq!(text("_RNvBz_1a"), "_RNvBz_1a");
}

#[test]
fn nesting_without_end_is_refused() {
    let mut name = String::from("_R");
    for _ in 0..200 {
        name.push('R');
    }
    name.push('h');
    assert_eq!(text(&name), name, "a reference nested past the bound");
}

#[test]
fn property_no_name_makes_the_demangler_panic_or_loop() {
    let generator = vec(bytes(0..=40), 1..=1);
    check("no name panics", &generator, |chunks: &Vec<Vec<u8>>| {
        let Some(chunk) = chunks.first() else {
            return Ok(());
        };
        let text: String = chunk
            .iter()
            .map(|byte| char::from(byte.wrapping_rem(0x5F).wrapping_add(0x20)))
            .collect();
        for name in [text.clone(), format!("_R{text}"), format!("_ZN{text}")] {
            let written = format!("{}", demangle(&name));
            if written.is_empty() && !name.is_empty() {
                return Err(format!("`{name}` wrote nothing"));
            }
        }
        Ok(())
    });
}

#[test]
fn a_function_type_is_parsed_whole_and_written_short() {
    // A signature carries a binder, the unsafe marker, an abi, the
    // parameters, and the return type; the output keeps none of it, but
    // the parser has to walk all of it to find what follows.
    for signature in ["FEu", "FG_UKChEu", "FK3sysEu", "FUhhEu"] {
        let name = format!("_RNvMCs1_4core{signature}4take");
        assert_eq!(text(&name), "fn(_)::take", "{signature}");
    }
}

#[test]
fn a_trait_object_is_parsed_whole_and_written_short() {
    for bounds in [
        "DEL_",
        "DCs1_3mehEL_",
        "DG_Cs1_3mehEL_",
        "DCs1_3mehp4ItemhEL_",
    ] {
        let name = format!("_RNvMCs1_4core{bounds}4take");
        assert_eq!(text(&name), "dyn _::take", "{bounds}");
    }
}

#[test]
fn the_shapes_a_constant_can_have_are_parsed() {
    // A placeholder, a negative number, and a number too large to write
    // all leave the length out; a plain one is written.
    assert_eq!(text("_RNvMCs1_4coreAhp4take"), "[u8; _]::take");
    assert_eq!(text("_RNvMCs1_4coreAllnff_4take"), "[i32; _]::take");
    assert_eq!(text("_RNvMCs1_4coreAhj0_4take"), "[u8; 0]::take");
    assert_eq!(
        text("_RNvMCs1_4coreAhjffffffffffffffff0_4take"),
        "[u8; _]::take",
        "a length that does not fit"
    );
}

#[test]
fn a_trait_definition_and_a_disambiguated_implementation_read() {
    assert_eq!(text("_RNvYhNtCs1_3meh3Bar4take"), "<u8 as meh::Bar>::take");
    assert_eq!(
        text("_RNvXs1_Cs1_3mehNtB5_3FooNtB5_3Bar3baz"),
        "<meh::Foo as meh::Bar>::baz",
        "an implementation with a disambiguator"
    );
}

#[test]
fn a_backreference_in_a_type_position_reads() {
    // The second parameter of the tuple points back at the first.
    assert_eq!(
        text("_RNvMCs1_4coreTNtCs1_3meh3FooBc_E4take"),
        "(meh::Foo, meh::Foo)::take"
    );
}

#[test]
fn a_variadic_marker_and_a_punycode_identifier_read() {
    assert_eq!(text("_RNvMCs1_4corev4take"), "...::take");
    assert_eq!(
        text("_RNvCs1_3mehu4abcd"),
        "meh::abcd",
        "a punycode identifier is written as it stands"
    );
}

#[test]
fn a_namespace_this_module_does_not_name_gets_a_placeholder() {
    assert_eq!(text("_RNSNvCs1_3meh4main0"), "meh::main::{shim}");
    assert_eq!(text("_RNQNvCs1_3meh4main0"), "meh::main::{unnamed}");
}
