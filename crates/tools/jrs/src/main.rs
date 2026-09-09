// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The command-line host for jrs. Scripts cannot access the host filesystem.
#![forbid(unsafe_code)]

use jrs::{Error, Host, Limits, Realm, Runtime, Value, compile};
use std::{
    io::{self, Read, Write},
    process::ExitCode,
    time::Instant,
};

const USAGE: &str = "usage: jrs [--fuel N] [--stats] [--bench N] (-e SOURCE | FILE | -)\n\
       jrs --wpt ROOT TEST_FILE...\n\
       jrs [--fuel N] --test262 ROOT (--all | TEST_PATH...) [--summary]\n\
       jrs [--fuel N] --realm FILE...\n\
Initial JavaScript subset; no browser APIs. '-' reads standard input.";

mod test262;
mod wpt;

struct Console<'a, W>(&'a mut W);
impl<W: Write> Host for Console<'_, W> {
    fn print(&mut self, arguments: &[Value]) -> Result<(), Error> {
        for (index, value) in arguments.iter().enumerate() {
            if index != 0 {
                write!(self.0, " ").map_err(|_| Error::Host)?;
            }
            write!(self.0, "{value}").map_err(|_| Error::Host)?;
        }
        writeln!(self.0).map_err(|_| Error::Host)
    }
}

fn main() -> ExitCode {
    entry(
        std::env::args().skip(1),
        io::stdin().lock(),
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
}

fn entry(
    args: impl Iterator<Item = String>,
    input: impl Read,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> ExitCode {
    match run(args, input, output, diagnostics) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(diagnostics, "jrs: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    mut args: impl Iterator<Item = String>,
    mut input: impl Read,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> Result<(), String> {
    let mut limits = Limits::default();
    let mut source = None;
    let mut stats = false;
    let mut iterations = 1u32;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--realm" if source.is_none() => return run_realm(args, limits, output),
            "--wpt" if source.is_none() => return wpt::run(args, output),
            "--test262" if source.is_none() => return test262::run(args, limits, output),
            "--help" | "-h" => {
                writeln!(output, "{USAGE}").map_err(|e| e.to_string())?;
                return Ok(());
            }
            "--stats" => stats = true,
            "--fuel" => {
                limits.fuel = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or("--fuel requires a nonnegative integer")?;
            }
            "--bench" => {
                iterations = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|n| *n > 0)
                    .ok_or("--bench requires a positive integer")?;
                stats = true;
            }
            "-e" if source.is_none() => {
                source = Some(args.next().ok_or("-e requires source text")?);
            }
            "-" if source.is_none() => {
                source = Some(read_source(&mut input, limits.source_bytes)?);
            }
            file if !file.starts_with('-') && source.is_none() => {
                source = Some(read_source(
                    std::fs::File::open(file).map_err(|e| format!("{file}: {e}"))?,
                    limits.source_bytes,
                )?);
            }
            _ => return Err(format!("unknown or extra argument: {arg}\n{USAGE}")),
        }
    }
    let source = source.ok_or(USAGE)?;
    let start = Instant::now();
    let program = compile(&source, limits).map_err(|e| e.to_string())?;
    let compiled = start.elapsed();
    let mut runtime = Runtime::new(limits);
    let start = Instant::now();
    let mut value = Value::Undefined;
    for _ in 0..iterations {
        value = runtime
            .run(std::hint::black_box(&program), &mut Console(output))
            .map_err(|e| e.to_string())?;
    }
    let executed = start.elapsed();
    if !matches!(value, Value::Undefined) {
        writeln!(output, "{value}").map_err(|e| e.to_string())?;
    }
    if stats {
        writeln!(diagnostics,
            "compile={compiled:?} execute_total={executed:?} iterations={iterations} instructions={} bindings={}",
            program.instruction_count(),
            program.binding_count()
        ).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

fn read_source(reader: impl Read, limit: usize) -> Result<String, String> {
    let mut text = String::new();
    reader
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > limit {
        return Err("source byte limit exceeded".to_owned());
    }
    Ok(text)
}

fn run_realm(
    args: impl Iterator<Item = String>,
    limits: Limits,
    output: &mut impl Write,
) -> Result<(), String> {
    let mut host = Console(output);
    let mut realm = Realm::new(limits, &mut host).map_err(|e| e.to_string())?;
    let mut any = false;
    for file in args {
        any = true;
        let source = read_source(
            std::fs::File::open(&file).map_err(|e| format!("{file}: {e}"))?,
            limits.source_bytes,
        )?;
        realm
            .evaluate(&source)
            .map_err(|e| format!("{file}: {e}"))?;
    }
    if any {
        Ok(())
    } else {
        Err("--realm requires at least one script file".to_owned())
    }
}
