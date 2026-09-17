// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Strict class method grammar and lexical super contexts.

use super::{
    AsyncKind, Box, Class, ConstructorKind, Error, Expr, ExprKind, Function, Kind, ObjectProperty,
    Parser, Stmt, String, SuperContext, Value, Vec, strict_binding,
};

impl Parser {
    /// 15.7.1: an Initializer that names `arguments` is a Syntax Error, and
    /// one that names `new.target` would read the frame of the constructor
    /// this lowering runs it in.
    ///
    /// Every token of the Initializer is read, so a property of those names
    /// is refused with them. `ContainsArguments` of 15.7.1 reaches into an
    /// arrow and stops at a function, which the tokens do not say, so an
    /// Initializer that carries a function names the gap instead.
    fn refuse_frame_names(&self, start: usize) -> Result<(), Error> {
        let nested = self.tokens.get(start..self.at).is_some_and(|tokens| {
            tokens
                .iter()
                .any(|token| matches!(&token.kind, Kind::Word(word) if word == "function"))
        });
        for offset in start..self.at {
            let Some(token) = self.tokens.get(offset) else {
                break;
            };
            if matches!(&token.kind, Kind::Word(word) if word == "arguments") {
                if nested {
                    return Err(Self::unsupported("a function of a field Initializer"));
                }
                return Err(Error::Syntax {
                    offset: token.offset,
                    message: "an Initializer of a field that names arguments",
                });
            }
            if matches!(&token.kind, Kind::Word(word) if word == "new")
                && self
                    .tokens
                    .get(offset.saturating_add(1))
                    .is_some_and(|token| token.kind == Kind::Punct("."))
            {
                return Err(Self::unsupported("a new.target of a field Initializer"));
            }
        }
        Ok(())
    }

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
        let mut fields = Vec::new();
        let mut static_fields = Vec::new();
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
            if self.is("{") {
                return Err(Self::unsupported("class static blocks"));
            }
            let method_start = self.token()?.offset;
            let async_method = self.async_method_head();
            if async_method {
                self.need("async")?;
                if self.is("*") {
                    return Err(Self::unsupported("async generator methods"));
                }
            }
            if self.is("*") {
                return Err(Self::unsupported("generator methods"));
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
            // 15.7.1: an element that is no method is a field, whose
            // Initializer runs with the instance as its `this`.
            if !self.is("(") {
                if accessor.is_some() || async_method {
                    return Err(self.error("a field with a method modifier"));
                }
                // 15.7.5 evaluates a computed name where the class is
                // defined, which is before the Initializer of an instance
                // field runs.
                let Some(text) = text.filter(|_| !computed) else {
                    return Err(Self::unsupported("a computed class field name"));
                };
                let name = String::from_utf16_lossy(&text);
                // 15.7.1: no field is named `constructor`, and no static field
                // is named `prototype`.
                if name == "constructor" || (is_static && name == "prototype") {
                    return Err(self.error("a field of a name a class cannot carry"));
                }
                let initializer = if self.eat("=") {
                    let start = self.at;
                    // 15.7.15 runs the Initializer in a frame whose
                    // `[[HomeObject]]` is the prototype of the class.
                    let context = core::mem::replace(&mut self.super_context, SuperContext::Method);
                    let value = self.expression(0);
                    self.super_context = context;
                    let value = value?;
                    // 15.7.1: an Initializer names no `arguments`, and a
                    // `new.target` would read the one of the constructor this
                    // lowering runs the Initializer in.
                    self.refuse_frame_names(start)?;
                    Some(value)
                } else {
                    None
                };
                if !self.eat(";") && !self.is("}") && !self.token()?.newline {
                    return Err(self.error("expected semicolon after a class field"));
                }
                if is_static {
                    static_fields.push((name, initializer));
                } else {
                    fields.push(Stmt::Field(name, initializer));
                }
                continue;
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
                        key: Some(key),
                        value,
                        computed,
                        prototype: false,
                        accessor,
                    },
                ));
            }
        }
        // 15.7.15 runs the field Initializers of an instance before the body
        // of the constructor; a derived constructor binds its `this` only
        // where 13.3.7.1 has run, which this lowering has no place after.
        if !fields.is_empty() && heritage.is_some() {
            return Err(Self::unsupported("a field of a derived class"));
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
        for field in fields.into_iter().rev() {
            constructor.body.insert(0, field);
        }
        constructor.source = Some(self.source_since(offset)?);
        self.make(
            ExprKind::Class(Box::new(Class {
                name,
                heritage,
                constructor,
                methods,
                static_fields,
            })),
            1,
            offset,
        )
    }
}
