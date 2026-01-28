use std::io::{self, IsTerminal, Write};

use base64::Engine;
use image::GenericImageView;
use streamdown_parser::{ParseEvent, Parser};
use streamdown_render::Renderer;

/// Configuration for markdown stream rendering.
#[derive(Clone)]
pub struct MarkdownConfig {
    pub render_markdown: bool,
    pub force_render: bool,
    pub render_images: bool,
    pub image_max_download_bytes: usize,
    pub image_fetch_timeout_seconds: u64,
    pub image_allow_http: bool,
    pub image_max_height_lines: u32,
    pub image_max_width_percent: u32,
    pub image_alignment: String,
    pub image_render_mode: String,
    pub image_enable_truecolor: bool,
    pub image_enable_ansi: bool,
    pub image_enable_ascii: bool,
    pub image_cache_dir: Option<std::path::PathBuf>,
    pub image_cache_max_bytes: u64,
    pub image_cache_max_age_days: u64,
    pub markdown_style: crate::config::MarkdownStyle,
}

impl MarkdownConfig {
    /// Build a MarkdownConfig from a ResolvedConfig.
    pub fn from_resolved(
        config: &crate::config::ResolvedConfig,
        chibi_dir: &std::path::Path,
        force_render: bool,
    ) -> Self {
        Self {
            render_markdown: config.render_markdown,
            force_render,
            render_images: config.render_images,
            image_max_download_bytes: config.image_max_download_bytes,
            image_fetch_timeout_seconds: config.image_fetch_timeout_seconds,
            image_allow_http: config.image_allow_http,
            image_max_height_lines: config.image_max_height_lines,
            image_max_width_percent: config.image_max_width_percent,
            image_alignment: config.image_alignment.clone(),
            image_render_mode: config.image_render_mode.clone(),
            image_enable_truecolor: config.image_enable_truecolor,
            image_enable_ansi: config.image_enable_ansi,
            image_enable_ascii: config.image_enable_ascii,
            image_cache_dir: if config.image_cache_enabled {
                Some(chibi_dir.join("image_cache"))
            } else {
                None
            },
            image_cache_max_bytes: config.image_cache_max_bytes,
            image_cache_max_age_days: config.image_cache_max_age_days,
            markdown_style: config.markdown_style.clone(),
        }
    }
}

/// Grouped fetch settings passed into image rendering functions.
struct ImageFetchConfig {
    max_download_bytes: usize,
    fetch_timeout_seconds: u64,
    allow_http: bool,
    cache_dir: Option<std::path::PathBuf>,
    cache_max_bytes: u64,
    cache_max_age_days: u64,
}

/// Display settings for rendered images.
struct ImageDisplayConfig {
    max_height_lines: u32,
    max_width_percent: u32,
    alignment: String,
}

/// Terminal rendering capabilities detected from environment
#[derive(Debug, Clone, Copy)]
enum TerminalCapability {
    Truecolor,
    Ansi256,
    Ansi16,
}

/// Resolved image rendering mode after capability detection
#[derive(Debug, Clone, Copy)]
enum ImageRenderMode {
    Truecolor,
    Ansi,
    Ascii,
    Placeholder,
}

/// Zero-sized newtype so `Renderer<StdoutWriter>` can persist across the stream
/// while each write locks stdout independently.
struct StdoutWriter;

impl Write for StdoutWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut lock = io::stdout().lock();
        let n = lock.write(buf)?;
        lock.flush()?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().lock().flush()
    }
}

struct RenderPipeline {
    parser: Parser,
    renderer: Renderer<StdoutWriter>,
}

/// Streaming markdown renderer with TTY detection.
///
/// When stdout is a terminal, chunks are line-buffered and rendered through
/// the streamdown parser/renderer pipeline. When piped, raw bytes pass
/// through directly (matching previous behavior exactly).
///
/// When `render_images` is enabled, standalone `ParseEvent::Image` events
/// are rendered inline using truecolor ANSI escape codes. Remote images
/// (HTTPS, and optionally HTTP) are fetched with configurable limits.
pub struct MarkdownStream {
    line_buffer: String,
    pipeline: Option<RenderPipeline>,
    render_images: bool,
    terminal_width: usize,
    fetch_config: ImageFetchConfig,
    display_config: ImageDisplayConfig,
    render_mode: ImageRenderMode,
}

