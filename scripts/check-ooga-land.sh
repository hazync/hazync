#!/usr/bin/env bash
# Check ooga-land/ (Hazync's page for Ooga Booga Land, served at https://hazync.org/ooga/) against the
# connection contract it is listed under, and against the choices that keep it small enough to review.
#
#   ./scripts/check-ooga-land.sh             # check ooga-land/
#   ./scripts/check-ooga-land.sh --control   # break copies of it in each way below; FAIL unless all are caught
#
# What it holds (ooga-land/README.md explains each):
#   * a literal return link: <a href="https://oogabooga.land/" rel="noopener noreferrer"
#     referrerpolicy="no-referrer">, same tab; any other use of oogabooga.land fails
#   * <meta name="referrer" content="no-referrer">
#   * no scripts of any kind (elements, handlers, javascript: URLs, .js files, script in the SVG), so
#     nothing can read the query, the fragment, document.referrer or storage
#   * every resource the page loads is a relative path to a file inside public/ (no other origin, no
#     protocol-relative URL, and no root-relative path, which would reach outside the land on hazync.org)
#   * a Content-Security-Policy meta that starts from default-src 'none' and allows no inline code
#   * no inline styles, so that policy can stay without 'unsafe-inline'
#   * cave-connection.json has exactly OBL's six fields, within their limits, pointing where it should
#
# WHY A POSITIVE CONTROL: a checker that parses nothing passes everything. --control mutates copies of the
# land (each mutation must apply, so the control cannot pass by matching nothing) and fails if the check
# accepts any of them.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

python3 - "$@" <<'PY'
import html.parser, json, os, re, shutil, sys, tempfile

LAND = 'ooga-land'
RETURN = {'href': 'https://oogabooga.land/', 'rel': 'noopener noreferrer', 'referrerpolicy': 'no-referrer'}
ENTRY_FIELDS = ('id', 'name', 'description', 'repository', 'url', 'status')
ENTRY_WANT = {'id': 'hazync', 'repository': 'hazync/hazync', 'url': 'https://hazync.org/ooga/', 'status': 'open'}
# Attributes whose value the browser FETCHES (a resource), as opposed to navigates to (a link).
RESOURCE_ATTRS = {'src', 'srcset', 'poster', 'data', 'action', 'formaction', 'background', 'manifest', 'xlink:href'}
FORBIDDEN_TAGS = {'script', 'style', 'iframe', 'frame', 'object', 'embed', 'form', 'base', 'applet', 'portal'}


def local_target(public, url):
    """'' if url is a relative path to an existing file under public/, else why not."""
    if re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*:', url):
        return f'{url!r} names a scheme/origin (only relative paths into public/ may be loaded)'
    if url.startswith('//'):
        return f'{url!r} is protocol-relative, i.e. another origin'
    if url.startswith('/'):
        return f'{url!r} is root-relative: it loads from outside the land on the serving site'
    path = url.split('#', 1)[0].split('?', 1)[0]
    full = os.path.normpath(os.path.join(public, path))
    if not (full + os.sep).startswith(os.path.normpath(public) + os.sep):
        return f'{url!r} climbs out of public/'
    if not os.path.isfile(full):
        return f'{url!r} does not resolve to a file in public/'
    return ''


class Page(html.parser.HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.tags = []          # (tag, {attr: value}, line)

    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, {k: (v or '') for k, v in attrs}, self.getpos()[0]))

    handle_startendtag = handle_starttag


