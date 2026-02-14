//! CLI-specific configuration types for presentation.
//!
//! This module contains presentation-related configuration that doesn't
//! belong in chibi-core (image rendering, markdown styling, etc.).

use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

// Re-export core config types for convenience
pub use chibi_core::config::ResolvedConfig as CoreResolvedConfig;

// ============================================================================
// Presentation Default Functions
// ============================================================================

fn default_render_images() -> bool {
    true
}

fn default_image_max_download_bytes() -> usize {
    10 * 1024 * 1024
}

fn default_image_fetch_timeout_seconds() -> u64 {
    5
}

fn default_image_max_height_lines() -> u32 {
    25
}

fn default_image_max_width_percent() -> u32 {
    80
}

fn default_image_cache_max_bytes() -> u64 {
    104_857_600 // 100 MB
}

fn default_image_cache_max_age_days() -> u64 {
    30
}

fn default_true_val() -> bool {
    true
}

// ============================================================================
// Presentation Configuration Types
// ============================================================================

/// Image alignment in terminal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ImageAlignment {
    Left,
    #[default]
    Center,
    Right,
}

impl ImageAlignment {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImageAlignment::Left => "left",
            ImageAlignment::Center => "center",
            ImageAlignment::Right => "right",
        }
    }
}

impl std::fmt::Display for ImageAlignment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Image rendering mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConfigImageRenderMode {
    #[default]
    Auto,
    Truecolor,
    Ansi,
    Ascii,
    Placeholder,
}

impl ConfigImageRenderMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfigImageRenderMode::Auto => "auto",
            ConfigImageRenderMode::Truecolor => "truecolor",
            ConfigImageRenderMode::Ansi => "ansi",
            ConfigImageRenderMode::Ascii => "ascii",
            ConfigImageRenderMode::Placeholder => "placeholder",
        }
    }
}

impl std::fmt::Display for ConfigImageRenderMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Grouped image rendering/fetching/caching configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageConfig {
    /// Render images inline in the terminal
    #[serde(default = "default_render_images")]
    pub render_images: bool,
    /// Maximum bytes to download for remote images
    #[serde(default = "default_image_max_download_bytes")]
    pub max_download_bytes: usize,
    /// Timeout in seconds for fetching remote images
    #[serde(default = "default_image_fetch_timeout_seconds")]
    pub fetch_timeout_seconds: u64,
    /// Allow fetching images over plain HTTP (default: false, HTTPS only)
    #[serde(default)]
    pub allow_http: bool,
    /// Maximum image height in terminal lines
    #[serde(default = "default_image_max_height_lines")]
    pub max_height_lines: u32,
    /// Percentage of terminal width to use for images
    #[serde(default = "default_image_max_width_percent")]
    pub max_width_percent: u32,
    /// Image alignment
    #[serde(default)]
    pub alignment: ImageAlignment,
    /// Image rendering mode
    #[serde(default)]
    pub render_mode: ConfigImageRenderMode,
    /// Enable truecolor (24-bit) rendering
    #[serde(default = "default_true_val")]
    pub enable_truecolor: bool,
    /// Enable ANSI (16-color) rendering
    #[serde(default = "default_true_val")]
    pub enable_ansi: bool,
    /// Enable ASCII art rendering
    #[serde(default = "default_true_val")]
    pub enable_ascii: bool,
    /// Enable image cache for remote images
    #[serde(default = "default_true_val")]
    pub cache_enabled: bool,
    /// Maximum total size of image cache in bytes
    #[serde(default = "default_image_cache_max_bytes")]
    pub cache_max_bytes: u64,
    /// Maximum age of cached images in days
    #[serde(default = "default_image_cache_max_age_days")]
    pub cache_max_age_days: u64,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            render_images: true,
            max_download_bytes: 10 * 1024 * 1024,
            fetch_timeout_seconds: 5,
            allow_http: false,
            max_height_lines: 25,
            max_width_percent: 80,
            alignment: ImageAlignment::default(),
            render_mode: ConfigImageRenderMode::default(),
            enable_truecolor: true,
            enable_ansi: true,
            enable_ascii: true,
            cache_enabled: true,
            cache_max_bytes: 104_857_600,
            cache_max_age_days: 30,
        }
    }
}

