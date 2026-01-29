//! CLI-specific configuration types for presentation.
//!
//! This module contains presentation-related configuration that doesn't
//! belong in chibi-core (image rendering, markdown styling, etc.).

use serde::{Deserialize, Serialize};

// Re-export core config types for convenience
pub use chibi_core::config::{
    ApiParams, ResolvedConfig as CoreResolvedConfig, ToolsConfig,
};

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

/// Commodore 128 inspired color scheme (VICE palette)
pub fn default_markdown_style() -> MarkdownStyle {
    MarkdownStyle {
        bright: "#FFFF54".to_string(), // Light Yellow - for emphasis
        head: "#54FF54".to_string(),   // Light Green - for h3 headers
        symbol: "#7ABFC7".to_string(), // Cyan - for bullets, language labels
        grey: "#808080".to_string(),   // Grey - for borders, muted text
        dark: "#000000".to_string(),   // Black - code block background
        mid: "#3E31A2".to_string(),    // Blue - table headers
        light: "#352879".to_string(),  // Dark Blue - alternate backgrounds
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

// Convenience accessors to forward to core fields
impl ResolvedConfig {
    pub fn api_key(&self) -> &str {
        &self.core.api_key
    }
    pub fn model(&self) -> &str {
        &self.core.model
    }
    pub fn context_window_limit(&self) -> usize {
        self.core.context_window_limit
    }
    pub fn warn_threshold_percent(&self) -> f32 {
        self.core.warn_threshold_percent
    }
    pub fn base_url(&self) -> &str {
        &self.core.base_url
    }
    pub fn auto_compact(&self) -> bool {
        self.core.auto_compact
    }
    pub fn auto_compact_threshold(&self) -> f32 {
        self.core.auto_compact_threshold
    }
    pub fn max_recursion_depth(&self) -> usize {
        self.core.max_recursion_depth
    }
    pub fn username(&self) -> &str {
        &self.core.username
    }
    pub fn reflection_enabled(&self) -> bool {
        self.core.reflection_enabled
    }
    pub fn tool_output_cache_threshold(&self) -> usize {
        self.core.tool_output_cache_threshold
    }
    pub fn tool_cache_max_age_days(&self) -> u64 {
        self.core.tool_cache_max_age_days
    }
    pub fn auto_cleanup_cache(&self) -> bool {
        self.core.auto_cleanup_cache
    }
    pub fn tool_cache_preview_chars(&self) -> usize {
        self.core.tool_cache_preview_chars
    }
    pub fn file_tools_allowed_paths(&self) -> &[String] {
        &self.core.file_tools_allowed_paths
    }
    pub fn api(&self) -> &ApiParams {
        &self.core.api
    }
    pub fn tools(&self) -> &ToolsConfig {
        &self.core.tools
    }
}
