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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// A date function that read the clock or the zone for a value of the
    /// schema, with the name it was called under and the place it stands
    /// in.
    NotPure(Vec<u8>, crate::date::Purely),
    /// A `RAISE` the statement reached, which says what the statement
    /// that reached it does, and the message it wrote as text.
    Raised(crate::ast::Raise, Vec<u8>),
    /// A statement used as a value that answers a number of columns
    /// the place it stands in does not take, which
    /// `sqlite3SubselectError` refuses as `sub-select returns N
    /// columns - expected M`, with the two counts.
    Columns(usize, usize),
    /// A row of values written where one value belongs, which
    /// `sqlite3VectorErrorMsg` refuses as `row value misused`, and a
    /// row compared against a row of another width.
    RowValue,
    /// A member of an `IN` list that holds another number of values
    /// than the row looked for, which `sqlite3ExprListIsVector` of
    /// `research/sqlite/src/expr.c` counts, with the two counts.
    InTerms(usize, usize),
    /// A shape of expression this engine does not answer yet.
    Unsupported,
    /// The tree names a node the arena does not hold.
    Malformed,
    /// The tree is deeper than the engine walks.
    TooDeep,
    /// A hex literal of more than sixteen digits, which SQLite refuses
    /// rather than reading as a real, as the statement wrote it with the
    /// sign before it.
    HexTooBig(Vec<u8>),
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
    /// An `ORDER BY` inside the brackets of a call that is no
    /// aggregate, with the name of the function.
    OrderedCall(Vec<u8>),
    /// A `LIKE` or `GLOB` pattern longer than the engine takes.
    PatternTooBig,
    /// A blob or a string longer than `SQLITE_MAX_LENGTH`, which is what
    /// `zeroblob` of a large number asks for.
    TooBig,
    /// What a JSON function refuses, which carries its own message.
    Json(crate::json::Refused),
    /// What the matcher of `crate::regexp` refuses a pattern for, which
    /// carries its own message.
    Regexp(&'static str),
    /// A function an expression of a schema object may not name, with
    /// the name as it was written.
    UnsafeFunction(Vec<u8>),
    /// What a statement used as a value was refused with, which carries
    /// that refusal: this module reads one row and the reader answers the
    /// statement, so the refusal of the reader stands.
    Refused(alloc::boxed::Box<crate::db::Error>),
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
            Error::Columns(answered, wanted) => {
                alloc::format!("sub-select returns {answered} columns - expected {wanted}")
            }
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
            Error::NotPure(name, place) => alloc::format!(
                "non-deterministic use of {}() in {}",
                shown(name),
                shown(place.words())
            ),
            Error::MisusedAggregate(name) => {
                alloc::format!("misuse of aggregate function {}()", shown(name))
            }
            Error::Filtered(_) => {
                "FILTER clause may only be used with aggregate window functions".to_string()
            }
            Error::RowValue => "row value misused".to_string(),
            Error::InTerms(held, wanted) => alloc::format!(
                "IN(...) element has {held} term{} - expected {wanted}",
                if *held == 1 { "" } else { "s" }
            ),
            Error::Raised(_, text) => shown(text),
            Error::HexTooBig(text) => {
                alloc::format!("hex literal too big: {}", shown(text))
            }
            Error::BadEscape => "ESCAPE expression must be a single character".to_string(),
            // `SQLITE_TOOBIG` of `sqlite3VdbeMemTooBig`, which the
            // library writes these words for.
            Error::TooBig => "string or blob too big".to_string(),
            Error::PatternTooBig => "LIKE or GLOB pattern too complex".to_string(),
            Error::OrderedCall(name) => alloc::format!(
                "ORDER BY may not be used with non-aggregate {}()",
                shown(name)
            ),
            Error::Overflow => "integer overflow".to_string(),
            Error::Json(refused) => refused.message(),
            Error::Regexp(why) => (*why).to_string(),
            Error::UnsafeFunction(name) => alloc::format!(
                "unsafe use of {}()",
                alloc::string::String::from_utf8_lossy(name)
            ),
            // The refusal of a statement used as a value is the refusal
            // the reader wrote, which `sqlite3_errmsg` answers for the
            // statement that holds it.
            Error::Refused(held) => held.message(),
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

    /// The collations the application defined on the connection, which
    /// a `COLLATE` names one of.
    fn collating(&self) -> &'static [crate::value::Collating] {
        &[]
    }

    /// The function the application defined under this name for this
    /// number of arguments, or nothing where it defined none.
    fn defined(&self, _name: &[u8], _count: usize) -> Option<crate::func::Defined> {
        None
    }

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
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] carries what the statement was refused with.
    fn answered(&self, _used: Used) -> Result<Option<Value>, Error> {
        Ok(None)
    }

    /// What the row was refused with while it read a name, which is
    /// what a name standing for an expression of the statement carries
    /// out of that expression, and nothing where no name reached a
    /// refusal.
    ///
    /// The refusal is taken off the row, so a name the row answers
    /// after it carries none.
    fn refused(&self) -> Option<Error> {
        None
    }

    /// Whether the connection trusts the schema, where the expression
    /// stands in an object of the schema, and nothing where a client
    /// wrote the statement.
    ///
    /// `EP_FromDDL` of `research/sqlite/src/sqliteInt.h` marks the
    /// expressions of an object of the schema: the statement of a view,
    /// the body of a trigger, and the expression of a computed column, of
    /// a `CHECK`, of a `DEFAULT` and of an index. An object of the temp
    /// schema carries no such expression, which `fixExprCb` of
    /// `research/sqlite/src/attach.c:468` reads `bTemp` for, and `PRAGMA
    /// trusted_schema` says whether the connection trusts the rest.
    fn schemed(&self) -> Option<bool> {
        None
    }

    /// The aggregates the application defined on the connection, which
    /// a caller that defined none answers an empty list for.
    fn grouped(&self) -> &'static [crate::func::Grouped] {
        &[]
    }

    /// What the clock says, as the julian day number times 86 400 000,
    /// which `now` names.
    ///
    /// A connection told no clock answers nothing, and a statement that
    /// names `now` then answers nothing as well.
    fn clock(&self) -> Option<i64> {
        None
    }

    /// The zone `localtime` and `utc` read, and nothing where the
    /// connection was told none.
    fn zone(&self) -> Option<crate::date::Zone> {
        None
    }

    /// Which part of the schema this row is read for, where the value it
    /// answers must be the same every time it is read, and nothing where
    /// the row stands in a statement of its own.
    fn purely(&self) -> Option<crate::date::Purely> {
        None
    }

    /// What the connection tells the date functions: the clock, the zone
    /// and the place the call stands in together.
    fn told(&self) -> crate::date::Told {
        crate::date::Told {
            now: self.clock(),
            zone: self.zone(),
            purely: self.purely(),
        }
    }

    /// Whether `LIKE` tells the twenty-six letters apart, which
    /// `PRAGMA case_sensitive_like` sets and a connection told nothing
    /// answers false for.
    fn sensitive(&self) -> bool {
        false
    }

    /// The limits the connection holds, which a value the statement
    /// answers is held to and a connection told nothing holds the hard
    /// limits of the build for.
    fn limits(&self) -> crate::db::Limits {
        crate::db::Limits::new()
    }

    /// The first row the statement `select` answers, each value with
    /// the affinity and the collation a comparison against it uses,
    /// which is what a row compared against `(SELECT a, b)` compares
    /// against. A statement that answers no row answers a null per
    /// column.
    ///
    /// # Errors
    ///
    /// [`Error::Refused`] carries what the statement was refused with.
    fn answered_items(&self, _select: SelectId) -> Result<Option<Vec<Item>>, Error> {
        Ok(None)
    }
}

