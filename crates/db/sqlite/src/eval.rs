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

use crate::ast::{Arena, BinaryOp, ExprId, LikeOp, Literal, Node, SelectId, Span, UnaryOp};
use crate::func::{self, Function};
use crate::header::Encoding;
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
    /// A collation the connection does not hold, which is what a
    /// `COLLATE` naming one the C library would have been given
    /// through its API names.
    NoCollation,
    /// A shape of expression this engine does not answer yet.
    Unsupported,
    /// The tree names a node the arena does not hold.
    Malformed,
    /// The tree is deeper than the engine walks.
    TooDeep,
    /// A hex literal of more than sixteen digits, which SQLite refuses
    /// rather than reading as a real.
    HexTooBig,
    /// A function called with a number of arguments it does not take.
    WrongArguments,
    /// A number that is not one: `abs` of the smallest integer, which
    /// has no positive, and a `sum` the integers stopped holding.
    Overflow,
    /// An `ESCAPE` that is not one character.
    BadEscape,
    /// A `\` in the argument of `unistr` that no run of hex digits
    /// follows.
    BadUnicode,
    /// The second argument of `likelihood` is not a fraction written as
    /// a literal.
    BadProbability,
    /// A `LIKE` or `GLOB` pattern longer than the engine takes.
    PatternTooBig,
    /// A blob or a string longer than `SQLITE_MAX_LENGTH`, which is what
    /// `zeroblob` of a large number asks for.
    TooBig,
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
    /// Whether a `COLLATE` wrote that collation rather than a column
    /// carrying it, which is `EP_Collate`: a comparison takes a written
    /// collation from either side before it takes a column's from
    /// either side.
    written: bool,
}

/// The collation a comparison between two answers uses, which is
/// `sqlite3BinaryCompareCollSeq`: a `COLLATE` on the left, else one on
/// the right, else the collation the left carries, else the right's.
const fn compared_under(left: &Answer, right: &Answer) -> Option<Collation> {
    if left.written {
        return left.collation;
    }
    if right.written {
        return right.collation;
    }
    match left.collation {
        Some(collation) => Some(collation),
        None => right.collation,
    }
}

impl Answer {
    /// A value no affinity and no collation belong to, which is every
    /// expression that is not a column, a cast or a `COLLATE`.
    const fn plain(value: Value) -> Self {
        Answer {
            value,
            affinity: Affinity::None,
            collation: None,
            written: false,
        }
    }
}

/// A statement an expression uses, which the engine that walks the
/// rows answers and this module only names.
#[derive(Clone, Copy, Debug)]
pub enum Used {
    /// `(SELECT ...)` as a value.
    Value(SelectId),
    /// `EXISTS (SELECT ...)`.
    Exists(SelectId),
    /// `x IN (SELECT ...)`: what is tested, where it is looked for, and
    /// whether `NOT` precedes it.
    In(ExprId, SelectId, bool),
    /// `x IN table`: what is tested, the schema where one was named,
    /// the table, and whether `NOT` precedes it.
    InTable(ExprId, Option<Span>, Span, bool),
}

/// Where the value of a column comes from.
pub trait Row {
    /// The column `column` of the table `table` of the schema `schema`,
    /// with the affinity and the collation it was declared with, or
    /// nothing where this row has no such column.
    fn column(
        &self,
        schema: Option<&[u8]>,
        table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)>;

    /// The collation a comparison uses where nothing writes one, which
    /// is `BINARY` over whatever encoding the database keeps its text
    /// in.
    fn collation(&self) -> Collation {
        Collation::Binary
    }

    /// The encoding the database keeps its text in, which three things
    /// answer differently under: `hex`, `octet_length` and a cast to a
    /// blob, each of which shows the bytes as they are stored.
    fn encoding(&self) -> Encoding {
        Encoding::Utf8
    }

    /// What the aggregate call `id` answered for the group this row
    /// stands for, or nothing where the call is not an aggregate.
    ///
    /// An aggregate is answered by the call it was written as and not by
    /// its name, so that two `count(*)` in one statement are one column
    /// each and this module needs to know nothing about grouping.
    fn aggregate(&self, _id: ExprId) -> Option<Value> {
        None
    }

