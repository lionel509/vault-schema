// Breaker tests for PR #5 (rare topic tags). Each test states the expected
// behaviour; the ones that fail against the PR head are the findings.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_vault-schema")
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(dir: &Path, args: &[&str]) -> Out {
    let o = Command::new(bin())
        .current_dir(dir)
        .env("PWD", dir)
        .args(args)
        .output()
        .expect("spawn vault-schema");
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn restore(p: &Path) {
    let Ok(m) = fs::symlink_metadata(p) else { return };
    if m.is_dir() {
        let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o755));
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                restore(&e.path());
            }
        }
    } else {
        let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o644));
    }
}

struct Fx(PathBuf);

impl Drop for Fx {
    fn drop(&mut self) {
        restore(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> Fx {
    let dir = std::env::temp_dir().join(format!("breaker-{}-{name}", std::process::id()));
    restore(&dir);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    Fx(dir)
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, text).unwrap();
}

/// One Kitchen scope, min_uses as given, topics as a YAML flow list body.
fn schema(min: &str, topics: &str) -> String {
    format!(
        "# S\n\n```yaml\nroot: vault\nscopes:\n  Kitchen:\n    domain: [kitchen]\n    min_uses: {min}\n    topics: [{topics}]\n    types:\n      recipe: {{}}\n```\n"
    )
}

/// B1: the walk includes a note with broken frontmatter, and the k count
/// is specified "exactly as the walk does". Three notes list `zesty`, so it
/// must not be rare on the two valid notes (min_uses 3).
#[test]
fn breaker_invalid_yaml_note_still_counts_toward_k() {
    let fx = fixture("b1-yaml");
    let d = &fx.0;
    put(d, "schema.md", &schema("3", ""));
    put(d, "vault/Kitchen/Soup.md", "---\ntags: [kitchen, recipe, zesty]\n---\n");
    put(d, "vault/Kitchen/Clean.md", "---\ntags: [kitchen, recipe, zesty]\n---\n");
    put(d, "vault/Kitchen/Broken.md", "---\ntags: [kitchen, recipe, zesty\n---\n");
    let o = run(d, &["schema.md", "vault/Kitchen"]);
    assert!(
        !o.stdout.contains("isn't in the topic list"),
        "zesty is on 3 notes, none should be rare.\ncode {}\nstdout:\n{}",
        o.code,
        o.stdout
    );
}

/// B2: single-file mode on a clean note must not report a problem in a
/// sibling note it never looked at. Here the sibling is unreadable.
#[test]
fn breaker_single_file_does_not_report_unreadable_sibling() {
    let fx = fixture("b2-locked");
    let d = &fx.0;
    put(d, "schema.md", &schema("2", ""));
    put(d, "vault/Kitchen/Soup.md", "---\ntags: [kitchen, recipe, pasta]\n---\n");
    put(d, "vault/Kitchen/Clean.md", "---\ntags: [kitchen, recipe, pasta]\n---\n");
    put(d, "vault/Kitchen/Locked.md", "---\ntags: [kitchen, recipe]\n---\n");
    fs::set_permissions(d.join("vault/Kitchen/Locked.md"), fs::Permissions::from_mode(0o000)).unwrap();
    let o = run(d, &["schema.md", "vault/Kitchen/Soup.md"]);
    assert_eq!(o.code, 0, "stdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
    assert!(!o.stdout.contains("Locked.md"), "stdout:\n{}", o.stdout);
    assert!(!o.stdout.contains("0 of 1"), "summary contradicts itself:\n{}", o.stdout);
    fs::set_permissions(d.join("vault/Kitchen/Locked.md"), fs::Permissions::from_mode(0o644)).unwrap();
}

/// B3: a topic typed with a stray space never matches its tag, and the
/// schema gets no error. The note tagged `noodles` is listed as a topic.
#[test]
fn breaker_topic_with_stray_space_does_not_go_silently_rare() {
    let fx = fixture("b3-space");
    let d = &fx.0;
    put(d, "schema.md", &schema("2", "\"noodles \""));
    put(d, "vault/Kitchen/N1.md", "---\ntags: [kitchen, recipe, noodles]\n---\n");
    let o = run(d, &["schema.md", "vault/Kitchen/N1.md"]);
    assert!(
        o.code == 2 || !o.stdout.contains("isn't in the topic list"),
        "topic \"noodles \" should match noodles or fail the schema.\ncode {}\nstdout:\n{}\nstderr:\n{}",
        o.code,
        o.stdout,
        o.stderr
    );
}