impl ImageConfig {
    /// Merge with an optional override config (from LocalConfig).
    pub fn merge_with(&self, other: &ImageConfigOverride) -> Self {
        Self {
            render_images: other.render_images.unwrap_or(self.render_images),
            max_download_bytes: other.max_download_bytes.unwrap_or(self.max_download_bytes),
            fetch_timeout_seconds: other
                .fetch_timeout_seconds
                .unwrap_or(self.fetch_timeout_seconds),
            allow_http: other.allow_http.unwrap_or(self.allow_http),
            max_height_lines: other.max_height_lines.unwrap_or(self.max_height_lines),
            max_width_percent: other.max_width_percent.unwrap_or(self.max_width_percent),
            alignment: other.alignment.unwrap_or(self.alignment),
            render_mode: other.render_mode.unwrap_or(self.render_mode),
            enable_truecolor: other.enable_truecolor.unwrap_or(self.enable_truecolor),
            enable_ansi: other.enable_ansi.unwrap_or(self.enable_ansi),
            enable_ascii: other.enable_ascii.unwrap_or(self.enable_ascii),
            cache_enabled: other.cache_enabled.unwrap_or(self.cache_enabled),
            cache_max_bytes: other.cache_max_bytes.unwrap_or(self.cache_max_bytes),
            cache_max_age_days: other.cache_max_age_days.unwrap_or(self.cache_max_age_days),
        }
    }
}

/// Per-context image config overrides (all fields optional).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageConfigOverride {
    pub render_images: Option<bool>,
    pub max_download_bytes: Option<usize>,
    pub fetch_timeout_seconds: Option<u64>,
    pub allow_http: Option<bool>,
    pub max_height_lines: Option<u32>,
    pub max_width_percent: Option<u32>,
    pub alignment: Option<ImageAlignment>,
    pub render_mode: Option<ConfigImageRenderMode>,
    pub enable_truecolor: Option<bool>,
    pub enable_ansi: Option<bool>,
    pub enable_ascii: Option<bool>,
    pub cache_enabled: Option<bool>,
    pub cache_max_bytes: Option<u64>,
    pub cache_max_age_days: Option<u64>,
}

/// Markdown rendering color scheme (re-exported from streamdown-render).
pub type MarkdownStyle = streamdown_render::RenderStyle;

/// Colodore-inspired color scheme (Commodore 64/128 palette by Pepto).
///
/// Colors can be specified as hex values (e.g., "#edf171") or as
/// Colodore preset names (e.g., "yellow", "cyan", "light_green").
pub fn default_markdown_style() -> MarkdownStyle {
    MarkdownStyle {
        // Headings
        h1: "white".to_string(),
        h2: "yellow".to_string(),
        h3: "light_green".to_string(),
        h4: "cyan".to_string(),
        h5: "light_grey".to_string(),
        h6: "grey".to_string(),

        // Code
        code_bg: "black".to_string(),
        code_label: "cyan".to_string(),

        // Lists
        bullet: "cyan".to_string(),

        // Tables
        table_header_bg: "blue".to_string(),
        table_border: "grey".to_string(),

        // Borders/decorations
        blockquote_border: "grey".to_string(),
        think_border: "grey".to_string(),
        hr: "dark_grey".to_string(),

        // Links & references
        link_url: "grey".to_string(),
        image_marker: "cyan".to_string(),
        footnote: "cyan".to_string(),
    }
}

/// CLI-extended resolved configuration with presentation fields.
/// Wraps the core ResolvedConfig and adds presentation-specific settings.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Core configuration (API, storage, etc.)
    pub core: CoreResolvedConfig,
    /// Render LLM output as formatted markdown in the terminal
    pub render_markdown: bool,
    /// Show thinking/reasoning content (default: false, verbose overrides)
    pub show_thinking: bool,
    /// Image rendering, fetching, and caching configuration
    pub image: ImageConfig,
    /// Markdown rendering color scheme
    pub markdown_style: MarkdownStyle,
}

