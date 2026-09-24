// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The authorizer of a connection, which `sqlite3_set_authorizer`
//! registers and `sqlite3AuthCheck` of `research/sqlite/src/auth.c:196`
//! asks before a statement runs.
//!
//! A connection is told one function. Every statement is read before it
//! runs: the reading asks the function once per action, refuses the
//! statement where the function denies, and leaves the operation undone
//! where the function ignores. A `SQLITE_READ` the function ignores
//! answers a null for that column, which is `pExpr->op = TK_NULL` of
//! `sqlite3AuthRead`.
//!
//! The numbers `research/sqlite/src/sqlite.h.in:3504` gives the actions
//! are not carried: an application reads [`Action`] by name, and the
//! harness of the suite writes the word `tclsqlite.c` writes.
//!
//! Reading one statement costs O(n) in its nodes and O(k) in the tables
//! of the schema it names.

use alloc::vec::Vec;

use crate::ast::{Arena, ExprId, Node, Range, SelectId, Span};
use crate::schema::dequote;

/// What a statement is about to do, which is the second argument the
/// function is given.
///
/// The words are the ones `research/sqlite/src/tclsqlite.c:1192` writes
/// the numbers of `research/sqlite/src/sqlite.h.in:3504` as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `SQLITE_CREATE_INDEX`.
    CreateIndex,
    /// `SQLITE_CREATE_TABLE`.
    CreateTable,
    /// `SQLITE_CREATE_TEMP_INDEX`.
    CreateTempIndex,
    /// `SQLITE_CREATE_TEMP_TABLE`.
    CreateTempTable,
    /// `SQLITE_CREATE_TEMP_TRIGGER`.
    CreateTempTrigger,
    /// `SQLITE_CREATE_TEMP_VIEW`.
    CreateTempView,
    /// `SQLITE_CREATE_TRIGGER`.
    CreateTrigger,
    /// `SQLITE_CREATE_VIEW`.
    CreateView,
    /// `SQLITE_DELETE`.
    Delete,
    /// `SQLITE_DROP_INDEX`.
    DropIndex,
    /// `SQLITE_DROP_TABLE`.
    DropTable,
    /// `SQLITE_DROP_TEMP_INDEX`.
    DropTempIndex,
    /// `SQLITE_DROP_TEMP_TABLE`.
    DropTempTable,
    /// `SQLITE_DROP_TEMP_TRIGGER`.
    DropTempTrigger,
    /// `SQLITE_DROP_TEMP_VIEW`.
    DropTempView,
    /// `SQLITE_DROP_TRIGGER`.
    DropTrigger,
    /// `SQLITE_DROP_VIEW`.
    DropView,
    /// `SQLITE_INSERT`.
    Insert,
    /// `SQLITE_PRAGMA`.
    Pragma,
    /// `SQLITE_READ`.
    Read,
    /// `SQLITE_SELECT`.
    Select,
    /// `SQLITE_TRANSACTION`.
    Transaction,
    /// `SQLITE_UPDATE`.
    Update,
    /// `SQLITE_ALTER_TABLE`.
    AlterTable,
    /// `SQLITE_REINDEX`.
    Reindex,
    /// `SQLITE_ANALYZE`.
    Analyze,
    /// `SQLITE_FUNCTION`.
    Function,
    /// `SQLITE_SAVEPOINT`.
    Savepoint,
    /// `SQLITE_RECURSIVE`.
    Recursive,
    /// `SQLITE_ATTACH`.
    Attach,
    /// `SQLITE_DETACH`.
    Detach,
}

impl Action {
    /// The word the action is written as.
    #[must_use]
    pub const fn word(self) -> &'static [u8] {
        match self {
            Action::CreateIndex => b"SQLITE_CREATE_INDEX",
            Action::CreateTable => b"SQLITE_CREATE_TABLE",
            Action::CreateTempIndex => b"SQLITE_CREATE_TEMP_INDEX",
            Action::CreateTempTable => b"SQLITE_CREATE_TEMP_TABLE",
            Action::CreateTempTrigger => b"SQLITE_CREATE_TEMP_TRIGGER",
            Action::CreateTempView => b"SQLITE_CREATE_TEMP_VIEW",
            Action::CreateTrigger => b"SQLITE_CREATE_TRIGGER",
            Action::CreateView => b"SQLITE_CREATE_VIEW",
            Action::Delete => b"SQLITE_DELETE",
            Action::DropIndex => b"SQLITE_DROP_INDEX",
            Action::DropTable => b"SQLITE_DROP_TABLE",
            Action::DropTempIndex => b"SQLITE_DROP_TEMP_INDEX",
            Action::DropTempTable => b"SQLITE_DROP_TEMP_TABLE",
            Action::DropTempTrigger => b"SQLITE_DROP_TEMP_TRIGGER",
            Action::DropTempView => b"SQLITE_DROP_TEMP_VIEW",
            Action::DropTrigger => b"SQLITE_DROP_TRIGGER",
            Action::DropView => b"SQLITE_DROP_VIEW",
            Action::Insert => b"SQLITE_INSERT",
            Action::Pragma => b"SQLITE_PRAGMA",
            Action::Read => b"SQLITE_READ",
            Action::Select => b"SQLITE_SELECT",
            Action::Transaction => b"SQLITE_TRANSACTION",
            Action::Update => b"SQLITE_UPDATE",
            Action::AlterTable => b"SQLITE_ALTER_TABLE",
            Action::Reindex => b"SQLITE_REINDEX",
            Action::Analyze => b"SQLITE_ANALYZE",
            Action::Function => b"SQLITE_FUNCTION",
            Action::Savepoint => b"SQLITE_SAVEPOINT",
            Action::Recursive => b"SQLITE_RECURSIVE",
            Action::Attach => b"SQLITE_ATTACH",
            Action::Detach => b"SQLITE_DETACH",
        }
    }
}

