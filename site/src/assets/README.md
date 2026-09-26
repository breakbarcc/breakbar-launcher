# Website assets

`shots/<language>/` holds the screenshots of that language's pages, the files next to it are shared
(icons, favicons, the "Made with Slint" badge and the social preview `og-image.png`).

The screenshots are rendered from the program itself (offscreen preview of the Slint UI, at twice the
size) and reduced to 256 colors so that each image stays around 50 KB.

Regenerate them after a UI change with
`BREAKBAR_PREVIEW_LANG=en BREAKBAR_PREVIEW_SCALE=2 BREAKBAR_PREVIEW_NO_TOAST=1 cargo test -p breakbar-launcher -- --ignored render_ui_previews`
(see docs/ARCHITECTURE.md, "Screenshots for the README"; use `BREAKBAR_PREVIEW_LANG=de` for `shots/de`).
The switcher image is the overlay render at `BREAKBAR_PREVIEW_SCALE=4` on a plain backdrop.

The two `MadeWithSlint-logo-*.svg` files are Slint's official badge (dark = white pill for dark
pages, light = dark pill for light pages).