impl ResolvedConfig {
    /// Get a config field value by path.
    /// First checks presentation fields, then delegates to core.
    pub fn get_field(&self, path: &str) -> Option<String> {
        match path {
            "render_markdown" => Some(self.render_markdown.to_string()),
            "show_thinking" => Some(self.show_thinking.to_string()),
            "image.render_images" => Some(self.image.render_images.to_string()),
            "image.max_download_bytes" => Some(self.image.max_download_bytes.to_string()),
            "image.fetch_timeout_seconds" => Some(self.image.fetch_timeout_seconds.to_string()),
            "image.allow_http" => Some(self.image.allow_http.to_string()),
            "image.max_height_lines" => Some(self.image.max_height_lines.to_string()),
            "image.max_width_percent" => Some(self.image.max_width_percent.to_string()),
            "image.alignment" => Some(self.image.alignment.to_string()),
            "image.render_mode" => Some(self.image.render_mode.to_string()),
            "image.enable_truecolor" => Some(self.image.enable_truecolor.to_string()),
            "image.enable_ansi" => Some(self.image.enable_ansi.to_string()),
            "image.enable_ascii" => Some(self.image.enable_ascii.to_string()),
            "image.cache_enabled" => Some(self.image.cache_enabled.to_string()),
            "image.cache_max_bytes" => Some(self.image.cache_max_bytes.to_string()),
            "image.cache_max_age_days" => Some(self.image.cache_max_age_days.to_string()),
            _ => self.core.get_field(path),
        }
    }

    /// List all inspectable config field paths.
    pub fn list_fields() -> Vec<&'static str> {
        let mut fields = vec![
            "render_markdown",
            "show_thinking",
            "image.render_images",
            "image.max_download_bytes",
            "image.fetch_timeout_seconds",
            "image.allow_http",
            "image.max_height_lines",
            "image.max_width_percent",
            "image.alignment",
            "image.render_mode",
            "image.enable_truecolor",
            "image.enable_ansi",
            "image.enable_ascii",
            "image.cache_enabled",
            "image.cache_max_bytes",
            "image.cache_max_age_days",
        ];
        fields.extend(CoreResolvedConfig::list_fields());
        fields
    }
}

// ============================================================================
// CLI Config File Types
// ============================================================================

/// Per-context markdown style overrides (all fields optional).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MarkdownStyleOverride {
    // Headings
    pub h1: Option<String>,
    pub h2: Option<String>,
    pub h3: Option<String>,
    pub h4: Option<String>,
    pub h5: Option<String>,
    pub h6: Option<String>,

    // Code
    pub code_bg: Option<String>,
    pub code_label: Option<String>,

    // Lists
    pub bullet: Option<String>,

    // Tables
    pub table_header_bg: Option<String>,
    pub table_border: Option<String>,

    // Borders/decorations
    pub blockquote_border: Option<String>,
    pub think_border: Option<String>,
    pub hr: Option<String>,

    // Links & references
    pub link_url: Option<String>,
    pub image_marker: Option<String>,
    pub footnote: Option<String>,
}

/// Raw CLI config as parsed from ~/.chibi/cli.toml
/// Uses MarkdownStyleOverride since MarkdownStyle requires all fields.
#[derive(Debug, Clone, Default, Deserialize)]
struct RawCliConfig {
    #[serde(default = "default_true_val")]
    pub render_markdown: bool,
    /// Show thinking/reasoning content (default: false, verbose overrides)
    #[serde(default)]
    pub show_thinking: bool,
    #[serde(default)]
    pub image: ImageConfig,
    #[serde(default)]
    pub markdown_style: MarkdownStyleOverride,
}

/// Resolved CLI config with all fields populated.
#[derive(Debug, Clone)]
pub struct CliConfig {
    pub render_markdown: bool,
    /// Show thinking/reasoning content (default: false, verbose overrides)
    pub show_thinking: bool,
    pub image: ImageConfig,
    pub markdown_style: MarkdownStyle,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            render_markdown: true,
            show_thinking: false,
            image: ImageConfig::default(),
            markdown_style: default_markdown_style(),
        }
    }
}

impl CliConfig {
    /// Merge with per-context overrides.
    pub fn merge_with(&self, overrides: &CliConfigOverride) -> Self {
        Self {
            render_markdown: overrides.render_markdown.unwrap_or(self.render_markdown),
            show_thinking: overrides.show_thinking.unwrap_or(self.show_thinking),
            image: self.image.merge_with(&overrides.image),
            markdown_style: merge_markdown_style(&self.markdown_style, &overrides.markdown_style),
        }
    }
}