    /// What the statement written at `id` answered for this row, or
    /// nothing where this row answers no statement.
    ///
    /// What the statement `used` answers for this row, or nothing where
    /// this row answers no statement.
    ///
    /// A statement used as a value is answered by whatever walks the
    /// rows, because this module reads one row and knows no tables.
    fn answered(&self, _used: Used) -> Option<Value> {
        None
    }
}

/// A row with no columns, which is what a constant expression is read
/// against.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRow;

impl Row for NoRow {
    fn column(
        &self,
        _schema: Option<&[u8]>,
        _table: Option<&[u8]>,
        _column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        None
    }
}

/// What the expression `id` of `arena` answers, where `sql` is the
/// statement its spans point into.
///
/// # Errors
///
/// [`Error`] names what it could not answer and why.
pub fn evaluate(arena: &Arena, id: ExprId, sql: &[u8]) -> Result<Value, Error> {
    evaluate_row(arena, id, sql, &NoRow)
}

/// The same, against a row whose columns the expression may name.
///
/// # Errors
///
/// [`Error`] names what it could not answer and why.
pub fn evaluate_row(arena: &Arena, id: ExprId, sql: &[u8], row: &dyn Row) -> Result<Value, Error> {
    Ok(answer(arena, id, sql, row, 0)?.value)
}

/// The same, with the collation a comparison against the answer would
/// use.
///
/// # Errors
///
/// [`Error`] names what it could not answer and why.
pub fn evaluate_collated(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    row: &dyn Row,
) -> Result<(Value, Collation), Error> {
    let (value, _, written) = evaluate_compared(arena, id, sql, row)?;
    Ok((value, written.unwrap_or(row.collation())))
}

/// The same, with the affinity a comparison converts under and the
/// collation written on the expression, where one is.
///
/// # Errors
///
/// [`Error`] names what it could not answer and why.
pub fn evaluate_compared(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    row: &dyn Row,
) -> Result<(Value, Affinity, Option<Collation>), Error> {
    let answered = answer(arena, id, sql, row, 0)?;
    Ok((answered.value, answered.affinity, answered.collation))
}

/// One node.
fn answer(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    row: &dyn Row,
    depth: u32,
) -> Result<Answer, Error> {
    if depth > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    let node = arena.node(id).ok_or(Error::Malformed)?;
    let deeper = depth.saturating_add(1);
    match node {
        Node::Literal(literal) => literal_value(literal, sql, false).map(Answer::plain),
        Node::Unary { op, operand } => unary(arena, op, operand, sql, row, deeper),
        Node::Binary { op, left, right } => binary(arena, op, left, right, sql, row, deeper),
        Node::Between {
            value,
            low,
            high,
            negated,
        } => between(arena, value, low, high, negated, sql, row, deeper),
        Node::InList {
            value,
            list,
            negated,
        } => listed(arena, value, list, negated, sql, row, deeper),
        Node::Cast { value, ty } => {
            let mut inner = answer(arena, value, sql, row, deeper)?;
            let affinity = Affinity::of_type(&crate::schema::dequote(ty.text(sql)));
            cast(&mut inner.value, affinity, row.encoding());
            inner.affinity = affinity;
            Ok(inner)
        }
        Node::Collate { value, name } => {
            let mut inner = answer(arena, value, sql, row, deeper)?;
            // A name no collation of this crate answers is refused, as
            // `sqlite3GetCollSeq` refuses one the connection was never
            // given.
            let named = crate::schema::dequote(name.text(sql));
            inner.collation = Some(Collation::of_name(&named).ok_or(Error::NoCollation)?);
            inner.written = true;
            Ok(inner)
        }
        Node::Case {
            operand,
            branches,
            otherwise,
        } => {
            let mut answered = case(arena, operand, branches, otherwise, sql, row, deeper)?;
            // `sqlite3ExprCollSeq` reads the collation of a `CASE` off
            // the tree and not off the branch a row takes, so a
            // `COLLATE` written in a branch no row takes is still the
            // collation the `CASE` compares with.
            answered.collation = written_collation(arena, id, sql);
            Ok(answered)
        }
        Node::Column {
            schema,
            table,
            column,
        } => {
            // A name is matched with its quotes off, which is
            // `sqlite3Dequote` before `lookupName`.
            let text = |span: crate::ast::Span| crate::schema::dequote(span.text(sql));
            let named = column.text(sql);
            let found = row.column(
                schema.map(text).as_deref(),
                table.map(text).as_deref(),
                &crate::schema::dequote(named),
            );
            let Some((value, affinity, collation)) = found else {
                // `sqlite3ExprIdToTrueFalse`: a name no table answers
                // to, written without quotes and without a table in
                // front of it, is the number one where it is `true` and
                // nought where it is `false`.
                let truth = truth_of(named)
                    .filter(|_| table.is_none())
                    .ok_or(Error::NoColumn)?;
                return Ok(Answer::plain(Value::Int(i64::from(truth))));
            };
            Ok(Answer {
                value,
                affinity,
                collation: Some(collation),
                written: false,
            })
        }
        Node::Call {
            name,
            args,
            distinct,
            star,
        } => match row.aggregate(id) {
            Some(value) => Ok(Answer::plain(value)),
            None => called(arena, name, args, distinct, star, sql, row, deeper),
        },
        Node::Like {
            op,
            value,
            pattern,
            escape,
            negated,
        } => like(arena, op, value, pattern, escape, negated, sql, row, deeper),
        Node::Subquery(select) => used(row, Used::Value(select)),
        Node::Exists(select) => used(row, Used::Exists(select)),
        Node::InSelect {
            value,
            select,
            negated,
        } => used(row, Used::In(value, select, negated)),
        Node::InTable {
            value,
            schema,
            table,
            negated,
        } => used(row, Used::InTable(value, schema, table, negated)),
        Node::Variable(_) | Node::Row(_) => Err(Error::Unsupported),
    }
}