/// One value of the first row a statement used as a value answered, with
/// what a comparison against it does.
pub type Item = (Value, Affinity, Option<Collation>);

/// What a function reads beside its arguments, taken off the row it is
/// read against.
///
/// Reading it costs O(1).
fn given_of(row: &dyn Row) -> func::Given<'_> {
    func::Given {
        random: row.random(),
        counted: row.counted(),
        clock: row.told(),
        sensitive: row.sensitive(),
        limits: row.limits(),
    }
}

/// A row with no columns, carrying the moment its connection's clock
/// says, which is what a constant expression is read against.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRow(pub Option<i64>);

impl Row for NoRow {
    fn clock(&self) -> Option<i64> {
        self.0
    }

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
pub fn evaluate(arena: &Arena, id: ExprId, sql: &[u8], clock: Option<i64>) -> Result<Value, Error> {
    evaluate_row(arena, id, sql, &NoRow(clock))
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

/// Whether an expression stands as a row of values, which decides how
/// many columns a statement compared against it answers.
#[must_use]
pub fn is_row_value(arena: &Arena, id: ExprId) -> bool {
    stands_as_row(arena, id)
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
    inner.collation = Some(
        crate::value::collation_of(&named, row.collating())
            .ok_or_else(|| Error::NoCollation(named.clone()))?,
    );
    inner.written = true;
    Ok(inner)
}
/// The value a name reaches on the row, which is `lookupName` of
/// `research/sqlite/src/resolve.c:377` over the sides the row holds.
///
/// # Errors
///
/// [`Error::NoColumn`] names what no side answers, [`Error::Ambiguous`]
/// what more than one answers, and whatever the expression a name stood
/// for was refused with.
fn named_value(
    held: (
        Option<crate::ast::Span>,
        Option<crate::ast::Span>,
        crate::ast::Span,
    ),
    sql: &[u8],
    row: &dyn Row,
) -> Result<Answer, Error> {
    let (schema, table, column) = held;

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
        // A name standing for an expression the statement
        // answers under carries what that expression was
        // refused with.
        if let Some(held) = row.refused() {
            return Err(held);
        }
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
        Node::Literal(literal) => plain_literal(literal, sql, row),
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
            answered.collation = written_collation(arena, id, sql, row.collating());
            Ok(answered)
        }
        Node::Column {
            schema,
            table,
            column,
        } => named_value((schema, table, column), sql, row),
        // An aggregate and a window function are both answered once
        // per group or per row before the row is read, so the walk
        // looks the answer up rather than working it out; a window
        // function no window was worked out for is a misuse of one,
        // and so is a scalar function carrying a `FILTER`, which only
        // an aggregate reads.
        Node::Call {
            name,
            args,
            filter,
            ordered,
            ..
        } => calling(arena, (id, name, args), (filter, ordered), sql, row, deeper),
        Node::Over { name, .. } => row
            .aggregate(id)
            .map(|value| answered_under(value, arena, id, sql, row))
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
        Node::Raise { action, message } => Err(raised(arena, (action, message), sql, row, deeper)),
        Node::Row(_) => Err(Error::RowValue),
        Node::Variable(_) => Err(Error::Unsupported),
    }
}