/// What the function is asked, which is the four arguments after the
/// action. A place the C library passes a null pointer for carries no
/// bytes here, because `tclsqlite.c` writes both as an empty word.
#[derive(Clone, Copy, Debug)]
pub struct Asked<'a> {
    /// What the statement is about to do.
    pub action: Action,
    /// The third argument: the table, the index, the view, the trigger,
    /// the pragma or the operation, by action.
    pub first: &'a [u8],
    /// The fourth argument: the column, the table the index is on, the
    /// value of the pragma or the name of the savepoint, by action.
    pub second: &'a [u8],
    /// The fifth argument: the schema, where the action names one.
    pub schema: &'a [u8],
    /// The sixth argument: the innermost trigger or view the access is
    /// under, and nothing where the statement itself holds it.
    pub inner: &'a [u8],
}

/// What the function answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The action stands.
    Ok,
    /// The statement is refused.
    Deny,
    /// The action is left undone, and a column read answers a null.
    Ignore,
}

/// The function a connection was told, which every statement is read
/// against.
pub type Asking = fn(&Asked<'_>) -> Answer;

/// One action of a statement, as the arguments the function is given:
/// the action, the third argument, the fourth, and the schema.
type Asks<'a> = (Action, &'a [u8], &'a [u8], &'a [u8]);

/// What the function refuses a statement with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The function denied the action.
    Denied,
    /// The function denied reading a column, with `table.column` as
    /// `sqlite3AuthReadCol` writes it.
    Prohibited(Vec<u8>),
    /// The function denied a function, with its name.
    Function(Vec<u8>),
}

/// The authorizer of one statement: the function the connection was
/// told, the schema the statement runs over, and the columns a read the
/// function ignored answers a null for.
pub(crate) struct Authorizer<'a> {
    /// The function.
    asking: Asking,
    /// The schema the statement runs over.
    database: &'a crate::db::Database<'a>,
    /// The columns a read the function ignored answers a null for, as
    /// the table and the column.
    ignored: Vec<(Vec<u8>, Vec<u8>)>,
    /// The columns of an `UPDATE` the function ignored, which keep the
    /// value they had, and which is `aXRef[j] = -1` of
    /// `research/sqlite/src/update.c:514`.
    unwritten: Vec<Vec<u8>>,
    /// The innermost trigger the reading stands in, which is the sixth
    /// argument every action of its body carries.
    inner: Vec<u8>,
    /// The rows `OLD` and `NEW` a statement of that body reads, which
    /// stand for the row the trigger fired over.
    fired: Vec<Reading>,
    /// The triggers the reading stands in, which a body that writes the
    /// table its own trigger is on would otherwise be read without end.
    walking: Vec<Vec<u8>>,
}

/// What the reading of one statement came to.
pub(crate) struct Read {
    /// What the function answered for the statement itself, which is
    /// `Ignore` where the statement is left undone.
    pub(crate) answer: Answer,
    /// The columns of an `UPDATE` the function ignored.
    pub(crate) unwritten: Vec<Vec<u8>>,
}

impl<'a> Authorizer<'a> {
    /// The authorizer of one statement.
    pub(crate) const fn new(asking: Asking, database: &'a crate::db::Database<'a>) -> Self {
        Authorizer {
            asking,
            database,
            ignored: Vec::new(),
            unwritten: Vec::new(),
            inner: Vec::new(),
            fired: Vec::new(),
            walking: Vec::new(),
        }
    }

    /// The columns a read the function ignored answers a null for.
    pub(crate) fn ignored(self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.ignored
    }

    /// What the reading came to, with `answer` for the statement itself.
    pub(crate) fn taken(self, answer: Answer) -> Read {
        Read {
            answer,
            unwritten: self.unwritten,
        }
    }

    /// What the function answers for one action, with the statement
    /// refused where it denies.
    ///
    /// # Errors
    ///
    /// [`Error::Denied`] where the function denies the action.
    fn ask(
        &self,
        action: Action,
        first: &[u8],
        second: &[u8],
        schema: &[u8],
    ) -> Result<Answer, Error> {
        let asked = Asked {
            action,
            first,
            second,
            schema,
            inner: &self.inner,
        };
        match (self.asking)(&asked) {
            Answer::Deny => Err(Error::Denied),
            answer => Ok(answer),
        }
    }

