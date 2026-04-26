//! OPF rewrite: register `kobo.js` in the manifest.
//!
//! Uses a streaming `quick-xml` event walk so the XML declaration and
//! attribute quoting style round-trip safely. The manifest entry is
//! inserted just before the `</manifest>` end tag.

use std::borrow::Cow;
use std::io::Cursor;

use archivis_core::errors::FormatError;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

/// Spine document descriptor returned from [`parse_spine`].
#[derive(Debug, Clone)]
pub struct SpineDoc {
    /// Path of the spine document inside the EPUB ZIP.
    pub path: String,
    /// Media type (defaulting to `application/xhtml+xml` when missing).
    pub media_type: String,
}

/// Parse the OPF and return:
/// * the OPF version (e.g. `"3.0"`),
/// * the rendition layout if declared (`"pre-paginated"` for fixed-layout),
/// * the list of spine documents in order.
#[allow(clippy::missing_panics_doc, clippy::too_many_lines)]
pub fn parse_spine(opf: &[u8], opf_dir: &str) -> Result<ParsedSpine, FormatError> {
    let mut reader = Reader::from_reader(opf);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut buf = Vec::new();
    let mut version: Option<String> = None;
    let mut layout: Option<String> = None;
    // manifest: id → (href, media-type)
    let mut manifest: Vec<(String, String, String)> = Vec::new();
    let mut spine_idrefs: Vec<String> = Vec::new();
    let mut in_layout_meta = false;

    loop {
        let evt = reader
            .read_event_into(&mut buf)
            .map_err(|e| parse_err(&format!("opf read: {e}")))?;
        match evt {
            Event::Eof => break,
            Event::End(ref end) if local_name(end.name().as_ref()) == b"meta" => {
                in_layout_meta = false;
            }
            Event::Text(ref text) if in_layout_meta && layout.is_none() => {
                let s = String::from_utf8_lossy(text.as_ref()).trim().to_string();
                if !s.is_empty() {
                    layout = Some(s);
                }
            }
            Event::Start(ref e) | Event::Empty(ref e) => {
                let local = local_name(e.name().as_ref()).to_vec();
                let is_empty = matches!(evt, Event::Empty(_));
                match local.as_slice() {
                    b"package" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"version" {
                                version = Some(String::from_utf8_lossy(&attr.value).into_owned());
                            }
                        }
                    }
                    b"meta" => {
                        let mut prop = None;
                        let mut content = None;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"property" => {
                                    prop = Some(String::from_utf8_lossy(&attr.value).into_owned());
                                }
                                b"content" => {
                                    content =
                                        Some(String::from_utf8_lossy(&attr.value).into_owned());
                                }
                                _ => {}
                            }
                        }
                        if prop.as_deref() == Some("rendition:layout") {
                            if let Some(c) = content {
                                layout = Some(c);
                            } else if !is_empty {
                                in_layout_meta = true;
                            }
                        }
                    }
                    b"item" => {
                        let mut id = None;
                        let mut href = None;
                        let mut mt = None;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"id" => id = Some(decode(&attr.value)),
                                b"href" => href = Some(decode(&attr.value)),
                                b"media-type" => mt = Some(decode(&attr.value)),
                                _ => {}
                            }
                        }
                        if let (Some(id), Some(href)) = (id, href) {
                            manifest.push((id, href, mt.unwrap_or_default()));
                        }
                    }
                    b"itemref" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"idref" {
                                spine_idrefs.push(decode(&attr.value));
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }

    // Resolve spine idrefs against manifest, normalizing each href against
    // the OPF directory.
    let mut spine_docs = Vec::new();
    for idref in &spine_idrefs {
        if let Some((_, href, mt)) = manifest.iter().find(|(id, _, _)| id == idref) {
            spine_docs.push(SpineDoc {
                path: resolve_opf_href(opf_dir, href),
                media_type: if mt.is_empty() {
                    "application/xhtml+xml".into()
                } else {
                    mt.clone()
                },
            });
        }
    }

    Ok(ParsedSpine {
        version: version.unwrap_or_default(),
        layout,
        spine: spine_docs,
    })
}

