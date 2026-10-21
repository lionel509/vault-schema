# Example vault schema

The checker reads the first `yaml` block below. Everything else is notes for humans.

```yaml
root: vault
exclude:
  - Private
skip_names:
  - Templates
  - node_modules
synonyms:
  created_date: created
  captured: created
max_topic_tags: 3
scopes:
  Kitchen:
    domain: [kitchen]
    min_uses: 2
    topics: [noodles]
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
