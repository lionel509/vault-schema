//! Parses the schema note. The schema is the first ```yaml fence in the file.

use std::fs;
use std::path::{Component, Path, PathBuf};

use yaml_rust2::yaml::Hash;
use yaml_rust2::{Yaml, YamlLoader};

pub struct Schema {
    /// The schema root as an absolute path, for messages.
    pub root_disp: String,
    /// Lexically normalised and lowercased root.
    pub root_lex: PathBuf,
    /// Canonical root (symlinks resolved) and lowercased.
    pub root_canon: PathBuf,
    /// Root-relative, normalised, lowercased.
    pub exclude: Vec<PathBuf>,
    /// Folder names to skip at any depth, lowercased.
    pub skip_names: Vec<String>,
    /// (alias, canonical field name).
    pub synonyms: Vec<(String, String)>,
    pub scopes: Vec<Scope>,
}

pub struct Scope {
    /// Root-relative, normalised, lowercased.
    pub path: PathBuf,
    pub domain: Vec<String>,
    pub max_topic: usize,
    /// (content-type name, declared fields in schema order).
    pub types: Vec<(String, Vec<(String, Field)>)>,
}

pub struct Field {
    pub required: bool,
    pub kind: Option<Kind>,
    pub values: Option<Vec<String>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Date,
    Number,
    List,
    Str,
}

impl Kind {
    fn parse(s: &str) -> Option<Kind> {
        match s {
            "date" => Some(Kind::Date),
            "number" => Some(Kind::Number),
            "list" => Some(Kind::List),
            "string" => Some(Kind::Str),
            _ => None,
        }
    }
}

/// Lexical normalisation: drops `.`, pops on `..` (never past a root or prefix).
pub fn normalise(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The working directory as the shell spells it. `current_dir` returns the
/// physical path (`/private/var/...` for `/var/...` on macOS), so a shell's
/// `$PWD` is used when it names the same directory. Otherwise the lexical
/// checks would miss a `/var` spelling and fall through to `canonicalize`.
pub fn logical_cwd() -> Result<PathBuf, String> {
    let phys = std::env::current_dir().map_err(|e| format!("cannot read current dir: {e}"))?;
    if let Some(pwd) = std::env::var_os("PWD").map(PathBuf::from)
        && pwd.is_absolute()
        && fs::canonicalize(&pwd).ok().as_ref() == Some(&phys)
    {
        return Ok(normalise(&pwd));
    }
    Ok(phys)
}

/// Lowercased copy of a path, for case-insensitive comparison.
pub fn lc(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().to_lowercase())
}