/// Per-context CLI overrides from contexts/<name>/cli.toml
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CliConfigOverride {
    pub render_markdown: Option<bool>,
    pub show_thinking: Option<bool>,
    #[serde(default)]
    pub image: ImageConfigOverride,
    #[serde(default)]
    pub markdown_style: MarkdownStyleOverride,
}

/// Merge a MarkdownStyle with optional overrides.
fn merge_markdown_style(base: &MarkdownStyle, overrides: &MarkdownStyleOverride) -> MarkdownStyle {
    MarkdownStyle {
        // Headings
        h1: overrides.h1.clone().unwrap_or_else(|| base.h1.clone()),
        h2: overrides.h2.clone().unwrap_or_else(|| base.h2.clone()),
        h3: overrides.h3.clone().unwrap_or_else(|| base.h3.clone()),
        h4: overrides.h4.clone().unwrap_or_else(|| base.h4.clone()),
        h5: overrides.h5.clone().unwrap_or_else(|| base.h5.clone()),
        h6: overrides.h6.clone().unwrap_or_else(|| base.h6.clone()),

        // Code
        code_bg: overrides
            .code_bg
            .clone()
            .unwrap_or_else(|| base.code_bg.clone()),
        code_label: overrides
            .code_label
            .clone()
            .unwrap_or_else(|| base.code_label.clone()),

        // Lists
        bullet: overrides
            .bullet
            .clone()
            .unwrap_or_else(|| base.bullet.clone()),

        // Tables
        table_header_bg: overrides
            .table_header_bg
            .clone()
            .unwrap_or_else(|| base.table_header_bg.clone()),
        table_border: overrides
            .table_border
            .clone()
            .unwrap_or_else(|| base.table_border.clone()),

        // Borders/decorations
        blockquote_border: overrides
            .blockquote_border
            .clone()
            .unwrap_or_else(|| base.blockquote_border.clone()),
        think_border: overrides
            .think_border
            .clone()
            .unwrap_or_else(|| base.think_border.clone()),
        hr: overrides.hr.clone().unwrap_or_else(|| base.hr.clone()),

        // Links & references
        link_url: overrides
            .link_url
            .clone()
            .unwrap_or_else(|| base.link_url.clone()),
        image_marker: overrides
            .image_marker
            .clone()
            .unwrap_or_else(|| base.image_marker.clone()),
        footnote: overrides
            .footnote
            .clone()
            .unwrap_or_else(|| base.footnote.clone()),
    }
}

