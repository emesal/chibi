use std::io::{self, IsTerminal, Write};

use base64::Engine;
use streamdown_parser::{ParseEvent, Parser};
use streamdown_render::Renderer;

/// Configuration for markdown stream rendering.
pub struct MarkdownConfig {
    pub render_markdown: bool,
    pub render_images: bool,
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
/// for local files are rendered inline using truecolor ANSI escape codes.
pub struct MarkdownStream {
    line_buffer: String,
    pipeline: Option<RenderPipeline>,
    render_images: bool,
    terminal_width: usize,
}

impl MarkdownStream {
    /// Creates a new markdown stream.
    ///
    /// When `render_markdown` is true and stdout is a TTY, creates a streamdown
    /// pipeline for formatted output. Otherwise, raw passthrough.
    pub fn new(config: MarkdownConfig) -> Self {
        let (pipeline, terminal_width) = if config.render_markdown && io::stdout().is_terminal() {
            let width = streamdown_render::terminal_width();
            (
                Some(RenderPipeline {
                    parser: Parser::new(),
                    renderer: Renderer::new(StdoutWriter, width),
                }),
                width,
            )
        } else {
            (None, 80)
        };

        MarkdownStream {
            line_buffer: String::new(),
            pipeline,
            render_images: config.render_images,
            terminal_width,
        }
    }

    /// Render a list of parse events, intercepting image events when enabled.
    fn render_events(
        pipeline: &mut RenderPipeline,
        events: &[ParseEvent],
        render_images: bool,
        terminal_width: usize,
    ) -> io::Result<()> {
        for event in events {
            if render_images
                && let ParseEvent::Image { alt, url } = event
                && try_render_image(url, alt, terminal_width).is_ok()
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
            Self::render_events(pipeline, &events, self.render_images, self.terminal_width)?;
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
            Self::render_events(pipeline, &events, self.render_images, self.terminal_width)?;
        }

        // Finalize parser (close any open blocks)
        let events = pipeline.parser.finalize();
        Self::render_events(pipeline, &events, self.render_images, self.terminal_width)?;

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

/// Attempt to render an image inline using truecolor ANSI output.
///
/// Supports local file paths, `file://` URLs, and `data:image/...;base64,...`
/// URIs. Returns `Ok(())` if the image was successfully rendered, or an error
/// if the image could not be loaded.
fn try_render_image(url: &str, alt: &str, term_width: usize) -> io::Result<()> {
    let img = if let Some(rest) = url.strip_prefix("data:") {
        decode_data_uri_image(rest)?
    } else if url.starts_with("http://") || url.starts_with("https://") {
        return Err(io::Error::other("remote URLs not supported"));
    } else {
        let path = url.strip_prefix("file://").unwrap_or(url);
        image::open(path)
            .map_err(|e| io::Error::other(format!("Failed to open image {}: {}", path, e)))?
    };

    let (orig_w, orig_h) = (img.width(), img.height());
    let term_w = term_width as u32;
    // image_resized_size expects (image_size, term_size, preserve_aspect)
    // term_size height: use a large value so width is the binding constraint
    let (new_w, mut new_h) =
        imgcatr::ops::image_resized_size((orig_w, orig_h), (term_w, 1000), true);

    // Ensure height is even (truecolor renderer prints pixel pairs)
    if new_h % 2 != 0 {
        new_h += 1;
    }

    let resized = imgcatr::ops::resize_image(&img, (new_w, new_h));

    let mut stdout = io::stdout().lock();
    // Spacing before image
    writeln!(stdout)?;
    imgcatr::ops::write_ansi_truecolor(&mut stdout, &resized);
    // Alt text below image (dimmed) if non-empty
    if !alt.is_empty() {
        write!(stdout, "\x1b[2m{}\x1b[0m", alt)?;
    }
    writeln!(stdout)?;
    stdout.flush()?;

    Ok(())
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
}
