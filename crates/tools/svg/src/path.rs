// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Path data, and the shapes that are paths written another way.
//!
//! Every command of the `d` attribute is read except the elliptical arc,
//! which is drawn as the straight line to where it ends. That is a
//! deliberate loss: an arc needs a square root and a division per point to
//! become the curves PDF draws, and no figure in this repository contains
//! one, so a wrong shape here would be a shape nobody sees. A quadratic
//! curve is not lost — it becomes the cubic it is, with its controls two
//! thirds of the way to the point it bends around.

use doc_pdf::page::Segment;

use crate::Unit;
use crate::number::{self, ONE};

/// How far a control point sits along the side of a circle's box, in
/// thousandths: the constant that makes four curves a circle.
const KAPPA: Unit = 552;

/// A path under construction.
struct Pen {
    /// The segments so far.
    segments: Vec<Segment>,
    /// Where the pen is.
    x: Unit,
    /// Where the pen is.
    y: Unit,
    /// Where the subpath began, which `Z` returns to.
    start: (Unit, Unit),
    /// The last control point, which a smooth curve mirrors.
    control: Option<(Unit, Unit)>,
}

impl Pen {
    /// A pen at the origin.
    const fn new() -> Self {
        Self {
            segments: Vec::new(),
            x: 0,
            y: 0,
            start: (0, 0),
            control: None,
        }
    }

    /// Moves without drawing.
    fn move_to(&mut self, x: Unit, y: Unit) {
        self.x = x;
        self.y = y;
        self.start = (x, y);
        self.control = None;
        self.segments.push(Segment::Move { x, y });
    }

    /// Draws a straight line.
    fn line_to(&mut self, x: Unit, y: Unit) {
        self.x = x;
        self.y = y;
        self.control = None;
        self.segments.push(Segment::Line { x, y });
    }

    /// Draws a cubic curve.
    fn curve_to(&mut self, x1: Unit, y1: Unit, x2: Unit, y2: Unit, x: Unit, y: Unit) {
        self.x = x;
        self.y = y;
        self.control = Some((x2, y2));
        self.segments.push(Segment::Curve {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        });
    }

    /// The control point a smooth curve begins with: the last one turned
    /// about the point the pen stands on.
    const fn mirrored(&self) -> (Unit, Unit) {
        match self.control {
            Some((x, y)) => (
                self.x.saturating_mul(2).saturating_sub(x),
                self.y.saturating_mul(2).saturating_sub(y),
            ),
            None => (self.x, self.y),
        }
    }

    /// Closes the subpath.
    fn close(&mut self) {
        let (x, y) = self.start;
        self.x = x;
        self.y = y;
        self.control = None;
        self.segments.push(Segment::Close);
    }
}

/// Reads the `d` attribute of a path.
pub(crate) fn data(text: &str) -> Vec<Segment> {
    let mut pen = Pen::new();
    let mut at = 0usize;
    let mut command = ' ';
    while let Some(rest) = text.get(at..) {
        let trimmed = rest.trim_start_matches([' ', ',', '\t', '\n', '\r']);
        at = at.saturating_add(rest.len().saturating_sub(trimmed.len()));
        let Some(character) = trimmed.chars().next() else {
            break;
        };
        if character.is_ascii_alphabetic() {
            command = character;
            at = at.saturating_add(1);
            if matches!(command, 'z' | 'Z') {
                pen.close();
            }
            continue;
        }
        let wanted = arguments(command);
        if wanted == 0 {
            at = at.saturating_add(1);
            continue;
        }
        let Some((values, length)) = read(text.get(at..).unwrap_or_default(), wanted) else {
            break;
        };
        at = at.saturating_add(length);
        step(&mut pen, command, &values);
        // A repeated set of arguments continues the command, except that
        // a second set after a move is a line, which is what SVG says.
        command = match command {
            'M' => 'L',
            'm' => 'l',
            other => other,
        };
    }
    pen.segments
}

