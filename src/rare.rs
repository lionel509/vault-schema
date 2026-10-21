//! Rare topic tags. A topic tag (not a domain or content-type tag, not listed
//! in the scope's `topics`) is `rare` on a note when fewer notes in the scope
//! use it than the scope's `min_uses`. Counts cover the whole scope, so a
//! single-file run walks the scope too, read-only and with the same exclusions.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::check::{note_tags, Problem};
use crate::schema::{scope_index, Schema};
use crate::walk::{self, Target};

/// Per scope (by index): how many notes in the scope use each tag.
pub struct Tally(Vec<HashMap<String, usize>>);

impl Tally {
    fn add(&mut self, s: &Schema, idx: usize, text: &str) {
        if s.scopes[idx].min_uses.is_none() {
            return;
        }
        if let Some((tags, _)) = note_tags(text) {
            for tag in tags {
                *self.0[idx].entry(tag).or_insert(0) += 1;
            }
        }
    }
}

fn add_once(problems: &mut Vec<Problem>, p: Problem) {
    if !problems.iter().any(|q| q.path == p.path && q.msg == p.msg) {
        problems.push(p);
    }
}

/// Counts tags for every scope that has `min_uses` and at least one target.
/// Notes that are not targets are walked and read; a read failure is an io
/// problem, since an unread note would undercount.
pub fn count(targets: &[Target], texts: &[io::Result<String>], s: &Schema, problems: &mut Vec<Problem>) -> Tally {
    let mut tally = Tally(vec![HashMap::new(); s.scopes.len()]);
    for (t, text) in targets.iter().zip(texts) {
        if let (Ok(text), Some(i)) = (text, scope_index(s, &t.rel_lc)) {
            tally.add(s, i, text);
        }
    }

    let seen: HashSet<PathBuf> = targets.iter().map(|t| t.path.clone()).collect();
    for idx in 0..s.scopes.len() {
        if s.scopes[idx].min_uses.is_none() {
            continue;
        }
        let Some(anchor) = targets.iter().find(|t| scope_index(s, &t.rel_lc) == Some(idx)) else {
            continue;
        };
        // The scope folder, spelled as the disk has it: the anchor's first
        // components below the root, as many as the scope path has.
        let Ok(rel) = anchor.path.strip_prefix(&s.root_real) else {
            continue;
        };
        let depth = s.scopes[idx].path.components().count();
        let dir_rel: PathBuf = rel.components().take(depth).collect();
        let dir = s.root_real.join(&dir_rel);

        let mut found = Vec::new();
        let mut walk_problems = Vec::new();
        walk::walk_dir(&dir, &s.scopes[idx].path, &dir_rel, s, &mut found, &mut walk_problems, false);
        for p in walk_problems {
            add_once(problems, p);
        }
        for t in found {
            if seen.contains(&t.path) || scope_index(s, &t.rel_lc) != Some(idx) {
                continue;
            }
            match fs::read_to_string(&t.path) {
                Ok(text) => tally.add(s, idx, &text),
                Err(e) => add_once(
                    problems,
                    Problem { path: t.display.clone(), line: 1, field: "file".into(), msg: e.to_string(), kind: "io" },
                ),
            }
        }
    }
    tally
}

/// The rare-tag problems of one target note.
pub fn problems(t: &Target, text: &str, s: &Schema, tally: &Tally) -> Vec<Problem> {
    let mut out = Vec::new();
    let Some(i) = scope_index(s, &t.rel_lc) else {
        return out;
    };
    let sc = &s.scopes[i];
    let Some(min) = sc.min_uses else {
        return out;
    };
    let Some((tags, line)) = note_tags(text) else {
        return out;
    };
    for tag in tags {
        let exempt = sc.topics.contains(&tag)
            || sc.domain.contains(&tag)
            || sc.types.iter().any(|(n, _)| n.to_lowercase() == tag);
        if exempt {
            continue;
        }
        let k = tally.0[i].get(&tag).copied().unwrap_or(0);
        if k < min {
            let notes = if k == 1 { "note" } else { "notes" };
            out.push(Problem {
                path: t.display.clone(),
                line,
                field: "tags".into(),
                msg: format!(
                    "\"{tag}\" is used by {k} {notes} and isn't in the topic list \u{2014} reuse an existing tag, or add it to the schema's topics"
                ),
                kind: "rare",
            });
        }
    }
    out
}
