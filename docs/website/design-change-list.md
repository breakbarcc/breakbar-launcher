# Change list for the site design (for Claude Design)

Feedback on `breakbar-launcher-site` (index.html, style.css, script.js). The structure, copy tone and
dark/light look are good and should stay. The changes below fix problems found when the page was
opened with the real screenshots. Please apply all of them and return the complete updated site.

All images are in `assets/` (files listed at the end). Every screenshot is a PNG at 2x, English.

## A. Images

1. **Do not crop screenshots.** The hero frame has a fixed height and cuts the top of the image off,
   so the toolbar with the "Start selection" button and the running counter is missing. Give every
   screenshot frame the aspect ratio of its image (`aspect-ratio: 7 / 10` for the 840 x 1200
   account list and settings images, `21 / 26` for `login-dialog.png` 840 x 1040) and use
   `object-fit: contain`, or size the frame from the image itself. Nothing may be clipped.
2. **Use a different image for each highlight** (the account list appeared twice):
   - Hero: `account-list-dark.png` (light variant `account-list-light.png` for the toggle).
   - 01 multi-launch: two windows side by side or slightly overlapping, `account-list-dark.png` and
     `account-list-light.png`, so it does not repeat the hero image on its own.
   - 02 separate logins: `login-dialog.png` (the guided login dialog).
   - 03 instance switcher: `instance-switcher.png` (already composed on a backdrop, 1200 x 560,
     show it full width in its frame, no extra black frame).
   - 04 companion apps and updates: `update-banner.png` (account list with the "The game was
     updated" banner and the Blish HUD icons at the right of the rows).
3. **Settings:** the highlight that used the settings image must not claim "companion app
   configuration" (the image shows paths, frame rate limit, behavior). If settings are shown, use
   `settings-dark.png` or `settings-light.png` with the caption "Settings: paths, frame rate limit,
   behavior, appearance". Consider a small extra row "See it in the app" with `settings-light.png`
   and `account-editor-dark.png`.
4. **Made with Slint badge:** replace the text placeholder with the official badge:
   `assets/MadeWithSlint-logo-dark.svg` (white pill, for the dark theme) and
   `assets/MadeWithSlint-logo-light.svg` (dark pill, for the light theme), height about 40 px,
   linked to https://slint.dev, alt text "Made with Slint". Swap the file with the theme toggle.
5. **Logo:** use the real icon in the header and footer (`assets/icon-48.png`), not the CSS
   fallback. Remove the `onerror` fallbacks and the placeholder labels; all files exist now.
6. **Favicon and social preview:** add `<link rel="icon" href="assets/favicon.ico">`, the 16/32 px
   PNGs, `<link rel="apple-touch-icon" href="assets/icon-256.png">`, `<meta name="theme-color">`
   (`#0d0f12`), and Open Graph / Twitter meta tags: `og:title`, `og:description`, `og:type=website`,
   `og:url=https://launcher.breakbar.cc/`, `og:image`. Please design a 1200 x 630 `og-image.png`
   (dark, icon, "Breakbar Launcher", the tagline and the account-list screenshot cropped to the
   toolbar and the first rows) and tell us the file name.

## B. Copy that does not match the software

7. **FAQ "Is it allowed? Is it safe?":** rewrite. Breakbar closes the single-instance lock inside
   the running client and starts the others with the game's own `-shareArchive` option, so
   "does not modify the game" is too strong. Use (adapt the tone, keep the facts):
   "Breakbar starts the game's own program with the game's own options and keeps a separate login
   file for each account. To allow several clients it closes the game's single-instance lock in the
   running client. It does not change game files, inject code or read game memory, and the source is
   public. It is not an official ArenaNet product and not endorsed by ArenaNet; you use it at your
   own risk."
8. **Companion apps:** change "Blish HUD and others" to "Blish HUD" everywhere (the app ships a
   Blish HUD preset; an editor for other programs is not there yet).
9. **"Native Rust, event-driven":** change to "Native Rust, lightweight" and keep "near-zero CPU
   while idle" only if it fits the layout; the app checks its state a few times per second, so do
   not say "event-driven".
10. **Add to "Can I run it from any folder?":** "Put it in your own folder, for example in your user
    folder, not in `C:\Program Files`, so it can update itself later without administrator rights."
11. **Keep** the SmartScreen answer, but shorten: "The first start may show a Windows SmartScreen
    notice because the program is new. Releases will be code-signed."
12. **Download facts:** add rows `checksum` (link "SHA-256", href
    `https://github.com/breakbarcc/breakbar-launcher/releases/latest/download/breakbar-launcher.exe.sha256`)
    and `signed` (text "not yet" for now; leave it easy to change).

## C. Behaviour and technical

13. **Version number:** it is hard-coded in five places (`v0.8.0` in two buttons, the eyebrow, the
    facts list, "Everything in 0.8.0"). Replace all of them with one placeholder each, written as
    `{{VERSION}}` in the HTML text, so the deploy can fill them in. Do not fetch anything in the
    browser for it. "Everything in {{VERSION}}." may become "Everything in the current version."
14. **Theme:** on the first visit follow `prefers-color-scheme` (dark by default when nothing is
    set), keep the stored choice, and keep the toggle. Set `<meta name="color-scheme">` accordingly.
15. **FAQ:** do not open every `<details>`; open only the first, or none.
16. **Header:** add a link to the main site: "breakbar.cc" (https://www.breakbar.cc/), in the header
    navigation and keep it in the footer.
17. **Links:** add "Report a problem" (https://github.com/breakbarcc/breakbar-launcher/issues/new)
    to the footer and next to the download facts. Every external link gets
    `rel="noopener"`; no tracking, no cookies, no third-party scripts or fonts.
18. **Legal pages:** add footer links "Imprint" (`/imprint/`) and "Privacy" (`/privacy/`) and two
    simple pages in the same style with placeholder text `TODO`. The disclaimer text in the footer
    stays as it is.
19. **404 page:** add `404.html` in the same style with a link back to the start page.
20. **Accessibility and performance:** keep the current focus styles; add `width`/`height`
    attributes to all images (to avoid layout shift), `loading="lazy"` for everything below the
    hero, and `decoding="async"`. Contrast of muted text on both themes at least 4.5:1.

## D. Keep as it is

Section order (Hero, Features, All features, How it works, Steam, FAQ, Download, Footer), the
mono `// LABELS`, the dot grid, the accent color, the three-column feature list, the step cards,
the mobile menu, the download card. No web fonts, no libraries.

## Files in `assets/`

| File | Size | Use |
|---|---|---|
| `account-list-dark.png`, `account-list-light.png` | 840 x 1200 | Hero, multi-launch |
| `login-dialog.png` | 840 x 1040 | 02 separate logins |
| `instance-switcher.png` | 1200 x 560 | 03 instance switcher |
| `update-banner.png`, `update-banner-light.png` | 840 x 1200 | 04 companion apps and updates |
| `login-setup-running.png` | 840 x 1040 | optional: login setup banner while a client runs |
| `settings-dark.png`, `settings-light.png` | 840 x 1200 | settings (top part) |
| `account-editor-dark.png` | 840 x 1560 | optional: account editor |
| `icon-48.png`, `icon-256.png`, `icon-512.png` | square | logo, touch icon |
| `favicon.ico`, `favicon-16.png`, `favicon-32.png` | | favicon |
| `MadeWithSlint-logo-dark.svg`, `MadeWithSlint-logo-light.svg` | 212 x 97 | official badge |
