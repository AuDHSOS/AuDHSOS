// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The keywords of SQL, as SQLite spells them.
//!
//! The list is the one `tool/mkkeywordhash.c` of the SQLite source
//! carries, in the order it carries them, which is alphabetical. A
//! keyword is matched without regard to case, and the table is searched
//! by halving, so a word costs O(log n) comparisons of at most its own
//! length.

/// One keyword.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Keyword {
    /// `ABORT`.
    Abort,
    /// `ACTION`.
    Action,
    /// `ADD`.
    Add,
    /// `AFTER`.
    After,
    /// `ALL`.
    All,
    /// `ALTER`.
    Alter,
    /// `ALWAYS`.
    Always,
    /// `ANALYZE`.
    Analyze,
    /// `AND`.
    And,
    /// `AS`.
    As,
    /// `ASC`.
    Asc,
    /// `ATTACH`.
    Attach,
    /// `AUTOINCREMENT`.
    Autoincrement,
    /// `BEFORE`.
    Before,
    /// `BEGIN`.
    Begin,
    /// `BETWEEN`.
    Between,
    /// `BY`.
    By,
    /// `CASCADE`.
    Cascade,
    /// `CASE`.
    Case,
    /// `CAST`.
    Cast,
    /// `CHECK`.
    Check,
    /// `COLLATE`.
    Collate,
    /// `COLUMN`.
    Column,
    /// `COMMIT`.
    Commit,
    /// `CONFLICT`.
    Conflict,
    /// `CONSTRAINT`.
    Constraint,
    /// `CREATE`.
    Create,
    /// `CROSS`.
    Cross,
    /// `CURRENT`.
    Current,
    /// `CURRENT_DATE`.
    CurrentDate,
    /// `CURRENT_TIME`.
    CurrentTime,
    /// `CURRENT_TIMESTAMP`.
    CurrentTimestamp,
    /// `DATABASE`.
    Database,
    /// `DEFAULT`.
    Default,
    /// `DEFERRED`.
    Deferred,
    /// `DEFERRABLE`.
    Deferrable,
    /// `DELETE`.
    Delete,
    /// `DESC`.
    Desc,
    /// `DETACH`.
    Detach,
    /// `DISTINCT`.
    Distinct,
    /// `DO`.
    Do,
    /// `DROP`.
    Drop,
    /// `END`.
    End,
    /// `EACH`.
    Each,
    /// `ELSE`.
    Else,
    /// `ESCAPE`.
    Escape,
    /// `EXCEPT`.
    Except,
    /// `EXCLUSIVE`.
    Exclusive,
    /// `EXCLUDE`.
    Exclude,
    /// `EXISTS`.
    Exists,
    /// `EXPLAIN`.
    Explain,
    /// `FAIL`.
    Fail,
    /// `FILTER`.
    Filter,
    /// `FIRST`.
    First,
    /// `FOLLOWING`.
    Following,
    /// `FOR`.
    For,
    /// `FOREIGN`.
    Foreign,
    /// `FROM`.
    From,
    /// `FULL`.
    Full,
    /// `GENERATED`.
    Generated,
    /// `GLOB`.
    Glob,
    /// `GROUP`.
    Group,
    /// `GROUPS`.
    Groups,
    /// `HAVING`.
    Having,
    /// `IF`.
    If,
    /// `IGNORE`.
    Ignore,
    /// `IMMEDIATE`.
    Immediate,
    /// `IN`.
    In,
    /// `INDEX`.
    Index,
    /// `INDEXED`.
    Indexed,
    /// `INITIALLY`.
    Initially,
    /// `INNER`.
    Inner,
    /// `INSERT`.
    Insert,
    /// `INSTEAD`.
    Instead,
    /// `INTERSECT`.
    Intersect,
    /// `INTO`.
    Into,
    /// `IS`.
    Is,
    /// `ISNULL`.
    Isnull,
    /// `JOIN`.
    Join,
    /// `KEY`.
    Key,
    /// `LAST`.
    Last,
    /// `LEFT`.
    Left,
    /// `LIKE`.
    Like,
    /// `LIMIT`.
    Limit,
    /// `MATCH`.
    Match,
    /// `MATERIALIZED`.
    Materialized,
    /// `NATURAL`.
    Natural,
    /// `NO`.
    No,
    /// `NOT`.
    Not,
    /// `NOTHING`.
    Nothing,
    /// `NOTNULL`.
    Notnull,
    /// `NULL`.
    Null,
    /// `NULLS`.
    Nulls,
    /// `OF`.
    Of,
    /// `OFFSET`.
    Offset,
    /// `ON`.
    On,
    /// `OR`.
    Or,
    /// `ORDER`.
    Order,
    /// `OTHERS`.
    Others,
    /// `OUTER`.
    Outer,
    /// `OVER`.
    Over,
    /// `PARTITION`.
    Partition,
    /// `PLAN`.
    Plan,
    /// `PRAGMA`.
    Pragma,
    /// `PRECEDING`.
    Preceding,
    /// `PRIMARY`.
    Primary,
    /// `QUERY`.
    Query,
    /// `RAISE`.
    Raise,
    /// `RANGE`.
    Range,
    /// `RECURSIVE`.
    Recursive,
    /// `REFERENCES`.
    References,
    /// `REGEXP`.
    Regexp,
    /// `REINDEX`.
    Reindex,
    /// `RELEASE`.
    Release,
    /// `RENAME`.
    Rename,
    /// `REPLACE`.
    Replace,
    /// `RESTRICT`.
    Restrict,
    /// `RETURNING`.
    Returning,
    /// `RIGHT`.
    Right,
    /// `ROLLBACK`.
    Rollback,
    /// `ROW`.
    Row,
    /// `ROWS`.
    Rows,
    /// `SAVEPOINT`.
    Savepoint,
    /// `SELECT`.
    Select,
    /// `SET`.
    Set,
    /// `TABLE`.
    Table,
    /// `TEMP`.
    Temp,
    /// `TEMPORARY`.
    Temporary,
    /// `THEN`.
    Then,
    /// `TIES`.
    Ties,
    /// `TO`.
    To,
    /// `TRANSACTION`.
    Transaction,
    /// `TRIGGER`.
    Trigger,
    /// `UNBOUNDED`.
    Unbounded,
    /// `UNION`.
    Union,
    /// `UNIQUE`.
    Unique,
    /// `UPDATE`.
    Update,
    /// `USING`.
    Using,
    /// `VACUUM`.
    Vacuum,
    /// `VALUES`.
    Values,
    /// `VIEW`.
    View,
    /// `VIRTUAL`.
    Virtual,
    /// `WHEN`.
    When,
    /// `WHERE`.
    Where,
    /// `WINDOW`.
    Window,
    /// `WITH`.
    With,
    /// `WITHIN`.
    Within,
    /// `WITHOUT`.
    Without,
}