    /// The actions of one statement asked for in turn, which stops at
    /// the first one the function ignores.
    ///
    /// Every caller of `sqlite3AuthCheck` leaves the statement undone
    /// for a nonzero answer, and the function is asked no further
    /// action of that statement.
    ///
    /// # Errors
    ///
    /// [`Error::Denied`] where the function denies one of the actions.
    fn all(&self, asks: &[Asks<'_>]) -> Result<Answer, Error> {
        for (action, first, second, schema) in asks {
            if self.ask(*action, first, second, schema)? == Answer::Ignore {
                return Ok(Answer::Ignore);
            }
        }
        Ok(Answer::Ok)
    }
}
impl Error {
    /// What the statement is refused with, as `sqlite3ErrorMsg` writes
    /// it.
    #[must_use]
    pub fn message(&self) -> alloc::string::String {
        match self {
            Error::Denied => alloc::string::String::from("not authorized"),
            Error::Prohibited(name) => alloc::format!(
                "access to {} is prohibited",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::Function(name) => alloc::format!(
                "not authorized to use function: {}",
                alloc::string::String::from_utf8_lossy(name)
            ),
        }
    }
}

/// One table a statement reads rows out of.
#[derive(Clone)]
struct Reading {
    /// The name in the schema.
    name: Vec<u8>,
    /// The name the statement knows it by, which is the alias where one
    /// was written and the schema name otherwise.
    known: Vec<u8>,
    /// The schema the statement named, or `main`.
    schema: Vec<u8>,
    /// The columns, in the order the table was written.
    columns: Vec<Vec<u8>>,
    /// The name a bare `rowid` is asked under, which is the column the
    /// rowid is another name for and `ROWID` where the table has none.
    key: Vec<u8>,
    /// Whether a column of it was asked for, which `pItem->colUsed`
    /// counts.
    used: bool,
}

/// What a statement names where it reads a column: one column by name,
/// every column of one table, or every column of every table.
#[derive(Clone, Copy, Debug)]
enum Named {
    /// A column, with the table and the schema where the statement wrote
    /// them.
    One(Option<Span>, Option<Span>, Span),
    /// `t.*`.
    Every(Span),
    /// `*`.
    All,
    /// A statement written inside an expression, which is read as a
    /// statement of its own.
    Inside(SelectId),
}

impl Authorizer<'_> {
    /// `SQLITE_SELECT` and every read of one statement, which is what
    /// `sqlite3Select` of `research/sqlite/src/select.c:7620` asks for
    /// before it reads a row.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    pub(crate) fn select(
        &mut self,
        arena: &Arena,
        id: SelectId,
        sql: &[u8],
    ) -> Result<Answer, Error> {
        if self.ask(Action::Select, b"", b"", b"")? == Answer::Ignore {
            return Ok(Answer::Ignore);
        }
        self.read_select(arena, id, sql)
    }

    /// The reads of one statement and of every statement inside it, in
    /// the order `resolveSelectStep` of
    /// `research/sqlite/src/resolve.c:1979` resolves the names.
    fn read_select(&mut self, arena: &Arena, id: SelectId, sql: &[u8]) -> Result<Answer, Error> {
        // `Arena::select` answers nothing only for a place no tree holds,
        // which a statement the parser wrote has none of, and the
        // statement of no clause at all reads nothing.
        let select = arena.select(id).unwrap_or_default();
        // A statement written inside the `FROM` is read first, and a
        // term of a `WITH` is read where the statement that names it is.
        for cte in arena.ctes(select.ctes).to_vec() {
            // `generateWithRecursiveQuery` of
            // `research/sqlite/src/select.c:2698` asks for the term that
            // reads its own name before it reads a row of it.
            if reads_itself(arena, &cte, sql) {
                self.ask(Action::Recursive, b"", b"", b"")?;
            }
            self.select(arena, cte.select, sql)?;
        }
        for source in arena.sources(select.from).to_vec() {
            if let crate::ast::SourceKind::Select(inner) = source.kind {
                self.select(arena, inner, sql)?;
            }
        }
        let mut reading = self.reading_of(arena, select.from, sql);
        for named in named_of(arena, &select) {
            self.read_named(arena, &mut reading, named, sql)?;
        }
        // A table no column of was asked for is asked for under the
        // empty name, which `sqlite3Select` of
        // `research/sqlite/src/select.c:7989` writes for
        // `SELECT count(*) FROM t1`.
        for table in &reading {
            if !table.used {
                self.ask(Action::Read, &table.name, b"", &table.schema)?;
            }
        }
        // The arms of a compound are statements of their own, which
        // `multiSelect` reads under the same `SQLITE_SELECT`.
        if let Some((_, other)) = select.compound {
            self.read_select(arena, other, sql)?;
        }
        Ok(Answer::Ok)
    }

    /// The tables of a `FROM` clause, each with the columns the schema
    /// says it holds. A source that is not a table of the schema is left
    /// out, because a column of it is no read of a file.
    fn reading_of(&self, arena: &Arena, from: Range, sql: &[u8]) -> Vec<Reading> {
        let mut out = self.fired.clone();
        for source in arena.sources(from) {
            let crate::ast::SourceKind::Table { schema, name, .. } = source.kind else {
                continue;
            };
            let named = dequote(name.text(sql));
            let Some((table, _)) = self.database.table(&named) else {
                continue;
            };
            let key = key_named(table);
            out.push(Reading {
                name: table.name.clone(),
                known: match source.alias {
                    Some(alias) => dequote(alias.text(sql)),
                    None => named,
                },
                schema: match schema {
                    Some(schema) => dequote(schema.text(sql)),
                    None => b"main".to_vec(),
                },
                columns: table
                    .columns
                    .iter()
                    .map(|column| column.name.clone())
                    .collect(),
                key,
                used: false,
            });
        }
        out
    }

    /// The rows `OLD` and `NEW` a statement of a trigger's body reads,
    /// which stand for the row the trigger fired over.
    ///
    /// The two are no table of the statement, so no read of the empty
    /// name names them.
    fn fired_rows(&self, table: &[u8]) -> Vec<Reading> {
        self.database
            .table(table)
            .into_iter()
            .flat_map(|(held, _)| {
                [b"old".as_slice(), b"new"].map(|known| Reading {
                    name: held.name.clone(),
                    known: known.to_vec(),
                    schema: b"main".to_vec(),
                    columns: held
                        .columns
                        .iter()
                        .map(|column| column.name.clone())
                        .collect(),
                    key: key_named(held),
                    used: true,
                })
            })
            .collect()
    }

