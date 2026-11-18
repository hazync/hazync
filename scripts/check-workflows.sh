#!/usr/bin/env bash
# Fail if any GitHub Actions workflow or local composite-action file is not valid YAML, or repeats a
# mapping key at any level.
#
#   ./scripts/check-workflows.sh
#
# Trap 1, invalid YAML: an unquoted "colon space" inside a step `name:` (e.g. "regression: v0.9.0")
# reparses as a nested mapping and breaks the ENTIRE workflow run — GitHub reports a bare failure with no
# jobs, which is easy to misread.
#
# Trap 2, a duplicate key: YAML loaders (PyYAML, and GitHub's own) silently keep the LAST value, so a
# repeated job key deletes the first job with no error anywhere. It happened: #724 and #731 each added a
# `zapps:` job to adversarial.yml, the merge was textually clean, and one of the two jobs vanished
# (fixed in b81704b). A plain safe_load reports that file as valid, so this gate used to pass it.
#
# Run this locally before pushing any workflow edit; CI runs it on every push. The positive controls at the
# bottom prove the checker still FAILS on both shapes, so it cannot go green by checking nothing.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 -c "import yaml" 2>/dev/null || pip install --quiet pyyaml

scan() {   # $@ = YAML files; prints one line per file; exits non-zero if any file is bad
    python3 - "$@" <<'PY'
import sys, yaml
from yaml.constructor import ConstructorError

class StrictLoader(yaml.SafeLoader):
    """SafeLoader that raises on a repeated mapping key instead of keeping the last one."""

def construct_mapping(loader, node, deep=False):
    if isinstance(node, yaml.MappingNode):
        seen = {}
        for key_node, _ in node.value:
            # A `<<` merge key is how YAML overrides inherited keys on purpose; it is not a duplicate.
            if key_node.tag == "tag:yaml.org,2002:merge":
                continue
            key = loader.construct_object(key_node, deep=True)
            try:
                hash(key)
            except TypeError:
                continue   # unhashable key: construct_mapping itself raises on it below
            if key in seen:
                raise ConstructorError(
                    "while constructing a mapping", node.start_mark,
                    f"duplicate key {key!r} (first at line {seen[key].line + 1})", key_node.start_mark)
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
            yaml.load(fh, Loader=StrictLoader)
        print(f"ok   {f}")
    except Exception as e:
        print(f"FAIL {f}: {e}"); bad += 1
sys.exit(1 if bad else 0)
PY
}

# BOTH extensions, workflows AND local composite actions: GitHub honours .yaml as well as .yml, and an
# action.yml is loaded the same way, so a file this glob misses is a file the gate reports "ok" over unread.
shopt -s nullglob
files=(.github/workflows/*.yml .github/workflows/*.yaml .github/actions/*/action.yml .github/actions/*/action.yaml)
shopt -u nullglob
scan "${files[@]}"

# ── positive controls ─────────────────────────────────────────────────────────────────────────────
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
control() {   # $1 = description; the fixture in $TMP/control.yml must FAIL the scan
    if scan "$TMP/control.yml" >/dev/null 2>&1; then
        echo "FAIL positive control passed — the checker cannot detect $1" >&2
        exit 1
    fi
    echo "ok   positive control: $1 is still detected"
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
control "a duplicate job key"
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
control "a duplicate key inside a step"
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
control "an unquoted colon in a step name"
# Negative control: a merge key overriding an anchored value is legitimate YAML, not a duplicate.
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    env: &base
      A: "1"
      B: "2"
    steps:
      - env:
          <<: *base
          B: "3"
        run: echo ok
YAML
if ! scan "$TMP/control.yml" >/dev/null 2>&1; then
    echo "FAIL negative control: a merge-key override was refused as a duplicate" >&2
    exit 1
fi
echo "ok   negative control: a merge-key override is accepted"