/// Every keyword with the word it is written as, sorted, which is what
/// lets the lookup halve the table.
const KEYWORDS: [(&[u8], Keyword); 148] = [
    (b"ABORT", Keyword::Abort),
    (b"ACTION", Keyword::Action),
    (b"ADD", Keyword::Add),
    (b"AFTER", Keyword::After),
    (b"ALL", Keyword::All),
    (b"ALTER", Keyword::Alter),
    (b"ALWAYS", Keyword::Always),
    (b"ANALYZE", Keyword::Analyze),
    (b"AND", Keyword::And),
    (b"AS", Keyword::As),
    (b"ASC", Keyword::Asc),
    (b"ATTACH", Keyword::Attach),
    (b"AUTOINCREMENT", Keyword::Autoincrement),
    (b"BEFORE", Keyword::Before),
    (b"BEGIN", Keyword::Begin),
    (b"BETWEEN", Keyword::Between),
    (b"BY", Keyword::By),
    (b"CASCADE", Keyword::Cascade),
    (b"CASE", Keyword::Case),
    (b"CAST", Keyword::Cast),
    (b"CHECK", Keyword::Check),
    (b"COLLATE", Keyword::Collate),
    (b"COLUMN", Keyword::Column),
    (b"COMMIT", Keyword::Commit),
    (b"CONFLICT", Keyword::Conflict),
    (b"CONSTRAINT", Keyword::Constraint),
    (b"CREATE", Keyword::Create),
    (b"CROSS", Keyword::Cross),
    (b"CURRENT", Keyword::Current),
    (b"CURRENT_DATE", Keyword::CurrentDate),
    (b"CURRENT_TIME", Keyword::CurrentTime),
    (b"CURRENT_TIMESTAMP", Keyword::CurrentTimestamp),
    (b"DATABASE", Keyword::Database),
    (b"DEFAULT", Keyword::Default),
    (b"DEFERRABLE", Keyword::Deferrable),
    (b"DEFERRED", Keyword::Deferred),
    (b"DELETE", Keyword::Delete),
    (b"DESC", Keyword::Desc),
    (b"DETACH", Keyword::Detach),
    (b"DISTINCT", Keyword::Distinct),
    (b"DO", Keyword::Do),
    (b"DROP", Keyword::Drop),
    (b"EACH", Keyword::Each),
    (b"ELSE", Keyword::Else),
    (b"END", Keyword::End),
    (b"ESCAPE", Keyword::Escape),
    (b"EXCEPT", Keyword::Except),
    (b"EXCLUDE", Keyword::Exclude),
    (b"EXCLUSIVE", Keyword::Exclusive),
    (b"EXISTS", Keyword::Exists),
    (b"EXPLAIN", Keyword::Explain),
    (b"FAIL", Keyword::Fail),
    (b"FILTER", Keyword::Filter),
    (b"FIRST", Keyword::First),
    (b"FOLLOWING", Keyword::Following),
    (b"FOR", Keyword::For),
    (b"FOREIGN", Keyword::Foreign),
    (b"FROM", Keyword::From),
    (b"FULL", Keyword::Full),
    (b"GENERATED", Keyword::Generated),
    (b"GLOB", Keyword::Glob),
    (b"GROUP", Keyword::Group),
    (b"GROUPS", Keyword::Groups),
    (b"HAVING", Keyword::Having),
    (b"IF", Keyword::If),
    (b"IGNORE", Keyword::Ignore),
    (b"IMMEDIATE", Keyword::Immediate),
    (b"IN", Keyword::In),
    (b"INDEX", Keyword::Index),
    (b"INDEXED", Keyword::Indexed),
    (b"INITIALLY", Keyword::Initially),
    (b"INNER", Keyword::Inner),
    (b"INSERT", Keyword::Insert),
    (b"INSTEAD", Keyword::Instead),
    (b"INTERSECT", Keyword::Intersect),
    (b"INTO", Keyword::Into),
    (b"IS", Keyword::Is),
    (b"ISNULL", Keyword::Isnull),
    (b"JOIN", Keyword::Join),
    (b"KEY", Keyword::Key),
    (b"LAST", Keyword::Last),
    (b"LEFT", Keyword::Left),
    (b"LIKE", Keyword::Like),
    (b"LIMIT", Keyword::Limit),
    (b"MATCH", Keyword::Match),
    (b"MATERIALIZED", Keyword::Materialized),
    (b"NATURAL", Keyword::Natural),
    (b"NO", Keyword::No),
    (b"NOT", Keyword::Not),
    (b"NOTHING", Keyword::Nothing),
    (b"NOTNULL", Keyword::Notnull),
    (b"NULL", Keyword::Null),
    (b"NULLS", Keyword::Nulls),
    (b"OF", Keyword::Of),
    (b"OFFSET", Keyword::Offset),
    (b"ON", Keyword::On),
    (b"OR", Keyword::Or),
    (b"ORDER", Keyword::Order),
    (b"OTHERS", Keyword::Others),
    (b"OUTER", Keyword::Outer),
    (b"OVER", Keyword::Over),
    (b"PARTITION", Keyword::Partition),
    (b"PLAN", Keyword::Plan),
    (b"PRAGMA", Keyword::Pragma),
    (b"PRECEDING", Keyword::Preceding),
    (b"PRIMARY", Keyword::Primary),
    (b"QUERY", Keyword::Query),
    (b"RAISE", Keyword::Raise),
    (b"RANGE", Keyword::Range),
    (b"RECURSIVE", Keyword::Recursive),
    (b"REFERENCES", Keyword::References),
    (b"REGEXP", Keyword::Regexp),
    (b"REINDEX", Keyword::Reindex),
    (b"RELEASE", Keyword::Release),
    (b"RENAME", Keyword::Rename),
    (b"REPLACE", Keyword::Replace),
    (b"RESTRICT", Keyword::Restrict),
    (b"RETURNING", Keyword::Returning),
    (b"RIGHT", Keyword::Right),
    (b"ROLLBACK", Keyword::Rollback),
    (b"ROW", Keyword::Row),
    (b"ROWS", Keyword::Rows),
    (b"SAVEPOINT", Keyword::Savepoint),
    (b"SELECT", Keyword::Select),
    (b"SET", Keyword::Set),
    (b"TABLE", Keyword::Table),
    (b"TEMP", Keyword::Temp),
    (b"TEMPORARY", Keyword::Temporary),
    (b"THEN", Keyword::Then),
    (b"TIES", Keyword::Ties),
    (b"TO", Keyword::To),
    (b"TRANSACTION", Keyword::Transaction),
    (b"TRIGGER", Keyword::Trigger),
    (b"UNBOUNDED", Keyword::Unbounded),
    (b"UNION", Keyword::Union),
    (b"UNIQUE", Keyword::Unique),
    (b"UPDATE", Keyword::Update),
    (b"USING", Keyword::Using),
    (b"VACUUM", Keyword::Vacuum),
    (b"VALUES", Keyword::Values),
    (b"VIEW", Keyword::View),
    (b"VIRTUAL", Keyword::Virtual),
    (b"WHEN", Keyword::When),
    (b"WHERE", Keyword::Where),
    (b"WINDOW", Keyword::Window),
    (b"WITH", Keyword::With),
    (b"WITHIN", Keyword::Within),
    (b"WITHOUT", Keyword::Without),
];