/// What a statement an expression uses answers, which is a refusal
/// where the row answers no statement.
fn used(row: &dyn Row, what: Used) -> Result<Answer, Error> {
    row.answered(what)
        .map_or(Err(Error::Unsupported), |value| Ok(Answer::plain(value)))
}

/// `name(args)`, which is a function where this engine has one.
#[expect(
    clippy::too_many_arguments,
    reason = "the node's own fields, and the walk the tree is read with"
)]
fn called(
    arena: &Arena,
    name: crate::ast::Span,
    args: crate::ast::Range,
    distinct: bool,
    star: bool,
    sql: &[u8],
    row: &dyn Row,
    deeper: u32,
) -> Result<Answer, Error> {
    if distinct || star {
        // Both belong to an aggregate, which is a later step.
        return Err(Error::Unsupported);
    }
    let mut values = Vec::new();
    // The collation the call compares under is the first argument that
    // carries one, which is what `sqlite3ExprCodeTarget` gives a
    // function that needs one. The collation the call answers with is
    // the first argument a `COLLATE` was written on, because
    // `sqlite3ExprCollSeq` reaches an argument only along a path a
    // `COLLATE` marked.
    let mut inside = None;
    let mut outward = None;
    for id in arena.children(args) {
        let argument = answer(arena, *id, sql, row, deeper)?;
        if inside.is_none() {
            inside = argument.collation;
        }
        if outward.is_none() && argument.written {
            outward = argument.collation;
        }
        values.push(argument.value);
    }
    let function = func::lookup(&crate::schema::dequote(name.text(sql)), values.len())?;
    if function == Function::Unlikely && values.len() == 2 {
        // `likelihood(X,Y)` tells the planner how often X holds,
        // so Y has to be a fraction and has to be written out.
        let second = arena
            .children(args)
            .get(1)
            .copied()
            .ok_or(Error::Malformed)?;
        if !probability(arena, second, sql) {
            return Err(Error::BadProbability);
        }
    }
    let value = func::call(
        function,
        &values,
        inside.unwrap_or(row.collation()),
        row.encoding(),
    )?;
    Ok(Answer {
        value,
        affinity: Affinity::None,
        collation: outward,
        written: outward.is_some(),
    })
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

/// Whether the node is a fraction between zero and one written as a
/// literal, which is what `exprProbability` asks of it.
fn probability(arena: &Arena, id: ExprId, sql: &[u8]) -> bool {
    let Some(Node::Literal(Literal::Float(span))) = arena.node(id) else {
        return false;
    };
    number::real(&dequote(span.text(sql))).value <= 1.0
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
    row: &dyn Row,
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
    let inner = answer(arena, operand, sql, row, depth)?;
    let value = inner.value;
    // A `COLLATE` written under the operator is the collation it
    // answers with, as it is under a binary operator.
    let carried = |value: Value| Answer {
        value,
        affinity: Affinity::None,
        collation: inner.collation,
        written: inner.written,
    };
    Ok(match op {
        // A sign before anything else is a subtraction from zero.
        UnaryOp::Negate => carried(arithmetic(BinaryOp::Subtract, &Value::Int(0), &value)),
        // `sqlite3ExprAffinity`: a sign written before an expression
        // has no affinity of its own and does not carry the affinity of
        // what it precedes, so `xt == +xi` compares a text column with
        // a number that takes text affinity from it rather than the
        // other way about. The collation does carry.
        UnaryOp::Identity => carried(value),
        UnaryOp::Not => carried(match logic(&value) {
            Some(truth) => Value::Int(i64::from(!truth)),
            None => Value::Null,
        }),
        UnaryOp::BitNot => carried(if value == Value::Null {
            Value::Null
        } else {
            Value::Int(!value.to_integer())
        }),
        UnaryOp::IsNull => carried(Value::Int(i64::from(value == Value::Null))),
        UnaryOp::NotNull => carried(Value::Int(i64::from(value != Value::Null))),
    })
}

/// `X IN (a, b, ...)`, and `X NOT IN` where `negated`.
fn listed(
    arena: &Arena,
    value: ExprId,
    list: crate::ast::Range,
    negated: bool,
    sql: &[u8],
    row: &dyn Row,
    deeper: u32,
) -> Result<Answer, Error> {
    let left = answer(arena, value, sql, row, deeper)?;
    let mut members = Vec::new();
    for member in arena.children(list) {
        members.push(answer(arena, *member, sql, row, deeper)?);
    }
    Ok(Answer::plain(in_list(
        &left,
        &members,
        negated,
        row.collation(),
    )))
}

/// The collation written under `id`, which is the `COLLATE` on the
/// expression itself or the first one written under it, left to right.
///
/// This reads the tree rather than the answer, because an expression
/// carries the collation of a branch no row takes. A column under it
/// carries none, because the walk of `sqlite3ExprCollSeq` reaches a
/// column only along a path a `COLLATE` marked.
///
/// The walk is O(n) in the nodes under `id`.
fn written_collation(arena: &Arena, id: ExprId, sql: &[u8]) -> Option<Collation> {
    let node = arena.node(id)?;
    if let Node::Collate { name, .. } = node {
        return Collation::of_name(&crate::schema::dequote(name.text(sql)));
    }
    // A `CASE` is read in the order it was written, because the first
    // `COLLATE` written under it is the one it answers with, and
    // `Arena::under` names the `ELSE` before the branches.
    let mut found = None;
    let mut first = |child: ExprId| {
        if found.is_none() {
            found = written_collation(arena, child, sql);
        }
    };
    if let Node::Case {
        operand,
        branches,
        otherwise,
    } = node
    {
        for child in operand
            .into_iter()
            .chain(arena.children(branches).iter().copied())
            .chain(otherwise)
        {
            first(child);
        }
        return found;
    }
    arena.under(node, first);
    found
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
    row: &dyn Row,
    depth: u32,
) -> Result<Answer, Error> {
    let left = answer(arena, left, sql, row, depth)?;
    let right = answer(arena, right, sql, row, depth)?;
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
        | BinaryOp::IsNot => comparison(op, &left, &right, row.collation()),
        BinaryOp::Extract | BinaryOp::ExtractText => return Err(Error::NoFunction),
    };
    // `sqlite3ExprCollSeq`: a `COLLATE` written under an operator is
    // the collation the operator answers with, the left operand before
    // the right, so `'ABC' || ('' COLLATE nocase)` compares without
    // case.
    Ok(Answer {
        value,
        affinity: Affinity::None,
        collation: compared_under(&left, &right),
        written: left.written || right.written,
    })
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
fn comparison(op: BinaryOp, left: &Answer, right: &Answer, default: Collation) -> Value {
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
        let collation = compared_under(left, right).unwrap_or(default);
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

/// `x LIKE y ESCAPE z`, and the three operators written like it.
///
/// The grammar has four; SQLite has a function for two of them and
/// nothing for `REGEXP` and `MATCH`, which is why those refuse.
#[expect(
    clippy::too_many_arguments,
    reason = "the node's own fields, which are what the operator is written with"
)]
fn like(
    arena: &Arena,
    op: LikeOp,
    value: ExprId,
    pattern: ExprId,
    escape: Option<ExprId>,
    negated: bool,
    sql: &[u8],
    row: &dyn Row,
    depth: u32,
) -> Result<Answer, Error> {
    let function = match op {
        LikeOp::Like => Function::Like,
        LikeOp::Glob => Function::Glob,
        LikeOp::Regexp | LikeOp::Match => return Err(Error::NoFunction),
    };
    // The pattern is the first argument and the value the second, which
    // is how `A LIKE B` is written as `like(B,A)`.
    let mut args = alloc::vec![
        answer(arena, pattern, sql, row, depth)?.value,
        answer(arena, value, sql, row, depth)?.value,
    ];
    if let Some(escape) = escape {
        args.push(answer(arena, escape, sql, row, depth)?.value);
    }
    let answered = func::call(function, &args, row.collation(), row.encoding())?;
    Ok(Answer::plain(match (negated, logic(&answered)) {
        (_, None) => Value::Null,
        (true, Some(truth)) => Value::Int(i64::from(!truth)),
        (false, Some(truth)) => Value::Int(i64::from(truth)),
    }))
}

