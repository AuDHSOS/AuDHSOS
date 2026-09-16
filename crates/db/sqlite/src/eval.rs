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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// A name that is not a column of this row, as it was written.
    NoColumn(Vec<u8>),
    /// A name more than one side of the `FROM` answers to, as it was
    /// written.
    Ambiguous(Vec<u8>),
    /// A function this engine does not have, by the name it was
    /// called under.
    NoFunction(Vec<u8>),
    /// A window function written where no window was worked out, with
    /// the name it was called under.
    NoWindow(Vec<u8>),
    /// An aggregate written where no group has been made, with the name
    /// it was called under.
    MisusedAggregate(Vec<u8>),
    /// A scalar function carrying a `FILTER`, which only an aggregate
    /// reads, with the name it was called under.
    Filtered(Vec<u8>),
    /// A collation the connection does not hold, which is what a
    /// `COLLATE` naming one the C library would have been given
    /// through its API names.
    NoCollation(Vec<u8>),
    /// `random` or `randomblob` where the caller gave the connection no
    /// source of bytes to answer them from.
    NoRandom,
    /// A `RAISE` the statement reached, which says what the statement
    /// that reached it does.
    Raised(crate::ast::Raise),
    /// A statement used as a value that answers a number of columns
    /// the place it stands in does not take, which
    /// `sqlite3SubselectError` refuses as `sub-select returns N
    /// columns - expected M`.
    Columns,
    /// A row of values written where one value belongs, which
    /// `sqlite3VectorErrorMsg` refuses as `row value misused`, and a
    /// row compared against a row of another width.
    RowValue,
    /// A shape of expression this engine does not answer yet.
    Unsupported,
    /// The tree names a node the arena does not hold.
    Malformed,
    /// The tree is deeper than the engine walks.
    TooDeep,
    /// A hex literal of more than sixteen digits, which SQLite refuses
    /// rather than reading as a real.
    HexTooBig,
    /// The fraction argument of a percentile aggregate that is not a
    /// number between nought and the largest the aggregate takes, with
    /// the name and that largest as it is written.
    Fraction(Vec<u8>, Vec<u8>),
    /// The fraction argument of a percentile aggregate written
    /// differently for two rows of one group, with the name.
    Fractions(Vec<u8>),
    /// A value a percentile aggregate was given that is neither nothing
    /// nor a number, with the name.
    NotNumeric(Vec<u8>),
    /// An infinity a percentile aggregate was given, with the name.
    Infinite(Vec<u8>),
    /// A function called with a number of arguments it does not take,
    /// by the name it was called under.
    WrongArguments(Vec<u8>),
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
    /// What a JSON function refuses, which carries its own message.
    Json(crate::json::Refused),
}

impl Error {
    /// The text the C library writes for this refusal, which is what a
    /// `catchsql` of SQLite's own test files compares.
    #[must_use]
    pub fn message(&self) -> alloc::string::String {
        use alloc::string::ToString as _;
        let shown = |name: &[u8]| alloc::string::String::from_utf8_lossy(name).into_owned();
        match self {
            Error::NoColumn(name) => alloc::format!("no such column: {}", shown(name)),
            Error::Ambiguous(name) => alloc::format!("ambiguous column name: {}", shown(name)),
            Error::NoFunction(name) => alloc::format!("no such function: {}", shown(name)),
            Error::NoCollation(name) => {
                alloc::format!("no such collation sequence: {}", shown(name))
            }
            Error::WrongArguments(name) => {
                alloc::format!("wrong number of arguments to function {}()", shown(name))
            }
            Error::Fraction(name, largest) => alloc::format!(
                "the fraction argument to {}() is not between 0.0 and {}",
                shown(name),
                shown(largest)
            ),
            Error::Fractions(name) => alloc::format!(
                "the fraction argument to {}() is not the same for all input rows",
                shown(name)
            ),
            Error::NotNumeric(name) => {
                alloc::format!("input to {}() is not numeric", shown(name))
            }
            Error::Infinite(name) => alloc::format!("Inf input to {}()", shown(name)),
            Error::NoWindow(name) => {
                alloc::format!("misuse of window function {}()", shown(name))
            }
            Error::MisusedAggregate(name) => {
                alloc::format!("misuse of aggregate function {}()", shown(name))
            }
            Error::Filtered(_) => {
                "FILTER clause may only be used with aggregate window functions".to_string()
            }
            Error::RowValue => "row value misused".to_string(),
            Error::Json(refused) => refused.message(),
            other => alloc::format!("{other:?}"),
        }
    }
}

