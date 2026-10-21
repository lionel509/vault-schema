//! End-to-end tests. Each one runs the real binary on a fixture built under the
//! temp dir with made-up notes only.

use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;

const SCHEMA: &str = r#"# Test schema

```yaml
root: .
synonyms:
  created_date: created
  captured: created
scopes:
  Kitchen:
    domain: [kitchen]
    types:
      recipe:
        created: {required: true, kind: date}
        status: {required: true, values: [made, want, tweaking]}
        servings: {kind: number}
      reference: {}
  School:
    domain: [course-a, course-b]
    types:
      lecture:
        created: {required: true, kind: date}
      homework:
        due: {required: true, kind: date}
        status: {values: [todo, done]}
        parts: {kind: list}
```
"#;

const GOOD_RECIPE: &str = "---\ntags: [kitchen, recipe, noodles]\ncreated: 2026-10-01\nstatus: made\n---\n# Garlic Noodles\n";

const POISON: &str = "---\ntitle: \"a \"b\" c\"\n---\n";

const INVARIANT_SCHEMA: &str = r#"# Invariant schema

```yaml
root: .
exclude: [Private]
skip_names: [Templates, node_modules]
scopes:
  Kitchen:
    domain: [kitchen]
    types:
      reference: {}
```
"#;

const EXCLUDE_SCHEMA: &str = r#"# Exclude schema

```yaml
root: .
exclude: [Private]
scopes:
  Kitchen:
    domain: [kitchen]
    types:
      recipe:
        created: {required: true, kind: date}
```
"#;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_vault-schema")
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(dir: &Path, args: &[&str]) -> Out {
    run_os(dir, args.iter().map(OsString::from).collect())
}

/// Like `run`, for arguments that are not valid UTF-8.
fn run_os(dir: &Path, args: Vec<OsString>) -> Out {
    // A shell sets PWD on cd, so the child sees the logical path it was started in.
    let o = Command::new(bin())
        .current_dir(dir)
        .env("PWD", dir)
        .args(&args)
        .output()
        .expect("spawn vault-schema");
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// Makes every directory under `p` readable again so it can be removed.
fn restore(p: &Path) {
    let is_dir = fs::symlink_metadata(p).map(|m| m.is_dir()).unwrap_or(false);
    if !is_dir {
        return;
    }
    let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            restore(&e.path());
        }
    }
}

/// A fixture directory that is restored and removed on drop, even on panic.
struct Fx(PathBuf);

impl Drop for Fx {
    fn drop(&mut self) {
        restore(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> Fx {
    let dir = std::env::temp_dir().join(format!("vs-{}-{name}", std::process::id()));
    restore(&dir);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    Fx(dir)
}

fn put(root: &Path, rel: &str, text: &str) -> PathBuf {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, text).unwrap();
    p
}

fn last_line(s: &str) -> &str {
    s.lines().last().unwrap_or("")
}

#[test]
fn clean_vault_exits_0() {
    let fx = fixture("clean");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Kitchen/Garlic Noodles.md", GOOD_RECIPE);
    put(d, "Kitchen/Rice.md", "---\ntags: [kitchen, reference, rice]\n---\n# Rice\n");
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 0, "stdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
    assert_eq!(last_line(&o.stdout), "vault-schema: 0 problems in 2 notes");
}

#[test]
fn unescaped_quote_reports_yaml_at_line() {
    let fx = fixture("quote");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Kitchen/recipe.md", "---\ntags: [kitchen, recipe]\ntitle: \"a \"b\" c\"\n---\n");
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 1, "stdout:\n{}", o.stdout);
    assert!(
        o.stdout.contains("recipe.md:3: frontmatter: invalid YAML"),
        "stdout:\n{}",
        o.stdout
    );
}

#[test]
fn duplicate_key_is_invalid_yaml() {
    let fx = fixture("dup");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(
        d,
        "Kitchen/dup.md",
        "---\ncreated: 2026-10-01\ncreated: 2026-10-02\nstatus: made\n---\n",
    );
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 1, "stdout:\n{}", o.stdout);
    assert!(o.stdout.contains("frontmatter: invalid YAML"), "stdout:\n{}", o.stdout);
}