/// A YAML scalar as text, or None for lists, maps and nulls.
pub fn scalar_str(y: &Yaml) -> Option<String> {
    match y {
        Yaml::String(s) | Yaml::Real(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

pub fn load(path: &Path) -> Result<Schema, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("schema: cannot read {}: {e}", path.display()))?;
    let (block, fence) = yaml_block(&text)?;
    let docs = YamlLoader::load_from_str(&block).map_err(|e| {
        format!(
            "schema: invalid YAML at line {}: {}",
            fence + 1 + e.marker().line(),
            e.info()
        )
    })?;
    let top = match docs.into_iter().next() {
        None | Some(Yaml::Null) => Hash::new(),
        Some(Yaml::Hash(h)) => h,
        Some(_) => return Err("schema: top level must be a mapping".into()),
    };

    let mut root = ".".to_string();
    let mut exclude = Vec::new();
    let mut skip_names = Vec::new();
    let mut synonyms = Vec::new();
    let mut max_topic = 3usize;
    let mut scopes_h: Option<&Hash> = None;
    for (k, v) in top.iter() {
        match key(k)? {
            "root" => root = string(v, "root")?,
            "exclude" => {
                for s in str_list(v, "exclude")? {
                    exclude.push(lc(&rel_path(&s, "exclude entry")?));
                }
            }
            "skip_names" => {
                skip_names = str_list(v, "skip_names")?
                    .into_iter()
                    .map(|s| s.to_lowercase())
                    .collect()
            }
            "synonyms" => synonyms = str_map(v, "synonyms")?,
            "max_topic_tags" => max_topic = uint(v, "max_topic_tags")?,
            "scopes" => scopes_h = Some(as_map(v, "scopes")?),
            other => return Err(format!("schema: unknown key {other} in schema")),
        }
    }

    let scopes_h = scopes_h.ok_or_else(|| "schema: scopes is required".to_string())?;
    if scopes_h.is_empty() {
        return Err("schema: scopes must not be empty".into());
    }
    let mut scopes = Vec::new();
    for (k, v) in scopes_h.iter() {
        let name = key(k)?;
        let path = rel_path(name, "scope path")?;
        let mut domain = None;
        let mut types = None;
        let mut scope_max = max_topic;
        let where_ = format!("scope {name}");
        for (fk, fv) in as_map(v, &where_)?.iter() {
            match key(fk)? {
                "domain" => domain = Some(str_list(fv, &format!("domain in {where_}"))?),
                "types" => types = Some(parse_types(fv, name)?),
                "max_topic_tags" => scope_max = uint(fv, &format!("max_topic_tags in {where_}"))?,
                other => return Err(format!("schema: unknown key {other} in {where_}")),
            }
        }
        let domain = domain
            .filter(|d| !d.is_empty())
            .ok_or_else(|| format!("schema: {where_} needs a non-empty domain"))?;
        let types = types
            .filter(|t| !t.is_empty())
            .ok_or_else(|| format!("schema: {where_} needs non-empty types"))?;
        scopes.push(Scope { path: lc(&path), domain, max_topic: scope_max, types });
    }

    for (alias, _) in &synonyms {
        let declared = scopes
            .iter()
            .flat_map(|s| s.types.iter())
            .any(|(_, fields)| fields.iter().any(|(f, _)| f == alias));
        if declared {
            return Err(format!("schema: {alias} is both a synonym and a field"));
        }
    }

    let base = normalise(&logical_cwd()?.join(path));
    let dir = base.parent().unwrap_or(Path::new("/"));
    let root_abs = normalise(&dir.join(&root));
    let root_canon = fs::canonicalize(&root_abs)
        .map_err(|e| format!("schema: root {root} not found: {e}"))?;

    Ok(Schema {
        root_disp: root_abs.display().to_string(),
        root_lex: lc(&root_abs),
        root_canon: lc(&root_canon),
        exclude,
        skip_names,
        synonyms,
        scopes,
    })
}

/// Returns the text between the first ```yaml line and the next ``` line,
/// and the 0-based index of the opening fence.
fn yaml_block(text: &str) -> Result<(String, usize), String> {
    let lines: Vec<&str> = text.lines().collect();
    let none = || "schema: no yaml block".to_string();
    let start = lines
        .iter()
        .position(|l| l.trim_end() == "```yaml")
        .ok_or_else(none)?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_end() == "```")
        .map(|i| i + start + 1)
        .ok_or_else(none)?;
    Ok((lines[start + 1..end].join("\n"), start))
}

fn key(k: &Yaml) -> Result<&str, String> {
    match k {
        Yaml::String(s) => Ok(s),
        other => Err(format!("schema: non-string key {}", scalar_str(other).unwrap_or_else(|| "?".into()))),
    }
}

fn string(v: &Yaml, what: &str) -> Result<String, String> {
    match v {
        Yaml::String(s) => Ok(s.clone()),
        _ => Err(format!("schema: {what} must be a string")),
    }
}

fn str_list(v: &Yaml, what: &str) -> Result<Vec<String>, String> {
    match v {
        Yaml::Null => Ok(Vec::new()),
        Yaml::Array(items) => items
            .iter()
            .map(|i| match i {
                Yaml::String(s) => Ok(s.clone()),
                _ => Err(format!("schema: {what} must be a list of strings")),
            })
            .collect(),
        _ => Err(format!("schema: {what} must be a list of strings")),
    }
}

