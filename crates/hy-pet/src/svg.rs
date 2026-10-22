//! A strict check for pet art (architecture 4.4: data-only packs, no executable content).
//!
//! Pet art is a small SVG that the pet page inserts into its document, so it must not be
//! able to run script, load anything, or restyle the page. Rather than parse arbitrary
//! XML, the check accepts a deliberately small subset and refuses everything else:
//!
//! - a single `<svg viewBox="0 0 128 128">` root;
//! - only basic shape, group and text elements;
//! - only presentation attributes from an allow-list (no `style`, no event handlers, no
//!   links, no `url(...)` references);
//! - only the palette and part classes the pet page knows;
//! - no comments, processing instructions, `DOCTYPE`, `CDATA` or entities other than the
//!   five XML ones and numeric references;
//! - balanced tags and at most 64 KB.

use std::collections::BTreeSet;

use thiserror::Error;

/// Largest accepted art file.
pub const MAX_ART_BYTES: usize = 64 * 1024;

/// The view box every pet uses, so the hitbox and badges line up.
pub const VIEW_BOX: &str = "0 0 128 128";

const ELEMENTS: &[&str] = &[
    "svg", "g", "path", "ellipse", "circle", "rect", "line", "polyline", "polygon", "text",
    "title", "desc",
];

const ATTRIBUTES: &[&str] = &[
    "viewBox",
    "class",
    "d",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "width",
    "height",
    "points",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "stroke-opacity",
    "opacity",
    "transform",
    "font-size",
    "font-weight",
    "font-family",
    "text-anchor",
    "dominant-baseline",
    "aria-hidden",
    "focusable",
];

/// Classes the pet page styles: palette slots (`hy-a` and `hy-b` fill with the chosen
/// colour variant, `hy-sa` and `hy-sb` stroke with it) and `eye`, which closes in the
/// sleepy states.
pub const CLASSES: &[&str] = &["hy-a", "hy-b", "hy-sa", "hy-sb", "eye"];

/// Why art was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArtError {
    /// Larger than [`MAX_ART_BYTES`].
    #[error("art is {0} bytes; the limit is {MAX_ART_BYTES}")]
    TooLarge(usize),
    /// Not well formed in the accepted subset.
    #[error("art is malformed at byte {at}: {what}")]
    Malformed {
        /// Byte offset.
        at: usize,
        /// What was wrong.
        what: String,
    },
    /// An element outside the allow-list.
    #[error("element <{0}> is not allowed in pet art")]
    Element(String),
    /// An attribute outside the allow-list.
    #[error("attribute {0:?} is not allowed in pet art")]
    Attribute(String),
    /// An attribute value that could load or run something.
    #[error("attribute {name:?} has a forbidden value")]
    Value {
        /// Attribute name.
        name: String,
    },
    /// A class the pet page does not define.
    #[error("class {0:?} is not one of the pet art classes")]
    Class(String),
    /// The root is not `<svg viewBox="0 0 128 128">`.
    #[error("the root must be <svg viewBox=\"{VIEW_BOX}\">")]
    Root,
}

fn malformed(at: usize, what: impl Into<String>) -> ArtError {
    ArtError::Malformed {
        at,
        what: what.into(),
    }
}

/// What the check found, for tests and diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtReport {
    /// Element names used.
    pub elements: BTreeSet<String>,
    /// Classes used.
    pub classes: BTreeSet<String>,
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b':' | b'.')
}

