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
# What GitHub calls the same key (its workflow parser as the runner carries it, actions/runner
# src/Sdk/WorkflowParser: Conversion/YamlObjectReader.cs AllowLiteral, ObjectTemplating/TemplateReader.cs, the
# Tokens/*.cs ToString; checked 18.11.2026): a key is first TYPED, then turned back into text, then compared
# case-INsensitively (HashSet with StringComparer.OrdinalIgnoreCase, "'x' is already defined").
#   - Typing, YAML 1.2 core schema, PLAIN (unquoted) scalars only: null is `` / `null` / `Null` / `NULL` / `~`; a
#     boolean is true/True/TRUE, false/False/FALSE (so `on`, `yes` are strings); an integer is [0-9]+, [-+][0-9]+,
#     0x<hex> (lower-case x; a 32-bit int) or 0o<octal> (32-bit); a float is
#     [-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)? or [-+].inf / .nan. A quoted or block scalar, or one tagged
#     `!!str`, is its text. Any other tag, or a tagged value that does not match its tag, refuses the whole file.
#   - Back to text: NumberToken.ToString() is the double's ToString("G15", InvariantCulture); BooleanToken's is
#     "true"/"false"; NullToken's is "" (the empty string).
# So `1`, `1.0`, `+1`, `01`, `0x1`, `0o1`, `1e0` and `"1"` are ONE key, as are `~`, `null`, `NULL` and `""`, and
# `true`, `True` and `"TRUE"`; `on:` and `true:` are TWO, and so are `"1.0"` and `1` (a quoted key keeps its text).
# That is what canonical_key() below computes; PyYAML's own values are never used (it reads `on` as True and
# `0b1` / `1_000` as numbers, YAML 1.1). actions/languageservices (the editor extension's parser) hands a pair's
# key to its template reader as the raw value of its YAML library (NumberToken's `${num}` is not reached for a key);
# it is the editor's parser, not the one that runs a workflow, and is not modelled. Upper-casing here is per character and only where the upper case is ONE character (as .NET's
# OrdinalIgnoreCase, unlike Python's str.upper(), which makes 'ß' 'SS').
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
import re, sys, yaml
from yaml.constructor import ConstructorError

MERGE = "tag:yaml.org,2002:merge"

STR, BOOL, INT, FLOAT, NULL = (f"tag:yaml.org,2002:{t}" for t in ("str", "bool", "int", "float", "null"))

class KeyError_(ValueError):
    pass

def match_null(t):
    return t in ("", "null", "Null", "NULL", "~")

def match_bool(t):
    return {"true": True, "True": True, "TRUE": True, "false": False, "False": False, "FALSE": False}.get(t)

def int32(v, t):
    # .NET Int32.TryParse(AllowHexSpecifier) / Convert.ToInt32(s, 8): up to 32 bits, read as two's complement
    if v >= 1 << 32:
        raise KeyError_(f"key {t!r} is out of range for GitHub's 32-bit integer (GitHub refuses the file)")
    return float(v - (1 << 32) if v >= 1 << 31 else v)

def match_int(t):
    if re.fullmatch(r"[0-9]+|[-+][0-9]+", t):
        return float(t)                      # Double.TryParse: past the range it is +-Infinity
    if re.fullmatch(r"0x[0-9a-fA-F]+", t):
        return int32(int(t[2:], 16), t)
    if re.fullmatch(r"0o[0-7]+", t):
        return int32(int(t[2:], 8), t)
    return None

def match_float(t):
    if t in (".inf", ".Inf", ".INF", "+.inf", "+.Inf", "+.INF"):
        return float("inf")
    if t in ("-.inf", "-.Inf", "-.INF"):
        return float("-inf")
    if t in (".nan", ".NaN", ".NAN"):
        return float("nan")
    if re.fullmatch(r"[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?", t):
        return float(t)
    return None

def g15(x):
    # .NET Double.ToString("G15", CultureInfo.InvariantCulture); Python's ".15G" is the same format (15 significant
    # digits, trailing zeros dropped, E+XX with two exponent digits at least, "-0" for minus zero) but for these three
    if x != x:
        return "NaN"
    if x in (float("inf"), float("-inf")):
        return "Infinity" if x > 0 else "-Infinity"
    return format(x, ".15G")

def to_text(v):
    if v is None:
        return ""
    if isinstance(v, bool):
        return "true" if v else "false"
    return g15(v)

def canonical_key(node):
    """The text GitHub compares a mapping key by (see the header): typed, then stringified, then upper-cased."""
    t, tag, plain = node.value, node.gh_tag, node.style is None
    if tag == STR or (tag is None and not plain):
        text = t
    elif tag is None:
        if match_null(t):
            text = to_text(None)
        elif match_bool(t) is not None:
            text = to_text(match_bool(t))
        else:
            n = match_int(t)
            n = match_float(t) if n is None else n
            text = t if n is None else to_text(n)
    else:
        if not plain:
            raise KeyError_(f"key {t!r}: GitHub takes the tag {tag} only on a plain scalar (it refuses the file)")
        if tag == NULL:
            ok, v = match_null(t), None
        elif tag in (BOOL, INT, FLOAT):
            v = {BOOL: match_bool, INT: match_int, FLOAT: match_float}[tag](t)
            ok = v is not None
        else:
            raise KeyError_(f"key {t!r}: tag {tag} (GitHub's parser takes only !!str, !!bool, !!int, !!float, !!null)")
        if not ok:
            raise KeyError_(f"key {t!r} is not a valid {tag} (GitHub refuses the file)")
        text = to_text(v)
    return "".join(c.upper() if len(c.upper()) == 1 else c for c in text)