fn str_map(v: &Yaml, what: &str) -> Result<Vec<(String, String)>, String> {
    match v {
        Yaml::Null => Ok(Vec::new()),
        Yaml::Hash(h) => h
            .iter()
            .map(|(k, v)| Ok((key(k)?.to_string(), string(v, what)?)))
            .collect(),
        _ => Err(format!("schema: {what} must be a map of strings")),
    }
}

fn uint(v: &Yaml, what: &str) -> Result<usize, String> {
    match v {
        Yaml::Integer(i) if *i >= 0 => Ok(*i as usize),
        _ => Err(format!("schema: {what} must be a non-negative integer")),
    }
}

fn as_map<'a>(v: &'a Yaml, what: &str) -> Result<&'a Hash, String> {
    match v {
        Yaml::Hash(h) => Ok(h),
        _ => Err(format!("schema: {what} must be a map")),
    }
}

/// A schema path (exclude entry or scope path): relative, no `..`, not empty.
/// Returned normalised.
fn rel_path(s: &str, what: &str) -> Result<PathBuf, String> {
    let p = Path::new(s);
    if p.is_absolute() || p.has_root() {
        return Err(format!("schema: {what} {s:?} must be relative"));
    }
    if p.components().any(|c| c == Component::ParentDir) {
        return Err(format!("schema: {what} {s:?} must not contain .."));
    }
    let n = normalise(p);
    if n.as_os_str().is_empty() {
        return Err(format!("schema: {what} {s:?} must not be empty"));
    }
    Ok(n)
}

type Types = Vec<(String, Vec<(String, Field)>)>;

fn parse_types(v: &Yaml, scope: &str) -> Result<Types, String> {
    let mut out = Vec::new();
    for (tk, tv) in as_map(v, &format!("types in scope {scope}"))?.iter() {
        let tname = key(tk)?.to_string();
        let where_ = format!("type {tname} in scope {scope}");
        let fields = match tv {
            Yaml::Null => Vec::new(),
            Yaml::Hash(fh) => {
                let mut fields = Vec::new();
                for (fk, fv) in fh.iter() {
                    let fname = key(fk)?.to_string();
                    if fname == "tags" {
                        return Err(format!("schema: tags cannot be a field ({where_})"));
                    }
                    let field = parse_field(&fname, fv, &where_)?;
                    fields.push((fname, field));
                }
                fields
            }
            _ => return Err(format!("schema: {where_} must be a map of fields")),
        };
        out.push((tname, fields));
    }
    Ok(out)
}

fn parse_field(name: &str, spec: &Yaml, type_where: &str) -> Result<Field, String> {
    let where_ = format!("field {name} of {type_where}");
    let mut f = Field { required: false, kind: None, values: None };
    let h = match spec {
        Yaml::Null => return Ok(f),
        Yaml::Hash(h) => h,
        _ => return Err(format!("schema: {where_} must be a map")),
    };
    for (k, v) in h.iter() {
        match key(k)? {
            "required" => match v {
                Yaml::Boolean(b) => f.required = *b,
                _ => return Err(format!("schema: required must be true or false in {where_}")),
            },
            "kind" => {
                let s = string(v, &format!("kind in {where_}"))?;
                f.kind = Some(Kind::parse(&s).ok_or_else(|| {
                    format!("schema: unknown kind {s} in {where_} (want date|number|list|string)")
                })?);
            }
            "values" => match v {
                Yaml::Null => {}
                Yaml::Array(items) => {
                    let mut vals = Vec::new();
                    for item in items {
                        vals.push(scalar_str(item).ok_or_else(|| {
                            format!("schema: values in {where_} must be scalars")
                        })?);
                    }
                    f.values = Some(vals);
                }
                _ => return Err(format!("schema: values in {where_} must be a list")),
            },
            other => return Err(format!("schema: unknown key {other} in {where_}")),
        }
    }
    Ok(f)
}
