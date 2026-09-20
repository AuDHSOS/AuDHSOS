// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The matcher of `research/sqlite/ext/misc/regexp.c`, which the
//! `regexp` extension registers as `regexp(P,S)` and `regexpi(P,S)`.
//!
//! A pattern compiles to a non-deterministic automaton whose states are
//! the steps of a program. A match holds every state the text has
//! reached so far and steps all of them over one character at a time,
//! so a match costs O(n*m) in the states of the program and the
//! characters of the text and never backtracks.
//!
//! The program here holds a character class in one step where the C
//! program holds one step per value and one more per range; the two
//! agree on what they match, because nothing in the C program names a
//! state inside a class.

use alloc::vec::Vec;

/// The character a reader answers past the end of the text, which is
/// the NUL a C string ends with.
const EOF: i64 = 0;

/// The character standing before the first one, which `^` matches
/// after. It is larger than any character UTF-8 carries.
const START: i64 = 0xfff_ffff;

/// The longest pattern the matcher takes, which is
/// `SQLITE_LIMIT_LIKE_PATTERN_LENGTH`.
pub const PATTERN: usize = 50_000;

/// The most states a program holds, which is `re_maxnfa` of
/// [`PATTERN`]: 75 plus half the pattern.
const STATES: usize = 25_075;

/// One value or one range of values a character class holds.
#[derive(Clone)]
struct Span {
    /// The first character of the range.
    low: i64,
    /// The last character of the range, which is `low` for one value.
    high: i64,
}

/// One step of the program, which is one state of the automaton.
#[derive(Clone)]
enum Step {
    /// `RE_OP_MATCH`: the one character it carries.
    Match(i64),
    /// `RE_OP_ANY`: any one character, which `.` writes.
    Any,
    /// `RE_OP_ANYSTAR`: any run of characters, which `.*` writes.
    AnyStar,
    /// `RE_OP_FORK`: the step it names as well as the next one.
    Fork(i64),
    /// `RE_OP_GOTO`: the step it names.
    Goto(i64),
    /// `RE_OP_ACCEPT`: the text matches.
    Accept,
    /// `RE_OP_CC_INC` and `RE_OP_CC_EXC`: a character the spans hold,
    /// or one they do not where the flag stands.
    Class(bool, Vec<Span>),
    /// `RE_OP_WORD`: `[A-Za-z0-9_]`.
    Word,
    /// `RE_OP_NOTWORD`: a character that is not a word character.
    NotWord,
    /// `RE_OP_DIGIT`: `[0-9]`.
    Digit,
    /// `RE_OP_NOTDIGIT`: a character that is not a digit.
    NotDigit,
    /// `RE_OP_SPACE`: `[ \t\n\r\x0b\x0c]`.
    Space,
    /// `RE_OP_NOTSPACE`: a character that is not a space.
    NotSpace,
    /// `RE_OP_BOUNDARY`: the place between a word character and one
    /// that is not, which `\b` writes.
    Boundary,
    /// `RE_OP_ATSTART`: the place before the first character, which `^`
    /// writes.
    AtStart,
}

/// A pattern read into a program, which [`compile`] answers.
pub struct Compiled {
    /// The steps of the program.
    steps: Vec<Step>,
    /// The bytes every match begins with, where the program begins with
    /// [`Step::AnyStar`] and characters follow it. A match skips ahead
    /// to those bytes rather than stepping the automaton over every
    /// place, which is `zInit`.
    init: Vec<u8>,
    /// Whether a capital and its small letter are one character.
    nocase: bool,
}

impl Compiled {
    /// Whether `text` holds a run the pattern matches, which is
    /// `re_match`.
    ///
    /// It costs O(n*m) in the steps of the program and the characters
    /// of `text`.
    #[must_use]
    pub fn matches(&self, text: &[u8]) -> bool {
        let mut at = self.skipped(text);
        if at > text.len() {
            return false;
        }
        // `re_match` stands the reader before the first character,
        // where `^` matches, and one place later where the bytes every
        // match begins with were skipped to.
        let mut held = if self.init.is_empty() {
            START
        } else {
            START.saturating_sub(1)
        };
        let mut this: Vec<usize> = Vec::new();
        let mut next: Vec<usize> = alloc::vec![0];
        while held != EOF && !next.is_empty() {
            let prev = held;
            let (read, after) = self.read(text, at);
            held = read;
            at = after;
            core::mem::swap(&mut this, &mut next);
            next.clear();
            let mut index = 0;
            while let Some(state) = this.get(index).copied() {
                if self.step(state, held, prev, &mut this, &mut next) {
                    return true;
                }
                index = index.saturating_add(1);
            }
        }
        next.iter()
            .any(|state| matches!(self.followed(*state), Some(Step::Accept)))
    }