def check(root):
    fails = []
    bad = lambda m: fails.append(m)
    public = os.path.join(root, 'public')
    if not os.path.isfile(os.path.join(public, 'index.html')):
        return [f'{public}/index.html: missing']

    for dp, _, fns in os.walk(public):
        for fn in sorted(fns):
            p = os.path.join(dp, fn)
            rel = os.path.relpath(p, root)
            if fn.endswith(('.js', '.mjs', '.cjs', '.wasm')):
                bad(f'{rel}: a script file in the land (it has none, by design)')
            if fn.endswith('.svg'):
                s = open(p, encoding='utf-8').read()
                if re.search(r'<script|\bon[a-z]+\s*=|javascript:', s, re.I):
                    bad(f'{rel}: script inside the SVG')
                if re.search(r'(?:href|src)\s*=\s*["\'](?!#)', s, re.I):
                    bad(f'{rel}: the SVG references another file')
            if fn.endswith('.css'):
                s = open(p, encoding='utf-8').read()
                if '@import' in s:
                    bad(f'{rel}: @import (a stylesheet loads only what it carries)')
                for m in re.finditer(r'url\(\s*["\']?([^"\')]+)', s):
                    why = local_target(os.path.dirname(p), m.group(1).strip())
                    if why:
                        bad(f'{rel}: url() {why}')
                if re.search(r'expression\(|javascript:', s, re.I):
                    bad(f'{rel}: script in CSS')
            if not fn.endswith('.html'):
                continue

            src = open(p, encoding='utf-8').read()
            pg = Page()
            pg.feed(src)
            returns, referrer_meta, csp = 0, False, None
            for tag, a, line in pg.tags:
                at = f'{rel}:{line}'
                if tag in FORBIDDEN_TAGS:
                    bad(f'{at}: <{tag}> is not allowed in the land')
                if tag == 'meta' and a.get('http-equiv', '').lower() == 'refresh':
                    bad(f'{at}: meta refresh (a redirect the visitor did not choose)')
                if tag == 'meta' and a.get('name', '').lower() == 'referrer':
                    referrer_meta = a.get('content', '') == 'no-referrer'
                if tag == 'meta' and a.get('http-equiv', '').lower() == 'content-security-policy':
                    csp = a.get('content', '')
                for k, v in a.items():
                    if k.startswith('on'):
                        bad(f'{at}: event handler {k}= on <{tag}>')
                    if k == 'style':
                        bad(f'{at}: inline style= on <{tag}> (the policy allows no inline styles)')
                    if v.strip().lower().startswith('javascript:'):
                        bad(f'{at}: javascript: URL in {k}=')
                    if 'oogabooga.land' in v.lower() and not (tag == 'a' and k == 'href'):
                        bad(f'{at}: oogabooga.land in <{tag} {k}=> (only the literal return link may name it)')
                for k in RESOURCE_ATTRS & a.keys():
                    for part in (a[k].split(',') if k == 'srcset' else [a[k]]):
                        u = part.strip().split(' ')[0]
                        why = u and local_target(dp, u)
                        if why:
                            bad(f'{at}: <{tag} {k}=> {why}')
                if tag == 'link' and 'href' in a:
                    why = local_target(dp, a['href'])
                    if why:
                        bad(f'{at}: <link rel="{a.get("rel", "")}"> {why}')
                if tag == 'a' and 'oogabooga.land' in a.get('href', '').lower():
                    if all(a.get(k) == v for k, v in RETURN.items()) and 'target' not in a:
                        returns += 1
                    else:
                        bad(f'{at}: a link to oogabooga.land that is not the literal return link '
                            f'(href, rel, referrerpolicy exactly as the contract; no target): {a}')
            if returns == 0:
                bad(f'{rel}: no literal return link to https://oogabooga.land/')
            if not referrer_meta:
                bad(f'{rel}: no <meta name="referrer" content="no-referrer">')
            if csp is None:
                bad(f'{rel}: no Content-Security-Policy meta')
            else:
                d = {x.split()[0]: x.split()[1:] for x in csp.split(';') if x.strip()}
                if d.get('default-src') != ["'none'"]:
                    bad(f"{rel}: CSP does not start from default-src 'none': {csp}")
                for name, vals in d.items():
                    for v in vals:
                        if v not in ("'self'", "'none'"):
                            bad(f'{rel}: CSP {name} allows {v} (only \'self\' or \'none\')')
                for need in ('base-uri', 'form-action'):
                    if d.get(need) != ["'none'"]:
                        bad(f"{rel}: CSP lacks {need} 'none'")

    ep = os.path.join(root, 'cave-connection.json')
    try:
        e = json.load(open(ep, encoding='utf-8'))
    except Exception as x:
        return fails + [f'cave-connection.json: unreadable ({x})']
    if not isinstance(e, dict) or tuple(sorted(e)) != tuple(sorted(ENTRY_FIELDS)):
        bad(f'cave-connection.json: fields must be exactly {ENTRY_FIELDS}, got {sorted(e) if isinstance(e, dict) else e}')
    else:
        for k, v in ENTRY_WANT.items():
            if e[k] != v:
                bad(f'cave-connection.json: {k} is {e[k]!r}, expected {v!r}')
        for k, lim in (('id', 48), ('name', 64), ('description', 240), ('repository', 140), ('url', 2048)):
            v = e[k]
            if not isinstance(v, str) or not 1 <= len(v) <= lim or v != v.strip() or re.search(r'[\x00-\x1f\x7f]', v):
                bad(f'cave-connection.json: {k} must be 1..{lim} characters, unpadded, no control characters')
        if not re.fullmatch(r'[a-z0-9]+(?:-[a-z0-9]+)*', e['id']):
            bad('cave-connection.json: id must be lowercase letters/digits separated by single hyphens')
    return fails


