# Image Rendering

Chibi can render images inline in the terminal when the markdown
renderer is active. Images referenced in LLM responses (e.g.,
`![alt text](path/to/image.png)`) are displayed using truecolor ANSI
escape codes, which work in most modern terminals.

## Supported image sources

- **Local file paths** (relative or absolute): `./diagram.png`,
  `/tmp/chart.jpg`
- **`file://` URLs**: `file:///home/user/image.png`
- **Data URIs**: `data:image/png;base64,...`
- **HTTPS URLs**: `https://example.com/photo.png`
- **HTTP URLs** (disabled by default): `http://example.com/photo.png`
  — requires `image_allow_http = true`

## Remote image fetching

Remote images (`https://` and optionally `http://`) are fetched at
render time with the following safeguards:

- **HTTPS only by default.** Plain `http://` URLs are rejected unless
  `image_allow_http = true`.
- **Size limit.** Downloads are capped at `image_max_download_bytes`
  (default: 10 MB). Both `Content-Length` headers and streamed body
  size are checked.
- **Timeout.** Fetches time out after `image_fetch_timeout_seconds`
  (default: 5 seconds).
- **Content-Type check.** If the server provides a `Content-Type`
  header, it must start with `image/`.
- **Redirect safety.** Up to 5 redirects are followed, but
  HTTPS-to-HTTP downgrades are blocked (unless `image_allow_http` is
  enabled).
- **Fallback.** If a remote fetch fails for any reason, the image
  falls back to the standard placeholder display — no error is shown
  to the user.

## Configuration

Image rendering is enabled by default. To disable it, set
`render_images = false` in your config:

**Global** (`~/.chibi/config.toml`):

```toml
render_images = false
```

**Per-context** (`~/.chibi/contexts/<name>/local.toml`):

```toml
render_images = false
```

### Remote fetch settings

```toml
# Maximum bytes to download per image (default: 10485760 = 10 MB)
image_max_download_bytes = 10485760

# Timeout in seconds for each image fetch (default: 5)
image_fetch_timeout_seconds = 5

# Allow fetching images over plain HTTP (default: false)
image_allow_http = false
```

### Image cache

Remote images are cached locally to avoid re-fetching on log review,
compaction replay, or repeated renders. The cache lives at
`~/.chibi/image_cache/` and is content-addressed by SHA-256 of the URL.

```toml
# Enable/disable image caching (default: true)
image_cache_enabled = true

# Maximum total cache size in bytes (default: 104857600 = 100 MB)
image_cache_max_bytes = 104857600

# Maximum age of cached images in days (default: 30)
image_cache_max_age_days = 30
```

Eviction runs automatically at exit:
1. Entries older than `image_cache_max_age_days` are removed first.
2. If total size still exceeds `image_cache_max_bytes`, the
   least-recently-accessed entries are removed until under the limit.

Cache writes are best-effort — failures never block rendering.

### Display options

```toml
# Maximum image height in terminal lines (default: 25)
image_max_height_lines = 25

# Percentage of terminal width to use for images (default: 80)
image_max_width_percent = 80

# Image alignment: "left", "center", or "right" (default: "center")
image_alignment = "center"
```

### Rendering modes

```toml
# Image rendering mode (default: "auto")
# Options: "auto", "truecolor", "ansi", "ascii", "placeholder"
image_render_mode = "auto"

# Enable individual rendering modes (default: all enabled)
# These control which modes are available to "auto" detection
# and whether explicit mode selection works
image_enable_truecolor = true   # 24-bit color (best quality)
image_enable_ansi = true         # 16-color ANSI (compatible)
image_enable_ascii = true        # ASCII art (universal)
```

**Mode descriptions:**

- **`auto`** (default) - Automatically detect terminal capabilities:
  - Checks `COLORTERM` environment variable for `truecolor` or `24bit`
  - Checks `TERM` environment variable for color support level
  - Falls back through enabled modes: truecolor → ansi → ascii → placeholder

- **`truecolor`** - Force 24-bit (16.7M colors) ANSI rendering. Best quality
  but requires modern terminal support. Falls back to placeholder if
  `image_enable_truecolor = false`.

- **`ansi`** - Force 16-color ANSI rendering. Compatible with most terminals
  since the 1990s. Lower quality than truecolor but universally supported.

- **`ascii`** - Force ASCII art rendering. Works in any terminal, including
  those without color support. Uses 8 characters to represent intensity:
  ` .,-~+=@`

- **`placeholder`** - Disable inline rendering entirely. Images show as
  `[🖼 alt text]` placeholders.

**Disabling rendering modes:**

Set any `image_enable_*` option to `false` to disable that mode. This affects
both auto-detection and explicit mode selection:

```toml
# Disable truecolor (useful for old terminals)
image_enable_truecolor = false

# Now auto mode will skip to ansi
image_render_mode = "auto"

# Or force a specific mode (ansi/ascii only)
image_render_mode = "ansi"
```

If you explicitly select a disabled mode (e.g., `image_render_mode =
"truecolor"` with `image_enable_truecolor = false`), chibi falls back to
auto-detection logic.

All settings can be overridden per-context in `local.toml`.

## Interaction with other settings

- `render_markdown = false` (or the `--raw` CLI flag) disables the
  entire markdown rendering pipeline, which includes image rendering.
- `render_images` only takes effect when the pipeline is active (i.e.,
  stdout is a TTY and markdown rendering is enabled).

## Fallback behavior

When image rendering is disabled or an image cannot be loaded (e.g.,
missing file, unsupported format, network error), the image falls back
to the standard placeholder: `[🖼 alt text]`.

Alt text is only shown in this fallback placeholder. When the image is
successfully rendered, alt text is omitted to avoid visual noise (it
serves as a replacement for the image, not a caption).

## Terminal compatibility

Image rendering supports three modes with different compatibility levels:

### Truecolor mode (default on modern terminals)

Uses 24-bit (16.7M colors) ANSI escape codes. Supported by:

- kitty, iTerm2, WezTerm, Alacritty, Windows Terminal
- xterm (with `--enable-truecolor`)
- Most terminals from ~2016 onwards

Enable explicitly with `image_render_mode = "truecolor"` or rely on
auto-detection via `COLORTERM` and `TERM` environment variables.

### ANSI mode (fallback for older terminals)

Uses 16-color ANSI codes (8 colors + bright variants). Supported by virtually
all color terminals since the 1990s:

- All modern terminals (truecolor-capable terminals also support ANSI)
- xterm, urxvt, gnome-terminal, konsole (without truecolor)
- PuTTY, macOS Terminal.app
- Linux virtual console (with framebuffer)

Enable with `image_render_mode = "ansi"`.

### ASCII mode (universal fallback)

Uses ASCII characters to represent image intensity. Works in any terminal,
including:

- Non-color terminals
- Serial consoles
- Text-mode virtual terminals
- Screen readers (though usefulness varies)

Enable with `image_render_mode = "ascii"`.

### Auto-detection

By default (`image_render_mode = "auto"`), chibi detects terminal
capabilities:

1. Checks `$COLORTERM` for `truecolor` or `24bit` → truecolor mode
2. Checks `$TERM` for `truecolor`, `24bit` → truecolor mode
3. Checks `$TERM` for `256color` or `color` → ansi mode
4. Falls back to ansi mode (safe default for unknown terminals)

Then applies the fallback chain based on enabled modes: truecolor → ansi →
ascii → placeholder.

To force a specific mode regardless of detection, set `image_render_mode`
explicitly.
