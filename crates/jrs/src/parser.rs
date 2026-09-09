// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bounded Pratt parser. Syntax is fully checked before host effects occur.

use crate::{
    Error, Limits, Value,
    lexer::{self, Kind, Token},
};
use alloc::{boxed::Box, rc::Rc, string::String, vec::Vec};
mod classes;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Unary {
    Plus,
    Minus,
    Not,
    Typeof,
    Void,
    BitNot,
    Delete,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Binary {
    Pow,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    And,
    Or,
    Nullish,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Ushr,
    In,
    InstanceOf,
}

#[derive(Debug)]
pub(crate) enum ExprKind {
    Sequence(Box<Expr>, Box<Expr>),
    Literal(Value),
    Regex(String, String),
    Template(Value, Vec<(Expr, Value)>),
    Await(Box<Expr>),
    Name(String),
    Group(Box<Expr>),
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
    Assign(String, Option<Binary>, Box<Expr>),
    Update(String, bool, bool),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Construct(Box<Expr>, Vec<Expr>),
    Function(Function),
    Class(Box<Class>),
    Super,
    NewTarget,
    DefaultSuper,
    This,
    Object(Vec<ObjectProperty>),
    Array(Vec<Option<Expr>>),
    Spread(Box<Expr>),
    Member(Box<Expr>, Box<Expr>),
    SetMember(Box<Expr>, Option<Binary>, Box<Expr>, bool),
    UpdateMember(Box<Expr>, bool, bool, bool),
}

#[derive(Debug)]
pub(crate) struct ObjectProperty {
    pub(crate) key: Expr,
    pub(crate) value: Expr,
    pub(crate) prototype: bool,
    pub(crate) accessor: Option<bool>,
}

#[derive(Debug)]
pub(crate) struct Function {
    pub(crate) source: Option<Source>,
    pub(crate) name: Option<String>,
    pub(crate) parameters: Vec<Parameter>,
    pub(crate) body: Vec<Stmt>,
    pub(crate) arrow: bool,
    pub(crate) strict: bool,
    pub(crate) constructible: bool,
    pub(crate) async_kind: AsyncKind,
    pub(crate) constructor_kind: ConstructorKind,
}

/// Shared original source and exact UTF-8 range of one function grammar node.
#[derive(Clone, Debug)]
pub(crate) struct Source {
    pub(crate) text: Rc<str>,
    pub(crate) range: core::ops::Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstructorKind {
    Ordinary,
    BaseClass,
    DerivedClass,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SuperContext {
    None,
    Method,
    Derived,
}

#[derive(Debug)]
pub(crate) struct Class {
    pub(crate) name: Option<String>,
    pub(crate) heritage: Option<Expr>,
    pub(crate) constructor: Function,
    pub(crate) methods: Vec<(bool, ObjectProperty)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsyncKind {
    Sync,
    Async,
}

#[derive(Debug)]
pub(crate) struct Parameter {
    pub(crate) name: String,
    pub(crate) default: Option<Expr>,
    pub(crate) rest: bool,
}

#[derive(Debug)]
pub(crate) enum BindingPattern {
    Name(String),
    Array(Vec<Option<BindingPattern>>),
}

impl BindingPattern {
    pub(crate) fn names(&self, names: &mut Vec<String>) {
        match self {
            Self::Name(name) => names.push(name.clone()),
            Self::Array(items) => {
                for item in items.iter().flatten() {
                    item.names(names);
                }
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Expr {
    pub(crate) kind: ExprKind,
    depth: usize,
    pub(crate) offset: usize,
    pub(crate) strict: bool,
}

impl Expr {
    pub(crate) fn member(&self) -> Option<(&Expr, &Expr)> {
        match &self.kind {
            ExprKind::Member(base, key) => Some((base, key)),
            ExprKind::Group(inner) => inner.member(),
            _ => None,
        }
    }
    pub(crate) fn reference_name(&self) -> Option<&str> {
        match &self.kind {
            ExprKind::Name(name) => Some(name),
            ExprKind::Group(inner) => inner.reference_name(),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum Stmt {
    Empty,
    Expr(Expr),
    Block(Vec<Stmt>),
    Declare(Vec<(String, bool, Option<Expr>)>),
    Var(Vec<(String, Option<Expr>)>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    For(Box<Stmt>, Option<Expr>, Option<Expr>, Box<Stmt>),
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    ForIn {
        binding: Option<(String, Option<bool>)>,
        target: Option<Expr>,
        object: Expr,
        body: Box<Stmt>,
    },
    ForOf {
        binding: Option<(BindingPattern, Option<bool>)>,
        target: Option<Expr>,
        object: Expr,
        body: Box<Stmt>,
    },
    Break,
    Continue,
    Function(String, Function),
    Return(Option<Expr>),
    Throw(Expr),
    Try {
        body: Vec<Stmt>,
        catch: Option<(Option<String>, Vec<Stmt>)>,
        finally: Option<Vec<Stmt>>,
    },
}

pub(crate) fn parse(source: &str, limits: Limits) -> Result<Vec<Stmt>, Error> {
    let tokens = lexer::lex(source, limits)?;
    let mut parser = Parser {
        source: Rc::from(source),
        tokens,
        at: 0,
        limits,
        depth: 0,
        loops: 0,
        switches: 0,
        functions: 0,
        new_target_context: 0,
        strict: false,
        async_context: false,
        super_context: SuperContext::None,
    };
    parser.strict = parser.strict_prologue();
    parser.statements(false)
}

/// Parse independently delimited dynamic Function parameters and body. No text
/// fragment can consume a delimiter or comment from the other fragment.
pub(crate) fn dynamic_function(
    parameters: &str,
    body: &str,
    limits: Limits,
    async_kind: AsyncKind,
) -> Result<Function, Error> {
    if parameters.starts_with("#!") {
        return Err(Error::Syntax {
            offset: 0,
            message: "hashbang is not a FormalParameters token",
        });
    }
    let mut params = Parser::dynamic(&alloc::format!("{parameters}\n)"), limits)?;
    let parameters = params.parameters_without_await()?;
    if params.token()?.kind != Kind::End {
        return Err(params.error("unexpected text after dynamic parameters"));
    }
    let mut parser = Parser::dynamic(body, limits)?;
    parser.async_context = async_kind == AsyncKind::Async;
    parser.strict = parser.strict_prologue();
    if parser.strict && parameters.iter().any(|p| p.rest || p.default.is_some()) {
        return Err(parser.error("use strict directive with non-simple parameters"));
    }
    let body = parser.statements(false)?;
    Ok(Function {
        source: None,
        name: None,
        parameters,
        body,
        arrow: false,
        strict: parser.strict,
        constructible: async_kind == AsyncKind::Sync,
        async_kind,
        constructor_kind: ConstructorKind::Ordinary,
    })
}

struct Parser {
    source: Rc<str>,
    tokens: Vec<Token>,
    at: usize,
    limits: Limits,
    depth: usize,
    loops: usize,
    switches: usize,
    functions: usize,
    new_target_context: usize,
    strict: bool,
    async_context: bool,
    super_context: SuperContext,
}

impl Parser {
    fn dynamic(source: &str, limits: Limits) -> Result<Self, Error> {
        Ok(Self {
            source: Rc::from(source),
            tokens: lexer::lex(source, limits)?,
            at: 0,
            limits,
            depth: 0,
            loops: 0,
            switches: 0,
            functions: 1,
            new_target_context: 1,
            strict: false,
            async_context: false,
            super_context: SuperContext::None,
        })
    }
    fn source_since(&self, start: usize) -> Result<Source, Error> {
        let end = self
            .tokens
            .get(self.at.checked_sub(1).ok_or(Error::InvalidBytecode)?)
            .ok_or(Error::InvalidBytecode)?
            .end;
        self.source.get(start..end).ok_or(Error::InvalidBytecode)?;
        Ok(Source {
            text: self.source.clone(),
            range: start..end,
        })
    }
    fn token(&self) -> Result<&Token, Error> {
        self.tokens.get(self.at).ok_or(Error::InvalidBytecode)
    }
    fn error(&self, message: &'static str) -> Error {
        Error::Syntax {
            offset: self.tokens.get(self.at).map_or(0, |t| t.offset),
            message,
        }
    }
    fn is(&self, text: &str) -> bool {
        self.tokens.get(self.at).is_some_and(|t| match &t.kind {
            Kind::Word(w) => w == text,
            Kind::Punct(p) => *p == text,
            _ => false,
        })
    }
    fn eat(&mut self, text: &str) -> bool {
        if self.is(text) {
            self.at = self.at.saturating_add(1);
            true
        } else {
            false
        }
    }
    fn need(&mut self, text: &str) -> Result<(), Error> {
        if self.eat(text) {
            Ok(())
        } else {
            Err(self.error("expected delimiter or keyword"))
        }
    }
    fn enter(&mut self) -> Result<(), Error> {
        if self.depth >= self.limits.nesting.min(48) {
            return Err(Error::Limit {
                resource: "syntax nesting",
            });
        }
        self.depth = self.depth.saturating_add(1);
        Ok(())
    }
    fn semicolon(&mut self) -> Result<(), Error> {
        if self.eat(";") || self.is("}") || self.token()?.kind == Kind::End || self.token()?.newline
        {
            Ok(())
        } else {
            Err(self.error("expected semicolon or line terminator"))
        }
    }
    fn name(&mut self) -> Result<String, Error> {
        let Kind::Word(name) = &self.token()?.kind else {
            return Err(self.error("expected binding identifier"));
        };
        if reserved(name) || (self.strict && strict_binding(name)) {
            return Err(self.error("reserved word is not a binding identifier"));
        }
        let name = name.clone();
        self.at = self.at.saturating_add(1);
        Ok(name)
    }
    fn statements(&mut self, block: bool) -> Result<Vec<Stmt>, Error> {
        let mut body = Vec::new();
        while self.token()?.kind != Kind::End && !(block && self.is("}")) {
            body.push(self.statement()?);
        }
        if block {
            self.need("}")?;
        }
        Ok(body)
    }
    fn statement(&mut self) -> Result<Stmt, Error> {
        self.enter()?;
        let result = self.statement_inner();
        self.depth = self.depth.saturating_sub(1);
        result
    }
    #[expect(
        clippy::too_many_lines,
        reason = "statement dispatch keeps grammar alternatives together"
    )]
    fn statement_inner(&mut self) -> Result<Stmt, Error> {
        if self.async_declaration_head() {
            return self.async_declaration();
        }
        if self.eat("switch") {
            return self.switch_statement();
        }
        if self.eat("try") {
            return self.try_statement();
        }
        if self.eat("throw") {
            return self.throw_statement();
        }
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        if self.eat("{") {
            return self.statements(true).map(Stmt::Block);
        }
        if self.eat("function") {
            return self.function_declaration();
        }
        if self.eat("class") {
            let offset = self
                .tokens
                .get(self.at.saturating_sub(1))
                .ok_or(Error::InvalidBytecode)?
                .offset;
            let name = self.name()?;
            let expr = self.class_expression(Some(name.clone()), offset)?;
            return Ok(Stmt::Declare(alloc::vec![(name, true, Some(expr))]));
        }
        if self.eat("return") {
            return self.return_statement();
        }
        if self.is("let") || self.is("const") || self.is("var") {
            let result = self.declaration()?;
            self.semicolon()?;
            return Ok(result);
        }
        if self.eat("if") {
            self.need("(")?;
            let cond = self.sequence()?;
            self.need(")")?;
            let yes = self.single_statement()?;
            let no = if self.eat("else") {
                Some(Box::new(self.single_statement()?))
            } else {
                None
            };
            return Ok(Stmt::If(cond, Box::new(yes), no));
        }
        if self.eat("while") {
            self.need("(")?;
            let cond = self.sequence()?;
            self.need(")")?;
            self.loops = self.loops.saturating_add(1);
            let body = self.single_statement()?;
            self.loops = self.loops.saturating_sub(1);
            return Ok(Stmt::While(cond, Box::new(body)));
        }
        if self.eat("for") {
            self.need("(")?;
            if self.for_in_head() {
                return self.for_in();
            }
            let init = if self.is("let") || self.is("const") || self.is("var") {
                self.declaration()?
            } else if self.is(";") {
                Stmt::Empty
            } else {
                Stmt::Expr(self.sequence()?)
            };
            self.need(";")?;
            let cond = if self.is(";") {
                None
            } else {
                Some(self.sequence()?)
            };
            self.need(";")?;
            let step = if self.is(")") {
                None
            } else {
                Some(self.sequence()?)
            };
            self.need(")")?;
            self.loops = self.loops.saturating_add(1);
            let body = self.single_statement()?;
            self.loops = self.loops.saturating_sub(1);
            return Ok(Stmt::For(Box::new(init), cond, step, Box::new(body)));
        }
        if self.is("break") || self.is("continue") {
            if self.loops == 0 && (self.is("continue") || self.switches == 0) {
                return Err(self.error("loop control outside a loop"));
            }
            let is_break = self.eat("break");
            if !is_break {
                self.need("continue")?;
            }
            self.semicolon()?;
            return Ok(if is_break {
                Stmt::Break
            } else {
                Stmt::Continue
            });
        }
        let expr = self.sequence()?;
        self.semicolon()?;
        Ok(Stmt::Expr(expr))
    }
    fn single_statement(&mut self) -> Result<Stmt, Error> {
        if self.is("let") || self.is("const") || self.is("function") || self.is("class") {
            return Err(self.error("lexical declaration requires a block"));
        }
        self.statement()
    }

    fn switch_statement(&mut self) -> Result<Stmt, Error> {
        self.need("(")?;
        let value = self.sequence()?;
        self.need(")")?;
        self.need("{")?;
        let mut clauses = Vec::new();
        let mut default = false;
        self.switches = self.switches.saturating_add(1);
        while !self.eat("}") {
            let selector = if self.eat("case") {
                Some(self.sequence()?)
            } else {
                self.need("default")?;
                if default {
                    return Err(self.error("duplicate default clause"));
                }
                default = true;
                None
            };
            self.need(":")?;
            let mut body = Vec::new();
            while !self.is("}") && !self.is("case") && !self.is("default") {
                body.push(self.statement()?);
            }
            clauses.push((selector, body));
        }
        self.switches = self.switches.saturating_sub(1);
        Ok(Stmt::Switch(value, clauses))
    }

    fn for_in_head(&self) -> bool {
        let mut depth = 0usize;
        for token in self.tokens.iter().skip(self.at) {
            match &token.kind {
                Kind::Punct("(" | "[" | "{") => depth = depth.saturating_add(1),
                Kind::Punct(")" | ";") if depth == 0 => return false,
                Kind::Punct(")" | "]" | "}") => depth = depth.saturating_sub(1),
                Kind::Word(name) if depth == 0 && matches!(name.as_str(), "in" | "of") => {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    fn for_in(&mut self) -> Result<Stmt, Error> {
        let (binding, target) = if self.is("var") || self.is("let") || self.is("const") {
            let kind = if self.eat("var") {
                None
            } else {
                Some(self.eat("let"))
            };
            if kind == Some(false) {
                self.need("const")?;
            }
            (Some((self.binding_pattern()?, kind)), None)
        } else {
            let target = self.expression(8)?;
            if let Some(name) = target.reference_name() {
                self.assignment_name(name)?;
            } else if target.member().is_none() {
                return Err(self.error("invalid for-in assignment target"));
            }
            (None, Some(target))
        };
        let of = self.eat("of");
        if !of {
            self.need("in")?;
        }
        let object = if of {
            self.expression(0)?
        } else {
            self.sequence()?
        };
        self.need(")")?;
        self.loops = self.loops.saturating_add(1);
        let body = self.single_statement()?;
        self.loops = self.loops.saturating_sub(1);
        if of {
            return Ok(Stmt::ForOf {
                binding,
                target,
                object,
                body: Box::new(body),
            });
        }
        let binding = if let Some((pattern, kind)) = binding {
            let BindingPattern::Name(name) = pattern else {
                return Err(self.error("destructuring for-in is not implemented yet"));
            };
            Some((name, kind))
        } else {
            None
        };
        Ok(Stmt::ForIn {
            binding,
            target,
            object,
            body: Box::new(body),
        })
    }

    fn binding_pattern(&mut self) -> Result<BindingPattern, Error> {
        self.enter()?;
        let pattern = if self.eat("[") {
            let mut items = Vec::new();
            while !self.eat("]") {
                if self.eat(",") {
                    items.push(None);
                    continue;
                }
                items.push(Some(self.binding_pattern()?));
                if !self.eat(",") {
                    self.need("]")?;
                    break;
                }
            }
            BindingPattern::Array(items)
        } else {
            BindingPattern::Name(self.name()?)
        };
        self.depth = self.depth.saturating_sub(1);
        Ok(pattern)
    }

    fn try_statement(&mut self) -> Result<Stmt, Error> {
        self.need("{")?;
        let body = self.statements(true)?;
        let catch = if self.eat("catch") {
            let name = if self.eat("(") {
                let name = self.name()?;
                self.need(")")?;
                Some(name)
            } else {
                None
            };
            self.need("{")?;
            Some((name, self.statements(true)?))
        } else {
            None
        };
        let finally = if self.eat("finally") {
            self.need("{")?;
            Some(self.statements(true)?)
        } else {
            None
        };
        if catch.is_none() && finally.is_none() {
            return Err(self.error("try requires catch or finally"));
        }
        Ok(Stmt::Try {
            body,
            catch,
            finally,
        })
    }

    fn throw_statement(&mut self) -> Result<Stmt, Error> {
        if self.token()?.newline {
            return Err(self.error("line terminator after throw"));
        }
        let value = self.sequence()?;
        self.semicolon()?;
        Ok(Stmt::Throw(value))
    }

    fn return_statement(&mut self) -> Result<Stmt, Error> {
        if self.functions == 0 {
            return Err(self.error("return outside a function"));
        }
        let value = if self.token()?.newline
            || self.is(";")
            || self.is("}")
            || self.token()?.kind == Kind::End
        {
            None
        } else {
            Some(self.sequence()?)
        };
        self.semicolon()?;
        Ok(Stmt::Return(value))
    }
    fn declaration(&mut self) -> Result<Stmt, Error> {
        let var = self.eat("var");
        let mutable = var || self.eat("let");
        if !mutable {
            self.need("const")?;
        }
        let mut bindings = Vec::new();
        loop {
            let name = self.name()?;
            let init = if self.eat("=") {
                Some(self.expression(0)?)
            } else {
                None
            };
            if !mutable && init.is_none() {
                return Err(self.error("const requires an initializer"));
            }
            bindings.push((name, mutable, init));
            if !self.eat(",") {
                return Ok(if var {
                    Stmt::Var(
                        bindings
                            .into_iter()
                            .map(|(name, _, init)| (name, init))
                            .collect(),
                    )
                } else {
                    Stmt::Declare(bindings)
                });
            }
        }
    }
    fn make(&self, kind: ExprKind, depth: usize, offset: usize) -> Result<Expr, Error> {
        if depth > self.limits.nesting.min(48) {
            return Err(Error::Limit {
                resource: "expression depth",
            });
        }
        Ok(Expr {
            kind,
            depth,
            offset,
            strict: self.strict,
        })
    }
    fn sequence(&mut self) -> Result<Expr, Error> {
        let mut left = self.expression(0)?;
        while self.eat(",") {
            let right = self.expression(0)?;
            let depth = left.depth.max(right.depth).saturating_add(1);
            let offset = left.offset;
            left = self.make(
                ExprKind::Sequence(Box::new(left), Box::new(right)),
                depth,
                offset,
            )?;
        }
        Ok(left)
    }
    fn expression(&mut self, min: u8) -> Result<Expr, Error> {
        self.enter()?;
        let result = self.expression_inner(min);
        self.depth = self.depth.saturating_sub(1);
        result
    }
    #[expect(
        clippy::too_many_lines,
        reason = "Pratt loop keeps operator precedence, assignments and early errors together"
    )]
    fn expression_inner(&mut self, min: u8) -> Result<Expr, Error> {
        if min == 0
            && self.is("async")
            && self
                .tokens
                .get(self.at.saturating_add(1))
                .is_some_and(|t| !t.newline)
        {
            let saved = self.at;
            self.at = self.at.saturating_add(1);
            let arrow = self.arrow_head();
            self.at = saved;
            if arrow {
                self.at = self.at.saturating_add(1);
                return self.arrow_kind(AsyncKind::Async);
            }
        }
        if min == 0 && self.arrow_head() {
            return self.arrow();
        }
        let mut left = self.prefix()?;
        loop {
            let offset = left.offset;
            if min == 0 && self.is("?") {
                self.need("?")?;
                let yes = self.expression(0)?;
                self.need(":")?;
                let no = self.expression(0)?;
                let depth = left.depth.max(yes.depth).max(no.depth).saturating_add(1);
                left = self.make(
                    ExprKind::Conditional(Box::new(left), Box::new(yes), Box::new(no)),
                    depth,
                    offset,
                )?;
                continue;
            }
            let assignment = match &self.token()?.kind {
                Kind::Punct("=") => Some(None),
                Kind::Punct("+=") => Some(Some(Binary::Add)),
                Kind::Punct("-=") => Some(Some(Binary::Sub)),
                Kind::Punct("*=") => Some(Some(Binary::Mul)),
                Kind::Punct("**=") => Some(Some(Binary::Pow)),
                Kind::Punct("/=") => Some(Some(Binary::Div)),
                Kind::Punct("%=") => Some(Some(Binary::Rem)),
                Kind::Punct("&=") => Some(Some(Binary::BitAnd)),
                Kind::Punct("|=") => Some(Some(Binary::BitOr)),
                Kind::Punct("^=") => Some(Some(Binary::BitXor)),
                Kind::Punct("<<=") => Some(Some(Binary::Shl)),
                Kind::Punct(">>=") => Some(Some(Binary::Shr)),
                Kind::Punct(">>>=") => Some(Some(Binary::Ushr)),
                _ => None,
            };
            if min == 0
                && let Some(op) = assignment
            {
                let name = left.reference_name().map(String::from);
                if let Some(name) = &name {
                    self.assignment_name(name)?;
                } else if left.member().is_none() {
                    return Err(self.error("invalid assignment target"));
                }
                self.at = self.at.saturating_add(1);
                let right = self.expression(0)?;
                let depth = right.depth.max(left.depth).saturating_add(1);
                let kind = if let Some(name) = name {
                    ExprKind::Assign(name, op, Box::new(right))
                } else {
                    ExprKind::SetMember(Box::new(left), op, Box::new(right), self.strict)
                };
                left = self.make(kind, depth, offset)?;
                continue;
            }
            let Some((op, precedence)) = binary(&self.token()?.kind) else {
                break;
            };
            if precedence < min {
                break;
            }
            if matches!(op, Binary::Pow)
                && matches!(left.kind, ExprKind::Unary(..) | ExprKind::Await(..))
            {
                return Err(self.error("unparenthesized unary expression before exponentiation"));
            }
            self.at = self.at.saturating_add(1);
            let right = self.expression(if matches!(op, Binary::Pow) {
                precedence
            } else {
                precedence.saturating_add(1)
            })?;
            if nullish_mix(op, &left.kind) || nullish_mix(op, &right.kind) {
                return Err(
                    self.error("parenthesize nullish coalescing mixed with logical operators")
                );
            }
            let depth = left.depth.max(right.depth).saturating_add(1);
            left = self.make(
                ExprKind::Binary(op, Box::new(left), Box::new(right)),
                depth,
                offset,
            )?;
        }
        Ok(left)
    }
    fn prefix(&mut self) -> Result<Expr, Error> {
        self.prefix_with_calls(true)
    }

    fn arrow(&mut self) -> Result<Expr, Error> {
        self.arrow_kind(AsyncKind::Sync)
    }

    fn arrow_kind(&mut self, async_kind: AsyncKind) -> Result<Expr, Error> {
        let offset = if async_kind == AsyncKind::Async {
            self.tokens
                .get(self.at.saturating_sub(1))
                .ok_or(Error::InvalidBytecode)?
                .offset
        } else {
            self.token()?.offset
        };
        let parameters = if self.eat("(") {
            self.parameters_without_await()?
        } else {
            alloc::vec![Parameter {
                name: self.name()?,
                default: None,
                rest: false
            }]
        };
        if self.token()?.newline {
            return Err(self.error("line terminator before arrow"));
        }
        self.need("=>")?;
        self.parameter_directive(&parameters)?;
        let (body, strict) = self.function_body(true, async_kind)?;
        self.make(
            ExprKind::Function(Function {
                source: Some(self.source_since(offset)?),
                name: None,
                parameters,
                body,
                arrow: true,
                strict,
                constructible: false,
                async_kind,
                constructor_kind: ConstructorKind::Ordinary,
            }),
            1,
            offset,
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "prefix and postfix grammar share the original token and source offset"
    )]
    fn prefix_with_calls(&mut self, calls: bool) -> Result<Expr, Error> {
        let token = self.token()?.clone();
        self.at = self.at.saturating_add(1);
        if matches!(&token.kind,Kind::Word(w) if w=="await") {
            return self.await_expression(token.offset);
        }
        let unary = match &token.kind {
            Kind::Punct("+") => Some(Unary::Plus),
            Kind::Punct("-") => Some(Unary::Minus),
            Kind::Punct("!") => Some(Unary::Not),
            Kind::Punct("~") => Some(Unary::BitNot),
            Kind::Word(s) if s == "typeof" => Some(Unary::Typeof),
            Kind::Word(s) if s == "void" => Some(Unary::Void),
            Kind::Word(s) if s == "delete" => Some(Unary::Delete),
            _ => None,
        };
        if let Some(op) = unary {
            return self.unary_expression(op, token.offset);
        }
        if matches!(token.kind, Kind::Punct("++" | "--")) {
            let operand = self.expression(12)?;
            return self.update(operand, token.kind == Kind::Punct("++"), true, token.offset);
        }
        let mut expr = match token.kind {
            Kind::Word(word)
                if word == "async" && self.is("function") && !self.token()?.newline =>
            {
                self.async_expression(token.offset)?
            }
            Kind::Template {
                value,
                tail,
                head: true,
            } => self.template(value, tail, token.offset)?,
            Kind::Regex(pattern, flags) => {
                self.make(ExprKind::Regex(pattern, flags), 1, token.offset)?
            }
            Kind::Word(word) if word == "new" => self.construct(token.offset)?,
            Kind::Word(word) if word == "class" => {
                let name = if self.is("extends") || self.is("{") {
                    None
                } else {
                    Some(self.name()?)
                };
                self.class_expression(name, token.offset)?
            }
            Kind::Word(word) if word == "super" => {
                if !(self.is("(") && self.super_context == SuperContext::Derived
                    || (self.is(".") || self.is("[")) && self.super_context != SuperContext::None)
                {
                    return Err(self.error("invalid super context"));
                }
                self.make(ExprKind::Super, 1, token.offset)?
            }
            Kind::Punct("[") => self.array(token.offset)?,
            Kind::Punct("{") => self.object(token.offset)?,
            Kind::Word(word) if word == "this" => self.make(ExprKind::This, 1, token.offset)?,
            Kind::Word(word) if word == "function" => self.function_expression(token.offset)?,
            Kind::Literal(value) => self.make(ExprKind::Literal(value), 1, token.offset)?,
            Kind::Word(name) if !reserved(&name) => {
                self.make(ExprKind::Name(name), 1, token.offset)?
            }
            Kind::Punct("(") => {
                let expr = self.sequence()?;
                self.need(")")?;
                let depth = expr.depth.saturating_add(1);
                self.make(ExprKind::Group(Box::new(expr)), depth, token.offset)?
            }
            _ => {
                return Err(Error::Syntax {
                    offset: token.offset,
                    message: "expected expression; syntax may not be implemented yet",
                });
            }
        };
        loop {
            if self.eat(".") {
                if !matches!(
                    self.token()?.kind,
                    Kind::Word(_) | Kind::Literal(Value::Boolean(_) | Value::Null)
                ) {
                    return Err(self.error("dot requires IdentifierName"));
                }
                let key = self.property_name()?;
                let key = self.make(ExprKind::Literal(key), 1, self.token()?.offset)?;
                let depth = expr.depth.saturating_add(1);
                expr = self.make(
                    ExprKind::Member(Box::new(expr), Box::new(key)),
                    depth,
                    token.offset,
                )?;
                continue;
            }
            if self.eat("[") {
                let key = self.sequence()?;
                self.need("]")?;
                let depth = expr.depth.max(key.depth).saturating_add(1);
                expr = self.make(
                    ExprKind::Member(Box::new(expr), Box::new(key)),
                    depth,
                    token.offset,
                )?;
                continue;
            }
            if !calls || !self.eat("(") {
                break;
            }
            expr = self.arguments(expr, token.offset)?;
        }
        if !self.token()?.newline && (self.is("++") || self.is("--")) {
            let add = self.eat("++");
            if !add {
                self.need("--")?;
            }
            expr = self.update(expr, add, false, token.offset)?;
        }
        Ok(expr)
    }

    fn construct(&mut self, offset: usize) -> Result<Expr, Error> {
        if self.eat(".") {
            self.need("target")?;
            if self.new_target_context == 0 {
                return Err(self.error("new.target outside a function"));
            }
            return self.make(ExprKind::NewTarget, 1, offset);
        }
        self.enter()?;
        if matches!(self.token()?.kind, Kind::Punct("+" | "-" | "!" | "~")) {
            return Err(self.error("invalid constructor expression"));
        }
        let callee = self.prefix_with_calls(false)?;
        let (callee, args, depth) = if self.eat("(") {
            let call = self.arguments(callee, offset)?;
            let ExprKind::Call(callee, args) = call.kind else {
                return Err(Error::InvalidBytecode);
            };
            (callee, args, call.depth)
        } else {
            let depth = callee.depth.saturating_add(1);
            (Box::new(callee), Vec::new(), depth)
        };
        self.depth = self.depth.saturating_sub(1);
        self.make(ExprKind::Construct(callee, args), depth, offset)
    }

    fn function_expression(&mut self, offset: usize) -> Result<Expr, Error> {
        let name = if self.is("(") {
            None
        } else {
            Some(self.name()?)
        };
        let mut function = self.function(name)?;
        function.source = Some(self.source_since(offset)?);
        self.make(ExprKind::Function(function), 1, offset)
    }

    fn async_declaration_head(&self) -> bool {
        self.is("async")
            && self
                .tokens
                .get(self.at.saturating_add(1))
                .is_some_and(|t| !t.newline && matches!(&t.kind,Kind::Word(w) if w=="function"))
    }
    fn async_declaration(&mut self) -> Result<Stmt, Error> {
        let offset = self.token()?.offset;
        self.at = self.at.saturating_add(2);
        let name = self.name()?;
        let mut function = self.function_kind(None, AsyncKind::Async)?;
        function.source = Some(self.source_since(offset)?);
        Ok(Stmt::Function(name, function))
    }
    fn async_expression(&mut self, offset: usize) -> Result<Expr, Error> {
        self.need("function")?;
        let name = if self.is("(") {
            None
        } else {
            Some(self.name()?)
        };
        let mut function = self.function_kind(name, AsyncKind::Async)?;
        function.source = Some(self.source_since(offset)?);
        self.make(ExprKind::Function(function), 1, offset)
    }
    fn await_expression(&mut self, offset: usize) -> Result<Expr, Error> {
        if !self.async_context {
            return Err(self.error("await outside async function"));
        }
        let value = self.expression(12)?;
        let depth = value.depth.saturating_add(1);
        self.make(ExprKind::Await(Box::new(value)), depth, offset)
    }

    fn unary_expression(&mut self, op: Unary, offset: usize) -> Result<Expr, Error> {
        let expr = self.expression(12)?;
        if matches!(op, Unary::Delete) && self.strict && expr.reference_name().is_some() {
            return Err(self.error("strict delete of a binding"));
        }
        let depth = expr.depth.saturating_add(1);
        self.make(ExprKind::Unary(op, Box::new(expr)), depth, offset)
    }

    fn template(&mut self, head: Value, mut tail: bool, offset: usize) -> Result<Expr, Error> {
        let mut parts = Vec::new();
        let mut depth = 1usize;
        while !tail {
            let value = self.sequence()?;
            depth = depth.max(value.depth.saturating_add(1));
            let Kind::Template {
                value: text,
                tail: last,
                head: false,
            } = self.token()?.kind.clone()
            else {
                return Err(self.error("expected template continuation"));
            };
            self.at = self.at.saturating_add(1);
            tail = last;
            parts.push((value, text));
        }
        self.make(ExprKind::Template(head, parts), depth, offset)
    }

    fn arguments(&mut self, expr: Expr, offset: usize) -> Result<Expr, Error> {
        let mut args = Vec::new();
        let mut depth = expr.depth.saturating_add(1);
        if !self.is(")") {
            loop {
                let arg = self.spread_or_expression()?;
                depth = depth.max(arg.depth.saturating_add(1));
                args.push(arg);
                if !self.eat(",") || self.is(")") {
                    break;
                }
            }
        }
        self.need(")")?;
        self.make(ExprKind::Call(Box::new(expr), args), depth, offset)
    }

    fn update(&self, operand: Expr, add: bool, prefix: bool, offset: usize) -> Result<Expr, Error> {
        let depth = operand.depth.saturating_add(1);
        let kind = if let Some(name) = operand.reference_name() {
            self.assignment_name(name)?;
            ExprKind::Update(String::from(name), add, prefix)
        } else if operand.member().is_some() {
            ExprKind::UpdateMember(Box::new(operand), add, prefix, self.strict)
        } else {
            return Err(self.error("invalid update target"));
        };
        self.make(kind, depth, offset)
    }

    fn property_name(&mut self) -> Result<Value, Error> {
        let value = match &self.token()?.kind {
            Kind::Word(name) => Value::string(name),
            Kind::Literal(value) => Value::String(value.units()),
            _ => return Err(self.error("expected property name")),
        };
        self.at = self.at.saturating_add(1);
        Ok(value)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "object member grammar combines method/accessor starts with literal and shorthand alternatives"
    )]
    fn object(&mut self, offset: usize) -> Result<Expr, Error> {
        let mut properties = Vec::new();
        let mut depth = 1usize;
        let mut has_prototype = false;
        while !self.is("}") {
            let method_start = self.token()?.offset;
            let async_method = self.async_method_head();
            if async_method {
                self.need("async")?;
            }
            let accessor = if (self.is("get") || self.is("set"))
                && self
                    .tokens
                    .get(self.at.saturating_add(1))
                    .is_some_and(|t| !matches!(t.kind, Kind::Punct(":" | "," | "}" | "(")))
            {
                let setter = self.eat("set");
                if !setter {
                    self.need("get")?;
                }
                Some(setter)
            } else {
                None
            };
            let computed = self.eat("[");
            let shorthand = if computed {
                None
            } else {
                match &self.token()?.kind {
                    Kind::Word(name) => Some(name.clone()),
                    _ => None,
                }
            };
            let key = if computed {
                let key = self.expression(0)?;
                self.need("]")?;
                key
            } else {
                let value = self.property_name()?;
                self.make(ExprKind::Literal(value), 1, offset)?
            };
            let colon = self.eat(":");
            if async_method && (colon || !self.is("(")) {
                return Err(self.error("async method requires parameters and body"));
            }
            if accessor.is_some() && colon {
                return Err(self.error("accessor requires parameter list"));
            }
            let prototype = colon
                && !computed
                && matches!(&key.kind, ExprKind::Literal(value) if *value == Value::string("__proto__"));
            if prototype && has_prototype {
                return Err(self.error("duplicate __proto__ setter"));
            }
            has_prototype |= prototype;
            let value = if colon {
                self.expression(0)?
            } else if self.is("(") {
                let mut function = self.method_function(
                    None,
                    if async_method {
                        AsyncKind::Async
                    } else {
                        AsyncKind::Sync
                    },
                    false,
                )?;
                function.constructible = false;
                function.source = Some(self.source_since(method_start)?);
                if let Some(setter) = accessor
                    && (function.parameters.len() != usize::from(setter)
                        || function.parameters.iter().any(|p| p.rest))
                {
                    return Err(self.error("invalid accessor parameter count"));
                }
                self.make(ExprKind::Function(function), 1, offset)?
            } else if let Some(name) = shorthand {
                if accessor.is_some() {
                    return Err(self.error("accessor requires body"));
                }
                if reserved(&name) {
                    return Err(self.error("reserved shorthand name"));
                }
                self.make(ExprKind::Name(name), 1, offset)?
            } else {
                return Err(self.error("property requires a value"));
            };
            depth = depth
                .max(key.depth.saturating_add(1))
                .max(value.depth.saturating_add(1));
            properties.push(ObjectProperty {
                key,
                value,
                prototype,
                accessor,
            });
            if !self.eat(",") {
                break;
            }
        }
        self.need("}")?;
        self.make(ExprKind::Object(properties), depth, offset)
    }

    fn array(&mut self, offset: usize) -> Result<Expr, Error> {
        let mut items = Vec::new();
        let mut depth = 1usize;
        while !self.is("]") {
            if self.eat(",") {
                items.push(None);
                continue;
            }
            let value = self.spread_or_expression()?;
            depth = depth.max(value.depth.saturating_add(1));
            items.push(Some(value));
            if !self.eat(",") {
                break;
            }
        }
        self.need("]")?;
        self.make(ExprKind::Array(items), depth, offset)
    }

    fn spread_or_expression(&mut self) -> Result<Expr, Error> {
        let offset = self.token()?.offset;
        let spread = self.eat("...");
        let expr = self.expression(0)?;
        if spread {
            let depth = expr.depth.saturating_add(1);
            self.make(ExprKind::Spread(Box::new(expr)), depth, offset)
        } else {
            Ok(expr)
        }
    }

    fn async_method_head(&self) -> bool {
        self.is("async")
            && self.tokens.get(self.at.saturating_add(1)).is_some_and(|t| {
                !t.newline && !matches!(t.kind, Kind::Punct(":" | "," | "}" | "("))
            })
    }

    fn function(&mut self, name: Option<String>) -> Result<Function, Error> {
        self.function_kind(name, AsyncKind::Sync)
    }

    fn function_kind(
        &mut self,
        name: Option<String>,
        async_kind: AsyncKind,
    ) -> Result<Function, Error> {
        let context = core::mem::replace(&mut self.super_context, SuperContext::None);
        let result = self.function_contents(name, async_kind);
        self.super_context = context;
        result
    }
    fn function_contents(
        &mut self,
        name: Option<String>,
        async_kind: AsyncKind,
    ) -> Result<Function, Error> {
        self.new_target_context = self.new_target_context.saturating_add(1);
        self.need("(")?;
        self.functions = self.functions.saturating_add(1);
        let parameters = self.parameters_without_await()?;
        self.functions = self.functions.saturating_sub(1);
        self.parameter_directive(&parameters)?;
        let (body, strict) = self.function_body(false, async_kind)?;
        self.new_target_context = self.new_target_context.saturating_sub(1);
        Ok(Function {
            source: None,
            name,
            parameters,
            body,
            arrow: false,
            strict,
            constructible: async_kind == AsyncKind::Sync,
            async_kind,
            constructor_kind: ConstructorKind::Ordinary,
        })
    }

    fn function_declaration(&mut self) -> Result<Stmt, Error> {
        let offset = self
            .tokens
            .get(self.at.saturating_sub(1))
            .ok_or(Error::InvalidBytecode)?
            .offset;
        let name = self.name()?;
        let mut function = self.function(None)?;
        function.source = Some(self.source_since(offset)?);
        if function.strict && strict_binding(&name) {
            return Err(self.error("invalid strict function name"));
        }
        Ok(Stmt::Function(name, function))
    }

    fn assignment_name(&self, name: &str) -> Result<(), Error> {
        if self.strict && matches!(name, "eval" | "arguments") {
            Err(self.error("invalid strict assignment target"))
        } else {
            Ok(())
        }
    }

    fn parameters(&mut self) -> Result<Vec<Parameter>, Error> {
        let mut names = Vec::new();
        if !self.is(")") {
            loop {
                let rest = self.eat("...");
                let name = self.name()?;
                let default = if self.eat("=") {
                    Some(self.expression(0)?)
                } else {
                    None
                };
                if rest && (default.is_some() || !self.is(")")) {
                    return Err(self.error("rest parameter must be last and have no default"));
                }
                names.push(Parameter {
                    name,
                    default,
                    rest,
                });
                if !self.eat(",") || self.is(")") {
                    break;
                }
            }
        }
        self.need(")")?;
        Ok(names)
    }

    fn parameters_without_await(&mut self) -> Result<Vec<Parameter>, Error> {
        let outer = core::mem::replace(&mut self.async_context, false);
        let result = self.parameters();
        self.async_context = outer;
        result
    }

    fn parameter_directive(&mut self, parameters: &[Parameter]) -> Result<(), Error> {
        if parameters.iter().any(|p| p.rest || p.default.is_some()) && self.is("{") {
            let saved = self.at;
            self.at = self.at.saturating_add(1);
            let strict = self.strict_prologue();
            self.at = saved;
            if strict {
                return Err(self.error("use strict directive with non-simple parameters"));
            }
        }
        Ok(())
    }

    fn function_body(
        &mut self,
        concise: bool,
        async_kind: AsyncKind,
    ) -> Result<(Vec<Stmt>, bool), Error> {
        self.enter()?;
        let loops = core::mem::replace(&mut self.loops, 0);
        let switches = core::mem::replace(&mut self.switches, 0);
        self.functions = self.functions.saturating_add(1);
        let inherited = self.strict;
        let outer_async =
            core::mem::replace(&mut self.async_context, async_kind == AsyncKind::Async);
        let body = if self.eat("{") {
            self.strict |= self.strict_prologue();
            self.statements(true)
        } else if concise {
            self.expression(0)
                .map(|expr| alloc::vec![Stmt::Return(Some(expr))])
        } else {
            Err(self.error("expected function body"))
        };
        self.functions = self.functions.saturating_sub(1);
        self.loops = loops;
        self.switches = switches;
        self.depth = self.depth.saturating_sub(1);
        let strict = self.strict;
        self.strict = inherited;
        self.async_context = outer_async;
        body.map(|body| (body, strict))
    }

    fn strict_prologue(&self) -> bool {
        let mut at = self.at;
        while let Some(token) = self.tokens.get(at) {
            if !matches!(token.kind, Kind::Literal(Value::String(_))) {
                break;
            }
            let next = at.saturating_add(1);
            let Some(after) = self.tokens.get(next) else {
                break;
            };
            let explicit = after.kind == Kind::Punct(";");
            let end = after.kind == Kind::End || after.kind == Kind::Punct("}");
            let continuation = binary(&after.kind).is_some()
                || matches!(after.kind, Kind::Punct("(" | "[" | "." | "?"));
            if !explicit && !end && (!after.newline || continuation) {
                break;
            }
            if token.use_strict {
                return true;
            }
            at = next.saturating_add(usize::from(explicit));
        }
        false
    }

    // Balanced lookahead does not interpret parameter expressions; parsing and
    // early errors still run once the closing parenthesis is followed by =>.
    fn arrow_head(&self) -> bool {
        if matches!(
            self.tokens.get(self.at).map(|t| &t.kind),
            Some(Kind::Word(_))
        ) {
            return self
                .tokens
                .get(self.at.saturating_add(1))
                .is_some_and(|t| t.kind == Kind::Punct("=>"));
        }
        if !self.is("(") {
            return false;
        }
        let mut depth = 0usize;
        for (at, token) in self.tokens.iter().enumerate().skip(self.at) {
            match token.kind {
                Kind::Punct("(") => depth = depth.saturating_add(1),
                Kind::Punct(")") => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return self
                            .tokens
                            .get(at.saturating_add(1))
                            .is_some_and(|t| t.kind == Kind::Punct("=>"));
                    }
                }
                Kind::End => return false,
                _ => {}
            }
        }
        false
    }
}

fn binary(kind: &Kind) -> Option<(Binary, u8)> {
    if matches!(kind, Kind::Word(name) if name == "instanceof") {
        return Some((Binary::InstanceOf, 7));
    }
    if matches!(kind, Kind::Word(name) if name == "in") {
        return Some((Binary::In, 7));
    }
    let Kind::Punct(op) = kind else {
        return None;
    };
    Some(match *op {
        "||" => (Binary::Or, 1),
        "??" => (Binary::Nullish, 1),
        "&&" => (Binary::And, 2),
        "|" => (Binary::BitOr, 3),
        "^" => (Binary::BitXor, 4),
        "&" => (Binary::BitAnd, 5),
        "==" => (Binary::Eq, 6),
        "!=" => (Binary::Ne, 6),
        "===" => (Binary::StrictEq, 6),
        "!==" => (Binary::StrictNe, 6),
        "<" => (Binary::Lt, 7),
        "<=" => (Binary::Le, 7),
        ">" => (Binary::Gt, 7),
        ">=" => (Binary::Ge, 7),
        "<<" => (Binary::Shl, 8),
        ">>" => (Binary::Shr, 8),
        ">>>" => (Binary::Ushr, 8),
        "+" => (Binary::Add, 9),
        "-" => (Binary::Sub, 9),
        "*" => (Binary::Mul, 10),
        "**" => (Binary::Pow, 11),
        "/" => (Binary::Div, 10),
        "%" => (Binary::Rem, 10),
        _ => return None,
    })
}

const fn nullish_mix(op: Binary, child: &ExprKind) -> bool {
    matches!(
        (op, child),
        (
            Binary::Nullish,
            ExprKind::Binary(Binary::And | Binary::Or, _, _)
        ) | (
            Binary::And | Binary::Or,
            ExprKind::Binary(Binary::Nullish, _, _)
        )
    )
}

fn reserved(word: &str) -> bool {
    matches!(
        word,
        "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "let"
            | "new"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
    )
}

pub(crate) fn strict_binding(name: &str) -> bool {
    matches!(
        name,
        "eval"
            | "arguments"
            | "implements"
            | "interface"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
    )
}
