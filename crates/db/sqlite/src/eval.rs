// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What an expression answers.
//!
//! The tree is walked once, which is O(n) in its nodes, and nothing is
//! compiled. Every operator is the opcode of `src/vdbe.c` it is compiled
//! into, because the order of the conversions is the semantics: an
//! integer plus a string is not the same as a string plus an integer
//! unless the engine says which is converted first.
//!
//! What is not here yet: columns, functions, the pattern operators, and
//! the statements an expression may hold. Each refuses rather than
//! guessing.

use alloc::vec::Vec;

use crate::ast::{Arena, BinaryOp, ExprId, Literal, Node, UnaryOp};
use crate::number::{self, Outcome};
use crate::parse::MAX_DEPTH;
use crate::value::{Affinity, Collation, Value, apply_comparison, cast, compare, compare_affinity};

/// Why an expression could not be answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// A name that is not a column of this row.
    NoColumn,
    /// A function this engine does not have.
    NoFunction,
    /// A shape of expression this engine does not answer yet.
    Unsupported,
    /// The tree names a node the arena does not hold.
    Malformed,
    /// The tree is deeper than the engine walks.
    TooDeep,
    /// A hex literal of more than sixteen digits, which SQLite refuses
    /// rather than reading as a real.
    HexTooBig,
}

/// A value, with what a comparison against it would do.
#[derive(Clone, Debug, PartialEq)]
struct Answer {
    /// The value.
    value: Value,
    /// The affinity of the expression that answered it.
    affinity: Affinity,
    /// The collation written on it, where one was.
    collation: Option<Collation>,
}

impl Answer {
    /// A value no affinity and no collation belong to, which is every
    /// expression that is not a column, a cast or a `COLLATE`.
    const fn plain(value: Value) -> Self {
        Answer {
            value,
            affinity: Affinity::None,
            collation: None,
        }
    }
}

/// What the expression `id` of `arena` answers, where `sql` is the
/// statement its spans point into.
///
/// # Errors
///
/// [`Error`] names what it could not answer and why.
pub fn evaluate(arena: &Arena, id: ExprId, sql: &[u8]) -> Result<Value, Error> {
    Ok(answer(arena, id, sql, 0)?.value)
}

/// One node.
fn answer(arena: &Arena, id: ExprId, sql: &[u8], depth: u32) -> Result<Answer, Error> {
    if depth > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    let node = arena.node(id).ok_or(Error::Malformed)?;
    let deeper = depth.saturating_add(1);
    match node {
        Node::Literal(literal) => literal_value(literal, sql, false).map(Answer::plain),
        Node::Unary { op, operand } => unary(arena, op, operand, sql, deeper),
        Node::Binary { op, left, right } => binary(arena, op, left, right, sql, deeper),
        Node::Between {
            value,
            low,
            high,
            negated,
        } => between(arena, value, low, high, negated, sql, deeper),
        Node::InList {
            value,
            list,
            negated,
        } => {
            let left = answer(arena, value, sql, deeper)?;
            let mut list_answers = Vec::new();
            for member in arena.children(list) {
                list_answers.push(answer(arena, *member, sql, deeper)?);
            }
            Ok(Answer::plain(in_list(&left, &list_answers, negated)))
        }
        Node::Cast { value, ty } => {
            let mut inner = answer(arena, value, sql, deeper)?;
            let affinity = Affinity::of_type(ty.text(sql));
            cast(&mut inner.value, affinity);
            inner.affinity = affinity;
            Ok(inner)
        }
        Node::Collate { value, name } => {
            let mut inner = answer(arena, value, sql, deeper)?;
            inner.collation = Collation::of_name(name.text(sql)).or(inner.collation);
            Ok(inner)
        }
        Node::Case {
            operand,
            branches,
            otherwise,
        } => case(arena, operand, branches, otherwise, sql, deeper),
        Node::Column { .. } => Err(Error::NoColumn),
        Node::Call { .. } | Node::Like { .. } => Err(Error::NoFunction),
        Node::Variable(_)
        | Node::Row(_)
        | Node::Subquery(_)
        | Node::Exists(_)
        | Node::InSelect { .. }
        | Node::InTable { .. } => Err(Error::Unsupported),
    }
}