#[test]
fn unclosed_frontmatter() {
    let fx = fixture("unclosed");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Kitchen/open.md", "---\ntags: [kitchen, recipe]\n# no closing fence\n");
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 1, "stdout:\n{}", o.stdout);
    assert!(
        o.stdout.contains(":1: frontmatter: no closing ---"),
        "stdout:\n{}",
        o.stdout
    );
}

#[test]
fn frontmatter_not_a_mapping() {
    let fx = fixture("notmap");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Kitchen/list.md", "---\n- a\n---\n");
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 1, "stdout:\n{}", o.stdout);
    assert!(
        o.stdout.contains("1: frontmatter: not a mapping"),
        "stdout:\n{}",
        o.stdout
    );
}

#[test]
fn synonym_reported_at_its_line_and_suppresses_missing() {
    let fx = fixture("synonym");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(
        d,
        "Kitchen/Pasta.md",
        r#"---
tags: [kitchen, recipe, pasta]
status: made
created_date: 2026-10-01
---
"#,
    );
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 1, "stdout:\n{}", o.stdout);
    assert!(
        o.stdout.contains(":4: created_date: use `created`"),
        "stdout:\n{}",
        o.stdout
    );
    assert!(!o.stdout.contains("created: missing"), "stdout:\n{}", o.stdout);
}

#[test]
fn tag_rules() {
    let fx = fixture("tags");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(
        d,
        "Kitchen/hashtag.md",
        r##"---
tags: [kitchen, "#recipe"]
created: 2026-10-01
status: made
---
"##,
    );
    put(
        d,
        "Kitchen/nodomain.md",
        r#"---
tags: [recipe, noodles]
created: 2026-10-01
status: made
---
"#,
    );
    put(d, "Kitchen/twotypes.md", "---\ntags: [kitchen, recipe, reference]\n---\n");
    put(
        d,
        "Kitchen/topics.md",
        "---\ntags: [kitchen, recipe, a, b, c, d]\ncreated: 2026-10-01\nstatus: made\n---\n",
    );
    put(d, "Kitchen/scalar.md", "---\ntags: kitchen\n---\n");
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 1, "stdout:\n{}", o.stdout);
    for want in [
        r##"tags: "#recipe": drop the #"##,
        "no domain tag (one of: kitchen)",
        "2 content-type tags",
        "4 topic tags, max 3",
        "tags: must be a list",
    ] {
        assert!(o.stdout.contains(want), "missing {want:?}, stdout:\n{}", o.stdout);
    }
}

#[test]
fn field_rules() {
    let fx = fixture("fields");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Kitchen/missing.md", "---\ntags: [kitchen, recipe]\nstatus: made\n---\n");
    put(
        d,
        "Kitchen/enum.md",
        "---\ntags: [kitchen, recipe]\ncreated: 2026-10-01\nstatus: boiled\n---\n",
    );
    put(
        d,
        "Kitchen/date-short.md",
        "---\ntags: [kitchen, recipe]\ncreated: 2026-1-5\nstatus: made\n---\n",
    );
    put(
        d,
        "Kitchen/date-month.md",
        "---\ntags: [kitchen, recipe]\ncreated: 2026-13-01\nstatus: made\n---\n",
    );
    put(
        d,
        "Kitchen/number.md",
        "---\ntags: [kitchen, recipe]\ncreated: 2026-10-01\nstatus: made\nservings: many\n---\n",
    );
    put(
        d,
        "Kitchen/empty.md",
        "---\ntags: [kitchen, recipe]\ncreated:\nstatus: made\n---\n",
    );
    put(
        d,
        "School/Homework 1.md",
        "---\ntags: [course-a, homework]\ndue: 2026-10-01\nparts: x\n---\n",
    );
    let o = run(d, &["schema.md", "Kitchen", "School"]);
    assert_eq!(o.code, 1, "stdout:\n{}", o.stdout);
    for want in [
        "created: missing (required for recipe)",
        r#"status: "boiled" not allowed (one of: made, want, tweaking)"#,
        r#"created: "2026-1-5" is not a YYYY-MM-DD date"#,
        r#"created: "2026-13-01" is not a YYYY-MM-DD date"#,
        "servings: want number, got string",
        "created: empty (required for recipe)",
        "parts: want list, got string",
    ] {
        assert!(o.stdout.contains(want), "missing {want:?}, stdout:\n{}", o.stdout);
    }
}

