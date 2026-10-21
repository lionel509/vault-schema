//! vault-schema: reports every note whose frontmatter breaks the schema note.
//!
//! Usage: vault-schema <schema.md> <path>...
//! Exit: 0 clean, 1 problems found, 2 usage or schema error.

/// Writes one line to stderr. Unlike `eprintln!`, a closed stderr (`| head -1`)
/// does not panic: the exit code the run computed still stands.
macro_rules! say {
    ($($arg:tt)*) => {
        let _ = std::io::Write::write_fmt(
            &mut std::io::stderr(),
            format_args!("{}\n", format_args!($($arg)*)),
        );
    };
}

mod check;
mod rare;
mod schema;
mod walk;

use std::collections::HashSet;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

use check::Problem;

/// Summary order.
const KINDS: [&str; 7] = ["yaml", "tags", "rare", "required", "value", "synonym", "io"];

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.len() < 2 {
        say!("usage: vault-schema <schema.md> <path>...");
        return ExitCode::from(2);
    }
    let schema = match schema::load(Path::new(&args[0])) {
        Ok(s) => s,
        Err(e) => {
            say!("vault-schema: {e}");
            return ExitCode::from(2);
        }
    };

    // Resolve every argument before checking any note.
    let mut targets = Vec::new();
    let mut problems: Vec<Problem> = Vec::new();
    for arg in &args[1..] {
        if let Err(e) = walk::collect(Path::new(arg), &schema, &mut targets, &mut problems) {
            say!("vault-schema: {e}");
            return ExitCode::from(2);
        }
    }
    // Overlapping arguments (a folder and a note in it) name a note twice.
    // Keep the first occurrence, in argument order.
    let mut seen = HashSet::new();
    targets.retain(|t| seen.insert(t.path.clone()));

    let total = targets.len();
    let texts: Vec<io::Result<String>> = targets.iter().map(|t| fs::read_to_string(&t.path)).collect();
    // Rare tags are judged against every note in the scope, not only the targets.
    let tally = rare::count(&targets, &texts, &schema, &mut problems);

    let mut bad_notes = 0;
    for (t, read) in targets.iter().zip(&texts) {
        let found = match read {
            Ok(text) => {
                let mut f = check::check_note(t, text, &schema);
                f.extend(rare::problems(t, text, &schema, &tally));
                f
            }
            Err(e) => vec![Problem {
                path: t.display.clone(),
                line: 1,
                field: "file".into(),
                msg: e.to_string(),
                kind: "io",
            }],
        };
        if !found.is_empty() {
            bad_notes += 1;
        }
        problems.extend(found);
    }

    // The exit code is known before anything is printed. A closed stdout
    // (`| head`) stops the printing, not the run.
    let exit = if problems.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) };
    let _ = report(&problems, total, bad_notes);
    exit
}

/// Prints one line per problem, then the summary. Stops at the first write
/// error, such as a closed pipe.
fn report(problems: &[Problem], total: usize, bad_notes: usize) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout().lock());
    let mut counts = [0usize; 7];
    for p in problems {
        writeln!(out, "{}:{}: {}: {}", p.path.display(), p.line, p.field, p.msg)?;
        if let Some(i) = KINDS.iter().position(|k| *k == p.kind) {
            counts[i] += 1;
        }
    }

    if problems.is_empty() {
        writeln!(out, "vault-schema: 0 problems in {total} notes")?;
    } else {
        let kinds: Vec<String> = KINDS
            .iter()
            .zip(counts)
            .filter(|(_, c)| *c > 0)
            .map(|(k, c)| format!("{k} {c}"))
            .collect();
        writeln!(
            out,
            "vault-schema: {} problems in {bad_notes} of {total} notes ({})",
            problems.len(),
            kinds.join(", ")
        )?;
    }
    out.flush()
}
