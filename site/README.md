# Website (launcher.breakbar.cc)

A static site built with [Eleventy](https://www.11ty.dev/) (Nunjucks templates, no client framework).
English lives at `/`, German at `/de/`. The output of `npm run build` is plain HTML, one small
`script.js` (theme, menu, screenshot toggle) and `style.css`.

```
npm install        # once
npm start          # preview at http://localhost:8080 (rebuilds on save)
npm run build      # writes _site/
```

Node 20 or newer.

## Where things are

| What | Where |
|---|---|
| All texts, per language | `src/_data/text/en.json`, `de.json` (same keys; the build fails if `de.json` lacks one) |
| Languages, URL prefixes, screenshot folder | `src/_data/locales.js` |
| Version, links | `src/_data/site.js` |
| Page templates | `src/home.njk`, `notfound.njk`, `sitemap.xml.njk` |
| Header, footer, `<head>` | `src/_includes/` |
| Styles, script | `src/style.css`, `src/script.js` |
| Images | `src/assets/` (see the README there) |

The version on the page is `SITE_VERSION` (set by the deploy to the newest release); without it the
build reads the workspace version from `Cargo.toml`.

## Adding a language

1. Add an entry to `src/_data/locales.js` and copy `en.json` to `xx.json`, translate it.
2. Render its screenshots (`BREAKBAR_PREVIEW_LANG=xx`, see `src/assets/README.md`) into
   `src/assets/shots/xx/` and point `shots` at that folder.

The site has no imprint or privacy page of its own: the footer links go to the pages of the main site
(`urls.imprint`, `urls.privacy` in `src/_data/site.js`). Those pages have to cover this site too (see
`docs/website/implementation-plan.md`, "Legal pages").
