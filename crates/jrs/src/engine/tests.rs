// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::similar_names,
    reason = "test bindings for property names and shape transitions"
)]

use super::{
    bytecode::{BytecodeFunction, FeedbackKind, Instruction, Reg},
    elements::ElementsKind,
    feedback::{FeedbackVector, NamedAccessIC},
    heap::GenerationalHeap,
    interpreter::RegisterVM,
    realm::{Realm, builtin_data},
    shape::PropertyFlags,
    value::{PropertyKey, VALUE_NULL, Value},
};

#[test]
fn the_global_object_carries_global_this_with_the_attributes_of_nineteen_one_one() {
    let mut heap = GenerationalHeap::new();
    let realm = Realm::new(&mut heap).unwrap();
    let environment = realm.global_environment();
    let name = PropertyKey::String(heap.strings.intern("globalThis").unwrap());

    let global = environment.global_object(&heap).unwrap();
    let flags = heap.own_named_flags(global, name).unwrap().unwrap();

    assert_eq!(
        environment.this_value(&heap).unwrap(),
        Value::from_object(global)
    );
    assert_eq!(
        environment.get_binding_value(&heap, name).unwrap(),
        Some(Value::from_object(global))
    );
    assert!(flags.writable);
    assert!(!flags.enumerable);
    assert!(flags.configurable);
    assert!(!flags.is_accessor);
}

#[test]
fn the_global_environment_asks_the_declarative_record_before_the_binding_object() {
    let mut heap = GenerationalHeap::new();
    let realm = Realm::new(&mut heap).unwrap();
    let shared = PropertyKey::String(heap.strings.intern("shared").unwrap());
    let only_global = PropertyKey::String(heap.strings.intern("onlyGlobal").unwrap());
    let unbound = PropertyKey::String(heap.strings.intern("unbound").unwrap());
    let environment = realm.global_environment();
    let global = environment.global_object(&heap).unwrap();
    let declarative = environment.declarative_object(&heap).unwrap();

    // 9.1.1.4.1 and 9.1.1.4.6 read the declarative record first, so a name it
    // binds shadows the same name on the binding object.
    heap.define_own_named(global, shared, Value::from_smi(1), builtin_data())
        .unwrap();
    heap.define_own_named(global, only_global, Value::from_smi(2), builtin_data())
        .unwrap();
    heap.define_own_named(declarative, shared, Value::from_smi(3), builtin_data())
        .unwrap();

    assert!(environment.has_binding(&heap, shared).unwrap());
    assert!(environment.has_binding(&heap, only_global).unwrap());
    assert!(!environment.has_binding(&heap, unbound).unwrap());

    assert_eq!(
        environment.get_binding_value(&heap, shared).unwrap(),
        Some(Value::from_smi(3))
    );
    assert_eq!(
        environment.get_binding_value(&heap, only_global).unwrap(),
        Some(Value::from_smi(2))
    );
    // 9.1.1.2.7 throws a ReferenceError here, which the caller raises.
    assert_eq!(environment.get_binding_value(&heap, unbound).unwrap(), None);
}