/// The name `affinity()` answers for one affinity, which
/// `research/sqlite/src/expr.c:4680` holds the six words of.
fn named_affinity(affinity: Affinity) -> Vec<u8> {
    match affinity {
        Affinity::None => b"none".to_vec(),
        Affinity::Blob => b"blob".to_vec(),
        Affinity::Text => b"text".to_vec(),
        Affinity::Numeric => b"numeric".to_vec(),
        Affinity::Integer => b"integer".to_vec(),
        Affinity::Real => b"real".to_vec(),
    }
}

/// What a `RAISE` the statement reached says, which is the action it
/// carries and the message it wrote as text.
///
/// `OP_Halt` of `research/sqlite/src/vdbe.c:1337` writes that message as
/// the words the statement is refused with, reading it as text, and a
/// message the engine cannot answer takes the place of it. Answering
/// costs what the message costs.
fn raised(
    arena: &Arena,
    raise: (crate::ast::Raise, Option<ExprId>),
    sql: &[u8],
    row: &dyn Row,
    deeper: u32,
) -> Error {
    let (action, message) = raise;
    let text = match message {
        None => Ok(Vec::new()),
        Some(id) => {
            answer(arena, id, sql, row, deeper).map(|held| held.value.text().unwrap_or_default())
        }
    };
    match text {
        Ok(text) => Error::Raised(action, text),
        Err(refused) => refused,
    }
}

/// One call of a function: what the group or the row it stands for
/// answered where it is an aggregate or a window function, and what the
/// function answers for its arguments otherwise.
///
/// A `FILTER` and an `ORDER BY` inside the brackets are each written for
/// an aggregate alone, so a call that is none and carries one is a
/// misuse of it, which `sqlite3ExprAddFunctionOrderBy` of
/// `research/sqlite/src/expr.c` refuses.
///
/// # Errors
///
/// [`Error::Filtered`] and [`Error::OrderedCall`] name the function, and
/// whatever the call refuses otherwise.
fn calling(
    arena: &Arena,
    held: (ExprId, Span, crate::ast::Range),
    written: (Option<ExprId>, crate::ast::Range),
    sql: &[u8],
    row: &dyn Row,
    deeper: u32,
) -> Result<Answer, Error> {
    let (id, name, args) = held;
    let (filter, ordered) = written;
    match row.aggregate(id) {
        Some(value) => Ok(answered_under(value, arena, id, sql, row)),
        None if filter.is_some() => Err(Error::Filtered(named_as(name, sql))),
        None if !ordered.is_empty() => Err(Error::OrderedCall(named_as(name, sql))),
        None => called(arena, name, args, sql, row, deeper),
    }
}