/// Output of [`parse_spine`].
#[derive(Debug, Clone)]
pub struct ParsedSpine {
    pub version: String,
    pub layout: Option<String>,
    pub spine: Vec<SpineDoc>,
}

impl ParsedSpine {
    pub fn is_fixed_layout(&self) -> bool {
        self.layout.as_deref() == Some("pre-paginated")
    }
}

/// Rewrite the OPF to add a manifest entry for `kobo.js`. Returns the
/// new OPF bytes. The href is given relative to the OPF file.
pub fn add_kobo_manifest_entry(opf: &[u8], kobo_href: &str) -> Result<Vec<u8>, FormatError> {
    if already_has_kobo_item(opf) {
        return Ok(opf.to_vec());
    }

    let mut reader = Reader::from_reader(opf);
    let config = reader.config_mut();
    config.expand_empty_elements = false;
    config.trim_text(false);

    let mut writer = Writer::new(Cursor::new(Vec::with_capacity(opf.len() + 128)));
    let mut buf = Vec::new();

    loop {
        let evt = reader
            .read_event_into(&mut buf)
            .map_err(|e| parse_err(&format!("opf read: {e}")))?;
        match evt {
            Event::Eof => break,
            Event::End(end) => {
                if local_name(end.name().as_ref()) == b"manifest" {
                    let mut item = BytesStart::new("item");
                    item.push_attribute(("id", "kobo-js"));
                    item.push_attribute(("href", kobo_href));
                    item.push_attribute(("media-type", "application/javascript"));
                    writer
                        .write_event(Event::Empty(item))
                        .map_err(|e| parse_err(&format!("opf write: {e}")))?;
                }
                writer
                    .write_event(Event::End(BytesEnd::new(
                        std::str::from_utf8(end.name().as_ref())
                            .map_err(|e| parse_err(&format!("non-utf8 element: {e}")))?,
                    )))
                    .map_err(|e| parse_err(&format!("opf write: {e}")))?;
            }
            other => writer
                .write_event(other)
                .map_err(|e| parse_err(&format!("opf write: {e}")))?,
        }
        buf.clear();
    }

    Ok(writer.into_inner().into_inner())
}

fn already_has_kobo_item(opf: &[u8]) -> bool {
    // Simple substring detection; both quoting styles supported.
    opf.windows(b"id=\"kobo-js\"".len())
        .any(|w| w == b"id=\"kobo-js\"")
        || opf
            .windows(b"id='kobo-js'".len())
            .any(|w| w == b"id='kobo-js'")
}

fn local_name(qname: &[u8]) -> &[u8] {
    qname
        .iter()
        .position(|c| *c == b':')
        .map_or(qname, |i| &qname[i + 1..])
}

/// Resolve a manifest `href` against the OPF directory inside the EPUB ZIP.
///
/// Thin wrapper over [`archivis_formats::epub::resolve_manifest_href`] so
/// KEPUB stays bit-for-bit aligned with Archivis's existing EPUB resolver:
/// leading `/` means archive root, otherwise relative to `opf_dir`, with
/// `.` / `..` segments collapsed.
pub(crate) fn resolve_opf_href(opf_dir: &str, href: &str) -> String {
    archivis_formats::epub::resolve_manifest_href(opf_dir, href)
}

fn decode(value: &[u8]) -> String {
    String::from_utf8_lossy(value).into_owned()
}

fn parse_err(msg: &str) -> FormatError {
    FormatError::Parse {
        format: "KEPUB".into(),
        message: msg.into(),
    }
}

// Suppress unused-import warning when compiled without writer changes.
#[allow(dead_code)]
fn _touch(_: Cow<'_, str>) {}