/// A value, with what a comparison against it would do.
#[derive(Clone, Debug, PartialEq)]
struct Answer {
    /// The value.
    value: Value,
    /// Whether the value is JSON of its own, which is what
    /// `JSON_SUBTYPE` marks a value with: a JSON function reads such a
    /// value as JSON and every other text as the text it is.
    json: bool,
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
            json: false,
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

    /// Whether more than one side of this row answers to the name,
    /// which `lookupName` refuses rather than choosing between. The
    /// walk asks this only where `column` answered nothing.
    fn ambiguous(&self, _schema: Option<&[u8]>, _table: Option<&[u8]>, _column: &[u8]) -> bool {
        false
    }

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

    /// Where `random` and `randomblob` take their bytes from, or
    /// nothing where the caller gave this row no source of them.
    fn random(&self) -> Option<&crate::random::Source> {
        None
    }

    /// What the connection has written, which `changes()`,
    /// `total_changes()` and `last_insert_rowid()` answer.
    fn counted(&self) -> crate::func::Counted {
        crate::func::Counted::default()
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

    /// The first row the statement `select` answers, each value with
    /// the affinity and the collation a comparison against it uses,
    /// which is what a row compared against `(SELECT a, b)` compares
    /// against. A statement that answers no row answers a null per
    /// column.
    fn answered_items(
        &self,
        _select: SelectId,
    ) -> Option<Vec<(Value, Affinity, Option<Collation>)>> {
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

/// The same, with whether the answer is JSON of its own, which a JSON
/// function reads as JSON and every other text as the text it is.
///
/// # Errors
///
/// [`Error`] names what it could not answer and why.
pub fn evaluate_carried(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    row: &dyn Row,
) -> Result<(Value, bool), Error> {
    let answered = answer(arena, id, sql, row, 0)?;
    Ok((answered.value, answered.json))
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

/// The values of the items of a row, each with the affinity and the
/// collation a comparison against it uses.
///
/// # Errors
///
/// [`Error::RowValue`] where the node is no row, and whatever an item
/// refuses with.
pub fn evaluate_items(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    row: &dyn Row,
) -> Result<Vec<(Value, Affinity, Option<Collation>)>, Error> {
    let answers = row_answers(arena, id, sql, row, 0)?;
    Ok(answers
        .into_iter()
        .map(|answered| (answered.value, answered.affinity, answered.collation))
        .collect())
}

/// Whether an expression is a row of values, which decides how many
/// columns a statement compared against it answers.
#[must_use]
pub fn is_row_value(arena: &Arena, id: ExprId) -> bool {
    is_row(arena, id)
}

/// One node.
/// `CAST(x AS type)`.
fn converted(
    arena: &Arena,
    value: ExprId,
    ty: crate::ast::Span,
    sql: &[u8],
    row: &dyn Row,
    deeper: u32,
) -> Result<Answer, Error> {
    let mut inner = answer(arena, value, sql, row, deeper)?;
    let affinity = Affinity::of_type(&crate::schema::dequote(ty.text(sql)));
    cast(&mut inner.value, affinity, row.encoding());
    inner.affinity = affinity;
    Ok(inner)
}

/// `x COLLATE name`, which refuses a name no collation of this crate
/// answers as `sqlite3GetCollSeq` refuses one the connection was never
/// given.
fn collated(
    arena: &Arena,
    value: ExprId,
    name: crate::ast::Span,
    sql: &[u8],
    row: &dyn Row,
    deeper: u32,
) -> Result<Answer, Error> {
    let mut inner = answer(arena, value, sql, row, deeper)?;
    let named = crate::schema::dequote(name.text(sql));
    inner.collation =
        Some(Collation::of_name(&named).ok_or_else(|| Error::NoCollation(named.clone()))?);
    inner.written = true;
    Ok(inner)
}

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
        Node::Cast { value, ty } => converted(arena, value, ty, sql, row, deeper),
        Node::Collate { value, name } => collated(arena, value, name, sql, row, deeper),
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
                if row.ambiguous(
                    schema.map(text).as_deref(),
                    table.map(text).as_deref(),
                    &crate::schema::dequote(named),
                ) {
                    return Err(Error::Ambiguous(written(schema, table, named, sql)));
                }
                // `sqlite3ExprIdToTrueFalse`: a name no table answers
                // to, written without quotes and without a table in
                // front of it, is the number one where it is `true` and
                // nought where it is `false`.
                let truth = truth_of(named)
                    .filter(|_| table.is_none())
                    .ok_or_else(|| Error::NoColumn(missed(schema, table, named, sql)))?;
                return Ok(Answer::plain(Value::Int(i64::from(truth))));
            };
            Ok(Answer {
                value,
                json: false,
                affinity,
                collation: Some(collation),
                written: false,
            })
        }
        // An aggregate and a window function are both answered once
        // per group or per row before the row is read, so the walk
        // looks the answer up rather than working it out; a window
        // function no window was worked out for is a misuse of one,
        // and so is a scalar function carrying a `FILTER`, which only
        // an aggregate reads.
        Node::Call {
            name, args, filter, ..
        } => match row.aggregate(id) {
            Some(value) => Ok(Answer::plain(value)),
            None if filter.is_some() => Err(Error::Filtered(named_as(name, sql))),
            None => called(arena, name, args, sql, row, deeper),
        },
        Node::Over { name, .. } => row
            .aggregate(id)
            .map(Answer::plain)
            .ok_or_else(|| Error::NoWindow(named_as(name, sql))),
        Node::Like {
            op,
            value,
            pattern,
            escape,
            negated,
        } => like(arena, op, value, pattern, escape, negated, sql, row, deeper),
        Node::Subquery(select) => alone(row, select),
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
        // `RAISE` answers no value: it says what the statement that
        // reached it does.
        Node::Raise { action, .. } => Err(Error::Raised(action)),
        Node::Row(_) => Err(Error::RowValue),
        Node::Variable(_) => Err(Error::Unsupported),
    }
}