    /// Where the first run of [`Compiled::init`] begins, or one past
    /// the end of `text` where the text holds none.
    fn skipped(&self, text: &[u8]) -> usize {
        if self.init.is_empty() {
            return 0;
        }
        let mut at = 0_usize;
        while at.saturating_add(self.init.len()) <= text.len() {
            if text
                .get(at..)
                .is_some_and(|rest| rest.starts_with(&self.init))
            {
                return at;
            }
            at = at.saturating_add(1);
        }
        text.len().saturating_add(1)
    }

    /// The step `state` ends on, following every [`Step::Goto`] from
    /// it, which is how `re_match` reads the states left at the end.
    ///
    /// A [`Step::Goto`] names a later step, so the walk ends.
    fn followed(&self, state: usize) -> Option<&Step> {
        let mut at = state;
        while let Some(Step::Goto(by)) = self.steps.get(at) {
            at = jumped(at, *by);
        }
        self.steps.get(at)
    }

    /// The character at `at` and where the one after it begins, under
    /// the letter case the pattern was read in.
    fn read(&self, text: &[u8], at: usize) -> (i64, usize) {
        let (held, after) = character(text, at);
        if self.nocase && (0x41..=0x5a).contains(&held) {
            return (held.saturating_add(0x20), after);
        }
        (held, after)
    }

    /// Steps the state `state` over the character `held`, whose
    /// forerunner is `prev`, and answers whether the text matches.
    ///
    /// A step that reads no character names the states it reaches in
    /// `this`, which the walk over `this` reaches later in this pass; a
    /// step that reads one names them in `next`.
    fn step(
        &self,
        state: usize,
        held: i64,
        prev: i64,
        this: &mut Vec<usize>,
        next: &mut Vec<usize>,
    ) -> bool {
        // A state the program does not hold names itself, which adds
        // nothing, because the walk holds each state once.
        let step = self.steps.get(state).unwrap_or(&Step::Goto(0));
        let after = state.saturating_add(1);
        match step {
            Step::Match(wanted) => reach(next, after, *wanted == held),
            Step::Any => reach(next, after, held != EOF),
            Step::Word => reach(next, after, word(held)),
            Step::NotWord => reach(next, after, !word(held) && held != EOF),
            Step::Digit => reach(next, after, digit(held)),
            Step::NotDigit => reach(next, after, !digit(held) && held != EOF),
            Step::Space => reach(next, after, space(held)),
            Step::NotSpace => reach(next, after, !space(held) && held != EOF),
            Step::Class(outside, spans) => {
                let hit = spans
                    .iter()
                    .any(|span| span.low <= held && span.high >= held);
                reach(
                    next,
                    after,
                    if *outside { !hit && held != EOF } else { hit },
                );
            }
            Step::AtStart => reach(this, after, prev == START),
            Step::Boundary => reach(this, after, word(held) != word(prev)),
            Step::AnyStar => {
                reach(next, state, true);
                reach(this, after, true);
            }
            Step::Fork(by) => {
                reach(this, jumped(state, *by), true);
                reach(this, after, true);
            }
            Step::Goto(by) => reach(this, jumped(state, *by), true),
            Step::Accept => return true,
        }
        false
    }
}

