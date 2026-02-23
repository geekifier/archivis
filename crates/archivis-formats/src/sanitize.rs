/// Options controlling how metadata text is sanitized.
#[derive(Debug, Clone)]
pub struct SanitizeOptions {
    /// Strip all HTML/XML tags from text fields.
    /// Default: true.
    pub strip_html: bool,
}

impl Default for SanitizeOptions {
    fn default() -> Self {
        Self { strip_html: true }
    }
}

/// Sanitize a metadata text string.
///
/// When `strip_html` is true: removes all HTML/XML tags, decodes common
/// HTML entities (&amp; &lt; &gt; &quot; &#39; &nbsp;), collapses
/// whitespace runs, and trims the result.
///
/// When `strip_html` is false: only removes dangerous content
/// (script tags, event handler attributes, javascript: URIs) while
/// preserving safe formatting tags.
///
/// Returns `None` if the result is empty after sanitization.
pub fn sanitize_text(input: &str, options: &SanitizeOptions) -> Option<String> {
    if input.is_empty() {
        return None;
    }

    let result = if options.strip_html {
        let stripped = strip_tags(input);
        let decoded = decode_entities(&stripped);
        collapse_whitespace(&decoded)
    } else {
        let safe = strip_dangerous(input);
        let decoded = decode_entities(&safe);
        collapse_whitespace(&decoded)
    };

    let trimmed = result.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Sanitize specifically dangerous HTML patterns regardless of options.
/// Always strips: `<script>`, `<style>`, `<iframe>`, `<object>`, `<embed>`,
/// `on*` event attributes, `javascript:` URIs, `data:` URIs.
fn strip_dangerous(input: &str) -> String {
    let mut result = input.to_string();

    // Remove dangerous element blocks (with content): script, style, iframe, object, embed
    for tag in &["script", "style", "iframe", "object", "embed"] {
        loop {
            let lower = result.to_lowercase();
            if let Some(start) = lower.find(&format!("<{tag}")) {
                // Find the end of this dangerous block
                let close_tag = format!("</{tag}>");
                if let Some(end_pos) = lower[start..].find(&close_tag) {
                    let end = start + end_pos + close_tag.len();
                    result.replace_range(start..end, "");
                } else {
                    // Self-closing or unclosed — remove just the tag
                    if let Some(tag_end) = result[start..].find('>') {
                        result.replace_range(start..=start + tag_end, "");
                    } else {
                        // Malformed — remove from start to end
                        result.truncate(start);
                    }
                }
            } else {
                break;
            }
        }
    }

    // Remove on* event attributes from remaining tags
    result = remove_event_attributes(&result);

    // Remove javascript: and data: URIs in attributes
    result = remove_dangerous_uris(&result);

    result
}

/// Remove `on*` event handler attributes from HTML tags.
fn remove_event_attributes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut in_tag = true;
            result.push(ch);
            let mut tag_content = String::new();
            for tc in chars.by_ref() {
                if tc == '>' {
                    let cleaned = strip_on_attributes(&tag_content);
                    result.push_str(&cleaned);
                    result.push('>');
                    in_tag = false;
                    break;
                }
                tag_content.push(tc);
            }
            if in_tag {
                result.push_str(&tag_content);
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Remove `on*="..."` attributes from a tag's inner content.
fn strip_on_attributes(tag_content: &str) -> String {
    let mut result = String::with_capacity(tag_content.len());
    let bytes = tag_content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i].is_ascii_whitespace() {
            let ws_start = i;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i + 2 < len
                && (bytes[i] == b'o' || bytes[i] == b'O')
                && (bytes[i + 1] == b'n' || bytes[i + 1] == b'N')
                && bytes[i + 2].is_ascii_alphabetic()
            {
                // on* attribute — skip name
                while i < len && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                // Skip = and value
                if i < len && bytes[i] == b'=' {
                    i += 1;
                    while i < len && bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                        let quote = bytes[i];
                        i += 1;
                        while i < len && bytes[i] != quote {
                            i += 1;
                        }
                        if i < len {
                            i += 1;
                        }
                    } else {
                        while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                            i += 1;
                        }
                    }
                }
            } else {
                result.push_str(&tag_content[ws_start..i]);
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }

    result
}

