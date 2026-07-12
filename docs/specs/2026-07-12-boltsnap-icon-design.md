# Boltsnap Icon Design

## Decision

Use the first accepted generated mark unchanged: the narrow white lowercase `b` shape with the small upper-right snap dot. Remove only the dark preview background.

## Tray behavior

- Ship the icon inside the Boltsnap binary instead of relying on an installed icon theme.
- Publish transparent white ARGB pixmaps through `ksni::Tray::icon_pixmap`.
- Publish 32 px and 64 px sizes so normal and HiDPI tray hosts can choose without enlarging a tiny bitmap.
- Return no theme icon name, preventing the current checkerboard fallback.

## Non-goals

- No further logo reshaping, extra hole, notch, color system, desktop-file packaging, or system icon-theme installation.
- No new dependency; the existing `image` crate decodes the embedded PNGs once.

## Acceptance

- The tray receives valid 32×32 and 64×64 ARGB pixmaps.
- Both pixmaps have transparent padding and visible white mark pixels.
- The icon source visually matches the accepted raster and remains legible around 18 px.
