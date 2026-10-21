//! Resolves path arguments to note targets. Excluded folders are decided on
//! paths alone, before the filesystem is touched, and symlinks are never followed.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::check::Problem;
use crate::schema::{lc, logical_cwd, normalise, Schema};

pub struct Target {
    /// The path as the user typed it, joined with the names walked.
    pub display: PathBuf,
    /// The real path to read.
    pub path: PathBuf,
    /// Root-relative, lowercased.
    pub rel_lc: PathBuf,
}

fn strip(abs_lc: &Path, root: &Path) -> Option<PathBuf> {
    abs_lc.strip_prefix(root).ok().map(Path::to_path_buf)
}

/// Root-relative path of a lowercased absolute path, trying the lexical root
/// first and then the canonical one.
pub fn rel_if_inside(abs_lc: &Path, s: &Schema) -> Option<PathBuf> {
    strip(abs_lc, &s.root_lex).or_else(|| strip(abs_lc, &s.root_canon))
}

/// True if a root-relative, lowercased path must never be opened or listed.
pub fn skipped(rel: &Path, s: &Schema) -> bool {
    for c in rel.components() {
        if let Component::Normal(n) = c {
            let n = n.to_string_lossy();
            if n.starts_with('.') || s.skip_names.iter().any(|k| k.as_str() == n) {
                return true;
            }
        }
    }
    if s.exclude.iter().any(|e| rel.starts_with(e)) {
        return true;
    }
    rel.file_name()
        .is_some_and(|f| f.to_string_lossy().ends_with(".excalidraw.md"))
}

/// True if a root-relative, lowercased folder is the schema root, a scope
/// folder, or an ancestor of one. Those are walked even when they hold a repo.
pub fn exempt(rel_lc: &Path, s: &Schema) -> bool {
    rel_lc.as_os_str().is_empty() || s.scopes.iter().any(|sc| sc.path.starts_with(rel_lc))
}

/// True if a folder is a nested git repo or worktree (a `.git` dir or file)
/// and is not exempt.
fn is_nested_repo(dir: &Path, rel_lc: &Path, s: &Schema) -> bool {
    !exempt(rel_lc, s) && fs::symlink_metadata(dir.join(".git")).is_ok()
}

fn is_md(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

fn io_problem(path: &Path, field: &str, msg: String) -> Problem {
    Problem { path: path.to_path_buf(), line: 1, field: field.to_string(), msg, kind: "io" }
}

/// Resolves one argument and adds the notes under it to `out`.
/// An Err means the argument is unusable and the run must exit 2.
pub fn collect(
    arg: &Path,
    s: &Schema,
    out: &mut Vec<Target>,
    problems: &mut Vec<Problem>,
) -> Result<(), String> {
    let shown = arg.display();
    let typed = arg.to_path_buf();
    let cwd = logical_cwd()?;

    // Lexical check first: no filesystem call at all.
    let abs = normalise(&cwd.join(&typed));
    if let Some(rel) = rel_if_inside(&lc(&abs), s)
        && skipped(&rel, s)
    {
        eprintln!("vault-schema: skipped {shown} (excluded)");
        return Ok(());
    }

    // Arguments never follow a symlink on the way down. The prefixes below the
    // root are not excluded (the check above passed), so nothing inside an
    // excluded folder is stat'ed here.
    if let Some(rel) = rel_if_inside(&lc(&abs), s) {
        let depth = abs.components().count() - rel.components().count();
        let mut cur = PathBuf::new();
        for (i, c) in abs.components().enumerate() {
            cur.push(c);
            if i >= depth && fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
                eprintln!("vault-schema: skipped {shown} (symlink)");
                return Ok(());
            }
        }
    }

    let canon = fs::canonicalize(&typed).map_err(|e| format!("cannot resolve {shown}: {e}"))?;
    let rel = strip(&lc(&canon), &s.root_canon)
        .ok_or_else(|| format!("{shown} is outside the schema root {}", s.root_disp))?;
    if skipped(&rel, s) {
        eprintln!("vault-schema: skipped {shown} (excluded)");
        return Ok(());
    }

    // A nested repo (or worktree) on the way down is its own vault. Check
    // from the root down, so the first repo found is the one named.
    let mut chain: Vec<&Path> = canon.ancestors().take_while(|a| a.starts_with(&s.root_real)).collect();
    chain.reverse();
    for dir in chain {
        let Ok(r) = dir.strip_prefix(&s.root_real) else { continue };
        if !r.as_os_str().is_empty() && is_nested_repo(dir, &lc(r), s) {
            eprintln!("vault-schema: skipped {shown} (git repo)");
            return Ok(());
        }
    }

    let meta = fs::metadata(&canon).map_err(|e| format!("cannot resolve {shown}: {e}"))?;
    if meta.is_dir() {
        walk_dir(&canon, &rel, &typed, s, out, problems);
    } else if meta.is_file() && is_md(&canon) {
        out.push(Target { display: typed, path: canon, rel_lc: rel });
    }
    Ok(())
}

fn walk_dir(
    dir: &Path,
    rel: &Path,
    display: &Path,
    s: &Schema,
    out: &mut Vec<Target>,
    problems: &mut Vec<Problem>,
) {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            problems.push(io_problem(display, "dir", e.to_string()));
            return;
        }
    };
    let mut entries = Vec::new();
    for ent in rd {
        match ent {
            Ok(e) => entries.push(e),
            Err(e) => problems.push(io_problem(display, "dir", e.to_string())),
        }
    }
    entries.sort_by_key(|e| e.file_name());

    for ent in entries {
        let name = ent.file_name();
        let child_rel = rel.join(name.to_string_lossy().to_lowercase());
        if skipped(&child_rel, s) {
            continue;
        }
        let child_display = display.join(&name);
        let ft = match ent.file_type() {
            Ok(t) => t,
            Err(e) => {
                problems.push(io_problem(&child_display, "file", e.to_string()));
                continue;
            }
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if is_nested_repo(&ent.path(), &child_rel, s) {
                eprintln!("vault-schema: skipped {} (git repo)", child_display.display());
                continue;
            }
            walk_dir(&ent.path(), &child_rel, &child_display, s, out, problems);
        } else if ft.is_file() && is_md(Path::new(&name)) {
            out.push(Target { display: child_display, path: ent.path(), rel_lc: child_rel });
        }
    }
}