def control():
    page = 'public/index.html'
    ret = '<a class="btn quiet" href="https://oogabooga.land/" rel="noopener noreferrer" referrerpolicy="no-referrer">'
    muts = [
        ('return link without referrerpolicy', page, ret, ret.replace(' referrerpolicy="no-referrer"', '')),
        ('return link without noreferrer', page, ret, ret.replace('noopener noreferrer', 'noopener')),
        ('return link carrying a query', page, ret, ret.replace('land/"', 'land/?from=hazync"')),
        ('return link in a new tab', page, ret, ret.replace('<a ', '<a target="_blank" ')),
        ('no return link at all', page, re.compile(re.escape('href="https://oogabooga.land/"')), 'href="/"'),
        ('meta referrer removed', page, '<meta name="referrer" content="no-referrer">\n', ''),
        ('a local script', page, '</body>', '<script src="favicon.svg"></script>\n</body>'),
        ('an inline handler', page, '<main id="main">', '<main id="main" onload="x()">'),
        ('a javascript: link', page, 'href="/how-it-works/"', 'href="javascript:history.back()"'),
        ('a third-party stylesheet', page, '<link rel="stylesheet" href="style.css">',
         '<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter">'),
        ('a root-relative stylesheet', page, 'href="style.css"', 'href="/assets/site.css"'),
        ('a protocol-relative image', page, '<main id="main">', '<main id="main"><img src="//cdn.example/x.png" alt="">'),
        ('a missing local file', page, 'href="style.css"', 'href="styles.css"'),
        ('an inline style', page, '<main id="main">', '<main id="main" style="color:red">'),
        ('CSP removed', page, re.compile(r'<meta http-equiv="Content-Security-Policy"[^>]*>\n'), ''),
        ('CSP with unsafe-inline', page, "style-src 'self'", "style-src 'self' 'unsafe-inline'"),
        ('a font from a CDN', 'public/style.css', 'url("fonts/schibsted.woff2")', 'url("https://cdn.example/s.woff2")'),
        ('a CSS @import', 'public/style.css', ':root {', '@import "x.css";\n:root {'),
        ('script in the SVG icon', 'public/favicon.svg', '</svg>', '<script>alert(1)</script></svg>'),
        ('a .js file in public/', 'public/app.js', None, 'void 0;\n'),
        ('an extra registry field', 'cave-connection.json', '"status": "open"', '"status": "open",\n  "api": "x"'),
        ('a plain-http registry url', 'cave-connection.json', '"https://hazync.org/ooga/"', '"http://hazync.org/ooga/"'),
    ]
    if check(LAND):
        print('FAIL control: the real land does not pass, so the control would prove nothing')
        return 1
    missed = 0
    for name, f, old, new in muts:
        tmp = tempfile.mkdtemp()
        try:
            root = os.path.join(tmp, 'land')
            shutil.copytree(LAND, root)
            fp = os.path.join(root, f)
            if old is None:
                open(fp, 'w').write(new)
            else:
                s = open(fp, encoding='utf-8').read()
                hit = old.search(s) if hasattr(old, 'search') else (old in s)
                if not hit:
                    print(f'FAIL control: mutation "{name}" did not apply (its anchor is gone); update the control')
                    missed += 1
                    continue
                s = old.sub(new, s) if hasattr(old, 'sub') else s.replace(old, new, 1)
                open(fp, 'w', encoding='utf-8').write(s)
            got = check(root)
            if got:
                print(f'  ok   caught: {name}  ({got[0][:110]})')
            else:
                print(f'FAIL control: NOT caught: {name}')
                missed += 1
        finally:
            shutil.rmtree(tmp)
    print(f'{len(muts) - missed}/{len(muts)} broken copies caught')
    return 1 if missed else 0


if '--control' in sys.argv[1:]:
    sys.exit(control())
fails = check(LAND)
for f in fails:
    print('FAIL', f)
print(f'ooga-land: {len(fails)} failures')
sys.exit(1 if fails else 0)
PY