    /// The actions of the body of every trigger of `table` for `event`,
    /// each asked for under the name of that trigger.
    ///
    /// `sqlite3CodeRowTrigger` of `research/sqlite/src/trigger.c:1468`
    /// walks the triggers the table carries in the order
    /// `sqlite3TriggerList` holds them, which is the one made last
    /// first, whatever time each runs at. A trigger whose body writes
    /// the table it is on carries itself, so a trigger already being
    /// read is passed over.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    fn fired(&mut self, table: &[u8], event: crate::ast::TriggerEvent) -> Result<(), Error> {
        let triggers: Vec<crate::db::Trigger> = self
            .database
            .triggers_over(table, event)
            .into_iter()
            .cloned()
            .collect();
        for trigger in &triggers {
            if self.walking.contains(&trigger.name) {
                continue;
            }
            self.walking.push(trigger.name.clone());
            let rows = self.fired_rows(table);
            let inner = core::mem::replace(&mut self.inner, trigger.name.clone());
            let held = core::mem::replace(&mut self.fired, rows);
            let answered = self.body(&trigger.arena, trigger.written.body, &trigger.sql);
            self.fired = held;
            self.inner = inner;
            self.walking.pop();
            answered?;
        }
        Ok(())
    }

    /// The actions of every statement of one trigger's body.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    fn body(&mut self, arena: &Arena, body: Range, sql: &[u8]) -> Result<(), Error> {
        for step in arena.steps(body).to_vec() {
            match step {
                crate::ast::TriggerStep::Insert(statement) => {
                    self.change(arena, crate::ast::Change::Insert(statement), sql)?;
                }
                crate::ast::TriggerStep::Update(statement) => {
                    self.change(arena, crate::ast::Change::Update(statement), sql)?;
                }
                crate::ast::TriggerStep::Delete(statement) => {
                    self.change(arena, crate::ast::Change::Delete(statement), sql)?;
                }
                crate::ast::TriggerStep::Select(select) => {
                    self.select(arena, select, sql)?;
                }
            }
        }
        Ok(())
    }

    /// One name a statement reads, asked for against the tables of its
    /// `FROM`.
    fn read_named(
        &mut self,
        arena: &Arena,
        reading: &mut [Reading],
        named: Named,
        sql: &[u8],
    ) -> Result<Answer, Error> {
        match named {
            Named::Inside(inner) => {
                self.select(arena, inner, sql)?;
            }
            Named::All => {
                for at in 0..reading.len() {
                    self.read_every(reading, at)?;
                }
            }
            Named::Every(table) => {
                let named = dequote(table.text(sql));
                let found = reading
                    .iter()
                    .position(|table| table.known.eq_ignore_ascii_case(&named));
                if let Some(at) = found {
                    self.read_every(reading, at)?;
                }
            }
            Named::One(schema, table, column) => {
                let held = dequote(column.text(sql));
                let named = table.map(|table| dequote(table.text(sql)));
                let under = schema.map(|schema| dequote(schema.text(sql)));
                let found = reading.iter().position(|reading| {
                    if under
                        .as_ref()
                        .is_some_and(|under| !reading.schema.eq_ignore_ascii_case(under))
                    {
                        return false;
                    }
                    match &named {
                        Some(named) => reading.known.eq_ignore_ascii_case(named),
                        None => {
                            reading
                                .columns
                                .iter()
                                .any(|name| name.eq_ignore_ascii_case(&held))
                                || is_rowid(&held)
                        }
                    }
                });
                let Some(at) = found else {
                    return Ok(Answer::Ok);
                };
                self.read_one(arena, reading, at, &held)?;
            }
        }
        Ok(Answer::Ok)
    }

    /// Every column of the table at `at`, which is what a `*` reads.
    fn read_every(&mut self, reading: &mut [Reading], at: usize) -> Result<Answer, Error> {
        let mut held = Vec::new();
        for table in reading.iter_mut().skip(at).take(1) {
            table.used = true;
            held.push((
                table.name.clone(),
                table.schema.clone(),
                table.known.clone(),
                table.columns.clone(),
            ));
        }
        for (name, schema, known, columns) in held {
            for column in columns {
                self.read_column(&name, &column, &schema, &known, &column)?;
            }
        }
        Ok(Answer::Ok)
    }

    /// One column of the table at `at`, with a bare `rowid` asked for
    /// under the name `sqlite3AuthRead` writes for it.
    fn read_one(
        &mut self,
        _arena: &Arena,
        reading: &mut [Reading],
        at: usize,
        column: &[u8],
    ) -> Result<Answer, Error> {
        let mut asked = Vec::new();
        for table in reading.iter_mut().skip(at).take(1) {
            let held = table
                .columns
                .iter()
                .find(|name| name.eq_ignore_ascii_case(column))
                .cloned();
            let held = match held {
                Some(name) => Some(name),
                None if is_rowid(column) => Some(table.key.clone()),
                None => None,
            };
            if let Some(held) = held {
                table.used = true;
                asked.push((
                    table.name.clone(),
                    held,
                    table.schema.clone(),
                    table.known.clone(),
                ));
            }
        }
        let mut answered = Answer::Ok;
        for (name, held, schema, known) in asked {
            answered = self.read_column(&name, &held, &schema, &known, column)?;
        }
        Ok(answered)
    }

    /// `SQLITE_READ` of one column, which answers a null for that column
    /// where the function ignores it and refuses the statement where it
    /// denies, which is `sqlite3AuthReadCol` of
    /// `research/sqlite/src/auth.c:104`.
    fn read_column(
        &mut self,
        table: &[u8],
        column: &[u8],
        schema: &[u8],
        known: &[u8],
        written: &[u8],
    ) -> Result<Answer, Error> {
        let asked = Asked {
            action: Action::Read,
            first: table,
            second: column,
            schema,
            inner: &self.inner,
        };
        match (self.asking)(&asked) {
            Answer::Deny => {
                // `sqlite3AuthReadCol` of `research/sqlite/src/auth.c:118`
                // writes the schema in front of the table where the
                // column stands in a database other than the one the
                // statement writes.
                let mut named = Vec::new();
                if !schema.eq_ignore_ascii_case(self.database.main_named()) {
                    named.extend_from_slice(schema);
                    named.push(b'.');
                }
                named.extend_from_slice(table);
                named.push(b'.');
                named.extend_from_slice(column);
                Err(Error::Prohibited(named))
            }
            Answer::Ignore => {
                // The rows are answered against the name the statement
                // knows the table by, and a bare `rowid` is asked for
                // under the name of the column it stands for, so both
                // names of the column are kept.
                self.ignored.push((known.to_vec(), column.to_vec()));
                if !written.eq_ignore_ascii_case(column) {
                    self.ignored.push((known.to_vec(), written.to_vec()));
                }
                Ok(Answer::Ignore)
            }
            Answer::Ok => Ok(Answer::Ok),
        }
    }
}