#[test]
fn a_global_read_answers_the_binding_and_throws_for_an_unbound_name() {
    let mut heap = GenerationalHeap::new();
    let realm = Realm::new(&mut heap).unwrap();
    let bound = PropertyKey::String(heap.strings.intern("bound").unwrap());
    let global = realm.global_environment().global_object(&heap).unwrap();
    heap.define_own_named(global, bound, Value::from_smi(7), builtin_data())
        .unwrap();

    // 9.1.1.4.6 answers the binding the global object carries.
    let mut code = BytecodeFunction::new(1, 0);
    let name = code.add_string_constant("bound".encode_utf16().collect());
    code.emit(Instruction::LdaGlobal(name));
    code.emit(Instruction::Return);
    let mut feedback = FeedbackVector::for_code(&code);
    let mut vm = RegisterVM::new(10_000);
    assert_eq!(
        vm.run(&code, &mut feedback, &mut heap, &realm)
            .unwrap()
            .as_smi(),
        Some(7)
    );

    // 9.1.1.2.7 throws a ReferenceError for a name the binding object lacks.
    let mut code = BytecodeFunction::new(1, 0);
    let name = code.add_string_constant("absent".encode_utf16().collect());
    code.emit(Instruction::LdaGlobal(name));
    code.emit(Instruction::Return);
    let mut feedback = FeedbackVector::for_code(&code);
    let mut vm = RegisterVM::new(10_000);
    let error = vm
        .run(&code, &mut feedback, &mut heap, &realm)
        .expect_err("an unbound name is a ReferenceError");
    let thrown = match error {
        crate::engine::interpreter::VMError::Thrown(value, _) => value,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        realm.native_error_kind(&heap, thrown.as_object().unwrap()),
        Some(crate::engine::realm::NativeErrorKind::ReferenceError)
    );

    // 13.5.3 answers undefined for the same name under `typeof`.
    let mut code = BytecodeFunction::new(1, 0);
    let name = code.add_string_constant("absent".encode_utf16().collect());
    code.emit(Instruction::LdaGlobalForTypeOf(name));
    code.emit(Instruction::Return);
    let mut feedback = FeedbackVector::for_code(&code);
    let mut vm = RegisterVM::new(10_000);
    assert!(
        vm.run(&code, &mut feedback, &mut heap, &realm)
            .unwrap()
            .is_undefined()
    );
}

#[test]
fn end_to_end_shape_transitions_and_shared_shapes() {
    let mut heap = GenerationalHeap::new();
    let root = heap.shapes.root_shape();

    let prop_a = PropertyKey::String(heap.strings.intern("a").unwrap());
    let prop_b = PropertyKey::String(heap.strings.intern("b").unwrap());

    // Object 1: { a: 1, b: 2 }
    let obj1 = heap.allocate_object(root, VALUE_NULL).unwrap();
    let (shape_a, slot_a) = heap
        .shapes
        .transition(root, prop_a, PropertyFlags::ordinary_data());
    heap.set_object_shape(obj1, shape_a).unwrap();
    heap.set_object_slot(obj1, slot_a, Value::from_smi(1))
        .unwrap();

    let (shape_ab, slot_b) =
        heap.shapes
            .transition(shape_a, prop_b, PropertyFlags::ordinary_data());
    heap.set_object_shape(obj1, shape_ab).unwrap();
    heap.set_object_slot(obj1, slot_b, Value::from_smi(2))
        .unwrap();

    // Object 2: { a: 10, b: 20 } - transitions along the same path
    let obj2 = heap.allocate_object(root, VALUE_NULL).unwrap();
    let (shape_a2, slot_a2) = heap
        .shapes
        .transition(root, prop_a, PropertyFlags::ordinary_data());
    assert_eq!(shape_a2, shape_a);
    assert_eq!(slot_a2, slot_a);
    heap.set_object_shape(obj2, shape_a2).unwrap();
    heap.set_object_slot(obj2, slot_a2, Value::from_smi(10))
        .unwrap();

    let (shape_ab2, slot_b2) =
        heap.shapes
            .transition(shape_a2, prop_b, PropertyFlags::ordinary_data());
    assert_eq!(shape_ab2, shape_ab);
    assert_eq!(slot_b2, slot_b);
    heap.set_object_shape(obj2, shape_ab2).unwrap();
    heap.set_object_slot(obj2, slot_b2, Value::from_smi(20))
        .unwrap();

    // Both objects share the exact same shape!
    assert_eq!(
        heap.get_object(obj1).unwrap().shape_id,
        heap.get_object(obj2).unwrap().shape_id
    );
}

#[test]
fn end_to_end_packed_array_operations() {
    let mut heap = GenerationalHeap::new();
    let arr_ref = heap.allocate_array(4).unwrap();
    let elem_ref = heap.get_object(arr_ref).unwrap().elements.unwrap();

    // Fast push of integers
    heap.push_element(elem_ref, Value::from_smi(100)).unwrap();
    heap.push_element(elem_ref, Value::from_smi(200)).unwrap();
    heap.push_element(elem_ref, Value::from_smi(300)).unwrap();

    assert_eq!(heap.get_elements(elem_ref).unwrap().len(), 3);
    assert_eq!(
        heap.get_elements(elem_ref).unwrap().get(0),
        Some(Value::from_smi(100))
    );
    assert_eq!(
        heap.get_elements(elem_ref).unwrap().get(1),
        Some(Value::from_smi(200))
    );
    assert_eq!(
        heap.get_elements(elem_ref).unwrap().get(2),
        Some(Value::from_smi(300))
    );
    assert!(matches!(
        heap.get_elements(elem_ref).unwrap(),
        ElementsKind::PackedSmi(_)
    ));
}

