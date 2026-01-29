use std::io::{self, IsTerminal, Write};

use base64::Engine;
use image::GenericImageView;
use streamdown_parser::{ParseEvent, Parser};
use streamdown_render::Renderer;

use crate::config::{ConfigImageRenderMode, ImageAlignment, ImageConfig, MarkdownStyle};

/// Configuration for markdown stream rendering.
#[derive(Clone)]
pub struct MarkdownConfig {
    pub render_markdown: bool,
    pub force_render: bool,
    pub image: ImageConfig,
    pub image_cache_dir: Option<std::path::PathBuf>,
    pub markdown_style: MarkdownStyle,
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
            image: config.image.clone(),
            image_cache_dir: if config.image.cache_enabled {
                Some(chibi_dir.join("image_cache"))
            } else {
                None
            },
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
    alignment: ImageAlignment,
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
        io::stdout().lock().write(buf)
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
    mode: ConfigImageRenderMode,
    enable_truecolor: bool,
    enable_ansi: bool,
    enable_ascii: bool,
) -> ImageRenderMode {
    match mode {
        ConfigImageRenderMode::Truecolor if enable_truecolor => ImageRenderMode::Truecolor,
        ConfigImageRenderMode::Ansi if enable_ansi => ImageRenderMode::Ansi,
        ConfigImageRenderMode::Ascii if enable_ascii => ImageRenderMode::Ascii,
        ConfigImageRenderMode::Placeholder => ImageRenderMode::Placeholder,
        ConfigImageRenderMode::Auto => {
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
            // Disabled mode, fallback to auto logic
            resolve_render_mode(
                ConfigImageRenderMode::Auto,
                enable_truecolor,
                enable_ansi,
                enable_ascii,
            )
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
            config.image.render_mode,
            config.image.enable_truecolor,
            config.image.enable_ansi,
            config.image.enable_ascii,
        );

        MarkdownStream {
            line_buffer: String::new(),
            pipeline,
            render_images: config.image.render_images,
            terminal_width,
            fetch_config: ImageFetchConfig {
                max_download_bytes: config.image.max_download_bytes,
                fetch_timeout_seconds: config.image.fetch_timeout_seconds,
                allow_http: config.image.allow_http,
                cache_dir: config.image_cache_dir,
                cache_max_bytes: config.image.cache_max_bytes,
                cache_max_age_days: config.image.cache_max_age_days,
            },
            display_config: ImageDisplayConfig {
                max_height_lines: config.image.max_height_lines,
                max_width_percent: config.image.max_width_percent,
                alignment: config.image.alignment,
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
                && let ParseEvent::Image { url, .. } = event
                && try_render_image(
                    url,
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
            let line = self.line_buffer[..newline_pos]
                .trim_end_matches('\r')
                .to_string();
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
            let line = line.trim_end_matches('\r');
            let events = pipeline.parser.parse_line(line);
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

    // Fetch from network — spawn on a separate task to isolate timeout/cancellation
    // handling. block_in_place temporarily parks the calling thread while awaiting
    // the result, which is necessary for synchronous rendering.
    let url_owned = url.to_string();
    let max_bytes = config.max_download_bytes;
    let timeout = config.fetch_timeout_seconds;
    let allow_http = config.allow_http;
    let bytes = tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            tokio::task::spawn(fetch_image_bytes(url_owned, max_bytes, timeout, allow_http))
                .await
                .map_err(|e| io::Error::other(format!("image fetch task failed: {}", e)))?
        })
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
    url: String,
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
    let pad = calculate_padding(new_w, term_width, display_config.alignment);

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
    let pad = calculate_padding(new_w, term_width, display_config.alignment);

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
    let pad = calculate_padding(width as usize, term_width, display_config.alignment);
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
fn calculate_padding(image_cols: usize, term_width: usize, alignment: ImageAlignment) -> usize {
    match alignment {
        ImageAlignment::Center => term_width.saturating_sub(image_cols) / 2,
        ImageAlignment::Right => term_width.saturating_sub(image_cols),
        ImageAlignment::Left => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_markdown_style;

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
        img.write_to(&mut buf, image::ImageFormat::Png)
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
            alignment: ImageAlignment::Center,
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

    #[test]
    fn ascii_char_maps_intensity_to_characters() {
        // Test minimum intensity (0) returns space
        assert_eq!(ascii_char(0), " ");

        // Test maximum intensity (255) returns @
        assert_eq!(ascii_char(255), "@");

        // Test bucket boundaries where division by 32 changes the index
        // intensity / 32 = 0 for 0-31, returns " "
        assert_eq!(ascii_char(1), " ");
        assert_eq!(ascii_char(31), " ");

        // intensity / 32 = 1 for 32-63, returns "."
        assert_eq!(ascii_char(32), ".");
        assert_eq!(ascii_char(63), ".");

        // intensity / 32 = 2 for 64-95, returns ","
        assert_eq!(ascii_char(64), ",");
        assert_eq!(ascii_char(95), ",");

        // intensity / 32 = 3 for 96-127, returns "-"
        assert_eq!(ascii_char(96), "-");
        assert_eq!(ascii_char(127), "-");

        // intensity / 32 = 4 for 128-159, returns "~"
        assert_eq!(ascii_char(128), "~");
        assert_eq!(ascii_char(159), "~");

        // intensity / 32 = 5 for 160-191, returns "+"
        assert_eq!(ascii_char(160), "+");
        assert_eq!(ascii_char(191), "+");

        // intensity / 32 = 6 for 192-223, returns "="
        assert_eq!(ascii_char(192), "=");
        assert_eq!(ascii_char(223), "=");

        // intensity / 32 = 7 (and .min(7)) for 224-255, returns "@"
        assert_eq!(ascii_char(224), "@");
        assert_eq!(ascii_char(254), "@");
    }

    #[test]
    fn ascii_char_never_panics() {
        // Test that all u8 values are handled without panic
        for intensity in 0..=u8::MAX {
            let result = ascii_char(intensity);
            // Verify we got a valid character from the lookup table
            assert!(
                matches!(result, " " | "." | "," | "-" | "~" | "+" | "=" | "@"),
                "unexpected character for intensity {}: {}",
                intensity,
                result
            );
        }
    }

    // ========== calculate_padding tests ==========

    #[test]
    fn padding_left_alignment_is_always_zero() {
        assert_eq!(calculate_padding(40, 80, ImageAlignment::Left), 0);
        assert_eq!(calculate_padding(80, 80, ImageAlignment::Left), 0);
        assert_eq!(calculate_padding(120, 80, ImageAlignment::Left), 0);
    }

    #[test]
    fn padding_center_alignment_splits_remaining_space() {
        // 80 - 40 = 40 remaining, 40 / 2 = 20
        assert_eq!(calculate_padding(40, 80, ImageAlignment::Center), 20);
        // 80 - 60 = 20 remaining, 20 / 2 = 10
        assert_eq!(calculate_padding(60, 80, ImageAlignment::Center), 10);
        // Odd remainder: 80 - 41 = 39, 39 / 2 = 19 (integer division floors)
        assert_eq!(calculate_padding(41, 80, ImageAlignment::Center), 19);
    }

    #[test]
    fn padding_right_alignment_uses_full_remaining_space() {
        // 80 - 40 = 40
        assert_eq!(calculate_padding(40, 80, ImageAlignment::Right), 40);
        // 80 - 60 = 20
        assert_eq!(calculate_padding(60, 80, ImageAlignment::Right), 20);
        // 80 - 1 = 79
        assert_eq!(calculate_padding(1, 80, ImageAlignment::Right), 79);
    }

    #[test]
    fn padding_image_wider_than_terminal_saturates_to_zero() {
        // Image wider than terminal: saturating_sub prevents underflow
        assert_eq!(calculate_padding(100, 80, ImageAlignment::Center), 0);
        assert_eq!(calculate_padding(100, 80, ImageAlignment::Right), 0);
        assert_eq!(calculate_padding(100, 80, ImageAlignment::Left), 0);
    }

    #[test]
    fn padding_image_equals_terminal_width_is_zero() {
        // No space remaining in any alignment
        assert_eq!(calculate_padding(80, 80, ImageAlignment::Center), 0);
        assert_eq!(calculate_padding(80, 80, ImageAlignment::Right), 0);
    }

    #[test]
    fn padding_zero_width_image() {
        // Degenerate case: zero-width image gets full padding
        assert_eq!(calculate_padding(0, 80, ImageAlignment::Center), 40);
        assert_eq!(calculate_padding(0, 80, ImageAlignment::Right), 80);
        assert_eq!(calculate_padding(0, 80, ImageAlignment::Left), 0);
    }

    // ========== resolve_render_mode tests ==========

    #[test]
    fn resolve_mode_explicit_truecolor_when_enabled() {
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Truecolor,
            true, // enable_truecolor
            true, // enable_ansi
            true, // enable_ascii
        );
        assert!(matches!(mode, ImageRenderMode::Truecolor));
    }

    #[test]
    fn resolve_mode_explicit_truecolor_falls_back_when_disabled() {
        // Requesting Truecolor but it's disabled: falls through to catch-all
        // which re-invokes Auto logic
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Truecolor,
            false, // enable_truecolor disabled
            true,  // enable_ansi
            true,  // enable_ascii
        );
        // Auto with truecolor disabled resolves to Ansi or Ascii depending on
        // terminal capability, but definitely not Truecolor
        assert!(!matches!(mode, ImageRenderMode::Truecolor));
    }

    #[test]
    fn resolve_mode_explicit_ansi_when_enabled() {
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Ansi,
            true,
            true, // enable_ansi
            true,
        );
        assert!(matches!(mode, ImageRenderMode::Ansi));
    }

    #[test]
    fn resolve_mode_explicit_ansi_falls_back_when_disabled() {
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Ansi,
            true,
            false, // enable_ansi disabled
            true,  // enable_ascii available
        );
        assert!(!matches!(mode, ImageRenderMode::Ansi));
    }

    #[test]
    fn resolve_mode_explicit_ascii_when_enabled() {
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Ascii,
            true,
            true,
            true, // enable_ascii
        );
        assert!(matches!(mode, ImageRenderMode::Ascii));
    }

    #[test]
    fn resolve_mode_explicit_ascii_falls_back_when_disabled() {
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Ascii,
            true,
            true,
            false, // enable_ascii disabled
        );
        assert!(!matches!(mode, ImageRenderMode::Ascii));
    }

    #[test]
    fn resolve_mode_placeholder_ignores_capability_flags() {
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Placeholder,
            false,
            false,
            false,
        );
        assert!(matches!(mode, ImageRenderMode::Placeholder));
    }

    #[test]
    fn resolve_mode_all_disabled_yields_placeholder() {
        // Auto mode with all render modes disabled: nothing available
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Auto,
            false, // no truecolor
            false, // no ansi
            false, // no ascii
        );
        assert!(matches!(mode, ImageRenderMode::Placeholder));
    }

    #[test]
    fn resolve_mode_auto_with_only_ascii_yields_ascii() {
        // Auto with only ASCII enabled: regardless of terminal capability
        // the final fallback `_ if enable_ascii` catches it
        let mode = resolve_render_mode(
            ConfigImageRenderMode::Auto,
            false, // no truecolor
            false, // no ansi
            true,  // ascii only
        );
        assert!(matches!(mode, ImageRenderMode::Ascii));
    }

    // ========== MarkdownStream passthrough mode tests ==========

    /// Helper: construct a MarkdownStream in passthrough mode (no pipeline).
    fn passthrough_stream() -> MarkdownStream {
        MarkdownStream {
            line_buffer: String::new(),
            pipeline: None,
            render_images: false,
            terminal_width: 80,
            fetch_config: default_fetch_config(),
            display_config: default_display_config(),
            render_mode: ImageRenderMode::Placeholder,
        }
    }

    #[test]
    fn passthrough_does_not_buffer_partial_lines() {
        let mut stream = passthrough_stream();

        // In passthrough mode, write_chunk goes directly to stdout without
        // touching line_buffer. The buffer should remain empty.
        let result = stream.write_chunk("no newline here");
        assert!(result.is_ok());
        assert!(
            stream.line_buffer.is_empty(),
            "passthrough mode must not buffer content"
        );
    }

    #[test]
    fn passthrough_finish_is_noop() {
        let mut stream = passthrough_stream();
        stream.line_buffer = "leftover".to_string();

        // finish() with no pipeline returns immediately without touching buffer
        let result = stream.finish();
        assert!(result.is_ok());
        // Buffer is NOT flushed because there is no pipeline to flush through
        assert_eq!(
            stream.line_buffer, "leftover",
            "passthrough finish must not consume the buffer"
        );
    }

    // ========== MarkdownStream line buffering tests ==========

    /// Helper: construct a MarkdownStream with a render pipeline for buffering tests.
    fn rendering_stream() -> MarkdownStream {
        let style = default_markdown_style();

        MarkdownStream {
            line_buffer: String::new(),
            pipeline: Some(RenderPipeline {
                parser: streamdown_parser::Parser::new(),
                renderer: streamdown_render::Renderer::with_style(StdoutWriter, 80, style),
            }),
            render_images: false,
            terminal_width: 80,
            fetch_config: default_fetch_config(),
            display_config: default_display_config(),
            render_mode: ImageRenderMode::Placeholder,
        }
    }

    #[test]
    fn write_chunk_buffers_partial_line_until_newline() {
        let mut stream = rendering_stream();

        // Write content without a newline: should stay buffered
        stream.write_chunk("hello world").unwrap();
        assert_eq!(
            stream.line_buffer, "hello world",
            "partial line must remain in buffer"
        );

        // Add more without newline: accumulates
        stream.write_chunk(" more text").unwrap();
        assert_eq!(
            stream.line_buffer, "hello world more text",
            "buffer must accumulate across chunks"
        );
    }

    #[test]
    fn write_chunk_processes_complete_lines() {
        let mut stream = rendering_stream();

        // Write a complete line: it should be processed (buffer cleared)
        stream.write_chunk("line one\n").unwrap();
        assert!(
            stream.line_buffer.is_empty(),
            "complete line must be flushed from buffer"
        );
    }

    #[test]
    fn write_chunk_retains_trailing_partial_after_newlines() {
        let mut stream = rendering_stream();

        // Two complete lines plus a trailing partial
        stream.write_chunk("first\nsecond\npartial").unwrap();
        assert_eq!(
            stream.line_buffer, "partial",
            "only the trailing partial must remain buffered"
        );
    }

    #[test]
    fn write_chunk_multiple_newlines_clears_buffer() {
        let mut stream = rendering_stream();

        // Multiple complete lines, no trailing content
        stream.write_chunk("a\nb\nc\n").unwrap();
        assert!(
            stream.line_buffer.is_empty(),
            "all complete lines consumed, buffer must be empty"
        );
    }

    #[test]
    fn write_chunk_empty_string_is_noop() {
        let mut stream = rendering_stream();
        stream.line_buffer = "existing".to_string();

        stream.write_chunk("").unwrap();
        assert_eq!(
            stream.line_buffer, "existing",
            "empty chunk must not modify buffer"
        );
    }

    #[test]
    fn finish_flushes_remaining_buffer() {
        let mut stream = rendering_stream();

        // Load a partial line that has no newline
        stream.write_chunk("dangling content").unwrap();
        assert_eq!(stream.line_buffer, "dangling content");

        // finish() must consume the buffer
        stream.finish().unwrap();
        assert!(
            stream.line_buffer.is_empty(),
            "finish must flush remaining buffered content"
        );
    }

    #[test]
    fn finish_on_empty_buffer_succeeds() {
        let mut stream = rendering_stream();
        assert!(stream.line_buffer.is_empty());

        // Should not error even with nothing to flush
        stream.finish().unwrap();
        assert!(stream.line_buffer.is_empty());
    }

    // ========== render_ascii output dimension verification ==========

    #[test]
    fn render_ascii_output_lines_match_image_height_div_two() {
        // render_ascii steps y by 2, so a 4-row image produces 2 content lines.
        // We verify this by counting the lines it would produce (height / 2)
        // and that the character width per line equals the image width.
        let width: u32 = 10;
        let height: u32 = 4;
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([128, 128, 128]));
        let dynamic: image::DynamicImage = img.into();

        // The expected content lines (excluding surrounding blank lines):
        // height stepped by 2 = height / 2 iterations
        let expected_content_lines = height / 2;
        assert_eq!(
            expected_content_lines, 2,
            "sanity: 4-row image produces 2 ASCII content lines"
        );

        // Each content line has exactly `width` characters of ASCII art.
        // Verify ascii_char produces one char per pixel column.
        let rgba = dynamic.to_rgba8();
        for y in (0..height).step_by(2) {
            let mut char_count = 0u32;
            for x in 0..width {
                let pix = rgba.get_pixel(x, y);
                let intensity = pix[0] / 3 + pix[1] / 3 + pix[2] / 3;
                let ch = ascii_char(intensity);
                assert!(!ch.is_empty());
                char_count += 1;
            }
            assert_eq!(
                char_count, width,
                "each ASCII line must have exactly image_width characters"
            );
        }
    }

    #[test]
    fn render_ascii_odd_height_skips_last_row() {
        // With an odd height, the step_by(2) loop skips the final row.
        // A 5-row image produces lines for y=0,2,4 = 3 content lines.
        let height: u32 = 5;
        let expected_lines = (0..height).step_by(2).count();
        assert_eq!(expected_lines, 3, "5-row image stepped by 2 yields y=0,2,4");
    }
}
