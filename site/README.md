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
| Page templates | `src/home.njk`, `imprint.njk`, `privacy.njk`, `notfound.njk`, `sitemap.xml.njk` |
| Header, footer, `<head>` | `src/_includes/` |
| Styles, script | `src/style.css`, `src/script.js` |
| Images | `src/assets/` (see the README there) |

The version on the page is `SITE_VERSION` (set by the deploy to the newest release); without it the
build reads the workspace version from `Cargo.toml`.

## Adding a language

1. Add an entry to `src/_data/locales.js` and copy `en.json` to `xx.json`, translate it.
2. Put its screenshots in `src/assets/shots/xx/` and point `shots` at them.

The legal pages (`imprint`, `privacy`) still contain `TODO` and are `noindex` until they have text.