#[test]
fn end_to_end_vm_execution_with_inline_caches() {
    // Function: create object, write fields, read fields via IC
    let mut code = BytecodeFunction::new(4, 0);
    let r_obj = Reg(0);
    let r_res = Reg(1);

    let slot_set_x = code.allocate_feedback_slot(FeedbackKind::NamedAccess);
    let slot_get_x = code.allocate_feedback_slot(FeedbackKind::NamedAccess);
    let slot_set_y = code.allocate_feedback_slot(FeedbackKind::NamedAccess);
    let slot_get_y = code.allocate_feedback_slot(FeedbackKind::NamedAccess);

    let mut heap = GenerationalHeap::new();

    let realm = Realm::new(&mut heap).unwrap();
    let prop_x = code.add_string_constant("x".encode_utf16().collect());
    let prop_y = code.add_string_constant("y".encode_utf16().collect());

    // obj = {}
    code.emit(Instruction::CreateObject);
    code.emit(Instruction::Star(r_obj));

    // obj.x = 10
    code.emit(Instruction::LdaSmi(10));
    code.emit(Instruction::SetNamed {
        obj: r_obj,
        name: prop_x,
        slot: slot_set_x,
    });

    // obj.y = 25
    code.emit(Instruction::LdaSmi(25));
    code.emit(Instruction::SetNamed {
        obj: r_obj,
        name: prop_y,
        slot: slot_set_y,
    });

    // res = obj.x + obj.y
    code.emit(Instruction::GetNamed {
        obj: r_obj,
        name: prop_x,
        slot: slot_get_x,
    });
    code.emit(Instruction::Star(r_res));

    code.emit(Instruction::GetNamed {
        obj: r_obj,
        name: prop_y,
        slot: slot_get_y,
    });
    code.emit(Instruction::Add(r_res));
    code.emit(Instruction::Return);

    let mut feedback = FeedbackVector::for_code(&code);
    let mut vm = RegisterVM::new(50_000);

    let result = vm.run(&code, &mut feedback, &mut heap, &realm).unwrap();
    assert_eq!(result.as_smi(), Some(35));

    // Verify that both getter ICs became monomorphic
    let ic_x = feedback.get_named_ic(slot_get_x).unwrap();
    assert!(matches!(ic_x, NamedAccessIC::Monomorphic(_)));

    let ic_y = feedback.get_named_ic(slot_get_y).unwrap();
    assert!(matches!(ic_y, NamedAccessIC::Monomorphic(_)));
}