/// A constant, with `negated` for the minus sign the parser leaves as a
/// node of its own and SQLite folds into the number.
fn literal_value(literal: Literal, sql: &[u8], negated: bool) -> Result<Value, Error> {
    match literal {
        Literal::Null => Ok(Value::Null),
        Literal::Integer(span) => integer_literal(&dequote(span.text(sql)), negated),
        Literal::Float(span) => {
            let read = number::real(&dequote(span.text(sql))).value;
            Ok(Value::Real(if negated { -read } else { read }))
        }
        Literal::Text(span) => Ok(Value::Text(unquote(span.text(sql)))),
        Literal::Blob(span) => Ok(Value::Blob(hex(span.text(sql)))),
        Literal::CurrentTime(_) => Err(Error::Unsupported),
    }
}

/// A number with the digit separators taken out, which is what
/// `sqlite3DequoteNumber` does before anything reads it.
fn dequote(text: &[u8]) -> Vec<u8> {
    text.iter().copied().filter(|byte| *byte != b'_').collect()
}

/// A whole number as written, which is a real where an integer does not
/// hold it and a refusal where a hex literal does not.
///
/// This is `codeInteger` over `sqlite3DecOrHexToI64`.
fn integer_literal(text: &[u8], negated: bool) -> Result<Value, Error> {
    let hexadecimal = text.first() == Some(&b'0') && matches!(text.get(1), Some(b'x' | b'X'));
    let (value, outcome) = if hexadecimal {
        hex_literal(text)
    } else {
        let read = number::integer(text);
        (read.value, read.outcome)
    };
    // The three ways a literal does not fit: past the largest, exactly
    // one past it, and the smallest with the sign already spent.
    if outcome == Outcome::Overflow
        || (outcome == Outcome::Limit && !negated)
        || (negated && value == i64::MIN)
    {
        if hexadecimal {
            return Err(Error::HexTooBig);
        }
        let real = number::real(text).value;
        return Ok(Value::Real(if negated { -real } else { real }));
    }
    Ok(Value::Int(if negated {
        if outcome == Outcome::Limit {
            i64::MIN
        } else {
            value.wrapping_neg()
        }
    } else {
        value
    }))
}

/// `0x` and its digits, as the bits they spell and whether there are too
/// many of them.
fn hex_literal(text: &[u8]) -> (i64, Outcome) {
    let digits: Vec<u8> = text
        .iter()
        .copied()
        .skip(2)
        .skip_while(|byte| *byte == b'0')
        .collect();
    let mut value: u64 = 0;
    for byte in &digits {
        let digit = u64::from(char::from(*byte).to_digit(16).unwrap_or(0));
        value = value.wrapping_mul(16).wrapping_add(digit);
    }
    let outcome = if digits.len() > 16 {
        Outcome::Overflow
    } else {
        Outcome::Exact
    };
    (value.cast_signed(), outcome)
}

/// A quoted string as its bytes, with the doubled quotes folded.
fn unquote(text: &[u8]) -> Vec<u8> {
    let quote = text.first().copied().unwrap_or(b'\'');
    let inner = text
        .get(1..text.len().saturating_sub(1))
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut doubled = false;
    for byte in inner {
        if doubled {
            doubled = false;
            continue;
        }
        out.push(*byte);
        doubled = *byte == quote;
    }
    out
}

/// The bytes of `x'..'`.
fn hex(text: &[u8]) -> Vec<u8> {
    let inner = text
        .get(2..text.len().saturating_sub(1))
        .unwrap_or_default();
    let mut out = Vec::new();
    for pair in inner.chunks(2) {
        let mut byte = 0u8;
        for digit in pair {
            let value = u8::try_from(char::from(*digit).to_digit(16).unwrap_or(0)).unwrap_or(0);
            byte = (byte << 4) | value;
        }
        out.push(byte);
    }
    out
}