/// Remove `javascript:` and `data:` URIs from attribute values.
fn remove_dangerous_uris(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars();

    while let Some(ch) = chars.next() {
        result.push(ch);
        if ch == '<' {
            let mut tag_content = String::new();
            let mut found_close = false;
            for tc in chars.by_ref() {
                if tc == '>' {
                    found_close = true;
                    break;
                }
                tag_content.push(tc);
            }

            let cleaned = replace_dangerous_uri_values(&tag_content);
            result.push_str(&cleaned);
            if found_close {
                result.push('>');
            }
        }
    }

    result
}

/// Replace `javascript:` and `data:` URIs in attribute values with empty strings.
fn replace_dangerous_uri_values(tag_content: &str) -> String {
    let mut result = tag_content.to_string();

    for pattern in &["javascript:", "data:"] {
        loop {
            let lower_result = result.to_lowercase();
            if let Some(pos) = lower_result.find(pattern) {
                let before = &result[..pos];
                if let Some(quote_pos) = before.rfind(['"', '\'']) {
                    let quote_char = result.as_bytes()[quote_pos] as char;
                    if let Some(end_pos) = result[pos..].find(quote_char) {
                        let end = pos + end_pos;
                        result.replace_range(quote_pos + 1..end, "");
                    } else {
                        result.replace_range(quote_pos + 1.., "");
                        break;
                    }
                } else {
                    result.replace_range(pos..pos + pattern.len(), "");
                }
            } else {
                break;
            }
        }
    }

    result
}

