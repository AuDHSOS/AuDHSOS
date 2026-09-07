// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The stylesheet an SVG carries with it.
//!
//! Not CSS. What is read here are the selectors a drawing is written
//! with — a name, a class, or a name and a class — and the declarations
//! under them. A selector of any other shape is dropped rather than
//! guessed at, and the declarations of the rules that do match are handed
//! back weakest first, so that applying them in order leaves the strongest
//! standing: a rule naming both a name and a class beats one naming a
//! class, which beats one naming a name.

/// A stylesheet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Sheet {
    /// The rules, in the order they were written.
    rules: Vec<Rule>,
}

/// One rule: what it matches, and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    /// The element name it names, if it names one.
    element: Option<String>,
    /// The class it names, if it names one.
    class: Option<String>,
    /// How strongly it matches.
    weight: u32,
    /// The declarations, as property and value.
    declarations: Vec<(String, String)>,
}

impl Sheet {
    /// Reads a stylesheet.
    pub(crate) fn parse(text: &str) -> Self {
        let mut rules = Vec::new();
        let stripped = without_comments(text);
        let mut rest = stripped.as_str();
        while let Some((head, tail)) = rest.split_once('{') {
            let Some((body, next)) = tail.split_once('}') else {
                break;
            };
            let declarations = declarations(body);
            if !declarations.is_empty() {
                for selector in head.split(',') {
                    if let Some((element, class, weight)) = selector_of(selector) {
                        rules.push(Rule {
                            element,
                            class,
                            weight,
                            declarations: declarations.clone(),
                        });
                    }
                }
            }
            rest = next;
        }
        Self { rules }
    }

    /// The declarations for an element, weakest first.
    pub(crate) fn declarations(&self, name: &str, classes: &str) -> Vec<(&str, &str)> {
        let mut matched: Vec<&Rule> = self
            .rules
            .iter()
            .filter(|rule| rule.matches(name, classes))
            .collect();
        matched.sort_by_key(|rule| rule.weight);
        matched
            .iter()
            .flat_map(|rule| {
                rule.declarations
                    .iter()
                    .map(|(property, value)| (property.as_str(), value.as_str()))
            })
            .collect()
    }
}

impl Rule {
    /// Whether the rule is about this element.
    fn matches(&self, name: &str, classes: &str) -> bool {
        if self.element.as_ref().is_some_and(|element| element != name) {
            return false;
        }
        self.class.as_ref().is_none_or(|class| {
            classes
                .split_whitespace()
                .any(|carried| carried == class.as_str())
        })
    }
}

/// The text with every `/* … */` taken out.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("/*") {
        out.push_str(before);
        match after.split_once("*/") {
            Some((_, next)) => rest = next,
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// The declarations inside the braces of a rule.
fn declarations(body: &str) -> Vec<(String, String)> {
    body.split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .map(|(property, value)| {
            (
                property.trim().to_ascii_lowercase(),
                value.trim().to_owned(),
            )
        })
        .filter(|(property, value)| !property.is_empty() && !value.is_empty())
        .collect()
}

/// The name, the class, and the weight of one selector, or nothing for a
/// selector of a shape this crate does not read.
fn selector_of(text: &str) -> Option<(Option<String>, Option<String>, u32)> {
    let selector = text.trim();
    if selector.is_empty() || selector.contains([' ', '>', '+', '~', '[', ':', '#', '*']) {
        return None;
    }
    let (name, class) = selector.split_once('.').unwrap_or((selector, ""));
    if class.contains('.') {
        return None;
    }
    let element = (!name.is_empty()).then(|| name.to_ascii_lowercase());
    let class = (!class.is_empty()).then(|| class.to_owned());
    let weight = match (element.is_some(), class.is_some()) {
        (true, true) => 11,
        (false, true) => 10,
        (true, false) => 1,
        (false, false) => return None,
    };
    Some((element, class, weight))
}
