# Hazync's land for Ooga Booga Land

A one-page arrival site for visitors who travel to Hazync from
[Ooga Booga Land](https://oogabooga.land/)'s Caves and lands directory. It says in plain terms what
Hazync is and offers three things to do (check a proof in the browser, prove blocks with the Windows
app, watch the board), then sends the visitor back.

It is an **independent land** in OBL's terms (their `docs/cave-connections.md` and
`docs/templates/cave-connection.md`): a separate origin, listed in their directory by one reviewed
registry entry, with a fixed return link. Nothing is shared with the game.

```text
README.md              this file
cave-connection.json   the registry entry submitted to OBL (src/js/cave-directory.js there)
public/index.html      the page; works on direct entry, with no OBL visit
public/style.css       the hazync.org look, cut down to what this page uses
public/favicon.svg     the hazync.org icon
public/fonts/          Schibsted Grotesk + Spline Sans Mono (.woff2), SIL OFL 1.1 (fonts/OFL.txt)
```

## Where it runs, and who owns it

- **Production:** <https://hazync.org/ooga/>. Not in the site's navigation or sitemap; it is an
  arrival page.
- **Deployment:** the hazync.org site (a separate, private repository) carries a byte-for-byte copy of
  `public/` at `/ooga/`, taken from a **pinned commit of this repository's `main`**. Its sync tool
  records the commit and the sha256 of every file, and its pre-deploy check refuses a copy that differs
  from the pin. So what is served is always a revision of this directory that anyone can read here.
- **Owner and operator:** the Hazync maintainer (GitHub `defenwycke`, for the `hazync` organisation),
  who also runs hazync.org. Security reports: see the repository's `SECURITY.md`.
- **Licence:** the code is under the repository's MIT licence ([`../LICENSE`](../LICENSE)); there is
  no separate licence file for this directory. The fonts are the exception: SIL OFL 1.1, with their
  licence in `public/fonts/OFL.txt` (also listed in `THIRD_PARTY_NOTICES.md`).

## Run it locally

There is no build step and nothing to install. The page's internal links (`/explorer/`, `/block-map/`,
...) point at hazync.org's own pages, so they only resolve when served there, but the page itself
renders from any static server:

```sh
python3 -m http.server -d ooga-land/public 8080   # then open http://localhost:8080/
```

## The connection contract, and the check that holds it

`scripts/check-ooga-land.sh` checks the page against OBL's contract and the choices made here, and CI
runs it on every pull request together with its positive control (`--control`), which breaks copies of
the page in each of the ways below and fails unless every one is caught.

- **A literal return link.** At least one `<a href="https://oogabooga.land/" rel="noopener noreferrer"
  referrerpolicy="no-referrer">`, opening in the same tab (no `target`). Any other mention of
  `oogabooga.land` (a query, a fragment, a different path) fails: the return URL is never built from
  anything the visitor brings.
- **No referrer.** `<meta name="referrer" content="no-referrer">`, so no link on the page sends one.
- **No scripts at all.** No `<script>`, no `on...=` handlers, no `javascript:` URLs. With no script,
  nothing reads the query, the fragment, `document.referrer` or storage; there is no state to import,
  nothing to dispose of on page-hide, and back/forward-cache restoration is the browser's plain page.
- **Nothing from another origin.** Every resource the page loads (the stylesheet, the fonts, the icon)
  is a relative path to a file in `public/`. A `Content-Security-Policy` meta (`default-src 'none'`,
  `style-src`/`font-src`/`img-src 'self'`, `base-uri 'none'`, `form-action 'none'`) says the same to
  the browser. Off-site *links* (GitHub, Delving Bitcoin) are plain navigation the visitor chooses; they
  open in a new tab.
- **No inline styles**, so the policy needs no `'unsafe-inline'`.
- **The registry entry** in `cave-connection.json` has exactly OBL's six fields, within their limits,
  `repository` `hazync/hazync` and `url` `https://hazync.org/ooga/`.

```sh
./scripts/check-ooga-land.sh             # the page and the entry
./scripts/check-ooga-land.sh --control   # must catch every broken copy
```

## Network and data

The page loads only its own files from hazync.org (`index.html`, `style.css`, `favicon.svg`, three
fonts). It sets no cookies, uses no storage, has no sign-in, payment, microphone or forms, and receives
nothing from OBL: no account, identity, pose, inventory, chat or voice. hazync.org's web server keeps
its ordinary access log.

## Changing it

Edit `public/`, run the check, open a pull request here. After it merges, the hazync.org copy is
re-synced to the new `main` commit and deployed. If the destination URL or the ownership ever changes,
the registry entry in OBL is updated by a pull request there; if the page is withdrawn, the entry's
`status` becomes `unavailable` or `retired` the same way.