/// One operand.
fn unary(
    arena: &Arena,
    op: UnaryOp,
    operand: ExprId,
    sql: &[u8],
    depth: u32,
) -> Result<Answer, Error> {
    if op == UnaryOp::Negate {
        // SQLite folds the sign into the number it precedes, which is the
        // only way to write the smallest integer there is.
        if let Some(Node::Literal(literal)) = arena.node(operand)
            && matches!(literal, Literal::Integer(_) | Literal::Float(_))
        {
            return literal_value(literal, sql, true).map(Answer::plain);
        }
    }
    let inner = answer(arena, operand, sql, depth)?;
    let value = inner.value;
    Ok(match op {
        // A sign before anything else is a subtraction from zero.
        UnaryOp::Negate => Answer::plain(arithmetic(BinaryOp::Subtract, &Value::Int(0), &value)),
        UnaryOp::Identity => Answer { value, ..inner },
        UnaryOp::Not => Answer::plain(match logic(&value) {
            Some(truth) => Value::Int(i64::from(!truth)),
            None => Value::Null,
        }),
        UnaryOp::BitNot => Answer::plain(if value == Value::Null {
            Value::Null
        } else {
            Value::Int(!value.to_integer())
        }),
        UnaryOp::IsNull => Answer::plain(Value::Int(i64::from(value == Value::Null))),
        UnaryOp::NotNull => Answer::plain(Value::Int(i64::from(value != Value::Null))),
    })
}

/// Whether a value is true, false, or neither.
fn logic(value: &Value) -> Option<bool> {
    if *value == Value::Null {
        None
    } else {
        Some(value.truth(false))
    }
}

/// Two operands.
fn binary(
    arena: &Arena,
    op: BinaryOp,
    left: ExprId,
    right: ExprId,
    sql: &[u8],
    depth: u32,
) -> Result<Answer, Error> {
    let left = answer(arena, left, sql, depth)?;
    let right = answer(arena, right, sql, depth)?;
    let value = match op {
        BinaryOp::Or | BinaryOp::And => {
            let (first, second) = (logic(&left.value), logic(&right.value));
            let truth = if op == BinaryOp::And {
                if first == Some(false) || second == Some(false) {
                    Some(false)
                } else {
                    first.and(second)
                }
            } else if first == Some(true) || second == Some(true) {
                Some(true)
            } else {
                first.and(second)
            };
            match truth {
                Some(truth) => Value::Int(i64::from(truth)),
                None => Value::Null,
            }
        }
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulo => arithmetic(op, &left.value, &right.value),
        BinaryOp::Concat => concatenate(&left.value, &right.value),
        BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::LShift | BinaryOp::RShift => {
            bitwise(op, &left.value, &right.value)
        }
        BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge
        | BinaryOp::Is
        | BinaryOp::IsNot => comparison(op, &left, &right),
        BinaryOp::Extract | BinaryOp::ExtractText => return Err(Error::NoFunction),
    };
    Ok(Answer::plain(value))
}

/// `+`, `-`, `*`, `/` and `%`, which count in integers where both sides
/// are integers and in doubles where either is not.
fn arithmetic(op: BinaryOp, left: &Value, right: &Value) -> Value {
    if let (Value::Int(left), Value::Int(right)) = (left, right) {
        return integer_arithmetic(op, *left, *right)
            .unwrap_or_else(|| real_arithmetic(op, &Value::Int(*left), &Value::Int(*right)));
    }
    if *left == Value::Null || *right == Value::Null {
        return Value::Null;
    }
    if let (Value::Int(whole_left), Value::Int(whole_right)) =
        (left.numeric_type(), right.numeric_type())
        && let Some(value) = integer_arithmetic(op, whole_left, whole_right)
    {
        return value;
    }
    real_arithmetic(op, left, right)
}