/// Detect terminal rendering capabilities from environment variables
fn detect_terminal_capability() -> TerminalCapability {
    // Check COLORTERM for truecolor support
    if let Ok(colorterm) = std::env::var("COLORTERM") {
        let ct = colorterm.to_lowercase();
        if ct.contains("truecolor") || ct.contains("24bit") {
            return TerminalCapability::Truecolor;
        }
    }

    // Check TERM for color support level
    if let Ok(term) = std::env::var("TERM") {
        let t = term.to_lowercase();
        if t.contains("truecolor") || t.contains("24bit") {
            return TerminalCapability::Truecolor;
        }
        if t.contains("256color") {
            return TerminalCapability::Ansi256;
        }
        if t.contains("color") {
            return TerminalCapability::Ansi16;
        }
    }

    // Default to basic ANSI if we can't determine capability
    TerminalCapability::Ansi16
}

/// Resolve the rendering mode based on config and terminal capabilities
fn resolve_render_mode(
    mode: &str,
    enable_truecolor: bool,
    enable_ansi: bool,
    enable_ascii: bool,
) -> ImageRenderMode {
    match mode {
        "truecolor" if enable_truecolor => ImageRenderMode::Truecolor,
        "ansi" if enable_ansi => ImageRenderMode::Ansi,
        "ascii" if enable_ascii => ImageRenderMode::Ascii,
        "placeholder" => ImageRenderMode::Placeholder,
        "auto" => {
            let cap = detect_terminal_capability();
            match cap {
                TerminalCapability::Truecolor if enable_truecolor => ImageRenderMode::Truecolor,
                TerminalCapability::Truecolor
                | TerminalCapability::Ansi256
                | TerminalCapability::Ansi16
                    if enable_ansi =>
                {
                    ImageRenderMode::Ansi
                }
                _ if enable_ascii => ImageRenderMode::Ascii,
                _ => ImageRenderMode::Placeholder,
            }
        }
        _ => {
            // Invalid mode or disabled, fallback to auto logic
            resolve_render_mode("auto", enable_truecolor, enable_ansi, enable_ascii)
        }
    }
}

impl MarkdownStream {
    /// Creates a new markdown stream.
    ///
    /// When `render_markdown` is true and stdout is a TTY, creates a streamdown
    /// pipeline for formatted output. Otherwise, raw passthrough.
    pub fn new(config: MarkdownConfig) -> Self {
        let (pipeline, terminal_width) =
            if config.render_markdown && (config.force_render || io::stdout().is_terminal()) {
                let width = streamdown_render::terminal_width();
                let style = config.markdown_style.clone();
                (
                    Some(RenderPipeline {
                        parser: Parser::new(),
                        renderer: Renderer::with_style(StdoutWriter, width, style),
                    }),
                    width,
                )
            } else {
                (None, 80)
            };

        // Determine rendering mode
        let render_mode = resolve_render_mode(
            &config.image_render_mode,
            config.image_enable_truecolor,
            config.image_enable_ansi,
            config.image_enable_ascii,
        );

        MarkdownStream {
            line_buffer: String::new(),
            pipeline,
            render_images: config.render_images,
            terminal_width,
            fetch_config: ImageFetchConfig {
                max_download_bytes: config.image_max_download_bytes,
                fetch_timeout_seconds: config.image_fetch_timeout_seconds,
                allow_http: config.image_allow_http,
                cache_dir: config.image_cache_dir,
                cache_max_bytes: config.image_cache_max_bytes,
                cache_max_age_days: config.image_cache_max_age_days,
            },
            display_config: ImageDisplayConfig {
                max_height_lines: config.image_max_height_lines,
                max_width_percent: config.image_max_width_percent,
                alignment: config.image_alignment,
            },
            render_mode,
        }
    }

    /// Render a list of parse events, intercepting image events when enabled.
    fn render_events(
        pipeline: &mut RenderPipeline,
        events: &[ParseEvent],
        render_images: bool,
        terminal_width: usize,
        fetch_config: &ImageFetchConfig,
        display_config: &ImageDisplayConfig,
        render_mode: ImageRenderMode,
    ) -> io::Result<()> {
        for event in events {
            if render_images
                && let ParseEvent::Image { alt, url } = event
                && try_render_image(
                    url,
                    alt,
                    terminal_width,
                    fetch_config,
                    display_config,
                    render_mode,
                )
                .is_ok()
            {
                continue;
            }
            pipeline
                .renderer
                .render_event(event)
                .map_err(|e| io::Error::other(format!("Render error: {}", e)))?;
        }
        Ok(())
    }

