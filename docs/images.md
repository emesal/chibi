# Image Rendering

Chibi can render local images inline in the terminal when the markdown
renderer is active. Images referenced in LLM responses (e.g.,
`![alt text](path/to/image.png)`) are displayed using truecolor ANSI
escape codes, which work in most modern terminals.

## Supported image sources

- **Local file paths** (relative or absolute): `./diagram.png`,
  `/tmp/chart.jpg`
- **`file://` URLs**: `file:///home/user/image.png`
- **Data URIs**: `data:image/png;base64,...`

Remote URLs (`http://`, `https://`) are not yet supported and will fall
back to the placeholder display.

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

## Interaction with other settings

- `render_markdown = false` (or the `--raw` CLI flag) disables the
  entire markdown rendering pipeline, which includes image rendering.
- `render_images` only takes effect when the pipeline is active (i.e.,
  stdout is a TTY and markdown rendering is enabled).

## Fallback behavior

When image rendering is disabled or an image cannot be loaded (e.g.,
missing file, unsupported format), the image falls back to the standard
placeholder: `[🖼 alt text]`.

## Terminal compatibility

Image rendering uses 24-bit (truecolor) ANSI escape codes. Most modern
terminals support this, including:

- kitty, iTerm2, WezTerm, Alacritty, Windows Terminal
- xterm (with `--enable-truecolor`)

Terminals that don't support truecolor will display garbled output for
images. In that case, disable image rendering with
`render_images = false`.
