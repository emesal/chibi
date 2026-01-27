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

### Display options

```toml
# Maximum image height in terminal lines (default: 25)
image_max_height_lines = 25

# Percentage of terminal width to use for images (default: 80)
image_max_width_percent = 80

# Image alignment: "left", "center", or "right" (default: "center")
image_alignment = "center"
```

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

Image rendering uses 24-bit (truecolor) ANSI escape codes. Most modern
terminals support this, including:

- kitty, iTerm2, WezTerm, Alacritty, Windows Terminal
- xterm (with `--enable-truecolor`)

Terminals that don't support truecolor will display garbled output for
images. In that case, disable image rendering with
`render_images = false`.
