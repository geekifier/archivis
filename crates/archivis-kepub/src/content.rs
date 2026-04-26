//! XHTML rewrite via streaming `quick-xml` events.
//!
//! Goals (P1 invariants):
//!
//! * Re-emit the XML declaration byte-for-byte.
//! * Preserve self-closing void elements.
//! * Keep attribute values XML-quoted.
//! * Preserve namespaces on the root element.
//! * Round-trip empty bodies (apart from injecting the kobo.js reference).
//!
//! The rewriter:
//!
//! 1. Detects already-converted input (presence of `class="koboSpan"`).
//!    Idempotent inputs skip span injection but still get re-emitted with
//!    a `kobo.js` script reference and run through the deterministic
//!    re-zip pipeline.
//! 2. Injects `<script src="…/kobo.js" type="text/javascript"/>` into the
//!    document's `<head>`.
//! 3. Wraps text runs inside block-level elements (`p, div, h1-h6, li,
//!    blockquote, td, dt, dd, caption, figcaption`) in
//!    `<span class="koboSpan" id="kobo.{para}.{seg}">…</span>`.

use std::borrow::Cow;
use std::io::Cursor;

use archivis_core::errors::FormatError;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

use crate::spans::{span_id, split_text};

const BLOCK_ELEMENTS: &[&[u8]] = &[
    b"p",
    b"div",
    b"h1",
    b"h2",
    b"h3",
    b"h4",
    b"h5",
    b"h6",
    b"li",
    b"blockquote",
    b"td",
    b"dt",
    b"dd",
    b"caption",
    b"figcaption",
];

fn is_block(local: &[u8]) -> bool {
    BLOCK_ELEMENTS
        .iter()
        .any(|b| eq_ignore_ascii_case(b, local))
}

fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.eq_ignore_ascii_case(y))
}

fn local_name(qname: &[u8]) -> &[u8] {
    qname
        .iter()
        .position(|c| *c == b':')
        .map_or(qname, |i| &qname[i + 1..])
}

/// Rewrite an XHTML spine document and return its new bytes.
///
/// `kobo_js_href` is the relative href to use for the injected
/// `<script src="…">` reference (e.g. `"../kobo.js"`).
///
/// Errors are returned for malformed XML so the caller can decide
/// whether to fall back to the unmodified source.
pub fn rewrite(source: &[u8], kobo_js_href: &str) -> Result<Vec<u8>, FormatError> {
    if is_already_kepub(source) {
        return rewrite_idempotent(source, kobo_js_href);
    }
    rewrite_full(source, kobo_js_href)
}

/// Cheap heuristic: KEPUB documents contain `class="koboSpan"`.
fn is_already_kepub(source: &[u8]) -> bool {
    // Accept either quoting style; case-sensitive class name matches kepubify.
    contains_subslice(source, b"class=\"koboSpan\"")
        || contains_subslice(source, b"class='koboSpan'")
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn rewrite_full(source: &[u8], kobo_js_href: &str) -> Result<Vec<u8>, FormatError> {
    let mut reader = Reader::from_reader(source);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(source.len() + 256)));

    let mut state = RewriteState::default();
    let mut buf = Vec::new();

    loop {
        let evt = reader
            .read_event_into(&mut buf)
            .map_err(|e| parse_err(&format!("xml read: {e}")))?;
        match evt {
            Event::Eof => break,
            Event::Start(start) => handle_start(&mut writer, &mut state, &start, kobo_js_href)?,
            Event::Empty(empty) => handle_empty(&mut writer, &mut state, &empty)?,
            Event::End(end) => handle_end(&mut writer, &mut state, &end)?,
            Event::Text(text) => handle_text(&mut writer, &mut state, &text)?,
            other => writer
                .write_event(other)
                .map_err(|e| parse_err(&format!("xml write: {e}")))?,
        }
        buf.clear();
    }

    Ok(writer.into_inner().into_inner())
}

/// Idempotent re-emission: only inject the kobo.js script reference if it
/// is not already present, leave everything else alone.
fn rewrite_idempotent(source: &[u8], kobo_js_href: &str) -> Result<Vec<u8>, FormatError> {
    let needle = format!("src=\"{kobo_js_href}\"");
    if contains_subslice(source, needle.as_bytes()) {
        return Ok(source.to_vec());
    }

    let mut reader = Reader::from_reader(source);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(source.len() + 128)));
    let mut buf = Vec::new();
    let mut head_handled = false;

    loop {
        let evt = reader
            .read_event_into(&mut buf)
            .map_err(|e| parse_err(&format!("xml read: {e}")))?;
        match evt {
            Event::Eof => break,
            Event::Start(start) => {
                writer
                    .write_event(Event::Start(start.clone()))
                    .map_err(|e| parse_err(&format!("xml write: {e}")))?;
                if !head_handled && eq_ignore_ascii_case(local_name(start.name().as_ref()), b"head")
                {
                    write_kobo_script(&mut writer, kobo_js_href)?;
                    head_handled = true;
                }
            }
            other => writer
                .write_event(other)
                .map_err(|e| parse_err(&format!("xml write: {e}")))?,
        }
        buf.clear();
    }

    Ok(writer.into_inner().into_inner())
}

