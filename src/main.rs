//! vault-schema: reports every note whose frontmatter breaks the schema note.
//!
//! Usage: vault-schema <schema.md> <path>...
//! Exit: 0 clean, 1 problems found, 2 usage or schema error.

mod check;
mod schema;
mod walk;

use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;

use check::Problem;

/// Summary order.
const KINDS: [&str; 6] = ["yaml", "tags", "required", "value", "synonym", "io"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: vault-schema <schema.md> <path>...");
        return ExitCode::from(2);
    }
    let schema = match schema::load(Path::new(&args[0])) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("vault-schema: {e}");
            return ExitCode::from(2);
        }
    };

    // Resolve every argument before checking any note.
    let mut targets = Vec::new();
    let mut problems: Vec<Problem> = Vec::new();
    for arg in &args[1..] {
        if let Err(e) = walk::collect(arg, &schema, &mut targets, &mut problems) {
            eprintln!("vault-schema: {e}");
            return ExitCode::from(2);
        }
    }

    let total = targets.len();
    let mut bad_notes = 0;
    for t in &targets {
        let found = match fs::read_to_string(&t.path) {
            Ok(text) => check::check_note(t, &text, &schema),
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
    let mut counts = [0usize; 6];
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