/// Whether a term of a `WITH` reads its own name, which is what makes
/// the term recursive.
fn reads_itself(arena: &Arena, cte: &crate::ast::Cte, sql: &[u8]) -> bool {
    let name = dequote(cte.name.text(sql));
    let mut held = alloc::vec![arena.select(cte.select).unwrap_or_default()];
    while let Some(select) = held.pop() {
        for source in arena.sources(select.from) {
            match source.kind {
                crate::ast::SourceKind::Table { name: table, .. } => {
                    if dequote(table.text(sql)).eq_ignore_ascii_case(&name) {
                        return true;
                    }
                }
                crate::ast::SourceKind::Select(inner) => {
                    held.extend(arena.select(inner));
                }
                crate::ast::SourceKind::Function { .. } => {}
            }
        }
        if let Some((_, other)) = select.compound {
            held.extend(arena.select(other));
        }
    }
    false
}

/// Whether a name is one of the three a rowid answers to.
const fn is_rowid(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"rowid")
        || name.eq_ignore_ascii_case(b"oid")
        || name.eq_ignore_ascii_case(b"_rowid_")
}

/// The names one statement reads, in the order the names are resolved:
/// the `LIMIT`, the `ON` of every source, the result columns, the
/// `HAVING`, the `WHERE`, the `ORDER BY` and the `GROUP BY`.
fn named_of(arena: &Arena, select: &crate::ast::Select) -> Vec<Named> {
    let mut out = Vec::new();
    if let Some(limit) = select.limit {
        named_under(arena, limit.count, &mut out);
        if let Some(offset) = limit.offset {
            named_under(arena, offset, &mut out);
        }
    }
    for source in arena.sources(select.from) {
        if let Some(on) = source.on {
            named_under(arena, on, &mut out);
        }
        if let crate::ast::SourceKind::Function { args, .. } = source.kind {
            for arg in arena.children(args) {
                named_under(arena, *arg, &mut out);
            }
        }
    }
    for column in arena.results(select.columns) {
        match column {
            crate::ast::ResultColumn::Star => out.push(Named::All),
            crate::ast::ResultColumn::TableStar(table) => out.push(Named::Every(*table)),
            crate::ast::ResultColumn::Expr { expr, .. } => {
                named_under(arena, *expr, &mut out);
            }
        }
    }
    for row in arena.children(select.values) {
        named_under(arena, *row, &mut out);
    }
    if let Some(having) = select.having {
        named_under(arena, having, &mut out);
    }
    if let Some(filter) = select.filter {
        named_under(arena, filter, &mut out);
    }
    for order in arena.orders(select.order) {
        named_under(arena, order.expr, &mut out);
    }
    for group in arena.children(select.group) {
        named_under(arena, *group, &mut out);
    }
    out
}

/// The columns and the statements one tree names, in the order they are
/// written, answering whether the place names a node at all: a place no
/// tree holds names none, which a statement the parser wrote has none of.
fn named_under(arena: &Arena, id: ExprId, out: &mut Vec<Named>) -> bool {
    arena.node(id).is_some_and(|node| {
        match node {
            Node::Column {
                schema,
                table,
                column,
            } => out.push(Named::One(schema, table, column)),
            // A statement written inside an expression is read as a
            // statement of its own, which `sqlite3Select` is called
            // again for.
            Node::Subquery(inner) | Node::Exists(inner) | Node::InSelect { select: inner, .. } => {
                out.push(Named::Inside(inner));
            }
            _ => {}
        }
        arena.under(node, |under| {
            named_under(arena, under, out);
        });
        true
    })
}