/// Reads `pattern` into a program, where `nocase` makes a capital and
/// its small letter one character.
///
/// This is `re_compile` and the length `re_sql_func` refuses a longer
/// pattern under. Reading costs O(n) in the characters of the pattern,
/// except that `X{m,n}` copies what it repeats.
///
/// # Errors
///
/// The message the C library writes for a pattern it does not read,
/// which is the text of the refusal.
pub fn compile(pattern: &[u8], nocase: bool) -> Result<Compiled, &'static str> {
    if pattern.len() > PATTERN {
        return Err("REGEXP pattern too big");
    }
    // `re_compile` writes `.*` before a pattern that does not begin at
    // the start of the text, which is what makes the match a search.
    let anchored = pattern.first() == Some(&b'^');
    let mut compiler = Compiler {
        text: if anchored {
            pattern.get(1..).unwrap_or_default()
        } else {
            pattern
        },
        at: 0,
        nocase,
        steps: if anchored {
            Vec::new()
        } else {
            alloc::vec![Step::AnyStar]
        },
        alloc: 30,
        error: None,
    };
    compiler.alternatives()?;
    if compiler.at < compiler.text.len() {
        return Err("unrecognized character");
    }
    compiler.append(Step::Accept);
    if let Some(why) = compiler.error {
        return Err(why);
    }
    let init = initial(&compiler.steps, nocase);
    Ok(Compiled {
        steps: compiler.steps,
        init,
        nocase,
    })
}

/// The bytes every match of `steps` begins with, which is `zInit`: the
/// characters the program matches one after another where it begins
/// with [`Step::AnyStar`], written as UTF-8 and cut to ten bytes.
fn initial(steps: &[Step], nocase: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if nocase || !matches!(steps.first(), Some(Step::AnyStar)) {
        return out;
    }
    let mut at = 1;
    while out.len() < 10 {
        let Some(Step::Match(held)) = steps.get(at) else {
            break;
        };
        // `re_compile` writes no character past the first plane, which
        // is rare enough that skipping ahead is worth nothing there.
        let Some(held) = u32::try_from(*held)
            .ok()
            .filter(|value| *value <= 0xffff)
            .and_then(char::from_u32)
        else {
            break;
        };
        let mut written = [0_u8; 4];
        out.extend_from_slice(held.encode_utf8(&mut written).as_bytes());
        at = at.saturating_add(1);
    }
    // A match of the NUL character stands for the end of the text,
    // which the bytes to skip to cannot carry.
    if out.last() == Some(&0) {
        out.pop();
    }
    out
}

/// Names `state` in `states` where `when` stands, which is
/// `re_add_state`: a state the set already holds is named once.
fn reach(states: &mut Vec<usize>, state: usize, when: bool) {
    if when && !states.contains(&state) {
        states.push(state);
    }
}

/// The state `by` steps from `at` name.
fn jumped(at: usize, by: i64) -> usize {
    i64::try_from(at)
        .unwrap_or(0)
        .saturating_add(by)
        .try_into()
        .unwrap_or(0)
}

/// Whether `held` is a word character, which is `re_word_char`.
const fn word(held: i64) -> bool {
    matches!(held, 0x30..=0x39 | 0x41..=0x5a | 0x5f | 0x61..=0x7a)
}

/// Whether `held` is a digit, which is `re_digit_char`.
const fn digit(held: i64) -> bool {
    matches!(held, 0x30..=0x39)
}

/// Whether `held` is a space character, which is `re_space_char`.
const fn space(held: i64) -> bool {
    matches!(held, 0x09..=0x0d | 0x20)
}

/// The character at `at` and where the one after it begins, which is
/// `re_next_char`: a sequence that is too short, an overlong one, a
/// surrogate and a value past the last plane all answer the
/// replacement character.
fn character(text: &[u8], at: usize) -> (i64, usize) {
    let Some(lead) = text.get(at).copied() else {
        return (EOF, at);
    };
    let next = at.saturating_add(1);
    if lead < 0x80 {
        return (i64::from(lead), next);
    }
    let carries = |step: usize| {
        text.get(next.saturating_add(step))
            .is_some_and(|held| held & 0xc0 == 0x80)
    };
    let bits =
        |step: usize| u32::from(text.get(next.saturating_add(step)).copied().unwrap_or(0) & 0x3f);
    let (value, after) = if lead & 0xe0 == 0xc0 && carries(0) {
        let value = u32::from(lead & 0x1f) << 6 | bits(0);
        (
            if value < 0x80 {
                crate::utf8::REPLACEMENT
            } else {
                value
            },
            next.saturating_add(1),
        )
    } else if lead & 0xf0 == 0xe0 && carries(0) && carries(1) {
        let value = u32::from(lead & 0x0f) << 12 | bits(0) << 6 | bits(1);
        (
            if value <= 0x7ff || (0xd800..=0xdfff).contains(&value) {
                crate::utf8::REPLACEMENT
            } else {
                value
            },
            next.saturating_add(2),
        )
    } else if lead & 0xf8 == 0xf0 && carries(0) && carries(1) && carries(2) {
        let value = u32::from(lead & 0x07) << 18 | bits(0) << 12 | bits(1) << 6 | bits(2);
        (
            if value <= 0xffff || value > 0x10_ffff {
                crate::utf8::REPLACEMENT
            } else {
                value
            },
            next.saturating_add(3),
        )
    } else {
        (crate::utf8::REPLACEMENT, next)
    };
    (i64::from(value), after)
}

