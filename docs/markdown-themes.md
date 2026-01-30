# Chibi Markdown Color Scheme Configuration

Chibi supports customizable color schemes for markdown rendering.

## Configuration

Add a `[markdown_style]` section to your global configuration
(~/.chibi/config.toml), for example:

```toml
[markdown_style]
bright = "#FF6B6B"   # Custom warm red for this context
head = "#4ECDC4"     # Custom teal
symbol = "#95E1D3"   # Custom mint
grey = "#888888"
dark = "#2C3E50"
mid = "#34495E"
light = "#7F8C8D"
```


You can override the color scheme for individual contexts by adding
the section to the context configuration file (~/.chibi/contexts/<name>/local.toml).

## Color Fields

| Field    | Purpose                                      | Default        |
|----------|----------------------------------------------|----------------|
| `bright` | H2 headers, emphasis                         | #FFFF54        |
| `head`   | H3 headers                                   | #54FF54        |
| `symbol` | Bullets, language labels, markers            | #7ABFC7        |
| `grey`   | Borders, muted text, horizontal rules        | #808080        |
| `dark`   | Code block backgrounds                       | #000000        |
| `mid`    | Table headers                                | #3E31A2        |
| `light`  | Alternate backgrounds                        | #352879        |

## Example Themes

### Classic Terminal

```toml
[markdown_style]
bright = "#00FF00"   # Bright green
head = "#00DD00"     # Medium green
symbol = "#00AA00"   # Dark green
grey = "#555555"     # Dark grey
dark = "#000000"     # Black
mid = "#003300"      # Very dark green
light = "#001100"    # Almost black green
```

### Warm Sunset

```toml
[markdown_style]
bright = "#FF6B6B"   # Coral red
head = "#4ECDC4"     # Turquoise
symbol = "#95E1D3"   # Mint
grey = "#888888"     # Medium grey
dark = "#2C3E50"     # Dark blue-grey
mid = "#34495E"      # Slate
light = "#7F8C8D"    # Light grey
```

### Monokai-inspired

```toml
[markdown_style]
bright = "#F92672"   # Pink
head = "#A6E22E"     # Green
symbol = "#66D9EF"   # Cyan
grey = "#75715E"     # Grey
dark = "#272822"     # Dark grey
mid = "#49483E"      # Mid grey
light = "#3E3D32"    # Light grey
```

## Implementation Details

- All colors must be specified as hex values (e.g., `#RRGGBB`)
- Changes take effect immediately on next chibi run
- Invalid hex colors may produce unexpected rendering results
- The streamdown-rs renderer handles all color application
- Colors are applied to ANSI terminal output via escape codes