#[test]
fn outside_scope_global_only() {
    let fx = fixture("outside");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Home.md", "# Home\n\nNo frontmatter here.\n");
    put(d, "Broken.md", POISON);
    let ok = run(d, &["schema.md", "Home.md"]);
    assert_eq!(ok.code, 0, "stdout:\n{}\nstderr:\n{}", ok.stdout, ok.stderr);
    assert_eq!(last_line(&ok.stdout), "vault-schema: 0 problems in 1 notes");
    let bad = run(d, &["schema.md", "Broken.md"]);
    assert_eq!(bad.code, 1, "stdout:\n{}", bad.stdout);
    assert!(bad.stdout.contains("frontmatter: invalid YAML"), "stdout:\n{}", bad.stdout);
}

#[test]
fn longest_scope_wins() {
    let fx = fixture("longest");
    let d = &fx.0;
    put(
        d,
        "schema.md",
        r#"# Scoped

```yaml
root: .
scopes:
  Kitchen:
    domain: [kitchen]
    types:
      recipe:
        created: {required: true, kind: date}
  Kitchen/Baking:
    domain: [baking]
    types:
      recipe:
        created: {required: true, kind: date}
```
"#,
    );
    put(d, "Kitchen/Baking/Bread.md", "---\ntags: [baking, recipe]\ncreated: 2026-10-01\n---\n");
    put(d, "Kitchen/Soup.md", "---\ntags: [kitchen, recipe]\ncreated: 2026-10-01\n---\n");
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 0, "stdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
    assert_eq!(last_line(&o.stdout), "vault-schema: 0 problems in 2 notes");
}

#[test]
fn summary_counts_by_kind() {
    let fx = fixture("summary");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Kitchen/a.md", "---\ntags: [kitchen]\ntags: [recipe]\n---\n");
    put(
        d,
        "Kitchen/c.md",
        r##"---
tags: [kitchen, "#recipe"]
created: 2026-10-01
status: made
---
"##,
    );
    put(
        d,
        "Kitchen/d.md",
        "---\ntags: [kitchen, recipe, a, b, c, d]\ncreated: 2026-10-01\nstatus: made\n---\n",
    );
    put(
        d,
        "Kitchen/e.md",
        "---\ntags: [kitchen, recipe]\ncreated_date: 2026-10-01\nstatus: made\n---\n",
    );
    put(d, "Kitchen/f.md", GOOD_RECIPE);
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 1, "stdout:\n{}", o.stdout);
    assert_eq!(
        last_line(&o.stdout),
        "vault-schema: 4 problems in 4 of 5 notes (yaml 1, tags 2, synonym 1)"
    );
}

