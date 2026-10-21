//! vault-schema: reports every note whose frontmatter breaks the schema note.
//!
//! Usage: vault-schema <schema.md> <path>...
//! Exit: 0 clean, 1 problems found, 2 usage or schema error.

mod check;
mod schema;
mod walk;

use std::fs;
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

    let mut counts = [0usize; 6];
    for p in &problems {
        println!("{}:{}: {}: {}", p.path.display(), p.line, p.field, p.msg);
        if let Some(i) = KINDS.iter().position(|k| *k == p.kind) {
            counts[i] += 1;
        }
    }

    if problems.is_empty() {
        println!("vault-schema: 0 problems in {total} notes");
        return ExitCode::SUCCESS;
    }
    let kinds: Vec<String> = KINDS
        .iter()
        .zip(counts)
        .filter(|(_, c)| *c > 0)
        .map(|(k, c)| format!("{k} {c}"))
        .collect();
    println!(
        "vault-schema: {} problems in {bad_notes} of {total} notes ({})",
        problems.len(),
        kinds.join(", ")
    );
    ExitCode::from(1)
}
