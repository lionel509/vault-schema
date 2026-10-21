//! Checks one note's frontmatter against the schema.

use std::path::PathBuf;

use yaml_rust2::yaml::Hash;
use yaml_rust2::{Yaml, YamlLoader};

use crate::schema::{scalar_str, Field, Kind, Schema, Scope};
use crate::walk::Target;

pub struct Problem {
    pub path: PathBuf,
    pub line: usize,
    pub field: String,
    pub msg: String,
    pub kind: &'static str,
}

fn push(out: &mut Vec<Problem>, t: &Target, line: usize, field: &str, msg: String, kind: &'static str) {
    out.push(Problem { path: t.display.clone(), line, field: field.to_string(), msg, kind });
}

/// File line of the first frontmatter line that starts with `<key>:`.
/// `fm` holds the frontmatter lines, which start at file line 2.
fn key_line(fm: &[&str], key: &str) -> usize {
    fm.iter()
        .position(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))
        .map_or(1, |i| i + 2)
}

fn yaml_type(y: &Yaml) -> &'static str {
    match y {
        Yaml::String(_) => "string",
        Yaml::Integer(_) => "integer",
        Yaml::Real(_) => "real",
        Yaml::Boolean(_) => "boolean",
        Yaml::Array(_) => "list",
        Yaml::Hash(_) => "mapping",
        Yaml::Null => "null",
        _ => "other",
    }
}

fn show(y: &Yaml) -> String {
    scalar_str(y).unwrap_or_else(|| yaml_type(y).to_string())
}

fn is_date(x: &str) -> bool {
    let b = x.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(|c| c.is_ascii_digit());
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let m: u32 = x[5..7].parse().unwrap_or(0);
    let d: u32 = x[8..10].parse().unwrap_or(0);
    (1..=12).contains(&m) && (1..=31).contains(&d)
}

fn kind_mismatch(k: Kind, v: &Yaml) -> Option<String> {
    let got = yaml_type(v);
    match k {
        Kind::Date => match v {
            Yaml::String(x) if is_date(x) => None,
            Yaml::String(x) => Some(format!("\"{x}\" is not a YYYY-MM-DD date")),
            _ => Some(format!("want date, got {got}")),
        },
        Kind::Number => match v {
            Yaml::Integer(_) | Yaml::Real(_) => None,
            _ => Some(format!("want number, got {got}")),
        },
        Kind::List => match v {
            Yaml::Array(_) => None,
            _ => Some(format!("want list, got {got}")),
        },
        Kind::Str => match v {
            Yaml::String(_) | Yaml::Integer(_) | Yaml::Real(_) | Yaml::Boolean(_) => None,
            _ => Some(format!("want string, got {got}")),
        },
    }
}