/// How many numbers a command takes.
const fn arguments(command: char) -> usize {
    match command {
        'M' | 'm' | 'L' | 'l' | 'T' | 't' => 2,
        'H' | 'h' | 'V' | 'v' => 1,
        'C' | 'c' => 6,
        'S' | 's' | 'Q' | 'q' => 4,
        'A' | 'a' => 7,
        _ => 0,
    }
}

/// Reads `count` numbers, and says how many bytes they took.
fn read(text: &str, count: usize) -> Option<(Vec<Unit>, usize)> {
    let mut values = Vec::with_capacity(count);
    let mut at = 0usize;
    for _ in 0..count {
        let rest = text.get(at..)?;
        let trimmed = rest.trim_start_matches([' ', ',', '\t', '\n', '\r']);
        at = at.saturating_add(rest.len().saturating_sub(trimmed.len()));
        let (value, length) = number::read(text.get(at..)?)?;
        values.push(value);
        at = at.saturating_add(length);
    }
    Some((values, at))
}

/// Applies one command with its arguments.
fn step(pen: &mut Pen, command: char, values: &[Unit]) {
    let relative = command.is_ascii_lowercase();
    let (dx, dy) = if relative { (pen.x, pen.y) } else { (0, 0) };
    let at = |index: usize| values.get(index).copied().unwrap_or(0);
    let x = |index: usize| at(index).saturating_add(dx);
    let y = |index: usize| at(index).saturating_add(dy);
    match command.to_ascii_uppercase() {
        'M' => pen.move_to(x(0), y(1)),
        'L' => pen.line_to(x(0), y(1)),
        'H' => pen.line_to(x(0), pen.y),
        'V' => pen.line_to(pen.x, y(0)),
        'C' => pen.curve_to(x(0), y(1), x(2), y(3), x(4), y(5)),
        'S' => {
            let (x1, y1) = pen.mirrored();
            pen.curve_to(x1, y1, x(0), y(1), x(2), y(3));
        }
        'Q' => {
            let (x0, y0) = (pen.x, pen.y);
            let (cx, cy) = (x(0), y(1));
            let (x2, y2) = (x(2), y(3));
            quadratic(pen, x0, y0, cx, cy, x2, y2);
        }
        'T' => {
            let (x0, y0) = (pen.x, pen.y);
            let (cx, cy) = pen.mirrored();
            quadratic(pen, x0, y0, cx, cy, x(0), y(1));
        }
        // The arc, drawn as the line to where it ends.
        'A' => pen.line_to(at(5).saturating_add(dx), at(6).saturating_add(dy)),
        _ => {}
    }
}

/// A quadratic curve, as the cubic it is.
fn quadratic(pen: &mut Pen, x0: Unit, y0: Unit, cx: Unit, cy: Unit, x: Unit, y: Unit) {
    let two_thirds = |from: Unit, to: Unit| {
        from.saturating_add(to.saturating_sub(from).saturating_mul(2).wrapping_div(3))
    };
    let (x1, y1) = (two_thirds(x0, cx), two_thirds(y0, cy));
    let (x2, y2) = (two_thirds(x, cx), two_thirds(y, cy));
    pen.curve_to(x1, y1, x2, y2, x, y);
    // A smooth curve after a quadratic mirrors the quadratic's own
    // control point, not the cubic's second one.
    pen.control = Some((cx, cy));
}