#[test]
fn inherited_named_access_uses_depth_cache_and_invalidates_on_mutation() {
    let mut heap = GenerationalHeap::new();
    let realm = Realm::new(&mut heap).unwrap();
    let root_shape = heap.shapes.root_shape();
    let name = PropertyKey::String(heap.strings.intern("answer").unwrap());
    let (prototype_shape, slot) =
        heap.shapes
            .transition(root_shape, name, PropertyFlags::ordinary_data());
    let first_prototype = heap.allocate_object(root_shape, VALUE_NULL).unwrap();
    heap.set_object_shape(first_prototype, prototype_shape)
        .unwrap();
    heap.set_object_slot(first_prototype, slot, Value::from_smi(41))
        .unwrap();
    let receiver = heap
        .allocate_object(root_shape, Value::from_object(first_prototype))
        .unwrap();

    let mut code = BytecodeFunction::new(1, 1);
    let feedback_slot = code.allocate_feedback_slot(FeedbackKind::NamedAccess);
    let name_constant = code.add_string_constant("answer".encode_utf16().collect());
    code.emit(Instruction::GetNamed {
        obj: Reg(0),
        name: name_constant,
        slot: feedback_slot,
    });
    code.emit(Instruction::Return);
    let mut feedback = FeedbackVector::for_code(&code);
    let mut vm = RegisterVM::new(100);

    assert_eq!(
        vm.run_with_arguments(
            &code,
            &[Value::from_object(receiver)],
            &mut feedback,
            &mut heap,
            &realm,
        )
        .unwrap()
        .as_smi(),
        Some(41)
    );
    assert!(matches!(
        feedback.get_named_ic(feedback_slot),
        Some(NamedAccessIC::Monomorphic(case)) if case.holder_depth == 1
    ));

    let second_prototype = heap.allocate_object(root_shape, VALUE_NULL).unwrap();
    heap.set_object_shape(second_prototype, prototype_shape)
        .unwrap();
    heap.set_object_slot(second_prototype, slot, Value::from_smi(42))
        .unwrap();
    heap.set_object_prototype(receiver, Value::from_object(second_prototype))
        .unwrap();

    assert_eq!(
        vm.run_with_arguments(
            &code,
            &[Value::from_object(receiver)],
            &mut feedback,
            &mut heap,
            &realm,
        )
        .unwrap()
        .as_smi(),
        Some(42)
    );
}

#[test]
fn prototype_updates_reject_cycles_and_non_objects() {
    let mut heap = GenerationalHeap::new();
    let root_shape = heap.shapes.root_shape();
    let parent = heap.allocate_object(root_shape, VALUE_NULL).unwrap();
    let child = heap
        .allocate_object(root_shape, Value::from_object(parent))
        .unwrap();

    assert_eq!(
        heap.set_object_prototype(parent, Value::from_object(child)),
        Err(super::heap::HeapError::PrototypeCycle)
    );
    assert_eq!(
        heap.set_object_prototype(parent, Value::from_smi(1)),
        Err(super::heap::HeapError::InvalidPrototype)
    );
    assert_eq!(heap.get_object(parent).unwrap().prototype, VALUE_NULL);
}

#[test]
fn inherited_cache_checks_the_holder_shape_for_equal_receiver_shapes() {
    let mut heap = GenerationalHeap::new();
    let realm = Realm::new(&mut heap).unwrap();
    let root_shape = heap.shapes.root_shape();
    let wanted = PropertyKey::String(heap.strings.intern("wanted").unwrap());
    let other = PropertyKey::String(heap.strings.intern("other").unwrap());
    let (wanted_shape, wanted_slot) =
        heap.shapes
            .transition(root_shape, wanted, PropertyFlags::ordinary_data());
    let (other_shape, other_slot) =
        heap.shapes
            .transition(root_shape, other, PropertyFlags::ordinary_data());
    let wanted_prototype = heap.allocate_object(root_shape, VALUE_NULL).unwrap();
    heap.set_object_shape(wanted_prototype, wanted_shape)
        .unwrap();
    heap.set_object_slot(wanted_prototype, wanted_slot, Value::from_smi(1))
        .unwrap();
    let other_prototype = heap.allocate_object(root_shape, VALUE_NULL).unwrap();
    heap.set_object_shape(other_prototype, other_shape).unwrap();
    heap.set_object_slot(other_prototype, other_slot, Value::from_smi(99))
        .unwrap();
    let first = heap
        .allocate_object(root_shape, Value::from_object(wanted_prototype))
        .unwrap();
    let second = heap
        .allocate_object(root_shape, Value::from_object(other_prototype))
        .unwrap();

    let mut code = BytecodeFunction::new(1, 1);
    let feedback_slot = code.allocate_feedback_slot(FeedbackKind::NamedAccess);
    let wanted_constant = code.add_string_constant("wanted".encode_utf16().collect());
    code.emit(Instruction::GetNamed {
        obj: Reg(0),
        name: wanted_constant,
        slot: feedback_slot,
    });
    code.emit(Instruction::Return);
    let mut feedback = FeedbackVector::for_code(&code);
    let mut vm = RegisterVM::new(100);
    assert_eq!(
        vm.run_with_arguments(
            &code,
            &[Value::from_object(first)],
            &mut feedback,
            &mut heap,
            &realm,
        )
        .unwrap()
        .as_smi(),
        Some(1)
    );

    assert!(
        vm.run_with_arguments(
            &code,
            &[Value::from_object(second)],
            &mut feedback,
            &mut heap,
            &realm,
        )
        .unwrap()
        .is_undefined()
    );
}