/// `(SELECT a)` as one value, with the affinity and the collation of
/// the column the statement answers, which is `sqlite3ExprAffinity` of
/// a `TK_SELECT`. A statement that answers any other number of columns
/// is a refusal.
fn alone(row: &dyn Row, select: SelectId) -> Result<Answer, Error> {
    let mut held = row
        .answered_items(select)
        .ok_or(Error::Unsupported)?
        .into_iter();
    let (value, affinity, collation) = held.next().ok_or(Error::Columns)?;
    if held.next().is_some() {
        return Err(Error::Columns);
    }
    Ok(Answer {
        value,
        json: false,
        affinity,
        collation,
        written: false,
    })
}

/// What a statement an expression uses answers, which is a refusal
/// where the row answers no statement.
fn used(row: &dyn Row, what: Used) -> Result<Answer, Error> {
    row.answered(what)
        .map_or(Err(Error::Unsupported), |value| Ok(Answer::plain(value)))
}

/// `name(args)`, which is a function where this engine has one.
///
/// `DISTINCT` before the arguments of a scalar function is read and
/// dropped, which is what `sqlite3FindFunction` does with it, and a `*`
/// for the arguments leaves the call with none, so a scalar that takes
/// one or more is refused for the number it was called with.
fn called(
    arena: &Arena,
    name: crate::ast::Span,
    args: crate::ast::Range,
    sql: &[u8],
    row: &dyn Row,
    deeper: u32,
) -> Result<Answer, Error> {
    let mut values = Vec::new();
    // The collation the call compares under is the first argument that
    // carries one, which is what `sqlite3ExprCodeTarget` gives a
    // function that needs one. The collation the call answers with is
    // the first argument a `COLLATE` was written on, because
    // `sqlite3ExprCollSeq` reaches an argument only along a path a
    // `COLLATE` marked.
    let mut inside = None;
    let mut outward = None;
    // Whether each argument carries JSON of its own, which is the
    // subtype `JSON_SUBTYPE` marks a value with.
    let mut carried = Vec::new();
    for id in arena.children(args) {
        let argument = answer(arena, *id, sql, row, deeper)?;
        if inside.is_none() {
            inside = argument.collation;
        }
        if outward.is_none() && argument.written {
            outward = argument.collation;
        }
        carried.push(argument.json);
        values.push(argument.value);
    }
    let called = crate::schema::dequote(name.text(sql));
    // One of the eleven window functions written under no `OVER`
    // reaches the scalars, which hold none of that name, and so does an
    // aggregate written where no group has been made. `resolveExprStep`
    // answers for the number of arguments before it answers for the
    // misuse, so a name called with a number it does not take is
    // refused for the number.
    let function = match func::lookup(&called, values.len()) {
        Err(Error::NoFunction(_)) if crate::window::named(&called) => {
            if crate::window::lookup(&called, values.len()).is_none() {
                return Err(Error::WrongArguments(called));
            }
            return Err(Error::NoWindow(called));
        }
        Err(Error::NoFunction(_)) if crate::agg::named(&called) => {
            if crate::agg::lookup(&called, values.len()).is_none() {
                return Err(Error::WrongArguments(called));
            }
            return Err(Error::MisusedAggregate(called));
        }
        held => held?,
    };
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
    let (value, json) = func::call(
        function,
        &values,
        &carried,
        inside.unwrap_or(row.collation()),
        row.encoding(),
        row.random(),
        row.counted(),
    )?;
    Ok(Answer {
        value,
        json,
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

/// A name as it was written, with its quotes taken off.
fn named_as(name: crate::ast::Span, sql: &[u8]) -> Vec<u8> {
    crate::schema::dequote(name.text(sql))
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
        json: false,
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
    if is_row(arena, value) {
        return rows_listed(arena, value, list, negated, sql, row, deeper).map(Answer::plain);
    }
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

/// `(a, b) IN ((1, 2), (3, 4))`: a row looked for among rows, every one
/// of which is a row of the same width.
fn rows_listed(
    arena: &Arena,
    value: ExprId,
    list: crate::ast::Range,
    negated: bool,
    sql: &[u8],
    row: &dyn Row,
    depth: u32,
) -> Result<Value, Error> {
    let left = row_answers(arena, value, sql, row, depth)?;
    let mut unknown = false;
    for member in arena.children(list) {
        let held = row_answers(arena, *member, sql, row, depth)?;
        if held.len() != left.len() {
            return Err(Error::RowValue);
        }
        match logic(&pairs_compared(BinaryOp::Eq, &left, &held, row.collation())) {
            Some(true) => return Ok(Value::Int(i64::from(!negated))),
            Some(false) => {}
            None => unknown = true,
        }
    }
    if unknown {
        return Ok(Value::Null);
    }
    Ok(Value::Int(i64::from(negated)))
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
    if is_row(arena, left) || is_row(arena, right) {
        return rows_compared(arena, op, left, right, sql, row, depth).map(Answer::plain);
    }
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
        BinaryOp::Extract | BinaryOp::ExtractText => {
            let text = op == BinaryOp::ExtractText;
            let (value, json) = crate::json::arrow(&left.value, &right.value, text)?;
            return Ok(Answer {
                value,
                json,
                affinity: Affinity::None,
                collation: None,
                written: false,
            });
        }
    };
    // `sqlite3ExprCollSeq`: a `COLLATE` written under an operator is
    // the collation the operator answers with, the left operand before
    // the right, so `'ABC' || ('' COLLATE nocase)` compares without
    // case.
    Ok(Answer {
        value,
        json: false,
        affinity: Affinity::None,
        collation: compared_under(&left, &right),
        written: left.written || right.written,
    })
}

/// `+`, `-`, `*`, `/` and `%`, which count in integers where both sides
/// are integers and in doubles where either is not.
#[must_use]
pub fn arithmetic(op: BinaryOp, left: &Value, right: &Value) -> Value {
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

/// How wide an expression is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Width {
    /// A row of this many values.
    Row(usize),
    /// A statement, the columns of which are counted when it runs.
    Unknown,
    /// One value, which every other expression answers.
    One,
}

impl Width {
    /// Whether the expression is a row of values.
    const fn is_row(self) -> bool {
        matches!(self, Width::Row(_))
    }
}

/// How wide the expression at `id` is.
fn width(arena: &Arena, id: ExprId) -> Width {
    match arena.node(id) {
        Some(Node::Row(items)) => Width::Row(items.len()),
        Some(Node::Subquery(_)) => Width::Unknown,
        _ => Width::One,
    }
}

/// Whether two widths may be compared against each other, which two
/// rows may where they hold as many values as each other, and a row
/// and a statement may whatever the statement answers.
const fn fit(one: Width, other: Width) -> bool {
    match (one, other) {
        (Width::Row(left), Width::Row(right)) => left == right,
        (Width::Row(_), Width::Unknown) | (Width::Unknown, Width::Row(_)) => true,
        _ => false,
    }
}

/// Whether every row of values the arena holds stands where a row may
/// stand: a comparison, a `BETWEEN` or an `IN` against a row as wide
/// as it.
///
/// `sqlite3VectorErrorMsg` refuses a row anywhere else, and refuses it
/// before the first row of a table is read, so a statement over a table
/// of no rows refuses as well. Reading the arena costs O(n) in its
/// nodes.
///
/// # Errors
///
/// [`Error::RowValue`] names the misuse.
pub fn rows_placed(arena: &Arena) -> Result<(), Error> {
    let mut allowed = alloc::vec![false; arena.len()];
    let mark = |id: ExprId, allowed: &mut Vec<bool>| {
        for slot in allowed.iter_mut().skip(id.place()).take(1) {
            *slot = true;
        }
    };
    // A row of a `VALUES` is a row of the statement and not a value
    // written under an expression, so every one of them stands where it
    // may.
    for select in arena.all_selects() {
        for held in arena.children(select.values) {
            mark(*held, &mut allowed);
        }
    }
    for (_, node) in arena.all() {
        match node {
            Node::Binary { op, left, right } => {
                let (one, other) = (width(arena, left), width(arena, right));
                if !one.is_row() && !other.is_row() {
                    continue;
                }
                if !matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                        | BinaryOp::Is
                        | BinaryOp::IsNot
                ) || !fit(one, other)
                {
                    return Err(Error::RowValue);
                }
                mark(left, &mut allowed);
                mark(right, &mut allowed);
            }
            Node::Between {
                value, low, high, ..
            } => {
                let middle = width(arena, value);
                if !middle.is_row() {
                    continue;
                }
                if !fit(middle, width(arena, low)) || !fit(middle, width(arena, high)) {
                    return Err(Error::RowValue);
                }
                mark(value, &mut allowed);
                mark(low, &mut allowed);
                mark(high, &mut allowed);
            }
            Node::InList { value, list, .. } => {
                let left = width(arena, value);
                if !left.is_row() {
                    continue;
                }
                for member in arena.children(list) {
                    if !fit(left, width(arena, *member)) {
                        return Err(Error::RowValue);
                    }
                    mark(*member, &mut allowed);
                }
                mark(value, &mut allowed);
            }
            Node::InSelect { value, .. } | Node::InTable { value, .. } => {
                mark(value, &mut allowed);
            }
            _ => {}
        }
    }
    for (id, node) in arena.all() {
        if matches!(node, Node::Row(_)) && allowed.get(id.place()) != Some(&true) {
            return Err(Error::RowValue);
        }
    }
    Ok(())
}

/// The name a statement wrote where no table answers it, which is what
/// `resolveExprStep` writes after `no such column: `: the parts with
/// their quotes taken off and a dot between them, and a name written
/// in double quotes alone with the question the C library asks.
fn missed(schema: Option<Span>, table: Option<Span>, column: &[u8], sql: &[u8]) -> Vec<u8> {
    if schema.is_some() || table.is_some() {
        return written(schema, table, column, sql);
    }
    if column.first() != Some(&b'"') {
        return column.to_vec();
    }
    let mut shown = alloc::vec![b'"'];
    shown.extend_from_slice(&crate::schema::dequote(column));
    shown.extend_from_slice(b"\" - should this be a string literal in single-quotes?");
    shown
}

/// The name as it was written, with its quotes off and the schema and
/// the table it names in front of it.
fn written(schema: Option<Span>, table: Option<Span>, column: &[u8], sql: &[u8]) -> Vec<u8> {
    let mut shown = Vec::new();
    for part in [schema, table].into_iter().flatten() {
        shown.extend_from_slice(&crate::schema::dequote(part.text(sql)));
        shown.push(b'.');
    }
    shown.extend_from_slice(&crate::schema::dequote(column));
    shown
}

/// Whether the node is a row of values, which `(a, b)` is and `(a)` is
/// not, the parser answering the expression itself for one item.
fn is_row(arena: &Arena, id: ExprId) -> bool {
    matches!(arena.node(id), Some(Node::Row(_)))
}

/// The answers of the items of a row, which a node that is no row is
/// a misuse of one.
fn row_answers(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    row: &dyn Row,
    depth: u32,
) -> Result<Vec<Answer>, Error> {
    let items = match arena.node(id) {
        Some(Node::Row(items)) => items,
        // `(SELECT a, b)` is a row of as many values as the statement
        // answers columns, which is `sqlite3ExprCodeSubselect` over a
        // vector.
        Some(Node::Subquery(select)) => {
            let held = row.answered_items(select).ok_or(Error::Unsupported)?;
            return Ok(held
                .into_iter()
                .map(|(value, affinity, collation)| Answer {
                    value,
                    json: false,
                    affinity,
                    collation,
                    written: false,
                })
                .collect());
        }
        _ => return Err(Error::RowValue),
    };
    let mut out = Vec::new();
    for item in arena.children(items) {
        out.push(answer(arena, *item, sql, row, depth)?);
    }
    Ok(out)
}

/// `(a, b) < (c, d)`: a comparison of a row against a row, which
/// `sqlite3ExprCodeVectorCompare` answers pair by pair. Every other
/// operator, and a row compared against one value, is a misuse.
fn rows_compared(
    arena: &Arena,
    op: BinaryOp,
    left: ExprId,
    right: ExprId,
    sql: &[u8],
    row: &dyn Row,
    depth: u32,
) -> Result<Value, Error> {
    if !matches!(
        op,
        BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::Is
            | BinaryOp::IsNot
    ) {
        return Err(Error::RowValue);
    }
    let left = row_answers(arena, left, sql, row, depth)?;
    let right = row_answers(arena, right, sql, row, depth)?;
    if left.len() != right.len() {
        return Err(Error::RowValue);
    }
    Ok(pairs_compared(op, &left, &right, row.collation()))
}

/// What a comparison of two rows of the same width answers.
///
/// An equality holds where every pair holds, and a null in a pair no
/// other pair settles leaves the whole unknown. An order is settled by
/// the first pair that is not equal, and by the last pair where every
/// pair before it is equal.
fn pairs_compared(op: BinaryOp, left: &[Answer], right: &[Answer], default: Collation) -> Value {
    if matches!(
        op,
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Is | BinaryOp::IsNot
    ) {
        let same = if matches!(op, BinaryOp::Is | BinaryOp::IsNot) {
            BinaryOp::Is
        } else {
            BinaryOp::Eq
        };
        let negated = matches!(op, BinaryOp::Ne | BinaryOp::IsNot);
        let mut unknown = false;
        for (one, other) in left.iter().zip(right) {
            match logic(&comparison(same, one, other, default)) {
                Some(true) => {}
                Some(false) => return Value::Int(i64::from(negated)),
                None => unknown = true,
            }
        }
        if unknown {
            return Value::Null;
        }
        return Value::Int(i64::from(!negated));
    }
    // A pair that is not equal answers the order, and `<=` answers
    // what `<` answers for such a pair, so the operator itself is what
    // each pair is read with.
    let mut answered = Value::Null;
    for (one, other) in left.iter().zip(right) {
        answered = comparison(op, one, other, default);
        if logic(&comparison(BinaryOp::Eq, one, other, default)) != Some(true) {
            break;
        }
    }
    answered
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
        LikeOp::Regexp => return Err(Error::NoFunction(b"REGEXP".to_vec())),
        LikeOp::Match => return Err(Error::NoFunction(b"MATCH".to_vec())),
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
    let (answered, _) = func::call(
        function,
        &args,
        &[],
        row.collation(),
        row.encoding(),
        row.random(),
        row.counted(),
    )?;
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
    let (above, below) = if is_row(arena, value) || is_row(arena, low) || is_row(arena, high) {
        let middle = row_answers(arena, value, sql, row, depth)?;
        let low = row_answers(arena, low, sql, row, depth)?;
        let high = row_answers(arena, high, sql, row, depth)?;
        if middle.len() != low.len() || middle.len() != high.len() {
            return Err(Error::RowValue);
        }
        (
            pairs_compared(BinaryOp::Ge, &middle, &low, row.collation()),
            pairs_compared(BinaryOp::Le, &middle, &high, row.collation()),
        )
    } else {
        let middle = answer(arena, value, sql, row, depth)?;
        let low = answer(arena, low, sql, row, depth)?;
        let high = answer(arena, high, sql, row, depth)?;
        (
            comparison(BinaryOp::Ge, &middle, &low, row.collation()),
            comparison(BinaryOp::Le, &middle, &high, row.collation()),
        )
    };
    let (above, below) = (logic(&above), logic(&below));
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
            json: false,
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
