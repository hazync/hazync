#!/usr/bin/env bash
# Fail if any GitHub Actions workflow or local action file is not valid YAML, is empty or not a mapping, repeats a
# mapping key at any level, or uses a `<<` merge key.
#
#   ./scripts/check-workflows.sh
#
# Trap 1, invalid YAML: an unquoted "colon space" inside a step `name:` (e.g. "regression: v0.9.0")
# reparses as a nested mapping and breaks the ENTIRE workflow run — GitHub reports a bare failure with no
# jobs, which is easy to misread.
#
# Trap 2, a duplicate key: GitHub REFUSES the whole workflow ("'zapps' is already defined"), so the run fails
# with ZERO jobs and "This run likely failed because of a workflow file issue" — and only after the push. It
# happened: #724 and #731 each added a `zapps:` job to adversarial.yml, the merge was textually clean, and run
# 44171269137 (c656bc0, release/v0.27-integration) ran nothing (fixed in b81704b). PyYAML's safe_load keeps the
# last value without a word, so this gate used to report that file as valid.
#
# What GitHub calls the same key (its template reader, actions/runner TemplateReader.cs and
# actions/languageservices workflow-parser template-reader.ts, checked 18.11.2026): the key's scalar TEXT,
# compared case-INsensitively (HashSet with OrdinalIgnoreCase / toUpperCase). So `on:` and `"on":` are one key,
# `build:` and `Build:` are one key, and `on:` and `true:` are two — which is why keys are compared here by their
# raw scalar text, upper-cased, never by the value PyYAML would construct (it reads `on` as True, and 1 == 1.0).
#
# `<<` merge keys: GitHub supports anchors and aliases (since 2025-09), and its docs point at YAML 1.2.2, which
# has no merge key; the workflow parser passes `<<` through as a plain key. A merge here would also hide the keys
# it brings in from this check. So `<<` is refused outright.
#
# Run this locally before pushing any workflow edit; CI runs it on every push. The controls at the bottom prove
# the checker still fails on each shape FOR THAT REASON, so it cannot go green by checking nothing.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 -c "import yaml" 2>/dev/null || pip install --quiet pyyaml

scan() {   # $@ = YAML files; prints one line per file; exits non-zero if any file is bad
    python3 - "$@" <<'PY'
import sys, yaml
from yaml.constructor import ConstructorError

MERGE = "tag:yaml.org,2002:merge"

class StrictLoader(yaml.SafeLoader):
    """SafeLoader that refuses a repeated mapping key (GitHub's notion of one) and any `<<` merge key."""

def construct_mapping(loader, node, deep=False):
    if isinstance(node, yaml.MappingNode):
        seen = {}
        for key_node, _ in node.value:
            if key_node.tag == MERGE:
                raise ConstructorError("while constructing a mapping", node.start_mark,
                                       "merge key `<<` (GitHub Actions does not merge; write the keys out)",
                                       key_node.start_mark)
            if not isinstance(key_node, yaml.ScalarNode):
                raise ConstructorError("while constructing a mapping", node.start_mark,
                                       "a mapping key that is not a scalar", key_node.start_mark)
            key = key_node.value.upper()   # the raw scalar text (quotes already removed), case-insensitive
            if key in seen:
                raise ConstructorError(
                    "while constructing a mapping", node.start_mark,
                    f"duplicate key {key_node.value!r} (first at line {seen[key].line + 1}; GitHub compares keys "
                    "case-insensitively)", key_node.start_mark)
            seen[key] = key_node.start_mark
    return yaml.SafeLoader.construct_mapping(loader, node, deep)

StrictLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, construct_mapping)

files = sys.argv[1:]
if not files:
    print("no workflow files found — the gate would pass vacuously"); sys.exit(2)
bad = 0
for f in files:
    try:
        with open(f) as fh:
            doc = yaml.load(fh, Loader=StrictLoader)
        if doc is None:
            raise ValueError("empty file (GitHub refuses a workflow with no content)")
        if not isinstance(doc, dict):
            raise ValueError(f"top level is a {type(doc).__name__}, not a mapping")
        print(f"ok   {f}")
    except Exception as e:
        print(f"FAIL {f}: {e}"); bad += 1
sys.exit(1 if bad else 0)
PY
}

# BOTH extensions, workflows AND local actions at any depth under .github/actions: GitHub honours .yaml as well as
# .yml, and an action.yml is loaded the same way, so a file this glob misses is a file the gate reports "ok" over
# unread.
shopt -s nullglob globstar
files=(.github/workflows/*.yml .github/workflows/*.yaml .github/actions/**/action.yml .github/actions/**/action.yaml)
shopt -u nullglob globstar
scan "${files[@]}"

# ── controls ──────────────────────────────────────────────────────────────────────────────────────
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
control() {   # $1 = description, $2 = the reason the scan must give; the fixture in $TMP/control.yml must FAIL for it
    local out
    if out=$(scan "$TMP/control.yml" 2>&1); then
        echo "FAIL positive control passed — the checker cannot detect $1" >&2
        exit 1
    fi
    if ! printf '%s\n' "$out" | grep -qF -- "$2"; then
        echo "FAIL positive control: $1 failed, but not for the reason '$2':" >&2
        printf '%s\n' "$out" >&2
        exit 1
    fi
    echo "ok   positive control: $1 is still detected ($2)"
}
accept() {    # $1 = description; the fixture in $TMP/control.yml must PASS
    if ! scan "$TMP/control.yml" >/dev/null 2>&1; then
        echo "FAIL negative control: $1 was refused" >&2
        scan "$TMP/control.yml" >&2 || true
        exit 1
    fi
    echo "ok   negative control: $1 is accepted"
}
# The exact shape b81704b fixed: the same job key twice at the top of `jobs:`.
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
jobs:
  zapps:
    runs-on: ubuntu-latest
    steps:
      - run: echo first
  other:
    runs-on: ubuntu-latest
    steps:
      - run: echo other
  zapps:
    runs-on: ubuntu-latest
    steps:
      - run: echo second
YAML
control "a duplicate job key" "duplicate key 'zapps'"
# Deeper level: the same key twice inside one step.
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - name: one
        run: echo a
        run: echo b
YAML
control "a duplicate key inside a step" "duplicate key 'run'"
# The same key quoted and unquoted (PyYAML reads `on` as True and "on" as a string, GitHub reads both as "on").
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
"on": [pull_request]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo a
YAML
control "a key repeated quoted and unquoted" "duplicate key 'on'"
# The same key in another case (GitHub compares keys case-insensitively).
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo a
  Build:
    runs-on: ubuntu-latest
    steps:
      - run: echo b
YAML
control "a job key repeated in another case" "duplicate key 'Build'"
# A merge key: refused outright.
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    env: &base
      A: "1"
    steps:
      - env:
          <<: *base
          B: "3"
        run: echo ok
YAML
control "a merge key" "merge key"
# Trap 1: the unquoted colon in a step name.
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - name: regression: v0.9.0
        run: echo a
YAML
control "an unquoted colon in a step name" "mapping values are not allowed here"
# An empty file.
: > "$TMP/control.yml"
control "an empty file" "empty file"
# Keys PyYAML would merge but GitHub keeps apart: `on` and `true` (both True to PyYAML), 1 and 1.0.
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
true: x
jobs:
  build:
    runs-on: ubuntu-latest
    env: &base
      A: "1"
    strategy:
      matrix:
        1: [a]
        1.0: [b]
    steps:
      - env: *base
        run: echo ok
YAML
accept "keys that differ in text but not in PyYAML's value (on/true, 1/1.0), and a plain alias"