#[test]
fn end_to_end_loop_sum_100k_with_smi_to_double_overflow() {
    let mut code = BytecodeFunction::new(4, 0);
    let r_sum = Reg(0);
    let r_i = Reg(1);
    let r_limit = Reg(2);
    let r_temp = Reg(3);
    let c_one = code.add_constant(Value::from_smi(1));

    code.emit(Instruction::LdaSmi(0));
    code.emit(Instruction::Star(r_sum));
    code.emit(Instruction::LdaSmi(0));
    code.emit(Instruction::Star(r_i));
    code.emit(Instruction::LdaSmi(100_000));
    code.emit(Instruction::Star(r_limit));

    code.emit(Instruction::Ldar(r_i));
    code.emit(Instruction::TestLessThan(r_limit));
    code.emit(Instruction::JumpIfFalse(9));

    code.emit(Instruction::Ldar(r_sum));
    code.emit(Instruction::Add(r_i));
    code.emit(Instruction::Star(r_sum));

    code.emit(Instruction::LdaConstant(c_one));
    code.emit(Instruction::Star(r_temp));
    code.emit(Instruction::Ldar(r_i));
    code.emit(Instruction::Add(r_temp));
    code.emit(Instruction::Star(r_i));

    code.emit(Instruction::Jump(-12));

    code.emit(Instruction::Ldar(r_sum));
    code.emit(Instruction::Return);

    let mut vm = RegisterVM::new(10_000_000);
    let mut heap = GenerationalHeap::new();
    let realm = Realm::new(&mut heap).unwrap();
    let mut feedback = FeedbackVector::for_code(&code);

    let res = vm.run(&code, &mut feedback, &mut heap, &realm).unwrap();
    // Sum of 0..99,999 = 4,999,950,000 (promotes from Smi to Double upon overflow)
    assert_eq!(res.as_f64(), Some(4_999_950_000.0));
}

#[test]
fn benchmark_register_vm_sum_10_runs_100k() {
    let mut code = BytecodeFunction::new(4, 0);
    let r_sum = Reg(0);
    let r_i = Reg(1);
    let r_limit = Reg(2);
    let r_temp = Reg(3);
    let c_one = code.add_constant(Value::from_smi(1));

    code.emit(Instruction::LdaSmi(0));
    code.emit(Instruction::Star(r_sum));
    code.emit(Instruction::LdaSmi(0));
    code.emit(Instruction::Star(r_i));
    code.emit(Instruction::LdaSmi(100_000));
    code.emit(Instruction::Star(r_limit));

    code.emit(Instruction::Ldar(r_i));
    code.emit(Instruction::TestLessThan(r_limit));
    code.emit(Instruction::JumpIfFalse(9));

    code.emit(Instruction::Ldar(r_sum));
    code.emit(Instruction::Add(r_i));
    code.emit(Instruction::Star(r_sum));

    code.emit(Instruction::LdaConstant(c_one));
    code.emit(Instruction::Star(r_temp));
    code.emit(Instruction::Ldar(r_i));
    code.emit(Instruction::Add(r_temp));
    code.emit(Instruction::Star(r_i));

    code.emit(Instruction::Jump(-12));

    code.emit(Instruction::Ldar(r_sum));
    code.emit(Instruction::Return);

    let mut vm = RegisterVM::new(10_000_000);
    let mut heap = GenerationalHeap::new();
    let realm = Realm::new(&mut heap).unwrap();
    let mut feedback = FeedbackVector::for_code(&code);

    let start = std::time::Instant::now();
    for _ in 0..10 {
        let _ = vm.run(&code, &mut feedback, &mut heap, &realm).unwrap();
    }
    let elapsed = start.elapsed();
    eprintln!("Register VM: 10 runs of 100k loop sum elapsed = {elapsed:?}");
}