pub fn check_note(t: &Target, text: &str, s: &Schema) -> Vec<Problem> {
    let mut out = Vec::new();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.lines().collect();
    let scope: Option<&Scope> = s
        .scopes
        .iter()
        .filter(|sc| t.rel_lc.starts_with(&sc.path))
        .max_by_key(|sc| sc.path.components().count());

    // Frontmatter: the block between two `---` lines at the top of the file.
    let mut fm_lines: Vec<&str> = Vec::new();
    let mut fm = Hash::new();
    let has_fm = lines.first() == Some(&"---");
    if has_fm {
        let Some(close) = lines.iter().skip(1).position(|l| *l == "---").map(|i| i + 1) else {
            push(&mut out, t, 1, "frontmatter", "no closing ---".into(), "yaml");
            return out;
        };
        fm_lines = lines[1..close].to_vec();
        match YamlLoader::load_from_str(&fm_lines.join("\n")) {
            Err(e) => {
                push(&mut out, t, e.marker().line() + 1, "frontmatter", format!("invalid YAML: {}", e.info()), "yaml");
                return out;
            }
            Ok(docs) => match docs.into_iter().next() {
                None | Some(Yaml::Null) => {}
                Some(Yaml::Hash(h)) => fm = h,
                Some(_) => {
                    push(&mut out, t, 1, "frontmatter", "not a mapping".into(), "yaml");
                    return out;
                }
            },
        }
    }

    // Synonyms: the alias is reported at its line.
    for (alias, canon) in &s.synonyms {
        if field(&fm, alias).is_some() {
            let line = key_line(&fm_lines, alias);
            push(&mut out, t, line, alias, format!("use `{canon}`"), "synonym");
        }
    }

    // Tags: a list of strings, with no leading `#`.
    let tags_line = key_line(&fm_lines, "tags");
    let mut tags: Option<Vec<String>> = None;
    if let Some(v) = field(&fm, "tags") {
        match v {
            Yaml::Array(items) => {
                let mut tv = Vec::new();
                for item in items {
                    match item {
                        Yaml::String(x) => match x.strip_prefix('#') {
                            Some(rest) => {
                                push(&mut out, t, tags_line, "tags", format!("\"{x}\": drop the #"), "tags");
                                tv.push(rest.to_string());
                            }
                            None => tv.push(x.clone()),
                        },
                        other => push(&mut out, t, tags_line, "tags", format!("{}: not a string", show(other)), "tags"),
                    }
                }
                tags = Some(tv);
            }
            _ => push(&mut out, t, tags_line, "tags", "must be a list".into(), "tags"),
        }
    }

    // Scope checks: one domain tag, one content-type tag, a cap on topic tags.
    let mut content_type: Option<&str> = None;
    if let Some(sc) = scope {
        if !has_fm {
            push(&mut out, t, 1, "tags", "no frontmatter".into(), "tags");
        } else if field(&fm, "tags").is_none() {
            push(&mut out, t, 1, "tags", "missing".into(), "tags");
        } else if let Some(tv) = &tags {
            let in_domain = |x: &String| sc.domain.iter().any(|d| d == x);
            let type_names: Vec<&str> = sc.types.iter().map(|(n, _)| n.as_str()).collect();
            let in_types = |x: &String| type_names.contains(&x.as_str());

            let domain_hits: Vec<&String> = tv.iter().filter(|x| in_domain(x)).collect();
            match domain_hits.len() {
                0 => push(&mut out, t, tags_line, "tags", format!("no domain tag (one of: {})", sc.domain.join(", ")), "tags"),
                1 => {}
                n => push(&mut out, t, tags_line, "tags", format!("{n} domain tags ({}), want 1", join(&domain_hits)), "tags"),
            }

            let type_hits: Vec<&String> = tv.iter().filter(|x| in_types(x)).collect();
            match type_hits.len() {
                0 => push(&mut out, t, tags_line, "tags", format!("no content-type tag (one of: {})", type_names.join(", ")), "tags"),
                1 => content_type = type_names.iter().copied().find(|n| *n == type_hits[0].as_str()),
                n => push(&mut out, t, tags_line, "tags", format!("{n} content-type tags ({}), want 1", join(&type_hits)), "tags"),
            }

            let topics = tv.iter().filter(|x| !in_domain(x) && !in_types(x)).count();
            if topics > sc.max_topic {
                push(&mut out, t, tags_line, "tags", format!("{topics} topic tags, max {}", sc.max_topic), "tags");
            }
        }
    }

    // Field checks, once the note has exactly one content type.
    if let (Some(sc), Some(ty)) = (scope, content_type) {
        let fields: &[(String, Field)] = sc
            .types
            .iter()
            .find(|(n, _)| n == ty)
            .map_or(&[], |(_, f)| f.as_slice());
        for (fname, spec) in fields {
            check_field(&mut out, t, s, &fm_lines, &fm, fname, spec, ty);
        }
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn check_field(
    out: &mut Vec<Problem>,
    t: &Target,
    s: &Schema,
    fm_lines: &[&str],
    fm: &Hash,
    fname: &str,
    spec: &Field,
    ty: &str,
) {
    let line = key_line(fm_lines, fname);
    let alias_present = s
        .synonyms
        .iter()
        .any(|(a, c)| c == fname && field(fm, a).is_some());
    match field(fm, fname) {
        None => {
            if spec.required && !alias_present {
                push(out, t, 1, fname, format!("missing (required for {ty})"), "required");
            }
        }
        Some(Yaml::Null) => {
            if spec.required {
                push(out, t, line, fname, format!("empty (required for {ty})"), "required");
            }
        }
        Some(v) => {
            let mut kind_ok = true;
            if let Some(k) = spec.kind
                && let Some(msg) = kind_mismatch(k, v)
            {
                push(out, t, line, fname, msg, "value");
                kind_ok = false;
            }
            if let (true, Some(allowed)) = (kind_ok, &spec.values) {
                let items: Vec<String> = match v {
                    Yaml::Array(xs) => xs.iter().filter_map(scalar_str).collect(),
                    other => scalar_str(other).into_iter().collect(),
                };
                for item in items {
                    if !allowed.contains(&item) {
                        push(out, t, line, fname, format!("\"{item}\" not allowed (one of: {})", allowed.join(", ")), "value");
                    }
                }
            }
        }
    }
}

/// The value of a top-level frontmatter key, if present.
fn field<'a>(fm: &'a Hash, k: &str) -> Option<&'a Yaml> {
    fm.get(&Yaml::String(k.to_string()))
}

fn join(xs: &[&String]) -> String {
    xs.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(", ")
}