impl Authorizer<'_> {
    /// What the schema's own table is called in a schema, which is
    /// `SCHEMA_TABLE` of `research/sqlite/src/sqliteInt.h`.
    const fn schema_table(temporary: bool) -> &'static [u8] {
        if temporary {
            b"sqlite_temp_master"
        } else {
            b"sqlite_master"
        }
    }

    /// `CREATE` and `DROP` and the four `ALTER TABLE` forms, each of
    /// which writes the schema's own table and so is asked for twice.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    pub(crate) fn definition(
        &mut self,
        arena: &Arena,
        definition: crate::ast::Definition,
        sql: &[u8],
    ) -> Result<Answer, Error> {
        match definition {
            crate::ast::Definition::Table(made) => self.made_table(arena, made, sql),
            crate::ast::Definition::View(made) => self.made_view(made, sql),
            crate::ast::Definition::Index(made) => self.made_index(made, sql),
            crate::ast::Definition::Trigger(made) => self.made_trigger(made, sql),
            crate::ast::Definition::Drop(dropped) => self.dropped(dropped, sql),
            crate::ast::Definition::Rename(altered) => {
                let schema =
                    Self::schema_word(altered.schema, self.temping(altered.schema, sql), sql);
                let table = dequote(altered.table.text(sql));
                self.ask(Action::AlterTable, &schema, &table, b"")
            }
            crate::ast::Definition::AddColumn(altered) => {
                let schema =
                    Self::schema_word(altered.schema, self.temping(altered.schema, sql), sql);
                let table = dequote(altered.table.text(sql));
                self.ask(Action::AlterTable, &schema, &table, b"")
            }
            crate::ast::Definition::DropColumn(altered) => {
                let schema =
                    Self::schema_word(altered.schema, self.temping(altered.schema, sql), sql);
                let table = dequote(altered.table.text(sql));
                let column = dequote(altered.column.text(sql));
                self.ask(Action::AlterTable, &schema, &table, &column)
            }
            crate::ast::Definition::RenameColumn(altered) => {
                let schema =
                    Self::schema_word(altered.schema, self.temping(altered.schema, sql), sql);
                let table = dequote(altered.table.text(sql));
                let column = dequote(altered.column.text(sql));
                self.ask(Action::AlterTable, &schema, &table, &column)
            }
            crate::ast::Definition::DropConstraint(altered) => {
                let schema =
                    Self::schema_word(altered.schema, self.temping(altered.schema, sql), sql);
                let table = dequote(altered.table.text(sql));
                self.ask(Action::AlterTable, &schema, &table, b"")
            }
            crate::ast::Definition::Vacuum(_) => Ok(Answer::Ok),
            // `sqlite3Attach` of `research/sqlite/src/attach.c:393`
            // hands the function the text the statement wrote and
            // nothing where it wrote an expression of its own.
            crate::ast::Definition::Attach(asked) => {
                let file = literal_of(arena, asked.file, sql);
                self.ask(Action::Attach, &file, b"", b"")
            }
            crate::ast::Definition::Detach(asked) => {
                let name = literal_of(arena, asked.name, sql);
                self.ask(Action::Detach, &name, b"", b"")
            }
        }
    }

    /// `CREATE TABLE`, which writes the schema's own table and so is
    /// asked for twice, and reads the statement of a `CREATE TABLE AS`.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    fn made_table(
        &mut self,
        arena: &Arena,
        made: crate::ast::CreateTable,
        sql: &[u8],
    ) -> Result<Answer, Error> {
        let temporary = self.temping(made.schema, sql);
        let schema = Self::schema_word(made.schema, temporary, sql);
        let name = dequote(made.name.text(sql));
        let action = if temporary {
            Action::CreateTempTable
        } else {
            Action::CreateTable
        };
        let answered = self.all(&[
            (Action::Insert, Self::schema_table(temporary), b"", &schema),
            (action, &name, b"", &schema),
        ])?;
        // The statement that reads the rows of a `CREATE TABLE AS` is a
        // statement of its own.
        if let crate::ast::TableBody::Select(select) = made.body {
            self.select(arena, select, sql)?;
        }
        Ok(answered)
    }

    /// `CREATE VIEW`, which writes the schema's own table and so is
    /// asked for twice.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    fn made_view(&self, made: crate::ast::CreateView, sql: &[u8]) -> Result<Answer, Error> {
        let temporary = self.temping(made.schema, sql);
        let schema = Self::schema_word(made.schema, temporary, sql);
        let name = dequote(made.name.text(sql));
        let action = if temporary {
            Action::CreateTempView
        } else {
            Action::CreateView
        };
        self.all(&[
            (Action::Insert, Self::schema_table(temporary), b"", &schema),
            (action, &name, b"", &schema),
        ])
    }

    /// `CREATE INDEX`, which is asked for under the temporary schema
    /// where the table it is over is no table of the schema this
    /// connection holds.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    fn made_index(&self, made: crate::ast::CreateIndex, sql: &[u8]) -> Result<Answer, Error> {
        let temporary = self.temping(made.schema, sql);
        let schema = Self::schema_word(made.schema, temporary, sql);
        let name = dequote(made.name.text(sql));
        let table = dequote(made.table.text(sql));
        let action = if temporary {
            Action::CreateTempIndex
        } else {
            Action::CreateIndex
        };
        self.all(&[
            (Action::Insert, Self::schema_table(temporary), b"", &schema),
            (action, &name, &table, &schema),
        ])
    }

    /// `CREATE TRIGGER`, which is asked for before the write of the
    /// schema's own table and not after it, which is the one order
    /// `sqlite3BeginTrigger` of `research/sqlite/src/trigger.c:250`
    /// writes.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    fn made_trigger(&self, made: crate::ast::CreateTrigger, sql: &[u8]) -> Result<Answer, Error> {
        let temporary = self.temping(made.schema, sql);
        let schema = Self::schema_word(made.schema, temporary, sql);
        let name = dequote(made.name.text(sql));
        let table = dequote(made.table.text(sql));
        let action = if temporary {
            Action::CreateTempTrigger
        } else {
            Action::CreateTrigger
        };
        self.all(&[
            (action, &name, &table, &schema),
            (Action::Insert, Self::schema_table(temporary), b"", &schema),
        ])
    }

    /// `DROP TABLE`, `DROP INDEX`, `DROP VIEW` and `DROP TRIGGER`, each
    /// of which takes a row out of the schema's own table.
    fn dropped(&self, dropped: crate::ast::Drop, sql: &[u8]) -> Result<Answer, Error> {
        let name = dequote(dropped.name.text(sql));
        let temporary = self.temping(dropped.schema, sql);
        let schema = Self::schema_word(dropped.schema, temporary, sql);
        let held = Self::schema_table(temporary);
        match dropped.kind {
            crate::ast::Dropped::Table | crate::ast::Dropped::View => {
                let action = match (dropped.kind, temporary) {
                    (crate::ast::Dropped::View, true) => Action::DropTempView,
                    (crate::ast::Dropped::View, _) => Action::DropView,
                    (_, true) => Action::DropTempTable,
                    (_, _) => Action::DropTable,
                };
                self.all(&[
                    (Action::Delete, held, b"", &schema),
                    (action, &name, b"", &schema),
                    (Action::Delete, &name, b"", &schema),
                ])
            }
            crate::ast::Dropped::Index => {
                let table = self
                    .database
                    .index(&name)
                    .map_or_else(Vec::new, |(index, _)| index.table.clone());
                let action = if temporary {
                    Action::DropTempIndex
                } else {
                    Action::DropIndex
                };
                self.all(&[
                    (Action::Delete, held, b"", &schema),
                    (action, &name, &table, &schema),
                ])
            }
            crate::ast::Dropped::Trigger => {
                let table = self
                    .database
                    .trigger(&name)
                    .map_or_else(Vec::new, |trigger| trigger.table.clone());
                let action = if temporary {
                    Action::DropTempTrigger
                } else {
                    Action::DropTrigger
                };
                self.all(&[
                    (action, &name, &table, &schema),
                    (Action::Delete, held, b"", &schema),
                ])
            }
        }
    }

    /// The schema a definition names, which is `temp` where `TEMP` was
    /// written and `main` where no schema was.
    fn schema_word(schema: Option<Span>, temporary: bool, sql: &[u8]) -> Vec<u8> {
        match schema {
            Some(schema) => dequote(schema.text(sql)),
            None if temporary => b"temp".to_vec(),
            None => b"main".to_vec(),
        }
    }

    /// Whether a statement names the temp schema: the schema it wrote, or
    /// the database the connection writes where it wrote none.
    ///
    /// The connection took the database a statement names as the one it
    /// writes before the function is asked, which D-325 records, so the
    /// name of that database says which schema a bare name stands in.
    fn temping(&self, schema: Option<Span>, sql: &[u8]) -> bool {
        match schema {
            Some(schema) => dequote(schema.text(sql)).eq_ignore_ascii_case(b"temp"),
            None => self.database.main_named().eq_ignore_ascii_case(b"temp"),
        }
    }

    /// `INSERT`, `DELETE` and `UPDATE`, each of which is asked for
    /// against the table it writes, and the statement it reads its rows
    /// from.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the function refused.
    pub(crate) fn change(
        &mut self,
        arena: &Arena,
        change: crate::ast::Change,
        sql: &[u8],
    ) -> Result<Answer, Error> {
        match change {
            crate::ast::Change::Insert(statement) => {
                let schema =
                    Self::schema_word(statement.schema, self.temping(statement.schema, sql), sql);
                let name = dequote(statement.name.text(sql));
                let answered = self.ask(Action::Insert, &name, b"", &schema)?;
                if !statement.defaults {
                    // `sqlite3Insert` reads a `VALUES` itself and hands a
                    // statement to `sqlite3Select`, which asks for
                    // `SQLITE_SELECT` of its own.
                    let values = arena
                        .select(statement.select)
                        .is_some_and(|select| !select.values.is_empty());
                    if values {
                        self.read_select(arena, statement.select, sql)?;
                    } else {
                        self.select(arena, statement.select, sql)?;
                    }
                }
                self.fired(&name, crate::ast::TriggerEvent::Insert)?;
                Ok(answered)
            }
            crate::ast::Change::Delete(statement) => {
                let schema =
                    Self::schema_word(statement.schema, self.temping(statement.schema, sql), sql);
                let name = dequote(statement.name.text(sql));
                // `sqlite3DeleteFrom` of
                // `research/sqlite/src/delete.c:393` keeps the rows where
                // the function ignores the action and only leaves the
                // whole-table shortcut, which this crate has none of.
                let answered = self.ask(Action::Delete, &name, b"", &schema)?;
                self.read_change(arena, statement.name, statement.filter, sql)?;
                self.fired(&name, crate::ast::TriggerEvent::Delete)?;
                Ok(answered)
            }
            crate::ast::Change::Update(statement) => {
                let schema =
                    Self::schema_word(statement.schema, self.temping(statement.schema, sql), sql);
                let name = dequote(statement.name.text(sql));
                let mut answered = Answer::Ok;
                let mut reading = self.reading_change(statement.name, sql);
                for set in arena.sets(statement.sets).to_vec() {
                    // `sqlite3Update` resolves the value the column is
                    // written with before it asks about that column.
                    self.read_expr(arena, &mut reading, set.value, sql)?;
                    let named = dequote(set.column.text(sql));
                    if self.ask(Action::Update, &name, &named, &schema)? == Answer::Ignore {
                        self.unwritten.push(named);
                        answered = Answer::Ignore;
                    }
                }
                if let Some(filter) = statement.filter {
                    self.read_expr(arena, &mut reading, filter, sql)?;
                }
                self.fired(&name, crate::ast::TriggerEvent::Update)?;
                Ok(answered)
            }
        }
    }

    /// The reads of a `WHERE` clause of a statement that writes, which
    /// are the columns of the one table it names.
    fn read_change(
        &mut self,
        arena: &Arena,
        table: Span,
        filter: Option<ExprId>,
        sql: &[u8],
    ) -> Result<Answer, Error> {
        let Some(filter) = filter else {
            return Ok(Answer::Ok);
        };
        let mut reading = self.reading_change(table, sql);
        self.read_expr(arena, &mut reading, filter, sql)
    }

    /// The one table a statement that writes names, and the rows a
    /// trigger's body reads beside it.
    fn reading_change(&self, table: Span, sql: &[u8]) -> Vec<Reading> {
        let named = dequote(table.text(sql));
        let mut out = self.fired.clone();
        let Some((held, _)) = self.database.table(&named) else {
            return out;
        };
        let key = key_named(held);
        out.push(Reading {
            name: held.name.clone(),
            known: named,
            schema: b"main".to_vec(),
            columns: held
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect(),
            key,
            used: false,
        });
        out
    }

    /// The reads of one expression, asked for against `reading`.
    fn read_expr(
        &mut self,
        arena: &Arena,
        reading: &mut [Reading],
        id: ExprId,
        sql: &[u8],
    ) -> Result<Answer, Error> {
        let mut out = Vec::new();
        named_under(arena, id, &mut out);
        for named in out {
            self.read_named(arena, reading, named, sql)?;
        }
        Ok(Answer::Ok)
    }

    /// `SQLITE_PRAGMA`, with the value the pragma was set to and the
    /// schema where one was written, which `sqlite3Pragma` of
    /// `research/sqlite/src/pragma.c:471` asks for.
    ///
    /// # Errors
    ///
    /// [`Error::Denied`] where the function denies the pragma.
    pub(crate) fn pragma(&self, name: &[u8], value: &[u8], schema: &[u8]) -> Result<Answer, Error> {
        self.ask(Action::Pragma, name, value, schema)
    }

    /// `SQLITE_TRANSACTION`, whose third argument is `BEGIN`, `COMMIT`
    /// or `ROLLBACK`.
    ///
    /// # Errors
    ///
    /// [`Error::Denied`] where the function denies the operation.
    pub(crate) fn transaction(&self, word: &[u8]) -> Result<Answer, Error> {
        self.ask(Action::Transaction, word, b"", b"")
    }

    /// `SQLITE_SAVEPOINT`, whose third argument is `BEGIN`, `RELEASE` or
    /// `ROLLBACK` and whose fourth is the name.
    ///
    /// # Errors
    ///
    /// [`Error::Denied`] where the function denies the operation.
    pub(crate) fn savepoint(&self, word: &[u8], name: &[u8]) -> Result<Answer, Error> {
        self.ask(Action::Savepoint, word, name, b"")
    }

    /// `SQLITE_ANALYZE` of one table.
    ///
    /// # Errors
    ///
    /// [`Error::Denied`] where the function denies the table.
    pub(crate) fn analyze(&self, name: &[u8], schema: &[u8]) -> Result<Answer, Error> {
        self.ask(Action::Analyze, name, b"", schema)
    }

    /// `SQLITE_REINDEX` of one index.
    ///
    /// # Errors
    ///
    /// [`Error::Denied`] where the function denies the index.
    pub(crate) fn reindex(&self, name: &[u8], schema: &[u8]) -> Result<Answer, Error> {
        self.ask(Action::Reindex, name, b"", schema)
    }
}