    /// Buffer chunk, render complete lines. Passthrough if not TTY.
    pub fn write_chunk(&mut self, chunk: &str) -> io::Result<()> {
        if chunk.is_empty() {
            return Ok(());
        }

        let pipeline = match self.pipeline.as_mut() {
            Some(p) => p,
            None => {
                // Passthrough: write raw bytes directly
                let mut lock = io::stdout().lock();
                lock.write_all(chunk.as_bytes())?;
                lock.flush()?;
                return Ok(());
            }
        };

        self.line_buffer.push_str(chunk);

        // Process all complete lines
        while let Some(newline_pos) = self.line_buffer.find('\n') {
            let line = self.line_buffer[..newline_pos].to_string();
            self.line_buffer = self.line_buffer[newline_pos + 1..].to_string();

            let events = pipeline.parser.parse_line(&line);
            Self::render_events(
                pipeline,
                &events,
                self.render_images,
                self.terminal_width,
                &self.fetch_config,
                &self.display_config,
                self.render_mode,
            )?;
        }

        Ok(())
    }

    /// Flush remaining partial line buffer. Call at end of response.
    pub fn finish(&mut self) -> io::Result<()> {
        let pipeline = match self.pipeline.as_mut() {
            Some(p) => p,
            None => return Ok(()),
        };

        // Flush any remaining partial line
        if !self.line_buffer.is_empty() {
            let line = std::mem::take(&mut self.line_buffer);
            let events = pipeline.parser.parse_line(&line);
            Self::render_events(
                pipeline,
                &events,
                self.render_images,
                self.terminal_width,
                &self.fetch_config,
                &self.display_config,
                self.render_mode,
            )?;
        }

        // Finalize parser (close any open blocks)
        let events = pipeline.parser.finalize();
        Self::render_events(
            pipeline,
            &events,
            self.render_images,
            self.terminal_width,
            &self.fetch_config,
            &self.display_config,
            self.render_mode,
        )?;

        Ok(())
    }
}

/// Decode a `data:image/...;base64,...` URI into a `DynamicImage`.
///
/// Expects the input with the `data:` prefix already stripped (i.e., starts
/// with `image/...`).
fn decode_data_uri_image(rest: &str) -> io::Result<image::DynamicImage> {
    let (mime, payload) = rest
        .split_once(";base64,")
        .ok_or_else(|| io::Error::other("data URI missing ;base64, delimiter"))?;

    if !mime.starts_with("image/") {
        return Err(io::Error::other(format!(
            "data URI MIME type is not an image: {}",
            mime
        )));
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| io::Error::other(format!("invalid base64 in data URI: {}", e)))?;

    image::load_from_memory(&bytes)
        .map_err(|e| io::Error::other(format!("failed to decode image from data URI: {}", e)))
}

/// Fetch a remote image over HTTP(S) and decode it.
fn fetch_remote_image(url: &str, config: &ImageFetchConfig) -> io::Result<image::DynamicImage> {
    if url.starts_with("http://") && !config.allow_http {
        return Err(io::Error::other(
            "plain HTTP image URLs are not allowed (set image_allow_http = true to enable)",
        ));
    }

    // Try cache first
    if let Some(ref cache_dir) = config.cache_dir
        && let Some(cached) = crate::image_cache::cache_get(cache_dir, url)
    {
        return image::load_from_memory(&cached)
            .map_err(|e| io::Error::other(format!("failed to decode cached image: {}", e)));
    }

    // Fetch from network
    let bytes = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(fetch_image_bytes(
            url,
            config.max_download_bytes,
            config.fetch_timeout_seconds,
            config.allow_http,
        ))
    })?;

    // Store in cache (best-effort)
    if let Some(ref cache_dir) = config.cache_dir {
        let _ = crate::image_cache::cache_put(
            cache_dir,
            url,
            &bytes,
            config.cache_max_bytes,
            config.cache_max_age_days,
        );
    }

    image::load_from_memory(&bytes)
        .map_err(|e| io::Error::other(format!("failed to decode fetched image: {}", e)))
}