/// The characters a `\` escape stands for, which is `zEsc`. The first
/// six are written differently, in [`TRANSLATED`].
const ESCAPED: &[u8; 21] = b"afnrtv\\()*.+?[$^{|}]-";

/// What the first six of [`ESCAPED`] stand for, which is `zTrans`.
const TRANSLATED: &[u8; 6] = b"\x07\x0c\x0a\x0d\x09\x0b";

/// A pattern being read into a program.
struct Compiler<'a> {
    /// The text of the pattern, past a `^` that begins it.
    text: &'a [u8],
    /// Where the next character is read.
    at: usize,
    /// Whether a capital and its small letter are one character.
    nocase: bool,
    /// The steps read so far.
    steps: Vec<Step>,
    /// How many steps the program has room for, which doubles as the
    /// program grows. `re_resize` refuses a program that would need
    /// room for more than [`STATES`] steps, so the limit stands at the
    /// last doubling below it rather than at [`STATES`] itself.
    alloc: usize,
    /// What the pattern is refused for, where a `\` escape named no
    /// character; `re_compile` answers it after the whole pattern is
    /// read.
    error: Option<&'static str>,
}

impl Compiler<'_> {
    /// The byte at the reader, or nought at the end, which is
    /// `rePeek`.
    fn peek(&self) -> u8 {
        self.text.get(self.at).copied().unwrap_or(0)
    }

    /// The character at the reader, under the letter case the pattern
    /// is read in, which steps the reader past it.
    fn next(&mut self) -> i64 {
        let (held, after) = character(self.text, self.at);
        self.at = after;
        if self.nocase && (0x41..=0x5a).contains(&held) {
            return held.saturating_add(0x20);
        }
        held
    }

    /// Writes `step` at the end of the program and answers where it
    /// stands, which is `re_append`.
    fn append(&mut self, step: Step) -> usize {
        self.insert(self.steps.len(), step)
    }

    /// Makes room for `wanted` steps, which is `re_resize`, and
    /// answers whether the program may hold that many.
    const fn resize(&mut self, wanted: usize) -> bool {
        if wanted > STATES {
            self.error = Some("REGEXP pattern too big");
            return false;
        }
        self.alloc = wanted;
        true
    }

    /// Writes `step` at `before`, moving the steps from there on one
    /// place along, and answers where it stands, which is `re_insert`.
    fn insert(&mut self, before: usize, step: Step) -> usize {
        if self.alloc <= self.steps.len() && !self.resize(self.alloc.saturating_mul(2)) {
            return 0;
        }
        self.steps.insert(before.min(self.steps.len()), step);
        before
    }

    /// Writes the `count` steps from `from` at the end of the program,
    /// which is `re_copy`.
    fn copy(&mut self, from: usize, count: usize) {
        if self.steps.len().saturating_add(count) >= self.alloc
            && !self.resize(self.alloc.saturating_mul(2).saturating_add(count))
        {
            return;
        }
        let held: Vec<Step> = self
            .steps
            .get(from..from.saturating_add(count))
            .unwrap_or_default()
            .to_vec();
        self.steps.extend(held);
    }

    /// Reads the alternatives of `X|Y|Z` up to the first `)` that
    /// closes none of them, which is `re_subcompile_re`.
    fn alternatives(&mut self) -> Result<(), &'static str> {
        let start = self.steps.len();
        self.sequence()?;
        while self.peek() == b'|' {
            let end = self.steps.len();
            let over = end.saturating_add(2).saturating_sub(start);
            self.insert(start, Step::Fork(whole(over)));
            self.at = self.at.saturating_add(1);
            // The step that leaves the alternative before this one is
            // written after this one is read, where its target is
            // known; the steps of this one are named one against
            // another, so writing a step before all of them leaves
            // every name standing.
            let held = self.steps.len();
            self.sequence()?;
            let over = self.steps.len().saturating_add(1).saturating_sub(held);
            self.insert(held, Step::Goto(whole(over)));
        }
        Ok(())
    }

    /// Reads one alternative, which is `re_subcompile_string`: every
    /// element up to a `|` or a `)`.
    fn sequence(&mut self) -> Result<(), &'static str> {
        let mut last: Option<usize> = None;
        loop {
            let held = self.next();
            if held == EOF {
                return Ok(());
            }
            let start = self.steps.len();
            // A character past the first 128 is written as itself,
            // which the narrowing below sends to the last arm.
            match u8::try_from(held).unwrap_or(0) {
                b'|' | b')' => {
                    self.at = self.at.saturating_sub(1);
                    return Ok(());
                }
                b'(' => {
                    self.alternatives()?;
                    if self.peek() != b')' {
                        return Err("unmatched '('");
                    }
                    self.at = self.at.saturating_add(1);
                }
                b'.' => {
                    if self.peek() == b'*' {
                        self.append(Step::AnyStar);
                        self.at = self.at.saturating_add(1);
                    } else {
                        self.append(Step::Any);
                    }
                }
                b'*' => self.star(last)?,
                b'+' => self.plus(last)?,
                b'?' => self.maybe(last)?,
                b'$' => {
                    self.append(Step::Match(EOF));
                }
                b'^' => {
                    self.append(Step::AtStart);
                }
                b'{' => self.repeat(last)?,
                b'[' => self.class()?,
                b'\\' => self.escape(),
                _ => {
                    self.append(Step::Match(held));
                }
            }
            last = Some(start);
        }
    }

    /// `X*`: the steps of `X` run none or more times.
    fn star(&mut self, last: Option<usize>) -> Result<(), &'static str> {
        let Some(last) = last else {
            return Err("'*' without operand");
        };
        let over = self.steps.len().saturating_sub(last).saturating_add(1);
        self.insert(last, Step::Goto(whole(over)));
        let at = self.steps.len();
        self.append(Step::Fork(between(at, last.saturating_add(1))));
        Ok(())
    }

    /// `X+`: the steps of `X` run once or more.
    fn plus(&mut self, last: Option<usize>) -> Result<(), &'static str> {
        let Some(last) = last else {
            return Err("'+' without operand");
        };
        let at = self.steps.len();
        self.append(Step::Fork(between(at, last)));
        Ok(())
    }

    /// `X?`: the steps of `X` run none or once.
    fn maybe(&mut self, last: Option<usize>) -> Result<(), &'static str> {
        let Some(last) = last else {
            return Err("'?' without operand");
        };
        let over = self.steps.len().saturating_sub(last).saturating_add(1);
        self.insert(last, Step::Fork(whole(over)));
        Ok(())
    }

    /// `X{m,n}`: the steps of `X` run between `m` and `n` times, which
    /// the reader writes out as copies of `X`.
    fn repeat(&mut self, from: Option<usize>) -> Result<(), &'static str> {
        let Some(mut from) = from else {
            return Err("'{m,n}' without operand");
        };
        let fewest = self.digits()?;
        let mut most = fewest;
        if self.peek() == b',' {
            self.at = self.at.saturating_add(1);
            most = self.digits()?;
        }
        if self.peek() != b'}' {
            return Err("unmatched '{'");
        }
        if most < fewest {
            return Err("n less than m in '{m,n}'");
        }
        self.at = self.at.saturating_add(1);
        let size = self.steps.len().saturating_sub(from);
        if fewest == 0 {
            if most == 0 {
                return Err("both m and n are zero in '{m,n}'");
            }
            self.insert(from, Step::Fork(whole(size.saturating_add(1))));
            from = from.saturating_add(1);
            most = most.saturating_sub(1);
        } else {
            for _ in 1..fewest {
                self.copy(from, size);
            }
        }
        for _ in fewest..most {
            self.append(Step::Fork(whole(size.saturating_add(1))));
            self.copy(from, size);
        }
        Ok(())
    }

    /// The number at the reader, which is how many times `X{m,n}`
    /// repeats.
    ///
    /// # Errors
    ///
    /// A number whose copies would hold more states than a program
    /// takes.
    fn digits(&mut self) -> Result<usize, &'static str> {
        let mut held = 0_usize;
        while self.peek().is_ascii_digit() {
            let value = usize::from(self.peek().wrapping_sub(b'0'));
            held = held.saturating_mul(10).saturating_add(value);
            if held.saturating_mul(2) > STATES {
                return Err("REGEXP pattern too big");
            }
            self.at = self.at.saturating_add(1);
        }
        Ok(held)
    }

    /// `[abc]` and `[^abc]`: one character the spans hold, or one they
    /// do not.
    fn class(&mut self) -> Result<(), &'static str> {
        let outside = self.peek() == b'^';
        if outside {
            self.at = self.at.saturating_add(1);
        }
        let mut spans = Vec::new();
        loop {
            let mut held = self.next();
            if held == EOF {
                return Err("unclosed '['");
            }
            if held == i64::from(b'[') && self.peek() == b':' {
                return Err("POSIX character classes not supported");
            }
            if held == i64::from(b'\\') {
                held = self.escaped();
            }
            if self.peek() == b'-' {
                self.at = self.at.saturating_add(1);
                let mut high = self.next();
                if high == i64::from(b'\\') {
                    high = self.escaped();
                }
                spans.push(Span { low: held, high });
            } else {
                spans.push(Span {
                    low: held,
                    high: held,
                });
            }
            if self.peek() == b']' {
                self.at = self.at.saturating_add(1);
                break;
            }
        }
        self.append(Step::Class(outside, spans));
        Ok(())
    }

    /// `\c`: the character class the letter names, or the one character
    /// the escape stands for.
    fn escape(&mut self) {
        let step = match self.peek() {
            b'b' => Some(Step::Boundary),
            b'd' => Some(Step::Digit),
            b'D' => Some(Step::NotDigit),
            b's' => Some(Step::Space),
            b'S' => Some(Step::NotSpace),
            b'w' => Some(Step::Word),
            b'W' => Some(Step::NotWord),
            _ => None,
        };
        if let Some(step) = step {
            self.at = self.at.saturating_add(1);
            self.append(step);
        } else {
            let held = self.escaped();
            self.append(Step::Match(held));
        }
    }

    /// The character a `\` stands before, which is `re_esc_char`:
    /// `\uXXXX` and `\xXX` name one by its value, six letters name a
    /// character written no other way, and the rest stand for
    /// themselves.
    fn escaped(&mut self) -> i64 {
        let Some(held) = self.text.get(self.at).copied() else {
            return EOF;
        };
        if held == b'u'
            && let Some(value) = self.hex(4)
        {
            self.at = self.at.saturating_add(5);
            return value;
        }
        if held == b'x'
            && let Some(value) = self.hex(2)
        {
            self.at = self.at.saturating_add(3);
            return value;
        }
        let Some(at) = ESCAPED.iter().position(|one| *one == held) else {
            self.error = Some("unknown \\ escape");
            return i64::from(held);
        };
        self.at = self.at.saturating_add(1);
        let written = TRANSLATED.get(at).copied().unwrap_or(held);
        i64::from(written)
    }

    /// The value of the `count` hexadecimal digits after the reader,
    /// where the pattern holds that many, which is `re_hex`.
    fn hex(&self, count: usize) -> Option<i64> {
        let from = self.at.saturating_add(1);
        let digits = self.text.get(from..from.saturating_add(count))?;
        let mut value = 0_i64;
        for digit in digits {
            let held = char::from(*digit).to_digit(16)?;
            value = value.saturating_mul(16).saturating_add(i64::from(held));
        }
        Some(value)
    }
}

/// How many steps `count` is, as a step counts them.
fn whole(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(0)
}

/// How many steps lie between `from` and `to`, which is what a step
/// that names `to` from `from` carries.
fn between(from: usize, to: usize) -> i64 {
    whole(to).saturating_sub(whole(from))
}