#[test]
fn schema_errors_exit_2() {
    let fx = fixture("schemaerr");
    let d = &fx.0;
    put(d, "Kitchen/x.md", GOOD_RECIPE);
    let cases: [(&str, &str, &str); 5] = [
        ("no-fence.md", "# no fence here\n", "no yaml block"),
        (
            "unknown-key.md",
            "# S\n\n```yaml\nroot: .\nscopes:\n  Kitchen:\n    domain: [kitchen]\n    scope: x\n    types:\n      reference: {}\n```\n",
            "unknown key scope",
        ),
        (
            "kind.md",
            "# S\n\n```yaml\nroot: .\nscopes:\n  Kitchen:\n    domain: [kitchen]\n    types:\n      recipe:\n        created: {kind: datetime}\n```\n",
            "datetime",
        ),
        (
            "exclude-dotdot.md",
            "# S\n\n```yaml\nroot: .\nexclude: [../x]\nscopes:\n  Kitchen:\n    domain: [kitchen]\n    types:\n      reference: {}\n```\n",
            "..",
        ),
        (
            "alias-field.md",
            "# S\n\n```yaml\nroot: .\nsynonyms:\n  captured: created\nscopes:\n  Kitchen:\n    domain: [kitchen]\n    types:\n      recipe:\n        captured: {kind: string}\n```\n",
            "both a synonym and a field",
        ),
    ];
    for (file, text, want) in cases {
        put(d, file, text);
        let o = run(d, &[file, "Kitchen"]);
        assert_eq!(o.code, 2, "{file}: stdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
        assert!(o.stdout.is_empty(), "{file}: stdout not empty:\n{}", o.stdout);
        assert!(o.stderr.contains("vault-schema: schema:"), "{file}: stderr:\n{}", o.stderr);
        assert!(o.stderr.contains(want), "{file}: want {want:?}, stderr:\n{}", o.stderr);
    }
    put(d, "schema.md", SCHEMA);
    let outside = std::env::temp_dir().to_string_lossy().into_owned();
    let o = run(d, &["schema.md", &outside]);
    assert_eq!(o.code, 2, "stdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
    assert!(o.stdout.is_empty(), "stdout not empty:\n{}", o.stdout);
    assert!(o.stderr.contains("is outside the schema root"), "stderr:\n{}", o.stderr);
}

#[test]
fn single_file_arg() {
    let fx = fixture("single");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Kitchen/good.md", GOOD_RECIPE);
    put(d, "Kitchen/bad.md", POISON);
    let ok = run(d, &["schema.md", "Kitchen/good.md"]);
    assert_eq!(ok.code, 0, "stdout:\n{}\nstderr:\n{}", ok.stdout, ok.stderr);
    assert_eq!(last_line(&ok.stdout), "vault-schema: 0 problems in 1 notes");
    let bad = run(d, &["schema.md", "Kitchen/bad.md"]);
    assert_eq!(bad.code, 1, "stdout:\n{}", bad.stdout);
    assert!(bad.stdout.contains("bad.md:2: frontmatter: invalid YAML"), "stdout:\n{}", bad.stdout);
    assert!(!bad.stdout.contains("good.md"), "stdout:\n{}", bad.stdout);
    assert_eq!(last_line(&bad.stdout), "vault-schema: 1 problems in 1 of 1 notes (yaml 1)");
}

#[test]
fn never_opens_skipped_paths() {
    let fx = fixture("invariant");
    let d = fx.0.clone();
    put(&d, "schema.md", INVARIANT_SCHEMA);
    for rel in [
        "Private/deep/n.md",
        ".obsidian/x.md",
        ".trash/y.md",
        "Kitchen/.hidden/z.md",
        "Kitchen/Templates/t.md",
        "Kitchen/node_modules/m.md",
        "Kitchen/d.excalidraw.md",
        "Kitchen/bad.md",
    ] {
        put(&d, rel, POISON);
    }

    // Liveness control: the poison is visible when the walk is allowed to see it.
    let control = run(&d, &["schema.md", "Kitchen/bad.md"]);
    assert_eq!(control.code, 1, "control: poison must be visible\n{}", control.stdout);
    fs::rename(d.join("Kitchen/bad.md"), d.join("Kitchen/bad.txt")).unwrap();

    // (a) Symlinks into the excluded folder are never followed.
    symlink("../Private/deep/n.md", d.join("Kitchen/link.md")).unwrap();
    symlink("../Private", d.join("Kitchen/linkdir")).unwrap();
    for arg in [".", "Kitchen/link.md", "Kitchen/linkdir"] {
        let o = run(&d, &["schema.md", arg]);
        assert_eq!(o.code, 0, "arg {arg:?}: stdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
        assert!(!o.stdout.contains("n.md"), "arg {arg:?} leaked:\n{}", o.stdout);
    }

    // (b) Private is unreadable, so any read of it shows up as an io problem.
    fs::set_permissions(d.join("Private"), fs::Permissions::from_mode(0o000)).unwrap();
    let abs = d.to_string_lossy().into_owned();
    let mut forms: Vec<String> = [
        ".",
        "Private",
        "private",
        "PRIVATE/",
        "./Private/deep/n.md",
        "Kitchen/../Private/deep",
        ".obsidian",
        ".obsidian/x.md",
        "Kitchen/.hidden/z.md",
        "Kitchen/Templates/t.md",
        "kitchen/templates",
        "Kitchen/node_modules",
        "Kitchen/d.excalidraw.md",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    forms.push(abs.clone());
    forms.push(format!("{abs}/Private/deep/n.md"));
    for arg in &forms {
        let o = run(&d, &["schema.md", arg]);
        assert_eq!(o.code, 0, "arg {arg:?}: stdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
        for bad in ["n.md", "x.md", "y.md", "z.md", "t.md", "m.md", "excalidraw", "dir:"] {
            assert!(!o.stdout.contains(bad), "arg {arg:?} leaked {bad}:\n{}", o.stdout);
        }
    }
}

#[test]
fn exclude_is_relative_to_root() {
    let fx = fixture("relroot");
    let vault = fx.0.join("Private").join("vault");
    put(&vault, "schema.md", EXCLUDE_SCHEMA);
    put(&vault, "Kitchen/bad.md", POISON);
    let rel = run(&vault, &["schema.md", "Kitchen"]);
    assert_eq!(rel.code, 1, "stdout:\n{}\nstderr:\n{}", rel.stdout, rel.stderr);
    assert!(rel.stdout.contains("bad.md:2: frontmatter: invalid YAML"), "stdout:\n{}", rel.stdout);
    let schema_abs = vault.join("schema.md");
    let note_abs = vault.join("Kitchen/bad.md");
    let abs = run(
        &vault,
        &[schema_abs.to_str().unwrap(), note_abs.to_str().unwrap()],
    );
    assert_eq!(abs.code, 1, "stdout:\n{}\nstderr:\n{}", abs.stdout, abs.stderr);
}

const NESTED_SCHEMA: &str = r#"# Nested schema

```yaml
root: .
scopes:
  Kitchen:
    domain: [kitchen]
    types:
      reference: {}
  Kitchen/Code:
    domain: [code]
    types:
      reference: {}
```
"#;

#[test]
fn nested_git_repos_are_skipped() {
    let fx = fixture("nested");
    let d = &fx.0;
    put(d, "schema.md", NESTED_SCHEMA);
    put(d, ".git/HEAD", "ref\n");
    put(d, "Kitchen/proj/.git/HEAD", "ref\n");
    put(d, "Kitchen/proj/README.md", "# Proj\n");
    put(d, "Kitchen/wt/.git", "gitdir: x\n");
    put(d, "Kitchen/wt/n.md", POISON);
    // A scope folder that is itself a repo is still checked.
    put(d, "Kitchen/Code/.git", "gitdir: y\n");
    put(d, "Kitchen/Code/bad.md", POISON);
    let all = run(d, &["schema.md", "."]);
    assert_eq!(all.code, 1, "stdout:\n{}\nstderr:\n{}", all.stdout, all.stderr);
    assert!(all.stdout.contains("Code/bad.md:2: frontmatter"), "stdout:\n{}", all.stdout);
    assert!(!all.stdout.contains("proj/"), "proj reported:\n{}", all.stdout);
    assert!(!all.stdout.contains("wt/"), "wt reported:\n{}", all.stdout);
    assert!(all.stderr.contains("proj") && all.stderr.contains("(git repo)"), "stderr:\n{}", all.stderr);
    assert!(all.stderr.contains("wt") && all.stderr.contains("(git repo)"), "stderr:\n{}", all.stderr);
    let one = run(d, &["schema.md", "Kitchen/wt/n.md"]);
    assert_eq!(one.code, 0, "stdout:\n{}\nstderr:\n{}", one.stdout, one.stderr);
    assert!(one.stderr.contains("skipped Kitchen/wt/n.md (git repo)"), "stderr:\n{}", one.stderr);
}

#[test]
fn breaker_tags_are_case_insensitive_like_obsidian() {
    // Obsidian treats #Kitchen and #kitchen as one tag.
    let fx = fixture("tagcase");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    put(d, "Kitchen/Cap.md", "---\ntags: [Kitchen, Recipe]\ncreated: 2026-10-01\nstatus: made\n---\n");
    let o = run(d, &["schema.md", "Kitchen"]);
    assert_eq!(o.code, 0, "capitalised tags flagged:\n{}", o.stdout);
}

#[test]
fn breaker_sigpipe_does_not_panic() {
    // Many problems, and the reader closes after one line. A panic on the
    // broken pipe exits 101.
    let fx = fixture("sigpipe");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    let tags: Vec<String> = (0..20000).map(|i| format!("\"#t{i}\"")).collect();
    put(
        d,
        "Kitchen/Many.md",
        &format!("---\ntags: [kitchen, recipe, {}]\ncreated: 2026-10-01\nstatus: made\n---\n", tags.join(", ")),
    );
    let mut child = Command::new(bin())
        .current_dir(d)
        .env("PWD", d)
        .args(["schema.md", "Kitchen"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn vault-schema");
    {
        use std::io::{BufRead, BufReader};
        let mut first = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut first).unwrap();
    }
    let st = child.wait().unwrap();
    assert_ne!(st.code(), Some(101), "panicked on a closed stdout");
}

#[test]
fn symlink_args_never_resolve_into_excluded() {
    let fx = fixture("symarg");
    let d = &fx.0;
    put(d, "schema.md", INVARIANT_SCHEMA);
    put(d, "Private/deep/n.md", POISON);
    put(d, "Kitchen/ok.md", GOOD_RECIPE);
    symlink("../Private/deep/n.md", d.join("Kitchen/link.md")).unwrap();
    symlink("../Private", d.join("Kitchen/linkdir")).unwrap();
    // Private is unreadable, so a resolve into it fails with permission denied.
    fs::set_permissions(d.join("Private"), fs::Permissions::from_mode(0o000)).unwrap();
    for arg in ["Kitchen/link.md", "Kitchen/linkdir", "Kitchen/linkdir/deep/n.md"] {
        let o = run(d, &["schema.md", arg]);
        assert_eq!(o.code, 0, "arg {arg:?}: stdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
        assert!(o.stderr.contains("skipped"), "arg {arg:?}: stderr:\n{}", o.stderr);
        assert!(!o.stderr.contains("cannot resolve"), "arg {arg:?} resolved:\n{}", o.stderr);
    }
}

#[test]
fn breaker_fifo_named_md_does_not_hang() {
    // An explicit argument that is a FIFO named *.md used to block forever in read.
    let fx = fixture("fifo");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    fs::create_dir_all(d.join("Kitchen")).unwrap();
    let st = Command::new("mkfifo").arg(d.join("Kitchen/Pipe.md")).status().unwrap();
    assert!(st.success());
    let mut child = Command::new(bin())
        .current_dir(d)
        .env("PWD", d)
        .args(["schema.md", "Kitchen/Pipe.md"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn vault-schema");
    let start = std::time::Instant::now();
    let finished = loop {
        if child.try_wait().unwrap().is_some() {
            break true;
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(finished, "hung reading a FIFO named *.md");
}

#[test]
fn breaker_non_utf8_argument_is_exit_2_not_a_panic() {
    let fx = fixture("nonutf8");
    let d = &fx.0;
    put(d, "schema.md", SCHEMA);
    let mut bad = b"Kitchen/".to_vec();
    bad.push(0xff);
    bad.extend_from_slice(b".md");
    let o = run_os(d, vec![OsString::from("schema.md"), OsString::from_vec(bad)]);
    // A panic exits 101. APFS refuses such a name, so exit 2 is the expected result.
    assert_ne!(o.code, 101, "panicked on a non-UTF-8 argument:\n{}", o.stderr);
    assert_ne!(o.code, -1, "killed by a signal:\n{}", o.stderr);
}