/// The integer answer, or nothing where it does not fit and the doubles
/// have to be used instead.
fn integer_arithmetic(op: BinaryOp, left: i64, right: i64) -> Option<Value> {
    let value = match op {
        BinaryOp::Add => left.checked_add(right)?,
        BinaryOp::Subtract => left.checked_sub(right)?,
        BinaryOp::Multiply => left.checked_mul(right)?,
        BinaryOp::Divide => {
            if right == 0 {
                return Some(Value::Null);
            }
            left.checked_div(right)?
        }
        _ => {
            if right == 0 {
                return Some(Value::Null);
            }
            // The smallest integer over minus one is the one remainder
            // that overflows, and one divides everything evenly.
            left.checked_rem(if right == -1 { 1 } else { right })
                .unwrap_or(0)
        }
    };
    Some(Value::Int(value))
}

/// The answer in doubles. A NaN is not a value SQLite has, so it is
/// `NULL`.
fn real_arithmetic(op: BinaryOp, left: &Value, right: &Value) -> Value {
    let (first, second) = (left.to_real(), right.to_real());
    let value = match op {
        BinaryOp::Add => first + second,
        BinaryOp::Subtract => first - second,
        BinaryOp::Multiply => first * second,
        BinaryOp::Divide => {
            if second == 0.0 {
                return Value::Null;
            }
            first / second
        }
        _ => {
            let divisor = right.to_integer();
            if divisor == 0 {
                return Value::Null;
            }
            let divisor = if divisor == -1 { 1 } else { divisor };
            let whole = left.to_integer().checked_rem(divisor).unwrap_or(0);
            crate::value::integer_as_real(whole)
        }
    };
    if value.is_nan() {
        Value::Null
    } else {
        Value::Real(value)
    }
}

/// `||`, which is text unless either side is nothing.
fn concatenate(left: &Value, right: &Value) -> Value {
    if *left == Value::Null || *right == Value::Null {
        return Value::Null;
    }
    let bytes = |value: &Value| -> Vec<u8> {
        value
            .bytes()
            .map(<[u8]>::to_vec)
            .or_else(|| value.stringify())
            .unwrap_or_default()
    };
    let mut out = bytes(left);
    out.extend(bytes(right));
    Value::Text(out)
}

/// `&`, `|`, `<<` and `>>`, which count in integers whatever they are
/// given.
fn bitwise(op: BinaryOp, left: &Value, right: &Value) -> Value {
    if *left == Value::Null || *right == Value::Null {
        return Value::Null;
    }
    let value = left.to_integer();
    let count = right.to_integer();
    Value::Int(match op {
        BinaryOp::BitAnd => value & count,
        BinaryOp::BitOr => value | count,
        _ => shift(value, count, op == BinaryOp::LShift),
    })
}

/// A shift, where a negative count shifts the other way and a count of
/// sixty-four or more shifts everything out.
fn shift(value: i64, count: i64, mut left: bool) -> i64 {
    if count == 0 {
        return value;
    }
    let count = if count < 0 {
        left = !left;
        if count > -64 {
            count.wrapping_neg()
        } else {
            64
        }
    } else {
        count
    };
    if count >= 64 {
        return if value >= 0 || left { 0 } else { -1 };
    }
    let places = u32::try_from(count).unwrap_or(0);
    let bits = value.cast_unsigned();
    let shifted = if left {
        bits.wrapping_shl(places)
    } else {
        let shifted = bits.wrapping_shr(places);
        if value < 0 {
            // A right shift carries the sign along with it.
            shifted | u64::MAX.wrapping_shl(64_u32.saturating_sub(places))
        } else {
            shifted
        }
    };
    shifted.cast_signed()
}

