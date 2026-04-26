//! `koboSpan` segmenter.
//!
//! Walks XHTML at the byte level and produces a stream of segments that
//! the [`crate::content`] rewriter wraps into `<span class="koboSpan"
//! id="kobo.{para}.{seg}">…</span>` tags. The segmenter operates on
//! contiguous text runs inside block-level contexts; the rewriter
//! decides which contexts are eligible.
//!
//! V1 simplification: a sentence boundary inside a text node splits the
//! run; otherwise the entire text run is wrapped as a single span.
//! Inline elements and entity references are preserved without
//! modification.

/// Result of segmenting one block element's text content.
///
/// Each segment corresponds to a `<span class="koboSpan">` to emit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment<'a> {
    /// Index within the block (1-based).
    pub seg_index: usize,
    /// Text content for this segment, with leading/trailing whitespace
    /// preserved.
    pub text: &'a str,
}

/// Split a block-level text run into sentence-bounded segments.
///
/// Returns `Vec<Segment>` because a single text run can span multiple
/// sentences. The rewriter pairs the segments with the surrounding
/// inline structure and emits one span per segment.
#[must_use]
pub fn split_text(text: &str) -> Vec<Segment<'_>> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start: usize = 0;
    let mut seg = 1usize;

    let mut i = 0;
    while i < bytes.len() {
        // Find next char boundary; we're scanning by byte but only treat
        // ASCII terminators as sentence boundaries. Non-ASCII RTL/CJK
        // terminators (U+06D4, U+3002) are left untouched in v1.
        let b = bytes[i];
        if matches!(b, b'.' | b'!' | b'?') {
            // Walk forward over closing quotes/parens that should attach
            // to the same segment.
            let mut end = i + 1;
            while end < bytes.len() {
                let nb = bytes[end];
                if matches!(nb, b'"' | b'\'' | b')' | b']' | b'}') {
                    end += 1;
                } else {
                    break;
                }
            }
            // Followed by whitespace? If so, treat as a boundary.
            if end < bytes.len() && (bytes[end] as char).is_whitespace() {
                let chunk = &text[start..end];
                out.push(Segment {
                    seg_index: seg,
                    text: chunk,
                });
                seg += 1;
                start = end;
                i = end;
                continue;
            }
            i = end;
            continue;
        }
        i += 1;
    }
    if start < bytes.len() {
        out.push(Segment {
            seg_index: seg,
            text: &text[start..],
        });
    }
    out
}

/// Format the koboSpan id for a given paragraph/segment pair.
#[must_use]
pub fn span_id(para: usize, seg: usize) -> String {
    format!("kobo.{para}.{seg}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_no_segments() {
        assert!(split_text("").is_empty());
    }

    #[test]
    fn single_sentence_is_one_segment() {
        let segs = split_text("Hello world.");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "Hello world.");
        assert_eq!(segs[0].seg_index, 1);
    }

    #[test]
    fn multiple_sentences_split() {
        let segs = split_text("First. Second! Third?");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].text, "First.");
        assert_eq!(segs[1].text, " Second!");
        assert_eq!(segs[2].text, " Third?");
    }

    #[test]
    fn closing_quote_attaches_to_segment() {
        let segs = split_text(r#"He said "hello." Then left."#);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, r#"He said "hello.""#);
    }

    #[test]
    fn no_terminator_yields_one_segment() {
        let segs = split_text("trailing text");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].text, "trailing text");
    }

    #[test]
    fn period_without_following_space_is_not_boundary() {
        let segs = split_text("e.g. inline");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "e.g.");
        assert_eq!(segs[1].text, " inline");
    }

    #[test]
    fn span_id_format() {
        assert_eq!(span_id(1, 2), "kobo.1.2");
        assert_eq!(span_id(42, 3), "kobo.42.3");
    }
}
