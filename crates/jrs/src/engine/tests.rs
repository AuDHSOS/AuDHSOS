// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::similar_names,
    reason = "test bindings for property names and shape transitions"
)]

use super::{
    bytecode::{BytecodeFunction, Instruction, Reg},
    elements::ElementsKind,
    feedback::{FeedbackVector, NamedAccessIC},
    heap::GenerationalHeap,
    interpreter::RegisterVM,
    shape::PropertyFlags,
    value::{VALUE_NULL, Value},
};

#[test]
fn end_to_end_shape_transitions_and_shared_shapes() {
    let mut heap = GenerationalHeap::new();
    let root = heap.shapes.root_shape();

    let prop_a = heap.strings.intern("a");
    let prop_b = heap.strings.intern("b");

    // Object 1: { a: 1, b: 2 }
    let obj1 = heap.allocate_object(root, VALUE_NULL);
    let (shape_a, slot_a) = heap
        .shapes
        .transition(root, prop_a, PropertyFlags::ordinary_data());
    heap.get_object_mut(obj1).unwrap().shape_id = shape_a;
    heap.get_object_mut(obj1)
        .unwrap()
        .set_slot(slot_a, Value::from_smi(1));

    let (shape_ab, slot_b) =
        heap.shapes
            .transition(shape_a, prop_b, PropertyFlags::ordinary_data());
    heap.get_object_mut(obj1).unwrap().shape_id = shape_ab;
    heap.get_object_mut(obj1)
        .unwrap()
        .set_slot(slot_b, Value::from_smi(2));

    // Object 2: { a: 10, b: 20 } - transitions along the same path
    let obj2 = heap.allocate_object(root, VALUE_NULL);
    let (shape_a2, slot_a2) = heap
        .shapes
        .transition(root, prop_a, PropertyFlags::ordinary_data());
    assert_eq!(shape_a2, shape_a);
    assert_eq!(slot_a2, slot_a);
    heap.get_object_mut(obj2).unwrap().shape_id = shape_a2;
    heap.get_object_mut(obj2)
        .unwrap()
        .set_slot(slot_a2, Value::from_smi(10));

    let (shape_ab2, slot_b2) =
        heap.shapes
            .transition(shape_a2, prop_b, PropertyFlags::ordinary_data());
    assert_eq!(shape_ab2, shape_ab);
    assert_eq!(slot_b2, slot_b);
    heap.get_object_mut(obj2).unwrap().shape_id = shape_ab2;
    heap.get_object_mut(obj2)
        .unwrap()
        .set_slot(slot_b2, Value::from_smi(20));

    // Both objects share the exact same shape!
    assert_eq!(
        heap.get_object(obj1).unwrap().shape_id,
        heap.get_object(obj2).unwrap().shape_id
    );
}

#[test]
fn end_to_end_packed_array_operations() {
    let mut heap = GenerationalHeap::new();
    let arr_ref = heap.allocate_array(4);
    let elem_ref = heap.get_object(arr_ref).unwrap().elements.unwrap();

    // Fast push of integers
    heap.get_elements_mut(elem_ref)
        .unwrap()
        .push(Value::from_smi(100));
    heap.get_elements_mut(elem_ref)
        .unwrap()
        .push(Value::from_smi(200));
    heap.get_elements_mut(elem_ref)
        .unwrap()
        .push(Value::from_smi(300));

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

    let slot_set_x = code.allocate_feedback_slot();
    let slot_get_x = code.allocate_feedback_slot();
    let slot_set_y = code.allocate_feedback_slot();
    let slot_get_y = code.allocate_feedback_slot();

    let mut heap = GenerationalHeap::new();
    let prop_x = heap.strings.intern("x");
    let prop_y = heap.strings.intern("y");

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

    let mut feedback = FeedbackVector::new(code.feedback_slot_count);
    let mut vm = RegisterVM::new(50_000);

    let result = vm.run(&code, &mut feedback, &mut heap).unwrap();
    assert_eq!(result.as_smi(), Some(35));

    // Verify that both getter ICs became monomorphic
    let ic_x = feedback.get_named_ic(slot_get_x).unwrap();
    assert!(matches!(ic_x, NamedAccessIC::Monomorphic { .. }));

    let ic_y = feedback.get_named_ic(slot_get_y).unwrap();
    assert!(matches!(ic_y, NamedAccessIC::Monomorphic { .. }));
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
    let mut feedback = FeedbackVector::new(code.feedback_slot_count);

    let res = vm.run(&code, &mut feedback, &mut heap).unwrap();
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
    let mut feedback = FeedbackVector::new(code.feedback_slot_count);

    let start = std::time::Instant::now();
    for _ in 0..10 {
        let _ = vm.run(&code, &mut feedback, &mut heap).unwrap();
    }
    let elapsed = start.elapsed();
    eprintln!("Register VM: 10 runs of 100k loop sum elapsed = {elapsed:?}");
}
