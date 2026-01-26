use std::io::{self, IsTerminal, Write};

use streamdown_parser::Parser;
use streamdown_render::Renderer;

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
pub struct MarkdownStream {
    line_buffer: String,
    pipeline: Option<RenderPipeline>,
}

impl MarkdownStream {
    /// Creates a new markdown stream.
    ///
    /// When `render` is true and stdout is a TTY, creates a streamdown
    /// pipeline for formatted output. Otherwise, raw passthrough.
    pub fn new(render: bool) -> Self {
        let pipeline = if render && io::stdout().is_terminal() {
            let width = streamdown_render::terminal_width();
            Some(RenderPipeline {
                parser: Parser::new(),
                renderer: Renderer::new(StdoutWriter, width),
            })
        } else {
            None
        };

        MarkdownStream {
            line_buffer: String::new(),
            pipeline,
        }
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
            for event in &events {
                pipeline
                    .renderer
                    .render_event(event)
                    .map_err(|e| io::Error::other(format!("Render error: {}", e)))?;
            }
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
            for event in &events {
                pipeline
                    .renderer
                    .render_event(event)
                    .map_err(|e| io::Error::other(format!("Render error: {}", e)))?;
            }
        }

        // Finalize parser (close any open blocks)
        let events = pipeline.parser.finalize();
        for event in &events {
            pipeline
                .renderer
                .render_event(event)
                .map_err(|e| io::Error::other(format!("Render error: {}", e)))?;
        }

        Ok(())
    }
}
