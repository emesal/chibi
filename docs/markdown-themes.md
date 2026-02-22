# Markdown Themes

Chibi supports customizable color schemes for markdown rendering via the
`[markdown_style]` section in your config.

## Configuration

Add a `[markdown_style]` section to `~/.chibi/cli.toml`:

```toml
[markdown_style]
h1 = "white"
h2 = "yellow"
h3 = "light_green"
```

Per-context overrides go in `~/.chibi/contexts/<name>/cli.toml` under the
same `[markdown_style]` section. Only the fields you set are overridden;
everything else inherits the global default.

## Color Values

Colors can be specified as:

- **Named Colodore presets** (Commodore 64/128 palette by Pepto):
  `"white"`, `"yellow"`, `"cyan"`, `"light_green"`, `"light_grey"`,
  `"grey"`, `"dark_grey"`, `"blue"`, `"black"`, etc.
- **Hex values**: `"#RRGGBB"` (e.g., `"#FF6B6B"`)

## Color Fields

| Field              | Purpose                          | Default        |
|--------------------|----------------------------------|----------------|
| `h1`               | H1 headings                      | `white`        |
| `h2`               | H2 headings                      | `yellow`       |
| `h3`               | H3 headings                      | `light_green`  |
| `h4`               | H4 headings                      | `cyan`         |
| `h5`               | H5 headings                      | `light_grey`   |
| `h6`               | H6 headings                      | `grey`         |
| `code_bg`          | Code block background            | `black`        |
| `code_label`       | Language label in code blocks    | `cyan`         |
| `bullet`           | List bullet points               | `cyan`         |
| `table_header_bg`  | Table header background          | `blue`         |
| `table_border`     | Table borders                    | `grey`         |
| `blockquote_border`| Blockquote border                | `grey`         |
| `think_border`     | Thinking/reasoning block border  | `grey`         |
| `hr`               | Horizontal rules                 | `dark_grey`    |
| `link_url`         | Link URLs                        | `grey`         |
| `image_marker`     | Image placeholder markers        | `cyan`         |
| `footnote`         | Footnote references              | `cyan`         |

## Example Themes

### Classic Terminal

```toml
[markdown_style]
h1 = "#00FF00"
h2 = "#00DD00"
h3 = "#00BB00"
h4 = "#00AA00"
h5 = "#009900"
h6 = "#007700"
code_bg = "#000000"
code_label = "#00FF00"
bullet = "#00CC00"
table_header_bg = "#003300"
table_border = "#005500"
blockquote_border = "#444444"
think_border = "#444444"
hr = "#333333"
link_url = "#555555"
image_marker = "#00AA00"
footnote = "#00AA00"
```

### Warm Sunset

```toml
[markdown_style]
h1 = "#FF6B6B"
h2 = "#FF8E53"
h3 = "#4ECDC4"
h4 = "#95E1D3"
h5 = "#AAAAAA"
h6 = "#888888"
code_bg = "#2C3E50"
code_label = "#4ECDC4"
bullet = "#FF8E53"
table_header_bg = "#34495E"
table_border = "#7F8C8D"
blockquote_border = "#7F8C8D"
think_border = "#7F8C8D"
hr = "#555555"
link_url = "#888888"
image_marker = "#4ECDC4"
footnote = "#4ECDC4"
```

### Monokai-inspired

```toml
[markdown_style]
h1 = "#F92672"
h2 = "#FD971F"
h3 = "#A6E22E"
h4 = "#66D9EF"
h5 = "#AE81FF"
h6 = "#75715E"
code_bg = "#272822"
code_label = "#66D9EF"
bullet = "#F92672"
table_header_bg = "#49483E"
table_border = "#75715E"
blockquote_border = "#75715E"
think_border = "#75715E"
hr = "#3E3D32"
link_url = "#75715E"
image_marker = "#66D9EF"
footnote = "#AE81FF"
```