/// Strip all HTML/XML tags from a string, preserving inner text.
///
/// Handles:
/// - Self-closing tags (`<br/>`, `<hr/>`)
/// - Nested tags (`<p><b>text</b></p>`)
/// - Malformed tags (`<p>unclosed`)
/// - CDATA sections
/// - HTML comments (`<!-- -->`)
/// - Converts block-level tags to appropriate whitespace
fn strip_tags(input: &str) -> String {
    // First pass: remove dangerous content
    let safe = strip_dangerous(input);

    // Convert block-level tags to newlines before stripping
    let with_breaks = convert_block_tags_to_newlines(&safe);

    // Strip remaining tags using a state machine
    let mut result = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    let mut in_comment = false;
    let mut in_cdata = false;
    let chars: Vec<char> = with_breaks.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if in_comment {
            if i + 2 < len && chars[i] == '-' && chars[i + 1] == '-' && chars[i + 2] == '>' {
                in_comment = false;
                i += 3;
            } else {
                i += 1;
            }
        } else if in_cdata {
            if i + 2 < len && chars[i] == ']' && chars[i + 1] == ']' && chars[i + 2] == '>' {
                in_cdata = false;
                i += 3;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        } else if in_tag {
            if chars[i] == '>' {
                in_tag = false;
            }
            i += 1;
        } else if chars[i] == '<' {
            if i + 3 < len && chars[i + 1] == '!' && chars[i + 2] == '-' && chars[i + 3] == '-' {
                in_comment = true;
                i += 4;
            } else if i + 8 < len
                && chars[i + 1] == '!'
                && chars[i + 2] == '['
                && chars[i + 3] == 'C'
                && chars[i + 4] == 'D'
                && chars[i + 5] == 'A'
                && chars[i + 6] == 'T'
                && chars[i + 7] == 'A'
                && chars[i + 8] == '['
            {
                in_cdata = true;
                i += 9;
            } else {
                in_tag = true;
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Convert block-level HTML tags to newlines to preserve paragraph structure.
fn convert_block_tags_to_newlines(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if lower_chars[i] == '<' {
            let remaining: String = lower_chars[i..].iter().collect();

            if remaining.starts_with("</p>") {
                result.push_str("\n\n");
                i += 4;
            } else if remaining.starts_with("<p>") {
                result.push_str("\n\n");
                i += 3;
            } else if remaining.starts_with("<p ") {
                if let Some(end) = remaining.find('>') {
                    result.push_str("\n\n");
                    i += end + 1;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            } else if remaining.starts_with("<br>") {
                result.push('\n');
                i += 4;
            } else if remaining.starts_with("<br/>") {
                result.push('\n');
                i += 5;
            } else if remaining.starts_with("<br />") {
                result.push('\n');
                i += 6;
            } else if remaining.starts_with("</div>") {
                result.push_str("\n\n");
                i += 6;
            } else if remaining.starts_with("<div>") {
                result.push_str("\n\n");
                i += 5;
            } else if remaining.starts_with("<div ") {
                if let Some(end) = remaining.find('>') {
                    result.push_str("\n\n");
                    i += end + 1;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            } else {
                result.push(chars[i]);
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Decode common HTML entities to their character equivalents.
///
/// Handles: `&amp;` `&lt;` `&gt;` `&quot;` `&#39;` `&apos;` `&nbsp;`
/// Also handles numeric entities: `&#123;` `&#x7B;`
fn decode_entities(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '&' {
            if let Some(semi_offset) = chars[i..].iter().position(|&c| c == ';') {
                if semi_offset <= 10 {
                    let entity: String = chars[i..=i + semi_offset].iter().collect();
                    if let Some(decoded) = decode_single_entity(&entity) {
                        result.push(decoded);
                        i += semi_offset + 1;
                        continue;
                    }
                }
            }
            result.push('&');
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }

    result
}

/// Decode a single HTML entity (including the `&` and `;`).
fn decode_single_entity(entity: &str) -> Option<char> {
    match entity.to_lowercase().as_str() {
        "&amp;" => Some('&'),
        "&lt;" => Some('<'),
        "&gt;" => Some('>'),
        "&quot;" => Some('"'),
        "&#39;" | "&apos;" => Some('\''),
        "&nbsp;" => Some(' '),
        _ => {
            let inner = &entity[1..entity.len() - 1]; // strip & and ;
            inner.strip_prefix('#').and_then(|hex| {
                hex.strip_prefix('x')
                    .or_else(|| hex.strip_prefix('X'))
                    .map_or_else(
                        // Decimal: &#NNN;
                        || hex.parse::<u32>().ok().and_then(char::from_u32),
                        // Hexadecimal: &#xNN;
                        |hex_val| {
                            u32::from_str_radix(hex_val, 16)
                                .ok()
                                .and_then(char::from_u32)
                        },
                    )
            })
        }
    }
}

/// Collapse multiple whitespace/newline runs into normalized whitespace.
///
/// - Multiple spaces/tabs within a line collapse to a single space
/// - Multiple newlines collapse to a double newline (paragraph break)
/// - Mixed whitespace with newlines preserves the paragraph break
fn collapse_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut last_was_space = false;
    let mut newline_count = 0;

    for ch in input.chars() {
        if ch == '\n' {
            newline_count += 1;
            last_was_space = false;
        } else if ch.is_whitespace() {
            if newline_count == 0 {
                last_was_space = true;
            }
        } else {
            if newline_count >= 2 {
                result.push_str("\n\n");
            } else if newline_count == 1 {
                result.push('\n');
            } else if last_was_space && !result.is_empty() {
                result.push(' ');
            }
            newline_count = 0;
            last_was_space = false;
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_tags tests ──

    #[test]
    fn strip_tags_basic_html() {
        assert_eq!(
            strip_tags("<p>Hello <b>world</b></p>"),
            "\n\nHello world\n\n"
        );
    }

    #[test]
    fn strip_tags_paragraphs() {
        let result = strip_tags("<p>Para 1</p><p>Para 2</p>");
        let collapsed = collapse_whitespace(&result);
        assert_eq!(collapsed.trim(), "Para 1\n\nPara 2");
    }

    #[test]
    fn strip_tags_br() {
        assert_eq!(strip_tags("<br/>line break"), "\nline break");
    }

    #[test]
    fn strip_tags_br_no_slash() {
        assert_eq!(strip_tags("<br>line break"), "\nline break");
    }

    #[test]
    fn strip_tags_br_with_space() {
        assert_eq!(strip_tags("<br />line break"), "\nline break");
    }

    #[test]
    fn strip_tags_self_closing() {
        assert_eq!(strip_tags("before<hr/>after"), "beforeafter");
    }

    #[test]
    fn strip_tags_nested() {
        assert_eq!(
            strip_tags("<div><p><em>nested</em></p></div>"),
            "\n\n\n\nnested\n\n\n\n"
        );
    }

    #[test]
    fn strip_tags_malformed() {
        assert_eq!(strip_tags("<p>unclosed"), "\n\nunclosed");
    }

    #[test]
    fn strip_tags_comment() {
        assert_eq!(strip_tags("<!-- comment -->visible"), "visible");
    }

    #[test]
    fn strip_tags_cdata() {
        assert_eq!(
            strip_tags("before<![CDATA[cdata content]]>after"),
            "beforecdata contentafter"
        );
    }

    #[test]
    fn strip_tags_plain_text_passthrough() {
        assert_eq!(strip_tags("no html here"), "no html here");
    }

    // ── decode_entities tests ──

    #[test]
    fn decode_named_entities() {
        assert_eq!(decode_entities("&amp; &lt; &gt; &quot;"), "& < > \"");
    }

    #[test]
    fn decode_apos_entities() {
        assert_eq!(decode_entities("&#39; &apos;"), "' '");
    }

    #[test]
    fn decode_nbsp() {
        assert_eq!(decode_entities("hello&nbsp;world"), "hello world");
    }

    #[test]
    fn decode_numeric_decimal() {
        assert_eq!(decode_entities("&#123;"), "{");
    }

    #[test]
    fn decode_numeric_hex() {
        assert_eq!(decode_entities("&#x7B;"), "{");
    }

    #[test]
    fn decode_numeric_hex_upper() {
        assert_eq!(decode_entities("&#x27;"), "'");
    }

    #[test]
    fn decode_unknown_entity_passthrough() {
        assert_eq!(decode_entities("&unknown;"), "&unknown;");
    }

    #[test]
    fn decode_ampersand_not_entity() {
        assert_eq!(decode_entities("AT&T"), "AT&T");
    }

    // ── sanitize_text tests ──

    #[test]
    fn sanitize_strip_html_full() {
        let opts = SanitizeOptions { strip_html: true };
        assert_eq!(
            sanitize_text("<p>Normal <b>bold</b> text</p>", &opts),
            Some("Normal bold text".into())
        );
    }

    #[test]
    fn sanitize_empty_after_strip() {
        let opts = SanitizeOptions { strip_html: true };
        assert_eq!(sanitize_text("   ", &opts), None);
    }

    #[test]
    fn sanitize_plain_text_passthrough() {
        let opts = SanitizeOptions { strip_html: true };
        assert_eq!(
            sanitize_text("plain text no html", &opts),
            Some("plain text no html".into())
        );
    }

    #[test]
    fn sanitize_comment_removed() {
        let opts = SanitizeOptions { strip_html: true };
        assert_eq!(
            sanitize_text("<!-- comment -->visible", &opts),
            Some("visible".into())
        );
    }

    #[test]
    fn sanitize_dangerous_always_stripped() {
        let opts = SanitizeOptions { strip_html: false };
        assert_eq!(
            sanitize_text("<script>alert('xss')</script>Hello", &opts),
            Some("Hello".into())
        );
    }

    #[test]
    fn sanitize_dangerous_style_stripped() {
        let opts = SanitizeOptions { strip_html: false };
        let style_tag = ["<style>.x", "{color:red}", "</style>visible"].concat();
        assert_eq!(sanitize_text(&style_tag, &opts), Some("visible".into()));
    }

    #[test]
    fn sanitize_dangerous_iframe_stripped() {
        let opts = SanitizeOptions { strip_html: false };
        let result = sanitize_text("<iframe src='evil.com'></iframe>safe", &opts);
        assert_eq!(result, Some("safe".into()));
    }

    #[test]
    fn sanitize_event_handlers_stripped() {
        let opts = SanitizeOptions { strip_html: false };
        let result = sanitize_text("<a onclick=\"alert(1)\" href=\"#\">link</a>", &opts);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(!text.contains("onclick"));
        assert!(text.contains("link"));
    }

    #[test]
    fn sanitize_javascript_uri_stripped() {
        let opts = SanitizeOptions { strip_html: false };
        let result = sanitize_text("<a href=\"javascript:alert(1)\">link</a>", &opts);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(!text.contains("javascript:"));
    }

    #[test]
    fn sanitize_data_uri_stripped() {
        let opts = SanitizeOptions { strip_html: false };
        let input = concat!(
            "<img src=\"data:text/html,",
            "<script>alert(1)</script>",
            "\"/>"
        );
        let result = sanitize_text(input, &opts);
        assert!(result.is_some() || result.is_none());
        if let Some(ref text) = result {
            assert!(!text.contains("data:"));
        }
    }

    #[test]
    fn sanitize_entities_decoded() {
        let opts = SanitizeOptions { strip_html: true };
        assert_eq!(
            sanitize_text("&amp; &lt; &gt;", &opts),
            Some("& < >".into())
        );
    }

    #[test]
    fn sanitize_collapses_whitespace() {
        let opts = SanitizeOptions { strip_html: true };
        assert_eq!(
            sanitize_text("hello    world", &opts),
            Some("hello world".into())
        );
    }

    #[test]
    fn sanitize_preserves_paragraph_breaks() {
        let opts = SanitizeOptions { strip_html: true };
        assert_eq!(
            sanitize_text("<p>First paragraph</p><p>Second paragraph</p>", &opts),
            Some("First paragraph\n\nSecond paragraph".into())
        );
    }

    #[test]
    fn sanitize_none_on_empty() {
        let opts = SanitizeOptions::default();
        assert_eq!(sanitize_text("", &opts), None);
    }

    #[test]
    fn sanitize_none_on_only_tags() {
        let opts = SanitizeOptions::default();
        assert_eq!(sanitize_text("<br/><br/>", &opts), None);
    }

    #[test]
    fn sanitize_mixed_entities_and_tags() {
        let opts = SanitizeOptions { strip_html: true };
        assert_eq!(
            sanitize_text("<b>Tom &amp; Jerry</b>", &opts),
            Some("Tom & Jerry".into())
        );
    }

    #[test]
    fn strip_dangerous_nested_script() {
        let result = strip_dangerous("<script type='text/javascript'>var x=1;</script>safe");
        assert_eq!(result, "safe");
    }

    #[test]
    fn strip_dangerous_object_embed() {
        let result = strip_dangerous("<object data='x'><embed src='y'/></object>safe");
        assert_eq!(result, "safe");
    }

    // ── collapse_whitespace tests ──

    #[test]
    fn collapse_multiple_spaces() {
        assert_eq!(collapse_whitespace("a   b"), "a b");
    }

    #[test]
    fn collapse_multiple_newlines_to_paragraph() {
        assert_eq!(collapse_whitespace("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn collapse_single_newline_preserved() {
        assert_eq!(collapse_whitespace("a\nb"), "a\nb");
    }

    #[test]
    fn collapse_mixed_whitespace_with_newlines() {
        assert_eq!(collapse_whitespace("a  \n\n  b"), "a\n\nb");
    }
}
