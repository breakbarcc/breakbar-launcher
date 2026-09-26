# Plan: publishing the Breakbar Launcher website

Goal: a static site at **https://launcher.breakbar.cc** that lists the launcher's features and offers
the download, linked from the main site **https://www.breakbar.cc**. The program itself is
distributed through GitHub releases (see the release workflow and CONTRIBUTING.md).

## 1. Decisions to make first

| Question | Recommendation |
|---|---|
| Where does the site's source live? | In this repository, folder `site/`. One place for version and links, and the deploy can read the newest release. A separate repository is only worth it if others should edit the site independently. |
| Hosting | GitHub Pages (free, HTTPS, no server), deployed by a GitHub Actions workflow. |
| Technology | Eleventy (Nunjucks templates, JSON text files per language, no client framework) in `site/`; English at `/`, German at `/de/`. Chosen over plain HTML because of the second language (one template, two text files) and over Vite + React because the site is content only. See `site/README.md`. |
| Version on the page | Passed to the build as `SITE_VERSION` from the latest GitHub release at deploy time, not fetched in the browser. Works without JavaScript and needs no cross-origin request. |
| Legal pages | The imprint is the one of the main site (https://www.breakbar.cc/impressum, same operator), linked from the footer. The privacy policy is its own page in German and English (`/privacy/`, `/de/datenschutz/`), because the hosting (GitHub Pages), the DNS service (Cloudflare), the downloads (GitHub) and the browser storage (theme) differ from the main site's. Keep it in step with the site and the program. Not legal advice. |
| Analytics | None. No cookies, no third-party scripts (keeps the privacy page short). |

## 2. Steps in order

1. **Revise the design** with `docs/website/design-change-list.md`. Done: the delivered design was
   converted into Eleventy templates (phase 1), the images live in `site/src/assets/`.
2. **Add the site to the repository** as `site/`, with `site/src/CNAME` containing
   `launcher.breakbar.cc`. Done, including the German version and screenshots. The legal pages are
   the imprint of the main site and the site's own privacy policy (see "Legal pages" above).
3. **Add the deploy workflow** `.github/workflows/site.yml` (done, see "Website" in CONTRIBUTING.md):
   - Triggers: push to `release` that changes `site/**` (so the site never runs ahead of the
     released program), pull requests (build only), `workflow_dispatch`.
   - Job: check out, look up the newest release (`gh release view --json tagName`), then
     `npm ci && npm run build` in `site/` with `SITE_VERSION` set, then `actions/upload-pages-artifact`
     (path `site/_site`) and `actions/deploy-pages` (permissions `pages: write`, `id-token: write`, environment
     `github-pages`).
   - Before the first release exists the build uses the version in `Cargo.toml`.
4. **Refresh the site after every release.** A release created by the release workflow with the
   default token does not start other workflows. Add a last step to `release.yml`:
   `gh workflow run site.yml` (needs `actions: write` on that job). Then the version on the page
   updates within a minute of a release. Done: the `site` job of `release.yml`.
5. **Turn on Pages:** repository Settings, Pages, Source "GitHub Actions". Then set the custom domain
   `launcher.breakbar.cc` and later "Enforce HTTPS" (available after the certificate is issued).
6. **DNS** (where breakbar.cc is managed): add `CNAME launcher -> breakbarcc.github.io`. Do not add
   an `A` record. Before that, verify `breakbar.cc` for the organization (Organization settings,
   Pages, Verified domains): it stops others from claiming subdomains that point at GitHub Pages.
7. **Main site entry:** on breakbar.cc add a project entry "Breakbar Launcher" with a short text and
   a link to `https://launcher.breakbar.cc/` (button "Open site"). This is a change on the main site,
   outside this repository.
8. **First release** (see CONTRIBUTING.md: `git push origin main:release`) and check that the
   download button works and that `{{VERSION}}` was replaced.
9. **SignPath application:** the site is the required download page that describes what the program
   does. Apply at https://signpath.org/apply.html with the repository and the site URL, then follow
   "Code signing (SignPath)" in CONTRIBUTING.md.
10. **After signing works:** change the `signed` row on the site, remove the SmartScreen wording that
    says "not signed", add the update check later (separate task).

## 3. All links on the site and where they lead

| Where on the site | Text | Target |
|---|---|---|
| Header | Features / How it works / Download | `#features`, `#how`, `#download` |
| Header, footer | breakbar.cc | https://www.breakbar.cc/ |
| Header, footer | GitHub | https://github.com/breakbarcc/breakbar-launcher |
| Hero, download card | Download for Windows | https://github.com/breakbarcc/breakbar-launcher/releases/latest/download/breakbar-launcher.exe |
| Hero | View on GitHub | https://github.com/breakbarcc/breakbar-launcher |
| Download card | All releases | https://github.com/breakbarcc/breakbar-launcher/releases |
| Download card | SHA-256 | https://github.com/breakbarcc/breakbar-launcher/releases/latest/download/breakbar-launcher.exe.sha256 |
| Download card, footer | Changelog | https://github.com/breakbarcc/breakbar-launcher/blob/main/CHANGELOG.md |
| Footer | License (MIT) | https://github.com/breakbarcc/breakbar-launcher/blob/main/LICENSE |
| Footer, download card | Report a problem | https://github.com/breakbarcc/breakbar-launcher/issues/new |
| Footer | Made with Slint (badge) | https://slint.dev |
| Footer | Imprint | https://www.breakbar.cc/impressum |
| Footer | Privacy | `/privacy/`, `/de/datenschutz/` |
| Not linked, used by the program | update manifest | https://github.com/breakbarcc/breakbar-launcher/releases/latest/download/latest.json |

Links that go back to the site:

| From | To |
|---|---|
| breakbar.cc (main site) | https://launcher.breakbar.cc/ |
| The repository README | add a "Website" line or badge with https://launcher.breakbar.cc/ |
| The program, About section | currently opens https://www.breakbar.cc/; change `WEBSITE_URL` in `crates/bb-app/src/gui/mod.rs` to the launcher site once it is live (a small app change with a new version) |
| SignPath application | https://launcher.breakbar.cc/ as the download page |

## 4. Checklist before announcing

- [ ] Download button starts the download of the newest `breakbar-launcher.exe`, from a private window.
- [ ] Version text on the page equals the version of the newest release.
- [ ] Every link in the table above works (check by hand once, or with a link checker in CI).
- [ ] `https://launcher.breakbar.cc` opens with a valid certificate, `http://` redirects to `https://`.
- [ ] Light and dark theme, phone width (375 px), keyboard only, no horizontal scrolling.
- [ ] The privacy policy is read once against the site as it is (hosting, external links, storage) and against the program (no network access of its own).
- [ ] Open Graph preview looks right (paste the URL into a chat to see the preview).
- [ ] The disclaimer text is in the footer, the Slint badge is shown, no game logos or artwork.
- [ ] The FAQ answer on "Is it allowed?" is the reviewed one.

## 5. Open points that need you

- Whether the privacy policy is right for you (a legal review is up to you; the text is a draft).
- The exact wording of the FAQ answer about permission and risk.
- DNS access for breakbar.cc and the change on the main site.
- Whether the site source stays in this repository (recommended) or moves out.
