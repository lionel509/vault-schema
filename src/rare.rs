//! Rare topic tags. A topic tag (not a domain or content-type tag, not listed
//! in the scope's `topics`) is `rare` on a note when fewer notes in the scope
//! use it than the scope's `min_uses`. Counts cover the whole scope, so a
//! single-file run walks the scope too, read-only and with the same exclusions.
//!
//! A note whose tags can't be read (a read error, or broken frontmatter) is
//! "unknown": it might use any tag. A tag is rare only when the known count
//! plus the unknown notes still falls under `min_uses`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::PathBuf;

use crate::check::{note_tags, Problem};
use crate::schema::{scope_index, Schema};
use crate::walk::{self, Target};

/// Per scope (by index): how many notes use each tag, and how many notes in
/// the scope could not be read, so their tags are unknown.
pub struct Tally {
    counts: Vec<HashMap<String, usize>>,
    unknown: Vec<usize>,
}

impl Tally {
    fn add(&mut self, s: &Schema, idx: usize, text: &str) {
        if s.scopes[idx].min_uses.is_none() {
            return;
        }
        match note_tags(text) {
            Ok(Some((tags, _))) => {
                for tag in tags {
                    *self.counts[idx].entry(tag).or_insert(0) += 1;
                }
            }
            Ok(None) => {}
            Err(()) => self.unknown[idx] = self.unknown[idx].saturating_add(1),
        }
    }
}

/// Counts tags for every scope that has `min_uses` and at least one target.
/// Notes that are not targets are walked and read. A failure here never
/// becomes a problem: it makes the scope's counts unknown and is printed to
/// stderr only.
pub fn count(targets: &[Target], texts: &[io::Result<String>], s: &Schema) -> Tally {
    let n = s.scopes.len();
    let mut tally = Tally { counts: vec![HashMap::new(); n], unknown: vec![0; n] };
    for (t, text) in targets.iter().zip(texts) {
        if let Some(i) = scope_index(s, &t.rel_lc) {
            match text {
                Ok(text) => tally.add(s, i, text),
                // The target's own read error is reported by main. Here it only
                // means the note's tags can't be counted.
                Err(_) if s.scopes[i].min_uses.is_some() => {
                    tally.unknown[i] = tally.unknown[i].saturating_add(1);
                }
                Err(_) => {}
            }
        }
    }

    let seen: HashSet<PathBuf> = targets.iter().map(|t| t.path.clone()).collect();
    for idx in 0..n {
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
        if !walk_problems.is_empty() {
            // Notes we could not list may use any tag, so no tag is known to be rare.
            tally.unknown[idx] = usize::MAX;
            for p in &walk_problems {
                say!("vault-schema: rare count for {}: {}", p.path.display(), p.msg);
            }
        }
        for t in found {
            if seen.contains(&t.path) || scope_index(s, &t.rel_lc) != Some(idx) {
                continue;
            }
            match fs::read_to_string(&t.path) {
                Ok(text) => tally.add(s, idx, &text),
                Err(e) => {
                    tally.unknown[idx] = tally.unknown[idx].saturating_add(1);
                    say!("vault-schema: rare count skipped {}: {}", t.display.display(), e);
                }
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
    let Ok(Some((tags, line))) = note_tags(text) else {
        return out;
    };
    for tag in tags {
        let exempt = sc.topics.contains(&tag)
            || sc.domain.contains(&tag)
            || sc.types.iter().any(|(n, _)| n.to_lowercase() == tag);
        if exempt {
            continue;
        }
        let k = tally.counts[i].get(&tag).copied().unwrap_or(0);
        if k.saturating_add(tally.unknown[i]) < min {
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
