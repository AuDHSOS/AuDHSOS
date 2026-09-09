// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Strict class method grammar and lexical super contexts.

use super::{
    AsyncKind, Box, Class, ConstructorKind, Error, Expr, ExprKind, Function, Kind, ObjectProperty,
    Parser, Stmt, String, SuperContext, Value, Vec, strict_binding,
};

impl Parser {
    pub(super) fn method_function(
        &mut self,
        name: Option<String>,
        kind: AsyncKind,
        derived: bool,
    ) -> Result<Function, Error> {
        let context = core::mem::replace(
            &mut self.super_context,
            if derived {
                SuperContext::Derived
            } else {
                SuperContext::Method
            },
        );
        let result = self.function_contents(name, kind);
        self.super_context = context;
        result
    }
    pub(super) fn class_expression(
        &mut self,
        name: Option<String>,
        offset: usize,
    ) -> Result<Expr, Error> {
        self.enter()?;
        let strict = core::mem::replace(&mut self.strict, true);
        let result = self.class_contents(name, offset);
        self.strict = strict;
        self.depth = self.depth.saturating_sub(1);
        result
    }
    #[expect(
        clippy::too_many_lines,
        reason = "class element grammar validates modifiers and constructor uniqueness in one bounded pass"
    )]
    fn class_contents(&mut self, name: Option<String>, offset: usize) -> Result<Expr, Error> {
        if name.as_ref().is_some_and(|n| strict_binding(n)) {
            return Err(self.error("invalid class name"));
        }
        let heritage = if self.eat("extends") {
            Some(self.prefix()?)
        } else {
            None
        };
        self.need("{")?;
        let mut methods = Vec::new();
        let mut constructor = None;
        while !self.eat("}") {
            if self.eat(";") {
                continue;
            }
            let is_static = self.is("static")
                && self
                    .tokens
                    .get(self.at.saturating_add(1))
                    .is_some_and(|t| t.kind != Kind::Punct("("));
            if is_static {
                self.need("static")?;
            }
            let method_start = self.token()?.offset;
            let async_method = self.async_method_head();
            if async_method {
                self.need("async")?;
            }
            let accessor = if (self.is("get") || self.is("set"))
                && self
                    .tokens
                    .get(self.at.saturating_add(1))
                    .is_some_and(|t| t.kind != Kind::Punct("("))
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
            let key = if computed {
                let k = self.expression(0)?;
                self.need("]")?;
                k
            } else {
                let k = self.property_name()?;
                self.make(ExprKind::Literal(k), 1, offset)?
            };
            let text = match &key.kind {
                ExprKind::Literal(Value::String(s)) => Some(s.clone()),
                _ => None,
            };
            let is_constructor = !computed
                && !is_static
                && text
                    .as_ref()
                    .is_some_and(|s| *s == Value::string("constructor").units());
            if is_static
                && !computed
                && text
                    .as_ref()
                    .is_some_and(|s| *s == Value::string("prototype").units())
            {
                return Err(self.error("static prototype method is forbidden"));
            }
            if is_constructor && (accessor.is_some() || async_method || constructor.is_some()) {
                return Err(self.error("invalid or duplicate constructor"));
            }
            if !self.is("(") {
                return Err(self.error(
                    "class fields, private elements and static blocks are not implemented",
                ));
            }
            let mut function = self.method_function(
                None,
                if async_method {
                    AsyncKind::Async
                } else {
                    AsyncKind::Sync
                },
                is_constructor && heritage.is_some(),
            )?;
            function.constructible = is_constructor;
            function.source = Some(self.source_since(method_start)?);
            function.constructor_kind = if is_constructor {
                if heritage.is_some() {
                    ConstructorKind::DerivedClass
                } else {
                    ConstructorKind::BaseClass
                }
            } else {
                ConstructorKind::Ordinary
            };
            if let Some(setter) = accessor
                && (function.parameters.len() != usize::from(setter)
                    || function.parameters.iter().any(|p| p.rest))
            {
                return Err(self.error("invalid class accessor parameters"));
            }
            if is_constructor {
                constructor = Some(function);
            } else {
                let value = self.make(ExprKind::Function(function), 1, offset)?;
                methods.push((
                    is_static,
                    ObjectProperty {
                        key,
                        value,
                        prototype: false,
                        accessor,
                    },
                ));
            }
        }
        let mut constructor = constructor.unwrap_or(Function {
            source: None,
            name: None,
            parameters: Vec::new(),
            body: if heritage.is_some() {
                alloc::vec![Stmt::Expr(self.make(ExprKind::DefaultSuper, 1, offset)?)]
            } else {
                Vec::new()
            },
            arrow: false,
            strict: true,
            constructible: true,
            async_kind: AsyncKind::Sync,
            constructor_kind: if heritage.is_some() {
                ConstructorKind::DerivedClass
            } else {
                ConstructorKind::BaseClass
            },
        });
        constructor.source = Some(self.source_since(offset)?);
        self.make(
            ExprKind::Class(Box::new(Class {
                name,
                heritage,
                constructor,
                methods,
            })),
            1,
            offset,
        )
    }
}