/// A rectangle, with corners as round as it asks for.
pub(crate) fn rectangle(
    x: Unit,
    y: Unit,
    width: Unit,
    height: Unit,
    rx: Unit,
    ry: Unit,
) -> Vec<Segment> {
    if width <= 0 || height <= 0 {
        return Vec::new();
    }
    let rx = rx.clamp(0, width.wrapping_div(2));
    let ry = ry.clamp(0, height.wrapping_div(2));
    let (right, bottom) = (x.saturating_add(width), y.saturating_add(height));
    if rx == 0 || ry == 0 {
        return vec![
            Segment::Move { x, y },
            Segment::Line { x: right, y },
            Segment::Line {
                x: right,
                y: bottom,
            },
            Segment::Line { x, y: bottom },
            Segment::Close,
        ];
    }
    let (cx, cy) = (bend(rx), bend(ry));
    vec![
        Segment::Move {
            x: x.saturating_add(rx),
            y,
        },
        Segment::Line {
            x: right.saturating_sub(rx),
            y,
        },
        Segment::Curve {
            x1: right.saturating_sub(rx).saturating_add(cx),
            y1: y,
            x2: right,
            y2: y.saturating_add(ry).saturating_sub(cy),
            x: right,
            y: y.saturating_add(ry),
        },
        Segment::Line {
            x: right,
            y: bottom.saturating_sub(ry),
        },
        Segment::Curve {
            x1: right,
            y1: bottom.saturating_sub(ry).saturating_add(cy),
            x2: right.saturating_sub(rx).saturating_add(cx),
            y2: bottom,
            x: right.saturating_sub(rx),
            y: bottom,
        },
        Segment::Line {
            x: x.saturating_add(rx),
            y: bottom,
        },
        Segment::Curve {
            x1: x.saturating_add(rx).saturating_sub(cx),
            y1: bottom,
            x2: x,
            y2: bottom.saturating_sub(ry).saturating_add(cy),
            x,
            y: bottom.saturating_sub(ry),
        },
        Segment::Line {
            x,
            y: y.saturating_add(ry),
        },
        Segment::Curve {
            x1: x,
            y1: y.saturating_add(ry).saturating_sub(cy),
            x2: x.saturating_add(rx).saturating_sub(cx),
            y2: y,
            x: x.saturating_add(rx),
            y,
        },
        Segment::Close,
    ]
}

/// An ellipse, as the four curves that make one.
pub(crate) fn ellipse(cx: Unit, cy: Unit, rx: Unit, ry: Unit) -> Vec<Segment> {
    if rx <= 0 || ry <= 0 {
        return Vec::new();
    }
    let (bx, by) = (bend(rx), bend(ry));
    let (left, right) = (cx.saturating_sub(rx), cx.saturating_add(rx));
    let (top, bottom) = (cy.saturating_sub(ry), cy.saturating_add(ry));
    vec![
        Segment::Move { x: right, y: cy },
        Segment::Curve {
            x1: right,
            y1: cy.saturating_add(by),
            x2: cx.saturating_add(bx),
            y2: bottom,
            x: cx,
            y: bottom,
        },
        Segment::Curve {
            x1: cx.saturating_sub(bx),
            y1: bottom,
            x2: left,
            y2: cy.saturating_add(by),
            x: left,
            y: cy,
        },
        Segment::Curve {
            x1: left,
            y1: cy.saturating_sub(by),
            x2: cx.saturating_sub(bx),
            y2: top,
            x: cx,
            y: top,
        },
        Segment::Curve {
            x1: cx.saturating_add(bx),
            y1: top,
            x2: right,
            y2: cy.saturating_sub(by),
            x: right,
            y: cy,
        },
        Segment::Close,
    ]
}

/// A run of points, joined and closed or not.
pub(crate) fn polygon(points: &[Unit], closed: bool) -> Vec<Segment> {
    let mut segments = Vec::new();
    for [x, y] in points.as_chunks::<2>().0.iter().copied() {
        if segments.is_empty() {
            segments.push(Segment::Move { x, y });
        } else {
            segments.push(Segment::Line { x, y });
        }
    }
    if closed && !segments.is_empty() {
        segments.push(Segment::Close);
    }
    segments
}

/// How far the control point of a quarter circle sits from its corner.
const fn bend(radius: Unit) -> Unit {
    radius.saturating_mul(KAPPA).wrapping_div(ONE)
}