#[derive(Default)]
struct RewriteState {
    /// Stack of currently-open elements (lowercased local names).
    open_stack: Vec<Vec<u8>>,
    /// Whether we are inside the document root and the `<head>` script
    /// reference has already been emitted.
    head_script_injected: bool,
    /// Tracks the innermost block (depth at which the current span lives).
    /// `None` when not inside a block.
    block_ctx: Option<BlockContext>,
    /// Stack of saved contexts for nested blocks (e.g. `<div><p>...</p></div>`).
    block_stack: Vec<BlockContext>,
    /// Running paragraph counter for the document (1-based).
    para: usize,
}

#[derive(Clone)]
struct BlockContext {
    /// Para id for this block.
    para: usize,
    /// Next segment id to allocate.
    next_seg: usize,
    /// Stack depth at which this block opened.
    open_at_depth: usize,
    /// Whether a span is currently open and awaiting close.
    span_open: bool,
}

fn handle_start<W: std::io::Write>(
    writer: &mut Writer<W>,
    state: &mut RewriteState,
    start: &BytesStart<'_>,
    kobo_js_href: &str,
) -> Result<(), FormatError> {
    let local: Vec<u8> = local_name(start.name().as_ref())
        .iter()
        .map(u8::to_ascii_lowercase)
        .collect();

    // Close any open span if we're entering a nested block (text run ends).
    if let Some(ctx) = state.block_ctx.as_mut() {
        if is_block(&local) && ctx.span_open {
            write_close_span(writer)?;
            ctx.span_open = false;
        }
    }

    state.open_stack.push(local.clone());
    writer
        .write_event(Event::Start(start.clone()))
        .map_err(|e| parse_err(&format!("xml write: {e}")))?;

    // Inject <script> reference once after entering <head>.
    if !state.head_script_injected && local == b"head" {
        write_kobo_script(writer, kobo_js_href)?;
        state.head_script_injected = true;
    }

    if is_block(&local) {
        // Push existing context onto stack, start a new one.
        if let Some(prev) = state.block_ctx.take() {
            state.block_stack.push(prev);
        }
        state.para += 1;
        state.block_ctx = Some(BlockContext {
            para: state.para,
            next_seg: 1,
            open_at_depth: state.open_stack.len(),
            span_open: false,
        });
    }

    Ok(())
}

fn handle_empty<W: std::io::Write>(
    writer: &mut Writer<W>,
    state: &mut RewriteState,
    empty: &BytesStart<'_>,
) -> Result<(), FormatError> {
    let local: Vec<u8> = local_name(empty.name().as_ref())
        .iter()
        .map(u8::to_ascii_lowercase)
        .collect();
    // An empty/self-closing block tag still bumps the paragraph counter
    // for parity with non-empty blocks, but emits no span.
    if is_block(&local) {
        if let Some(ctx) = state.block_ctx.as_mut() {
            if ctx.span_open {
                write_close_span(writer)?;
                ctx.span_open = false;
            }
        }
        state.para += 1;
    }
    writer
        .write_event(Event::Empty(empty.clone()))
        .map_err(|e| parse_err(&format!("xml write: {e}")))?;
    Ok(())
}

fn handle_end<W: std::io::Write>(
    writer: &mut Writer<W>,
    state: &mut RewriteState,
    end: &BytesEnd<'_>,
) -> Result<(), FormatError> {
    let local: Vec<u8> = local_name(end.name().as_ref())
        .iter()
        .map(u8::to_ascii_lowercase)
        .collect();

    // Close the active span before the matching block end tag.
    if let Some(ctx) = state.block_ctx.as_mut() {
        if ctx.open_at_depth == state.open_stack.len() && ctx.span_open {
            write_close_span(writer)?;
            ctx.span_open = false;
        }
    }

    state.open_stack.pop();
    writer
        .write_event(Event::End(end.clone()))
        .map_err(|e| parse_err(&format!("xml write: {e}")))?;

    // Pop block context when we exit it.
    if let Some(ctx) = state.block_ctx.as_ref() {
        if ctx.open_at_depth > state.open_stack.len() {
            // Restore the parent block context (if any).
            state.block_ctx = state.block_stack.pop();
        }
    }

    let _ = local;
    Ok(())
}

