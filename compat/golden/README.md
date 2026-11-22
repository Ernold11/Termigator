# Golden fixtures

`compat/golden/*.json` are reference cases (input and expected result) that the Rust tests check.

## Layout

- One file per module and topic: `<module>_<topic>.json` (or `<module>.json` when there is one topic),
  e.g. `yaml_load.json`, `yaml_dump.json`, `yaml_sync.json`.
- File inputs live next to the Rust tests, e.g. `crates/termigator-core/tests/fixtures/<module>/`.
  Fixtures are **synthetic**: never copy anything from a real configuration (real hosts and passwords).
  `scripts/secret-audit.sh` checks this (optionally with gitleaks and `.gitleaks.toml`; `compat/golden/`
  is allowlisted).
- The files are deterministic (no dates, randomness or directory order), so `git diff` shows exactly
  which behaviour changed.

## JSON format

```json
{
 "cases": [
  {"name": "type:plain:yes", "input": "yes\n", "expect": true},
  {"name": "file:config_commented.yaml", "file": "crates/termigator-core/tests/fixtures/yaml/config_commented.yaml", "expect": {"$map": []}},
  {"name": "err_multi_doc", "input": "a: 1\n---\nb: 2\n", "error": "ComposerError"}
 ]
}
```

- `cases[].name` is unique within a file and appears in test failure messages.
- Input: `"input"` (inline text) or `"file"` (a path relative to the repository root).
- Result: `"expect"`, or `"error"` (an error class name, e.g. `ComposerError`). The Rust tests only check that
  they fail too; message texts are compared only where a test says so.
- Other fields are module-specific; the test of that module reads them.
- Encoding: UTF-8, non-ASCII kept as is, `indent=1`, trailing newline.

### Values

| Value | JSON |
|---|---|
| null, boolean, string, list | `null`, `true`/`false`, string, array |
| integer | `{"$int": "511"}` (text, arbitrary precision) |
| float | `{"$float": "<repr>"}`, e.g. `"1.5"`, `"inf"`, `"-inf"`, `"nan"` |
| map | `{"$map": [[key, value], …]}`, keeps the order and non-string keys |
| date / date-time | `{"$timestamp": "<ISO 8601>"}` (a space between date and time) |
| `bytes`, text that is not UTF-8 | `{"$bytes": "<lowercase hex>"}` |

An object with exactly one key starting with `$` is always a tagged value. Strings with lone
surrogates (not representable as a Rust `String`) are not in the fixtures.

## Rust tests

`crates/termigator-core/tests/common/mod.rs` (`mod common;` in a test file):

- `golden("yaml_load")` returns the `cases` as `Vec<serde_json::Value>`,
- `input_text(&case)` returns `"input"` or the contents of `"file"`,
- `bytes(&v)` decodes a string or `{"$bytes"}` into `Vec<u8>`,
- `tagged(&v)` returns `Some(("$int", payload))` for a tagged value.

Decoding values into a module's types is up to that module's test (e.g. `from_golden` in `tests/yaml.rs`).