impl Authorizer<'_> {
    /// `SQLITE_FUNCTION` of every function one statement calls, which
    /// `sqlite3ResolveExprNames` of
    /// `research/sqlite/src/resolve.c:1195` asks for where it looks the
    /// name up.
    ///
    /// # Errors
    ///
    /// [`Error::Function`] names the first function the function denied.
    pub(crate) fn functions(&self, arena: &Arena, sql: &[u8]) -> Result<Answer, Error> {
        for (name, _) in arena.calls() {
            let named = dequote(name.text(sql));
            let asked = Asked {
                action: Action::Function,
                first: b"",
                second: &named,
                schema: b"",
                inner: &self.inner,
            };
            if (self.asking)(&asked) == Answer::Deny {
                return Err(Error::Function(named));
            }
        }
        Ok(Answer::Ok)
    }
}

/// The name a bare `rowid` of `table` is asked under: the column the
/// rowid is another name for, and `ROWID` where the table has none, which
/// is what `sqlite3AuthRead` of `research/sqlite/src/auth.c:170` writes.
fn key_named(table: &crate::schema::Table) -> Vec<u8> {
    match table.rowid_alias.and_then(|at| table.columns.get(at)) {
        Some(column) => column.name.clone(),
        None => b"ROWID".to_vec(),
    }
}

/// The text of a string the statement wrote, and nothing where it wrote
/// anything else.
///
/// `sqlite3Attach` of `research/sqlite/src/attach.c:387` hands the
/// function `pAuthArg->u.zToken` for a `TK_STRING` and a null pointer
/// otherwise, which the empty slice stands for here.
fn literal_of(arena: &Arena, id: ExprId, sql: &[u8]) -> Vec<u8> {
    match arena.node(id) {
        Some(Node::Literal(crate::ast::Literal::Text(span))) => dequote(span.text(sql)),
        _ => Vec::new(),
    }
}