/// Load CLI config with per-context overrides merged in.
///
/// Loads from:
/// 1. `{home}/cli.toml` (global, uses defaults if missing)
/// 2. `{home}/contexts/{context}/cli.toml` (per-context, merged if exists)
pub fn load_cli_config(home: &Path, context_name: Option<&str>) -> io::Result<CliConfig> {
    // Load global cli.toml (use defaults if missing)
    let global_path = home.join("cli.toml");
    let global_config: CliConfig = if global_path.exists() {
        let content = std::fs::read_to_string(&global_path)?;
        let raw: RawCliConfig = toml::from_str(&content).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to parse {}: {}", global_path.display(), e),
            )
        })?;
        // Convert raw config to resolved config by merging with defaults
        CliConfig {
            render_markdown: raw.render_markdown,
            show_thinking: raw.show_thinking,
            image: raw.image,
            markdown_style: merge_markdown_style(&default_markdown_style(), &raw.markdown_style),
        }
    } else {
        CliConfig::default()
    };

    // If no context, return global config
    let Some(context) = context_name else {
        return Ok(global_config);
    };

    // Load per-context cli.toml (merge if exists)
    let context_path = home.join("contexts").join(context).join("cli.toml");
    if context_path.exists() {
        let content = std::fs::read_to_string(&context_path)?;
        let overrides: CliConfigOverride = toml::from_str(&content).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to parse {}: {}", context_path.display(), e),
            )
        })?;
        Ok(global_config.merge_with(&overrides))
    } else {
        Ok(global_config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_cli_config_defaults_when_no_file() {
        let temp = TempDir::new().unwrap();
        let config = load_cli_config(temp.path(), None).unwrap();

        assert!(config.render_markdown);
        assert!(config.image.render_images);
        assert_eq!(config.image.max_height_lines, 25);
        assert_eq!(config.markdown_style.h2, "yellow");
    }

    #[test]
    fn test_load_cli_config_from_global_file() {
        let temp = TempDir::new().unwrap();
        let cli_toml = r##"
render_markdown = false

[image]
max_height_lines = 50
alignment = "left"

[markdown_style]
h2 = "#FF0000"
"##;
        std::fs::write(temp.path().join("cli.toml"), cli_toml).unwrap();

        let config = load_cli_config(temp.path(), None).unwrap();

        assert!(!config.render_markdown);
        assert_eq!(config.image.max_height_lines, 50);
        assert_eq!(config.image.alignment, ImageAlignment::Left);
        assert_eq!(config.markdown_style.h2, "#FF0000");
        // Defaults preserved for unspecified fields
        assert!(config.image.render_images);
        assert_eq!(config.markdown_style.h3, "light_green");
    }

    #[test]
    fn test_load_cli_config_per_context_overrides() {
        let temp = TempDir::new().unwrap();

        // Global config
        let global_toml = r#"
render_markdown = true

[image]
max_height_lines = 30
"#;
        std::fs::write(temp.path().join("cli.toml"), global_toml).unwrap();

        // Per-context config
        let context_dir = temp.path().join("contexts").join("coding");
        std::fs::create_dir_all(&context_dir).unwrap();
        let context_toml = r#"
render_markdown = false

[image]
max_height_lines = 60
"#;
        std::fs::write(context_dir.join("cli.toml"), context_toml).unwrap();

        let config = load_cli_config(temp.path(), Some("coding")).unwrap();

        // Per-context overrides applied
        assert!(!config.render_markdown);
        assert_eq!(config.image.max_height_lines, 60);
    }

    #[test]
    fn test_load_cli_config_partial_context_override() {
        let temp = TempDir::new().unwrap();

        // Global config
        let global_toml = r#"
render_markdown = true

[image]
max_height_lines = 30
alignment = "right"
"#;
        std::fs::write(temp.path().join("cli.toml"), global_toml).unwrap();

        // Per-context config (only overrides one field)
        let context_dir = temp.path().join("contexts").join("minimal");
        std::fs::create_dir_all(&context_dir).unwrap();
        let context_toml = r#"
[image]
max_height_lines = 10
"#;
        std::fs::write(context_dir.join("cli.toml"), context_toml).unwrap();

        let config = load_cli_config(temp.path(), Some("minimal")).unwrap();

        // Only max_height_lines overridden, others from global
        assert!(config.render_markdown);
        assert_eq!(config.image.max_height_lines, 10);
        assert_eq!(config.image.alignment, ImageAlignment::Right);
    }

    #[test]
    fn test_cli_config_merge_with() {
        let base = CliConfig {
            render_markdown: true,
            show_thinking: false,
            image: ImageConfig {
                max_height_lines: 25,
                ..Default::default()
            },
            markdown_style: default_markdown_style(),
        };

        let overrides = CliConfigOverride {
            render_markdown: Some(false),
            show_thinking: None,
            image: ImageConfigOverride {
                max_height_lines: Some(50),
                ..Default::default()
            },
            markdown_style: MarkdownStyleOverride {
                h2: Some("#00FF00".to_string()),
                ..Default::default()
            },
        };

        let merged = base.merge_with(&overrides);

        assert!(!merged.render_markdown);
        assert_eq!(merged.image.max_height_lines, 50);
        assert_eq!(merged.markdown_style.h2, "#00FF00");
        // Unoverridden fields preserved
        assert!(merged.image.render_images);
        assert_eq!(merged.markdown_style.h3, "light_green");
    }

    #[test]
    fn test_markdown_style_override_partial() {
        let base = default_markdown_style();
        let overrides = MarkdownStyleOverride {
            h2: Some("#AABBCC".to_string()),
            code_bg: Some("#112233".to_string()),
            ..Default::default()
        };

        let merged = merge_markdown_style(&base, &overrides);

        assert_eq!(merged.h2, "#AABBCC");
        assert_eq!(merged.code_bg, "#112233");
        // Rest unchanged
        assert_eq!(merged.h3, base.h3);
        assert_eq!(merged.bullet, base.bullet);
        assert_eq!(merged.table_border, base.table_border);
        assert_eq!(merged.link_url, base.link_url);
        assert_eq!(merged.hr, base.hr);
    }
}
