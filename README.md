# vault-schema

Checks Obsidian note frontmatter against one schema note. Reports every note
(or a single note) that breaks the schema, so a writer cannot leave slop
behind unnoticed.

## Usage

    vault-schema <schema.md> <path>...

Each `<path>` is a note or a folder. Folders are walked. Excluded folders are
never opened, listed or stat'ed, whichever spelling the path uses.
A path argument that goes through a symlink inside the root is skipped and named
on stderr. An argument spelled through a symlink outside the root is still
resolved with `canonicalize`.

Output is one line per problem, `path:line: field: message`, then a summary:

    vault-schema: 4 problems in 3 of 5 notes (yaml 1, tags 2, synonym 1)

A `rare` problem counts toward exit 1 like the others. Its counts cover the
whole scope, so a single-file run reads the rest of the scope too (read-only,
same exclusions, nothing reported for them).
A note in the scope that can't be read or has broken frontmatter counts as
unknown; a tag is flagged rare only if it would stay under `min_uses` even if
every unknown note used it. Read failures go to stderr and never change the
exit code.

## Exit codes

- `0`: clean.
- `1`: problems found.
- `2`: usage error, schema error, or an unusable path argument.

## Schema format

The schema is the first ```` ```yaml ```` block in the schema note. Everything
else in the file is for humans. Top-level keys:

- `root`: folder the scopes and excludes are relative to. Default `.` (the
  schema's own folder).
- `exclude`: root-relative folders never opened.
- `skip_names`: folder names skipped at any depth (`Templates`, `node_modules`).
- `synonyms`: alias to canonical field name. An alias is reported at its line
  and suppresses the "missing" error for its canonical field.
- `max_topic_tags`: default cap on topic tags (3).
- `scopes`: required. Map of root-relative folder to:
  - `domain`: non-empty list. Exactly one domain tag per note.
  - `types`: non-empty map of content type to fields. Exactly one content-type
    tag per note. Declare `reference: {}` for a type with no fields.
  - `max_topic_tags`: optional override.
  - `topics`: optional list of reviewed topic tags, the vault's registry. A tag
    listed here is never `rare`.
  - `min_uses`: optional positive integer. A topic tag on a note that is not in
    `topics` and that fewer notes in the scope use is reported as `rare`. Domain
    and content-type tags are never `rare`.

Field spec keys: `required` (bool), `kind` (`date`, `number`, `list`, `string`),
`values` (list of allowed scalars). Anything else is a schema error.

The longest matching scope path wins. Only global rules (synonyms, tag shape)
apply outside every scope. `*.excalidraw.md` files are always skipped.
A nested git repo or worktree (a folder holding `.git`) is skipped and named on
stderr, unless it is the schema root, a scope folder, or an ancestor of one.
Any `.git` entry, including a symlink, marks a nested repo; the link is never followed.

## Follow-ups (not in this version)

- A Claude Code hook that runs the single-file form on every note an agent writes.
- An Obsidian view of the report.
- Auto-fix, flagging unknown fields, and `datetime` kinds.