/// Asynchronously fetch image bytes from a URL with size and timeout limits.
async fn fetch_image_bytes(
    url: &str,
    max_bytes: usize,
    timeout_seconds: u64,
    allow_http: bool,
) -> io::Result<Vec<u8>> {
    use futures_util::StreamExt;

    // Build a redirect policy that prevents HTTPS→HTTP downgrades
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("too many redirects (max 5)")
        } else if !allow_http {
            // Block HTTPS→HTTP downgrade
            if let Some(prev) = attempt.previous().last()
                && prev.scheme() == "https"
                && attempt.url().scheme() == "http"
            {
                return attempt.error("redirect from HTTPS to HTTP is not allowed");
            }
            attempt.follow()
        } else {
            attempt.follow()
        }
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_seconds))
        .redirect(redirect_policy)
        .build()
        .map_err(|e| io::Error::other(format!("failed to build HTTP client: {}", e)))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| io::Error::other(format!("image fetch failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(io::Error::other(format!(
            "image fetch returned HTTP {}",
            response.status()
        )));
    }

    // Validate Content-Type if present
    if let Some(ct) = response.headers().get(reqwest::header::CONTENT_TYPE)
        && let Ok(ct_str) = ct.to_str()
        && !ct_str.starts_with("image/")
    {
        return Err(io::Error::other(format!(
            "remote URL Content-Type is not an image: {}",
            ct_str
        )));
    }

    // Early reject if Content-Length exceeds limit
    if let Some(len) = response.content_length()
        && len as usize > max_bytes
    {
        return Err(io::Error::other(format!(
            "image too large: Content-Length {} exceeds limit {}",
            len, max_bytes
        )));
    }

    // Stream body with size enforcement
    let mut stream = response.bytes_stream();
    let mut buf = Vec::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk =
            chunk_result.map_err(|e| io::Error::other(format!("image download error: {}", e)))?;
        if buf.len() + chunk.len() > max_bytes {
            return Err(io::Error::other(format!(
                "image download exceeded size limit of {} bytes",
                max_bytes
            )));
        }
        buf.extend_from_slice(&chunk);
    }

    Ok(buf)
}

/// Attempt to render an image inline with the appropriate mode
fn try_render_image(
    url: &str,
    _alt: &str,
    term_width: usize,
    fetch_config: &ImageFetchConfig,
    display_config: &ImageDisplayConfig,
    render_mode: ImageRenderMode,
) -> io::Result<()> {
    // Early return for placeholder mode
    if matches!(render_mode, ImageRenderMode::Placeholder) {
        return Err(io::Error::other("placeholder mode"));
    }

    // Load the image
    let img = if let Some(rest) = url.strip_prefix("data:") {
        decode_data_uri_image(rest)?
    } else if url.starts_with("http://") || url.starts_with("https://") {
        fetch_remote_image(url, fetch_config)?
    } else {
        let path = url.strip_prefix("file://").unwrap_or(url);
        image::open(path)
            .map_err(|e| io::Error::other(format!("Failed to open image {}: {}", path, e)))?
    };

    let (orig_w, orig_h) = (img.width(), img.height());
    let term_w = term_width as u32;

    // Apply width percent constraint
    let effective_width = term_w * display_config.max_width_percent.min(100) / 100;
    // Height limit in pixels (truecolor renderer prints 2 pixels per terminal line)
    let max_height_pixels = display_config.max_height_lines * 2;

    let (new_w, mut new_h) = imgcatr::ops::image_resized_size(
        (orig_w, orig_h),
        (effective_width, max_height_pixels),
        true,
    );

    // Clamp height if it still exceeds the limit
    if new_h > max_height_pixels {
        new_h = max_height_pixels;
    }

    // Ensure height is even (truecolor renderer prints pixel pairs)
    if new_h % 2 != 0 {
        new_h += 1;
    }

    let resized = imgcatr::ops::resize_image(&img, (new_w, new_h));

    // Render with the appropriate mode
    match render_mode {
        ImageRenderMode::Truecolor => render_truecolor(&resized, term_width, display_config),
        ImageRenderMode::Ansi => render_ansi(&resized, term_width, display_config),
        ImageRenderMode::Ascii => render_ascii(&resized, term_width, display_config),
        ImageRenderMode::Placeholder => unreachable!(),
    }
}