fn handle_text<W: std::io::Write>(
    writer: &mut Writer<W>,
    state: &mut RewriteState,
    text: &BytesText<'_>,
) -> Result<(), FormatError> {
    let Some(ctx) = state.block_ctx.as_mut() else {
        return writer
            .write_event(Event::Text(text.clone()))
            .map_err(|e| parse_err(&format!("xml write: {e}")));
    };

    // Decode unescaped content for the segmenter, but emit raw escaped
    // text so entities round-trip unchanged.
    let raw = text.as_ref();
    let raw_str =
        std::str::from_utf8(raw).map_err(|e| parse_err(&format!("non-utf8 text node: {e}")))?;

    if raw_str.trim().is_empty() {
        // Whitespace-only nodes do not start a span.
        return writer
            .write_event(Event::Text(text.clone()))
            .map_err(|e| parse_err(&format!("xml write: {e}")));
    }

    let segments = split_text(raw_str);
    if segments.is_empty() {
        return writer
            .write_event(Event::Text(text.clone()))
            .map_err(|e| parse_err(&format!("xml write: {e}")));
    }

    let last = segments.len() - 1;
    for (i, seg) in segments.iter().enumerate() {
        if !ctx.span_open {
            write_open_span(writer, ctx.para, ctx.next_seg)?;
            ctx.span_open = true;
            ctx.next_seg += 1;
        }
        // BytesText preserves the raw escape state; we slice the original
        // bytes via the byte indices reported by `split_text`.
        let bytes = seg.text.as_bytes();
        let chunk = BytesText::from_escaped(Cow::Borrowed(
            std::str::from_utf8(bytes).expect("utf-8 slice from utf-8 source"),
        ));
        writer
            .write_event(Event::Text(chunk))
            .map_err(|e| parse_err(&format!("xml write: {e}")))?;

        if i != last {
            write_close_span(writer)?;
            ctx.span_open = false;
        }
    }
    Ok(())
}

fn write_open_span<W: std::io::Write>(
    writer: &mut Writer<W>,
    para: usize,
    seg: usize,
) -> Result<(), FormatError> {
    let id = span_id(para, seg);
    let mut start = BytesStart::new("span");
    start.push_attribute(("class", "koboSpan"));
    start.push_attribute(("id", id.as_str()));
    writer
        .write_event(Event::Start(start))
        .map_err(|e| parse_err(&format!("xml write: {e}")))
}

fn write_close_span<W: std::io::Write>(writer: &mut Writer<W>) -> Result<(), FormatError> {
    writer
        .write_event(Event::End(BytesEnd::new("span")))
        .map_err(|e| parse_err(&format!("xml write: {e}")))
}

fn write_kobo_script<W: std::io::Write>(
    writer: &mut Writer<W>,
    href: &str,
) -> Result<(), FormatError> {
    let mut tag = BytesStart::new("script");
    tag.push_attribute(("type", "text/javascript"));
    tag.push_attribute(("src", href));
    writer
        .write_event(Event::Empty(tag))
        .map_err(|e| parse_err(&format!("xml write: {e}")))
}

fn parse_err(msg: &str) -> FormatError {
    FormatError::Parse {
        format: "KEPUB".into(),
        message: msg.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite_str(input: &str) -> String {
        let bytes = rewrite(input.as_bytes(), "kobo.js").expect("rewrite ok");
        String::from_utf8(bytes).expect("utf-8 output")
    }

    #[test]
    fn idempotent_when_kobospan_present() {
        let input = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title><script src="kobo.js" type="text/javascript"/></head><body><p><span class="koboSpan" id="kobo.1.1">Hi.</span></p></body></html>"#;
        let out = rewrite_str(input);
        // No double wrap of the koboSpan.
        let span_count = out.matches("class=\"koboSpan\"").count();
        assert_eq!(span_count, 1);
    }

    #[test]
    fn injects_kobo_script_into_head() {
        let input = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body></body></html>"#;
        let out = rewrite_str(input);
        assert!(
            out.contains("kobo.js"),
            "kobo.js script not injected: {out}"
        );
    }

    #[test]
    fn wraps_text_in_block() {
        let input = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body><p>Hello.</p></body></html>"#;
        let out = rewrite_str(input);
        assert!(out.contains(r#"<span class="koboSpan" id="kobo.1.1">Hello."#));
    }

    #[test]
    fn empty_body_only_injects_script() {
        let input = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body></body></html>"#;
        let out = rewrite_str(input);
        assert!(!out.contains("koboSpan"));
    }

    #[test]
    fn multiple_sentences_split_into_segments() {
        let input = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body><p>One. Two. Three.</p></body></html>"#;
        let out = rewrite_str(input);
        let count = out.matches("class=\"koboSpan\"").count();
        assert_eq!(count, 3, "expected 3 koboSpan segments, got {count}: {out}");
    }
}