/// `(SELECT a)` as one value, with the affinity and the collation of
/// the column the statement answers, which is `sqlite3ExprAffinity` of
/// a `TK_SELECT`. A statement that answers any other number of columns
/// is a refusal.
fn alone(row: &dyn Row, select: SelectId) -> Result<Answer, Error> {
    let items = row.answered_items(select)?.ok_or(Error::Unsupported)?;
    let answered = items.len();
    let mut held = items.into_iter();
    let (value, affinity, collation) = held.next().ok_or(Error::Columns(answered, 1))?;
    if held.next().is_some() {
        return Err(Error::Columns(answered, 1));
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
    row.answered(what)?
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
    // The affinity of the first argument, which `affinity()` answers the
    // name of.
    let mut affinity = Affinity::None;
    for id in arena.children(args) {
        let argument = answer(arena, *id, sql, row, deeper)?;
        if values.is_empty() {
            affinity = argument.affinity;
        }
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
    // What the row says about the schema, read before the name, because
    // every call is read against the same row.
    let schemed = row.schemed();
    // `INLINEFUNC_affinity` of `research/sqlite/src/expr.c:4675` answers
    // the name of the affinity of the expression it is given, which the
    // walk of that expression carries and no value holds, and which only
    // a connection the internal functions were turned on for reaches.
    if values.len() == 1
        && called.eq_ignore_ascii_case(b"affinity")
        && row.defined(&called, 1).is_some()
    {
        return Ok(Answer::plain(Value::Text(named_affinity(affinity))));
    }
    // `sqlite3FindFunction` reads the functions the application defined
    // before the ones the library holds.
    if let Some(defined) = row.defined(&called, values.len()) {
        // `sqlite3ExprFunctionUsable` of `research/sqlite/src/expr.c:1276`
        // holds an expression a schema object carries to the functions a
        // schema may name: no expression names one the application marked
        // `SQLITE_DIRECTONLY`, and one it marked neither way is named
        // only where the connection trusts the schema.
        if let Some(trusted) = schemed
            && match defined.safety {
                crate::func::Safety::Direct => true,
                crate::func::Safety::Unsafe => !trusted,
                crate::func::Safety::Innocuous => false,
            }
        {
            return Err(Error::UnsafeFunction(called));
        }
        return Ok(Answer::plain((defined.answer)(
            defined.name,
            &values,
            row.random(),
        )?));
    }
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
        Err(Error::NoFunction(_)) if crate::agg::named_in(row.grouped(), &called) => {
            if crate::agg::lookup_in(row.grouped(), &called, values.len()).is_none() {
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
        given_of(row),
    )?;
    Ok(Answer {
        value,
        json,
        affinity: Affinity::None,
        collation: outward,
        written: outward.is_some(),
    })
}

/// A constant read against `row`, which the three clock literals read
/// for the moment the caller told the connection.
///
/// Reading one costs O(n) in the bytes it was written with.
fn plain_literal(literal: Literal, sql: &[u8], row: &dyn Row) -> Result<Answer, Error> {
    let held = literal_value(literal, sql, false, row.told())?;
    // A literal longer than the length the connection holds is refused,
    // which `sqlite3VdbeMemTooBig` refuses the value of every statement
    // for.
    crate::func::held_length(&held, row.limits())?;
    Ok(Answer::plain(held))
}

/// A constant, with `negated` for the minus sign the parser leaves as a
/// node of its own and SQLite folds into the number.
pub(crate) fn literal_value(
    literal: Literal,
    sql: &[u8],
    negated: bool,
    clock: crate::date::Told,
) -> Result<Value, Error> {
    match literal {
        Literal::Null => Ok(Value::Null),
        Literal::Integer(span) => integer_literal(&dequote(span.text(sql)), negated),
        Literal::Float(span) => {
            let read = number::real(&dequote(span.text(sql))).value;
            Ok(Value::Real(if negated { -read } else { read }))
        }
        Literal::Text(span) => Ok(Value::Text(unquote(span.text(sql)))),
        Literal::Blob(span) => Ok(Value::Blob(hex(span.text(sql)))),
        // `CURRENT_TIME`, `CURRENT_DATE` and `CURRENT_TIMESTAMP` are
        // `time`, `date` and `datetime` of the clock, which
        // `currentTimeFunc` writes in the same shapes.
        Literal::CurrentTime(which) => {
            if clock.now.is_none() {
                return Err(Error::Unsupported);
            }
            match which {
                crate::ast::CurrentTime::Time => crate::date::time(&[], clock),
                crate::ast::CurrentTime::Date => crate::date::date(&[], clock),
                crate::ast::CurrentTime::Timestamp => crate::date::datetime(&[], clock),
            }
        }
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
            let mut shown = if negated { b"-".to_vec() } else { Vec::new() };
            shown.extend_from_slice(text);
            return Err(Error::HexTooBig(shown));
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
///
/// A text that opens with no quote is its own bytes, which
/// `sqlite3Dequote` of `research/sqlite/src/util.c` leaves alone: the
/// name after `DEFAULT` is read as a string, and `DEFAULT hi` stands for
/// the two letters and not for nothing.
fn unquote(text: &[u8]) -> Vec<u8> {
    let open = text.first().copied().unwrap_or(b'\'');
    let quote = match open {
        b'\'' | b'"' | b'`' => open,
        b'[' => b']',
        _ => return text.to_vec(),
    };
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
            return literal_value(literal, sql, true, row.told()).map(Answer::plain);
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
        // other way about. The collation does carry, and so does the
        // subtype, because `sqlite3ExprCodeTarget` codes `TK_UPLUS` as
        // the operand itself.
        UnaryOp::Identity => Answer {
            json: inner.json,
            ..carried(value)
        },
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
    // `X IN (a, b)` reads a statement written for `X` as one value,
    // which `sqlite3ExprCodeIN` does for a list it holds the members of
    // itself, so a statement of more than one column is a refusal.
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
            return Err(Error::InTerms(held.len(), left.len()));
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

/// What a call an aggregate or a window answered, under the collation a
/// `COLLATE` of its arguments wrote.
///
/// `sqlite3ExprCollSeq` reaches the arguments of a call along a path a
/// `COLLATE` marked, so `max(c COLLATE nocase)` answers under `nocase`
/// and a comparison against the answer reads that collation.
fn answered_under(value: Value, arena: &Arena, id: ExprId, sql: &[u8], row: &dyn Row) -> Answer {
    let written = written_collation(arena, id, sql, row.collating());
    Answer {
        value,
        json: false,
        affinity: Affinity::None,
        collation: written,
        written: written.is_some(),
    }
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
fn written_collation(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    collating: &[crate::value::Collating],
) -> Option<Collation> {
    let node = arena.node(id)?;
    if let Node::Collate { name, .. } = node {
        return crate::value::collation_of(&crate::schema::dequote(name.text(sql)), collating);
    }
    // A `CASE` is read in the order it was written, because the first
    // `COLLATE` written under it is the one it answers with, and
    // `Arena::under` names the `ELSE` before the branches.
    let mut found = None;
    let mut first = |child: ExprId| {
        if found.is_none() {
            found = written_collation(arena, child, sql, collating);
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
    // A row of values stands under a comparison and nowhere else, and a
    // statement stands as a row only there, because every other
    // operator reads it as the one value `sqlite3ExprCodeSubselect`
    // answers.
    if is_row(arena, left)
        || is_row(arena, right)
        || (compares(op) && (stands_as_row(arena, left) || stands_as_row(arena, right)))
    {
        return rows_compared(arena, op, left, right, sql, row, depth);
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
        BinaryOp::Concat => {
            let held = concatenate(&left.value, &right.value);
            // `sqlite3VdbeMemTooBig` holds the text two values make to
            // the length the connection holds.
            crate::func::held_length(&held, row.limits())?;
            held
        }
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

/// How many values a width holds, which a statement holds as many of as
/// it answers columns and every other expression one of.
const fn counted(held: Width) -> usize {
    match held {
        Width::Row(held) => held,
        _ => 1,
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
    // A row of a `VALUES` is a row of the statement and not a value
    // written under an expression, so every one of them stands where it
    // may.
    for select in arena.all_selects() {
        for held in arena.children(select.values) {
            marked(*held, &mut allowed);
        }
    }
    // `SET (a, b) = (1, 2)` writes one column per value of the row, so
    // the row stands where it may there as well.
    for set in arena.all_sets() {
        if set.at.is_some() {
            marked(set.value, &mut allowed);
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
                marked(left, &mut allowed);
                marked(right, &mut allowed);
            }
            Node::Between {
                value, low, high, ..
            } => {
                let middle = width(arena, value);
                if !middle.is_row() {
                    // A statement stands as a row, so the rows written
                    // against one stand where they may and how wide it
                    // is is read when it runs.
                    if middle == Width::Unknown {
                        marked(low, &mut allowed);
                        marked(high, &mut allowed);
                    }
                    continue;
                }
                if !fit(middle, width(arena, low)) || !fit(middle, width(arena, high)) {
                    return Err(Error::RowValue);
                }
                marked(value, &mut allowed);
                marked(low, &mut allowed);
                marked(high, &mut allowed);
            }
            Node::InList { value, list, .. } => {
                let left = width(arena, value);
                if !left.is_row() {
                    continue;
                }
                for member in arena.children(list) {
                    let held = width(arena, *member);
                    if !fit(left, held) {
                        return Err(Error::InTerms(counted(held), counted(left)));
                    }
                    marked(*member, &mut allowed);
                }
                marked(value, &mut allowed);
            }
            Node::InSelect { value, .. } | Node::InTable { value, .. } => {
                marked(value, &mut allowed);
            }
            Node::Case {
                operand, branches, ..
            } => case_placed(arena, operand, branches, &mut allowed)?,
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

/// Marks the node as one that stands where a row may stand.
fn marked(id: ExprId, allowed: &mut [bool]) {
    for slot in allowed.iter_mut().skip(id.place()).take(1) {
        *slot = true;
    }
}

/// Marks the `WHEN` clauses of a `CASE` whose operand stands as a row,
/// which `sqlite3ExprCodeVector` compares the operand against pair by
/// pair.
///
/// # Errors
///
/// [`Error::RowValue`] where a `WHEN` holds another number of values
/// than the operand.
fn case_placed(
    arena: &Arena,
    operand: Option<ExprId>,
    branches: crate::ast::Range,
    allowed: &mut [bool],
) -> Result<(), Error> {
    let Some(operand) = operand else {
        return Ok(());
    };
    let subject = width(arena, operand);
    if subject == Width::One {
        return Ok(());
    }
    let children = arena.children(branches);
    let mut at = 0;
    while let Some(when) = children.get(at) {
        if subject.is_row() && !fit(subject, width(arena, *when)) {
            return Err(Error::RowValue);
        }
        marked(*when, allowed);
        at = at.saturating_add(2);
    }
    marked(operand, allowed);
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
    asked_of(column)
}

/// A bare name as `no such column: ` writes it: the name with its quotes
/// taken off, and one written in double quotes with the question the C
/// library asks after it.
///
/// `resolveExprStep` of `research/sqlite/src/resolve.c:791` reads
/// `EP_DblQuoted`, which the parser marks a name it read between double
/// quotes, so `"b"` carries the question and `'b'` does not.
pub(crate) fn asked_of(written: &[u8]) -> Vec<u8> {
    if written.first() != Some(&b'"') {
        return crate::schema::dequote(written);
    }
    let mut shown = alloc::vec![b'"'];
    shown.extend_from_slice(&crate::schema::dequote(written));
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

/// Whether the node stands as a row of values: a row the statement
/// wrote, or a statement of its own, the columns of which are the
/// values of the row.
///
/// `sqlite3ExprCodeSubselect` reads a vector out of a statement of more
/// than one column, and one of one column stands for one value.
fn stands_as_row(arena: &Arena, id: ExprId) -> bool {
    matches!(arena.node(id), Some(Node::Row(_) | Node::Subquery(_)))
}

/// Whether the operator compares two values, which is the only kind a
/// row of values stands under.
const fn compares(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::Is
            | BinaryOp::IsNot
    )
}

/// The values an expression answers as a row: the items of a row, the
/// columns of a statement, or the one value every other expression
/// answers.
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
            let held = row.answered_items(select)?.ok_or(Error::Unsupported)?;
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
        // Every other expression answers one value, which stands as a
        // row of one value.
        _ => return Ok(alloc::vec![answer(arena, id, sql, row, depth)?]),
    };
    let mut out = Vec::new();
    for item in arena.children(items) {
        out.push(answer(arena, *item, sql, row, depth)?);
    }
    Ok(out)
}

/// `(a, b) < (c, d)`: a comparison of a row against a row, which
/// `sqlite3ExprCodeVectorCompare` answers pair by pair. Every other
/// operator, and a row compared against a row of another width, is a
/// misuse.
fn rows_compared(
    arena: &Arena,
    op: BinaryOp,
    left: ExprId,
    right: ExprId,
    sql: &[u8],
    row: &dyn Row,
    depth: u32,
) -> Result<Answer, Error> {
    if !compares(op) {
        return Err(Error::RowValue);
    }
    let left = row_answers(arena, left, sql, row, depth)?;
    let right = row_answers(arena, right, sql, row, depth)?;
    if left.len() != right.len() {
        return Err(Error::RowValue);
    }
    // A statement of one column stands for one value, which is compared
    // as every other value is and carries the collation of the two
    // sides.
    if let ([left], [right]) = (left.as_slice(), right.as_slice()) {
        return Ok(Answer {
            value: comparison(op, left, right, row.collation()),
            json: false,
            affinity: Affinity::None,
            collation: compared_under(left, right),
            written: left.written || right.written,
        });
    }
    Ok(Answer::plain(pairs_compared(
        op,
        &left,
        &right,
        row.collation(),
    )))
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
    // The pattern is the first argument and the value the second, which
    // is how `A LIKE B` is written as `like(B,A)`.
    let mut args = alloc::vec![
        answer(arena, pattern, sql, row, depth)?.value,
        answer(arena, value, sql, row, depth)?.value,
    ];
    // `REGEXP` and `MATCH` are names the library holds no function
    // under, so a connection that was told one of them reaches it and a
    // connection that was told neither is refused the name.
    let function = match op {
        LikeOp::Like => Function::Like,
        LikeOp::Glob => Function::Glob,
        LikeOp::Regexp | LikeOp::Match => {
            let called: &[u8] = if op == LikeOp::Regexp {
                b"regexp"
            } else {
                b"match"
            };
            let Some(defined) = row.defined(called, args.len()) else {
                let named = if op == LikeOp::Regexp {
                    b"REGEXP".to_vec()
                } else {
                    b"MATCH".to_vec()
                };
                return Err(Error::NoFunction(named));
            };
            let answered = (defined.answer)(defined.name, &args, row.random())?;
            return Ok(Answer::plain(match (negated, logic(&answered)) {
                (_, None) => Value::Null,
                (true, Some(truth)) => Value::Int(i64::from(!truth)),
                (false, Some(truth)) => Value::Int(i64::from(truth)),
            }));
        }
    };
    if let Some(escape) = escape {
        args.push(answer(arena, escape, sql, row, depth)?.value);
    }
    let (answered, _) = func::call(
        function,
        &args,
        &[],
        row.collation(),
        row.encoding(),
        given_of(row),
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
    let (above, below) =
        if stands_as_row(arena, value) || stands_as_row(arena, low) || stands_as_row(arena, high) {
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
pub(crate) const fn truth_of(text: &[u8]) -> Option<bool> {
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
    // The operand and every `WHEN` stand as rows, which
    // `sqlite3ExprCodeVector` compares pair by pair: a row of one value
    // is the one value every other expression answers.
    let subject = match operand {
        Some(id) => Some(row_answers(arena, id, sql, row, depth)?),
        None => None,
    };
    let children = arena.children(branches);
    let mut at = 0;
    while let (Some(when), Some(then)) = (children.get(at), children.get(at.saturating_add(1))) {
        let taken = match &subject {
            Some(subject) => {
                let condition = row_answers(arena, *when, sql, row, depth)?;
                if condition.len() != subject.len() {
                    return Err(Error::RowValue);
                }
                logic(&pairs_compared(
                    BinaryOp::Eq,
                    subject,
                    &condition,
                    row.collation(),
                ))
            }
            None => logic(&answer(arena, *when, sql, row, depth)?.value),
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