impl Keyword {
    /// The word, in the upper case SQLite writes it in.
    #[must_use]
    pub fn word(self) -> &'static [u8] {
        KEYWORDS
            .iter()
            .find(|(_, keyword)| *keyword == self)
            .map_or(b"".as_slice(), |(word, _)| word)
    }
}

/// The keyword `word` spells, whatever case it is written in.
///
/// The table is halved rather than indexed: what is left of it is a
/// slice, and the walk ends when that slice is empty, which is the one
/// way a word that is no keyword ends.
#[must_use]
pub fn lookup(word: &[u8]) -> Option<Keyword> {
    let mut table: &[(&[u8], Keyword)] = KEYWORDS.as_slice();
    loop {
        let middle = table.len() / 2;
        let (candidate, keyword) = *table.get(middle)?;
        match compare(word, candidate) {
            core::cmp::Ordering::Equal => return Some(keyword),
            core::cmp::Ordering::Less => table = table.get(..middle).unwrap_or_default(),
            core::cmp::Ordering::Greater => {
                table = table.get(middle.saturating_add(1)..).unwrap_or_default();
            }
        }
    }
}

/// Compares a word against a keyword, ignoring the case of the word. The
/// keywords are ASCII, so folding one byte at a time is the whole rule.
fn compare(word: &[u8], keyword: &[u8]) -> core::cmp::Ordering {
    for (left, right) in word.iter().zip(keyword) {
        let folded = left.to_ascii_uppercase();
        match folded.cmp(right) {
            core::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    word.len().cmp(&keyword.len())
}