/// The six comparisons, and the two that treat nothing as a value.
fn comparison(op: BinaryOp, left: &Answer, right: &Answer) -> Value {
    let null_equals = matches!(op, BinaryOp::Is | BinaryOp::IsNot);
    let (mut first, mut second) = (left.value.clone(), right.value.clone());
    let missing = first == Value::Null || second == Value::Null;
    if missing && !null_equals {
        return Value::Null;
    }
    let order = if missing {
        (first == Value::Null) == (second == Value::Null)
    } else {
        let affinity = compare_affinity(left.affinity, right.affinity);
        apply_comparison(&mut first, &mut second, affinity);
        let collation = left.collation.or(right.collation).unwrap_or_default();
        let order = compare(&first, &second, collation);
        return Value::Int(i64::from(holds(op, order)));
    };
    Value::Int(i64::from(order == (op == BinaryOp::Is)))
}

/// Whether an ordering answers the operator.
fn holds(op: BinaryOp, order: core::cmp::Ordering) -> bool {
    use core::cmp::Ordering;
    match op {
        BinaryOp::Eq | BinaryOp::Is => order == Ordering::Equal,
        BinaryOp::Ne | BinaryOp::IsNot => order != Ordering::Equal,
        BinaryOp::Lt => order == Ordering::Less,
        BinaryOp::Le => order != Ordering::Greater,
        BinaryOp::Gt => order == Ordering::Greater,
        _ => order != Ordering::Less,
    }
}

/// `x BETWEEN low AND high`, which is two comparisons over one value.
fn between(
    arena: &Arena,
    value: ExprId,
    low: ExprId,
    high: ExprId,
    negated: bool,
    sql: &[u8],
    depth: u32,
) -> Result<Answer, Error> {
    let middle = answer(arena, value, sql, depth)?;
    let low = answer(arena, low, sql, depth)?;
    let high = answer(arena, high, sql, depth)?;
    let above = logic(&comparison(BinaryOp::Ge, &middle, &low));
    let below = logic(&comparison(BinaryOp::Le, &middle, &high));
    let inside = if above == Some(false) || below == Some(false) {
        Some(false)
    } else {
        above.and(below)
    };
    let inside = match inside {
        Some(truth) => truth != negated,
        None => return Ok(Answer::plain(Value::Null)),
    };
    Ok(Answer::plain(Value::Int(i64::from(inside))))
}

/// `x IN (a, b)`. An empty list answers false whatever is looked for in
/// it, nothing included.
fn in_list(left: &Answer, list: &[Answer], negated: bool) -> Value {
    if list.is_empty() {
        return Value::Int(i64::from(negated));
    }
    let mut unknown = false;
    for member in list {
        // The affinity is the left side's alone, which is what
        // `comparisonAffinity` answers where the right side is a list.
        let against = Answer {
            value: member.value.clone(),
            affinity: left.affinity,
            collation: member.collation,
        };
        match logic(&comparison(BinaryOp::Eq, left, &against)) {
            Some(true) => return Value::Int(i64::from(!negated)),
            Some(false) => {}
            None => unknown = true,
        }
    }
    if unknown {
        Value::Null
    } else {
        Value::Int(i64::from(negated))
    }
}

/// `CASE`, in both of its shapes.
fn case(
    arena: &Arena,
    operand: Option<ExprId>,
    branches: crate::ast::Range,
    otherwise: Option<ExprId>,
    sql: &[u8],
    depth: u32,
) -> Result<Answer, Error> {
    let subject = match operand {
        Some(id) => Some(answer(arena, id, sql, depth)?),
        None => None,
    };
    let children = arena.children(branches);
    let mut at = 0;
    while let (Some(when), Some(then)) = (children.get(at), children.get(at.saturating_add(1))) {
        let condition = answer(arena, *when, sql, depth)?;
        let taken = match &subject {
            Some(subject) => logic(&comparison(BinaryOp::Eq, subject, &condition)),
            None => logic(&condition.value),
        };
        if taken == Some(true) {
            return answer(arena, *then, sql, depth);
        }
        at = at.saturating_add(2);
    }
    match otherwise {
        Some(id) => answer(arena, id, sql, depth),
        None => Ok(Answer::plain(Value::Null)),
    }
}