/// Render image with 24-bit truecolor ANSI codes
fn render_truecolor(
    img: &image::DynamicImage,
    term_width: usize,
    display_config: &ImageDisplayConfig,
) -> io::Result<()> {
    let new_w = img.width() as usize;
    let new_h = img.height() as usize;
    let pad = calculate_padding(new_w, term_width, &display_config.alignment);

    let mut stdout = io::stdout().lock();
    writeln!(stdout)?;

    if pad == 0 {
        imgcatr::ops::write_ansi_truecolor(&mut stdout, img);
    } else {
        let padding: String = " ".repeat(pad);
        let pixels = img.to_rgba8();
        let mut y = 0;
        while y + 1 < new_h {
            write!(stdout, "{}", padding)?;
            for x in 0..new_w {
                let top = pixels.get_pixel(x as u32, y as u32);
                let bot = pixels.get_pixel(x as u32, (y + 1) as u32);
                write!(
                    stdout,
                    "\x1b[38;2;{};{};{};48;2;{};{};{}m\u{2580}",
                    top[0], top[1], top[2], bot[0], bot[1], bot[2]
                )?;
            }
            writeln!(stdout, "\x1b[0m")?;
            y += 2;
        }
        if y < new_h {
            write!(stdout, "{}", padding)?;
            for x in 0..new_w {
                let top = pixels.get_pixel(x as u32, y as u32);
                write!(
                    stdout,
                    "\x1b[38;2;{};{};{}m\u{2580}",
                    top[0], top[1], top[2]
                )?;
            }
            writeln!(stdout, "\x1b[0m")?;
        }
    }

    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}

/// Render image with 16-color ANSI codes
fn render_ansi(
    img: &image::DynamicImage,
    term_width: usize,
    display_config: &ImageDisplayConfig,
) -> io::Result<()> {
    let new_w = img.width() as usize;
    let pad = calculate_padding(new_w, term_width, &display_config.alignment);

    let mut stdout = io::stdout().lock();
    writeln!(stdout)?;

    // Use imgcatr's ANSI color approximation
    use imgcatr::util::ANSI_COLOURS_WHITE_BG;

    if pad == 0 {
        imgcatr::ops::write_ansi(&mut stdout, img, &ANSI_COLOURS_WHITE_BG);
    } else {
        // Manual rendering with padding
        let padding: String = " ".repeat(pad);
        let colourtable = imgcatr::ops::create_colourtable(
            img,
            &ANSI_COLOURS_WHITE_BG,
            imgcatr::util::bg_colours_for(&ANSI_COLOURS_WHITE_BG),
        );

        for line in colourtable {
            write!(stdout, "{}", padding)?;
            for (upper_clr, lower_clr) in line {
                write!(
                    stdout,
                    "{}{}\u{2580}",
                    imgcatr::util::ANSI_COLOUR_ESCAPES[upper_clr],
                    imgcatr::util::ANSI_BG_COLOUR_ESCAPES[lower_clr]
                )?;
            }
            writeln!(stdout, "{}", imgcatr::util::ANSI_RESET_ATTRIBUTES)?;
        }
    }

    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}

/// Render image as ASCII art
fn render_ascii(
    img: &image::DynamicImage,
    term_width: usize,
    display_config: &ImageDisplayConfig,
) -> io::Result<()> {
    let (width, height) = img.dimensions();
    let pad = calculate_padding(width as usize / 2, term_width, &display_config.alignment);
    let padding: String = " ".repeat(pad);

    let mut stdout = io::stdout().lock();
    writeln!(stdout)?;

    // Convert to RGBA8 to ensure we can read pixels safely
    let img = img.to_rgba8();

    for y in (0..height).step_by(2) {
        write!(stdout, "{}", padding)?;
        for x in (0..width).step_by(1) {
            let pix = img.get_pixel(x, y);
            let mut intensity = pix[0] / 3 + pix[1] / 3 + pix[2] / 3;
            // Handle transparency
            if pix[3] == 0 {
                intensity = 0;
            }
            write!(stdout, "{}", ascii_char(intensity))?;
        }
        writeln!(stdout)?;
    }

    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}