fn read_name(b: &[u8], mut i: usize) -> (usize, usize) {
    let start = i;
    if i < b.len() && is_name_start(b[i]) {
        i += 1;
        while i < b.len() && is_name_char(b[i]) {
            i += 1;
        }
    }
    (start, i)
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Check character data or an attribute value for entities: only `&amp;`, `&lt;`,
/// `&gt;`, `&quot;`, `&apos;` and numeric references are accepted.
fn check_entities(text: &str, at: usize) -> Result<(), ArtError> {
    let mut rest = text;
    let mut offset = at;
    while let Some(p) = rest.find('&') {
        let tail = &rest[p + 1..];
        let end = tail
            .find(';')
            .ok_or_else(|| malformed(offset + p, "unterminated entity"))?;
        let name = &tail[..end];
        let ok = matches!(name, "amp" | "lt" | "gt" | "quot" | "apos")
            || (name.len() > 1
                && name.starts_with('#')
                && (name[1..].chars().all(|c| c.is_ascii_digit())
                    || (name.len() > 2
                        && name[1..].starts_with('x')
                        && name[2..].chars().all(|c| c.is_ascii_hexdigit()))));
        if !ok {
            return Err(malformed(
                offset + p,
                format!("entity &{name}; is not allowed"),
            ));
        }
        offset += p + 1 + end + 1;
        rest = &tail[end + 1..];
    }
    Ok(())
}

fn check_value(name: &str, value: &str) -> Result<(), ArtError> {
    let lower = value.to_ascii_lowercase();
    let forbidden = [
        "url(",
        "javascript:",
        "data:",
        "expression(",
        "http:",
        "https:",
        "//",
    ];
    if forbidden.iter().any(|f| lower.contains(f)) {
        return Err(ArtError::Value {
            name: name.to_owned(),
        });
    }
    Ok(())
}

/// Check pet art and report what it uses.
///
/// # Errors
/// [`ArtError`] naming the first rule the art breaks.
pub fn check_art(svg: &str) -> Result<ArtReport, ArtError> {
    if svg.len() > MAX_ART_BYTES {
        return Err(ArtError::TooLarge(svg.len()));
    }
    let b = svg.as_bytes();
    let mut report = ArtReport::default();
    let mut stack: Vec<&str> = Vec::new();
    let mut saw_root = false;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let next = svg[i..].find('<').map_or(b.len(), |p| i + p);
            let text = &svg[i..next];
            if text.contains('>') {
                return Err(malformed(i, "stray '>'"));
            }
            if stack.is_empty() && !text.trim().is_empty() {
                return Err(malformed(i, "text outside the root element"));
            }
            check_entities(text, i)?;
            i = next;
            continue;
        }
        let tag_at = i;
        i += 1;
        if i < b.len() && (b[i] == b'!' || b[i] == b'?') {
            return Err(malformed(
                tag_at,
                "comments, DOCTYPE, CDATA and processing instructions are not allowed",
            ));
        }
        if i < b.len() && b[i] == b'/' {
            let (s, e) = read_name(b, i + 1);
            let name = &svg[s..e];
            let j = skip_ws(b, e);
            if name.is_empty() || j >= b.len() || b[j] != b'>' {
                return Err(malformed(tag_at, "bad closing tag"));
            }
            match stack.pop() {
                Some(open) if open == name => {}
                _ => return Err(malformed(tag_at, format!("unbalanced </{name}>"))),
            }
            if stack.is_empty() {
                // Only whitespace may follow the root.
                if !svg[j + 1..].trim().is_empty() {
                    return Err(malformed(j + 1, "content after the root element"));
                }
                return if saw_root {
                    Ok(report)
                } else {
                    Err(ArtError::Root)
                };
            }
            i = j + 1;
            continue;
        }
        let (s, e) = read_name(b, i);
        let name = &svg[s..e];
        if name.is_empty() {
            return Err(malformed(tag_at, "bad tag name"));
        }
        if !ELEMENTS.contains(&name) {
            return Err(ArtError::Element(name.to_owned()));
        }
        let is_root = stack.is_empty();
        if is_root && (name != "svg" || saw_root) {
            return Err(ArtError::Root);
        }
        report.elements.insert(name.to_owned());
        i = e;
        let mut view_box = None;
        let self_closing;
        loop {
            i = skip_ws(b, i);
            if i >= b.len() {
                return Err(malformed(tag_at, "unterminated tag"));
            }
            if b[i] == b'>' {
                self_closing = false;
                i += 1;
                break;
            }
            if b[i] == b'/' {
                if i + 1 < b.len() && b[i + 1] == b'>' {
                    self_closing = true;
                    i += 2;
                    break;
                }
                return Err(malformed(i, "bad '/' in tag"));
            }
            let (as_, ae) = read_name(b, i);
            let attr = &svg[as_..ae];
            if attr.is_empty() {
                return Err(malformed(i, "bad attribute name"));
            }
            if !ATTRIBUTES.contains(&attr) {
                return Err(ArtError::Attribute(attr.to_owned()));
            }
            i = skip_ws(b, ae);
            if i >= b.len() || b[i] != b'=' {
                return Err(malformed(i, "attribute without a value"));
            }
            i = skip_ws(b, i + 1);
            let quote = *b.get(i).ok_or_else(|| malformed(i, "missing value"))?;
            if quote != b'"' && quote != b'\'' {
                return Err(malformed(i, "unquoted attribute value"));
            }
            let vs = i + 1;
            let ve = svg[vs..]
                .find(char::from(quote))
                .map(|p| vs + p)
                .ok_or_else(|| malformed(i, "unterminated attribute value"))?;
            let value = &svg[vs..ve];
            if value.contains('<') {
                return Err(malformed(vs, "'<' in attribute value"));
            }
            check_entities(value, vs)?;
            check_value(attr, value)?;
            if attr == "class" {
                for class in value.split_ascii_whitespace() {
                    if !CLASSES.contains(&class) {
                        return Err(ArtError::Class(class.to_owned()));
                    }
                    report.classes.insert(class.to_owned());
                }
            }
            if attr == "viewBox" {
                view_box = Some(value);
            }
            i = ve + 1;
        }
        if is_root {
            if view_box != Some(VIEW_BOX) || self_closing {
                return Err(ArtError::Root);
            }
            saw_root = true;
        }
        if !self_closing {
            stack.push(name);
        }
        if is_root && self_closing {
            return Err(ArtError::Root);
        }
    }
    Err(if saw_root {
        malformed(b.len(), "unterminated <svg>")
    } else {
        ArtError::Root
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r##"<svg viewBox="0 0 128 128" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
  <ellipse cx="64" cy="118" rx="38" ry="5" fill="#0000002a"/>
  <g><circle class="hy-a" cx="10" cy="10" r="4"/><ellipse class="eye" cx="1" cy="2" rx="1" ry="1" fill="#1f2a30"/></g>
  <text x="64" y="70" text-anchor="middle" font-size="40">P &amp; Q</text>
</svg>"##;

    #[test]
    fn accepts_the_subset() {
        let ok = OK.replace(" xmlns=\"http://www.w3.org/2000/svg\"", "");
        let r = check_art(&ok).expect("valid");
        assert!(r.elements.contains("ellipse") && r.elements.contains("text"));
        assert!(r.classes.contains("hy-a") && r.classes.contains("eye"));
    }

    #[test]
    fn xmlns_is_not_needed_and_refused() {
        // The pet page inserts art with the HTML parser, which puts <svg> in the SVG
        // namespace itself, so art carries no namespace URL at all.
        assert_eq!(check_art(OK), Err(ArtError::Attribute("xmlns".into())));
    }

    fn refuses(svg: &str) -> ArtError {
        check_art(svg).expect_err("should be refused")
    }

    #[test]
    fn refuses_script_and_foreign_content() {
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 128 128"><script>alert(1)</script></svg>"#),
            ArtError::Element("script".into())
        );
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 128 128"><foreignObject/></svg>"#),
            ArtError::Element("foreignObject".into())
        );
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 128 128"><style>*{}</style></svg>"#),
            ArtError::Element("style".into())
        );
        assert_eq!(
            refuses(r##"<svg viewBox="0 0 128 128"><use href="#a"/></svg>"##),
            ArtError::Element("use".into())
        );
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 128 128"><image/></svg>"#),
            ArtError::Element("image".into())
        );
    }

    #[test]
    fn refuses_handlers_styles_links_and_urls() {
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 128 128" onload="x()"></svg>"#),
            ArtError::Attribute("onload".into())
        );
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 128 128"><g style="fill:red"/></svg>"#),
            ArtError::Attribute("style".into())
        );
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 128 128"><g xlink:href="x"/></svg>"#),
            ArtError::Attribute("xlink:href".into())
        );
        assert!(matches!(
            refuses(r#"<svg viewBox="0 0 128 128"><path fill="url(#g)" d="M0 0"/></svg>"#),
            ArtError::Value { .. }
        ));
        assert!(matches!(
            refuses(r#"<svg viewBox="0 0 128 128"><path fill="JavaScript:x" d="M0 0"/></svg>"#),
            ArtError::Value { .. }
        ));
    }

    #[test]
    fn refuses_unknown_classes() {
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 128 128"><g class="hy-a bubble"/></svg>"#),
            ArtError::Class("bubble".into())
        );
    }

    #[test]
    fn refuses_markup_tricks() {
        for bad in [
            r#"<?xml version="1.0"?><svg viewBox="0 0 128 128"></svg>"#,
            r#"<!DOCTYPE svg><svg viewBox="0 0 128 128"></svg>"#,
            r#"<svg viewBox="0 0 128 128"><!-- hi --></svg>"#,
            r#"<svg viewBox="0 0 128 128"><text>&xxe;</text></svg>"#,
            r#"<svg viewBox="0 0 128 128"><g></svg>"#,
            r#"<svg viewBox="0 0 128 128"><g/></g></svg>"#,
            r#"<svg viewBox="0 0 128 128"></svg><svg viewBox="0 0 128 128"></svg>"#,
            r#"<svg viewBox="0 0 128 128"><g fill=red/></svg>"#,
            r#"<svg viewBox="0 0 128 128"><g fill="a<b"/></svg>"#,
            r#"<svg viewBox="0 0 128 128">"#,
            r#"text<svg viewBox="0 0 128 128"></svg>"#,
            r#"<svg viewBox="0 0 128 128"><g fill></g></svg>"#,
        ] {
            assert!(
                matches!(
                    check_art(bad),
                    Err(ArtError::Malformed { .. } | ArtError::Root)
                ),
                "{bad} -> {:?}",
                check_art(bad)
            );
        }
    }

    #[test]
    fn refuses_a_wrong_root() {
        assert_eq!(
            refuses(r#"<svg viewBox="0 0 64 64"></svg>"#),
            ArtError::Root
        );
        assert_eq!(refuses(r#"<g></g>"#), ArtError::Root);
        assert_eq!(refuses(r#"<svg viewBox="0 0 128 128"/>"#), ArtError::Root);
        assert_eq!(refuses(""), ArtError::Root);
    }

    #[test]
    fn refuses_oversized_art() {
        let big = format!(
            r#"<svg viewBox="0 0 128 128">{}</svg>"#,
            "<g/>".repeat(MAX_ART_BYTES / 4)
        );
        assert!(matches!(check_art(&big), Err(ArtError::TooLarge(_))));
    }

    #[test]
    fn numeric_entities_are_fine() {
        check_art(r#"<svg viewBox="0 0 128 128"><text>&#65;&#x42;</text></svg>"#).expect("ok");
    }
}