class StrictLoader(yaml.SafeLoader):
    """SafeLoader that refuses a repeated mapping key (GitHub's notion of one) and any `<<` merge key."""
    def compose_scalar_node(self, anchor):
        # keep whether the tag was EXPLICIT (PyYAML otherwise stores the tag it resolved, YAML 1.1's)
        ev = self.peek_event()
        node = super().compose_scalar_node(anchor)
        node.gh_tag = ev.tag
        return node

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
            try:
                key = canonical_key(key_node)
            except KeyError_ as e:
                raise ConstructorError("while constructing a mapping", node.start_mark, str(e), key_node.start_mark)
            if key in seen:
                first_mark, first_text = seen[key]
                raise ConstructorError(
                    "while constructing a mapping", node.start_mark,
                    f"duplicate key {key_node.value!r} (GitHub reads it as {key!r}, as {first_text!r} at line "
                    f"{first_mark.line + 1}; it types a plain key, turns it back into text and compares "
                    "case-insensitively)", key_node.start_mark)
            seen[key] = (key_node.start_mark, key_node.value)
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
# Keys GitHub types first: a plain number, null or boolean is compared by the text GitHub writes back
# (NumberToken G15, "" for null, "true"/"false"), so these spellings are one key each.
for pair in "1|1.0" "16|0x10" "16|0o20" "1|+1" "1|01" "10|1e1" "1|'1'" "~|null" "~|''" "NULL|''" "true|'TRUE'" "1e15|1000000000000000"; do
    a=${pair%%|*} b=${pair#*|}
    printf 'name: control\non: [push]\njobs:\n  build:\n    runs-on: ubuntu-latest\n    strategy:\n      matrix:\n        %s: [x]\n        %s: [y]\n    steps:\n      - run: echo a\n' "$a" "$b" > "$TMP/control.yml"
    control "the keys $a and $b (one key to GitHub)" "duplicate key '${b//\'/}'"
done
# A key GitHub cannot read refuses the file: a hex integer past 32 bits, a value that does not match its tag, a tag
# GitHub's parser does not take.
for k in "0x100000000|out of range" "!!int 1.5|is not a valid tag:yaml.org,2002:int" "!!binary aGk=|tag tag:yaml.org,2002:binary"; do
    key=${k%%|*} why=${k#*|}
    printf 'name: control\non: [push]\njobs:\n  build:\n    runs-on: ubuntu-latest\n    env:\n      %s: x\n    steps:\n      - run: echo a\n' "$key" > "$TMP/control.yml"
    control "the key $key" "$why"
done
# A key that is not a scalar (a sequence as a complex key).
cat > "$TMP/control.yml" <<'YAML'
name: control
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    env:
      ? [a, b]
      : x
    steps:
      - run: echo a
YAML
control "a non-scalar key" "a mapping key that is not a scalar"
# A top level that is not a mapping.
printf -- '- name: control\n- on: [push]\n' > "$TMP/control.yml"
control "a top level that is a list" "top level is a list, not a mapping"
# No files: the scan refuses to pass vacuously (exit 2 with its reason), not "ok" over nothing.
if out=$(scan 2>&1); then
    echo "FAIL positive control passed — the checker passes with no files" >&2; exit 1
else
    rc=$?
fi
if [ "$rc" != 2 ] || ! printf '%s\n' "$out" | grep -qF -- "no workflow files found"; then
    echo "FAIL positive control: no files gave exit $rc, not 2 with 'no workflow files found':" >&2; printf '%s\n' "$out" >&2; exit 1
fi
echo "ok   positive control: no files is still refused (exit 2, no workflow files found)"
# Keys that look alike but GitHub keeps apart: `on` (a string) and `true` (a boolean, "true"); the quoted "1.0" (its
# text) and the number 1 ("1"); `0X10` (not GitHub's hex: upper-case X, a string) and 16; `yes` (a string, not YAML
# 1.1's boolean) and `true`; `1_000` (a string, not YAML 1.1's 1000) and 1000; and a plain alias.
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
        "1.0": [a]
        1: [b]
        0X10: [c]
        16: [d]
        yes: [e]
        true: [f]
        1_000: [g]
        1000: [h]
    steps:
      - env: *base
        run: echo ok
YAML
accept "keys that differ to GitHub though alike in text or in PyYAML's value (on/true, \"1.0\"/1, 0X10/16, yes/true, 1_000/1000), and a plain alias"