/// Map intensity to ASCII character
fn ascii_char(intensity: u8) -> &'static str {
    let index = (intensity / 32).min(7);
    [" ", ".", ",", "-", "~", "+", "=", "@"][index as usize]
}

/// Calculate left padding for image alignment
fn calculate_padding(image_cols: usize, term_width: usize, alignment: &str) -> usize {
    match alignment {
        "center" => term_width.saturating_sub(image_cols) / 2,
        "right" => term_width.saturating_sub(image_cols),
        _ => 0, // "left" or anything else
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode raw bytes as a data URI string for test helpers.
    fn make_data_uri(mime: &str, data: &[u8]) -> String {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        format!("data:{};base64,{}", mime, encoded)
    }

    /// Build a valid 1x1 red PNG in memory.
    fn tiny_png() -> Vec<u8> {
        use std::io::Cursor;
        let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]));
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageOutputFormat::Png)
            .expect("encoding 1x1 PNG");
        buf.into_inner()
    }

    fn default_fetch_config() -> ImageFetchConfig {
        ImageFetchConfig {
            max_download_bytes: 10 * 1024 * 1024,
            fetch_timeout_seconds: 5,
            allow_http: false,
            cache_dir: None,
            cache_max_bytes: 104_857_600,
            cache_max_age_days: 30,
        }
    }

    fn default_display_config() -> ImageDisplayConfig {
        ImageDisplayConfig {
            max_height_lines: 25,
            max_width_percent: 80,
            alignment: "center".to_string(),
        }
    }

    #[test]
    fn decode_valid_png_data_uri() {
        let uri = make_data_uri("image/png", &tiny_png());
        let rest = uri.strip_prefix("data:").unwrap();
        let img = decode_data_uri_image(rest).expect("should decode valid PNG data URI");
        assert_eq!(img.width(), 1);
        assert_eq!(img.height(), 1);
    }

    #[test]
    fn decode_missing_base64_delimiter() {
        let err = decode_data_uri_image("image/png,abc").unwrap_err();
        assert!(
            err.to_string().contains("missing ;base64, delimiter"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn decode_non_image_mime() {
        let err = decode_data_uri_image("text/plain;base64,dGVzdA==").unwrap_err();
        assert!(
            err.to_string().contains("not an image"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn decode_invalid_base64() {
        let err = decode_data_uri_image("image/png;base64,!!!not-base64!!!").unwrap_err();
        assert!(
            err.to_string().contains("invalid base64"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn decode_valid_base64_but_not_image() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"not image bytes");
        let input = format!("image/png;base64,{}", encoded);
        let err = decode_data_uri_image(&input).unwrap_err();
        assert!(
            err.to_string().contains("failed to decode image"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn fetch_rejects_http_when_not_allowed() {
        let config = ImageFetchConfig {
            allow_http: false,
            ..default_fetch_config()
        };
        let err = fetch_remote_image("http://example.com/image.png", &config).unwrap_err();
        assert!(
            err.to_string()
                .contains("plain HTTP image URLs are not allowed"),
            "unexpected error: {}",
            err
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn fetch_allows_http_when_configured() {
        // http://example.com/image.png won't resolve to an actual image,
        // but the point is it should NOT be rejected by the protocol check.
        let config = ImageFetchConfig {
            allow_http: true,
            fetch_timeout_seconds: 1,
            ..default_fetch_config()
        };
        let err = fetch_remote_image("http://example.com/nonexistent.png", &config);
        // Should fail with a network/HTTP error, not a protocol error
        if let Err(e) = err {
            assert!(
                !e.to_string()
                    .contains("plain HTTP image URLs are not allowed"),
                "should not reject http:// when allow_http is true, got: {}",
                e
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn try_render_image_dispatches_https() {
        // https URL should attempt fetch (and fail in test env), not return
        // "remote URLs not supported"
        let config = default_fetch_config();
        let display = default_display_config();
        let err = try_render_image(
            "https://example.com/nonexistent.png",
            "test",
            80,
            &config,
            &display,
            ImageRenderMode::Truecolor,
        )
        .unwrap_err();
        assert!(
            !err.to_string().contains("remote URLs not supported"),
            "should dispatch to fetch path, got: {}",
            err
        );
    }
}