/// `x BETWEEN low AND high`, which is two comparisons over one value.
#[expect(
    clippy::too_many_arguments,
    reason = "the node's own fields, and the walk the tree is read with"
)]
fn between(
    arena: &Arena,
    value: ExprId,
    low: ExprId,
    high: ExprId,
    negated: bool,
    sql: &[u8],
    row: &dyn Row,
    depth: u32,
) -> Result<Answer, Error> {
    let middle = answer(arena, value, sql, row, depth)?;
    let low = answer(arena, low, sql, row, depth)?;
    let high = answer(arena, high, sql, row, depth)?;
    let above = logic(&comparison(BinaryOp::Ge, &middle, &low, row.collation()));
    let below = logic(&comparison(BinaryOp::Le, &middle, &high, row.collation()));
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
fn in_list(left: &Answer, list: &[Answer], negated: bool, default: Collation) -> Value {
    if list.is_empty() {
        return Value::Int(i64::from(negated));
    }
    let mut unknown = false;
    for member in list {
        // The affinity is the left side's alone, which is what
        // `comparisonAffinity` answers where the right side is a list:
        // the member carries none, so the two together are the left
        // side's.
        let against = Answer {
            value: member.value.clone(),
            affinity: Affinity::None,
            collation: member.collation,
            written: member.written,
        };
        match logic(&comparison(BinaryOp::Eq, left, &against, default)) {
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

/// Whether a name written without quotes is one of the two SQLite
/// reads as a number rather than as a column.
const fn truth_of(text: &[u8]) -> Option<bool> {
    if text.eq_ignore_ascii_case(b"true") {
        return Some(true);
    }
    if text.eq_ignore_ascii_case(b"false") {
        return Some(false);
    }
    None
}

/// `CASE`, in both of its shapes.
fn case(
    arena: &Arena,
    operand: Option<ExprId>,
    branches: crate::ast::Range,
    otherwise: Option<ExprId>,
    sql: &[u8],
    row: &dyn Row,
    depth: u32,
) -> Result<Answer, Error> {
    let subject = match operand {
        Some(id) => Some(answer(arena, id, sql, row, depth)?),
        None => None,
    };
    let children = arena.children(branches);
    let mut at = 0;
    while let (Some(when), Some(then)) = (children.get(at), children.get(at.saturating_add(1))) {
        let condition = answer(arena, *when, sql, row, depth)?;
        let taken = match &subject {
            Some(subject) => logic(&comparison(
                BinaryOp::Eq,
                subject,
                &condition,
                row.collation(),
            )),
            None => logic(&condition.value),
        };
        if taken == Some(true) {
            return answer(arena, *then, sql, row, depth);
        }
        at = at.saturating_add(2);
    }
    match otherwise {
        Some(id) => answer(arena, id, sql, row, depth),
        None => Ok(Answer::plain(Value::Null)),
    }
}
