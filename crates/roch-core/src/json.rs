//! JSON output compatible with the original `System.Text.Json` files.
//!
//! The .NET writer used two-space indentation, `Environment.NewLine` line
//! endings and the default `JavaScriptEncoder`, which escapes every non-ASCII
//! character and the HTML-sensitive characters `<`, `>`, `&`, `'`, `+` and
//! `` ` `` as `\uXXXX`. Reproducing this keeps newly written `package.json` and
//! `validation.json` files in the same textual format as existing packages.
//! Readers (both the original app and this port) accept either form.

use std::io;

use serde::Serialize;
use serde_json::ser::{CharEscape, Formatter, PrettyFormatter};

use crate::{Error, Result};

struct DotNetFormatter<'a> {
    inner: PrettyFormatter<'a>,
}

fn write_u_escape<W: ?Sized + io::Write>(writer: &mut W, unit: u16) -> io::Result<()> {
    write!(writer, "\\u{unit:04X}")
}

impl Formatter for DotNetFormatter<'_> {
    fn write_string_fragment<W: ?Sized + io::Write>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()> {
        let mut start = 0;
        for (i, ch) in fragment.char_indices() {
            let escape = !ch.is_ascii() || matches!(ch, '<' | '>' | '&' | '\'' | '+' | '`' | '\u{7f}');
            if escape {
                writer.write_all(&fragment.as_bytes()[start..i])?;
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    write_u_escape(writer, *unit)?;
                }
                start = i + ch.len_utf8();
            }
        }
        writer.write_all(&fragment.as_bytes()[start..])
    }

    fn write_char_escape<W: ?Sized + io::Write>(&mut self, writer: &mut W, char_escape: CharEscape) -> io::Result<()> {
        match char_escape {
            CharEscape::Quote => write_u_escape(writer, 0x22),
            CharEscape::ReverseSolidus => writer.write_all(b"\\\\"),
            CharEscape::Solidus => writer.write_all(b"/"),
            CharEscape::Backspace => writer.write_all(b"\\b"),
            CharEscape::FormFeed => writer.write_all(b"\\f"),
            CharEscape::LineFeed => writer.write_all(b"\\n"),
            CharEscape::CarriageReturn => writer.write_all(b"\\r"),
            CharEscape::Tab => writer.write_all(b"\\t"),
            CharEscape::AsciiControl(b) => write_u_escape(writer, b as u16),
        }
    }

    fn begin_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.inner.begin_array(w)
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.inner.end_array(w)
    }
    fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        self.inner.begin_array_value(w, first)
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.inner.end_array_value(w)
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.inner.begin_object(w)
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.inner.end_object(w)
    }
    fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        self.inner.begin_object_key(w, first)
    }
    fn end_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.inner.end_object_key(w)
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.inner.begin_object_value(w)
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.inner.end_object_value(w)
    }
}

/// Serialize with indentation like `JsonSerializer.Serialize(value, WriteIndented = true)`.
pub fn to_indented<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    let mut out = Vec::new();
    let formatter = DotNetFormatter { inner: PrettyFormatter::with_indent(b"  ") };
    let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);
    value.serialize(&mut ser).map_err(|e| Error::invalid(format!("JSON serialization failed: {e}")))?;
    let text = String::from_utf8(out).map_err(|e| Error::invalid(e.to_string()))?;
    // Strings never contain raw newlines (they are escaped), so this only changes layout.
    Ok(if cfg!(windows) { text.replace('\n', "\r\n") } else { text })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a JSON `\uXXXX` escape without writing one literally in this source.
    fn u(code: &str) -> String {
        format!("{}u{code}", '\\')
    }

    #[test]
    fn escapes_like_system_text_json() {
        let v = serde_json::json!({ "a": "ROCH \u{2014} x<y>&'+`\"\\\n\u{1}", "b": [1, true] });
        let s = to_indented(&v).unwrap().replace("\r\n", "\n");
        let expected = format!(
            "\"ROCH {} x{}y{}{}{}{}{}{}\\\\\\n{}\"",
            u("2014"),
            u("003C"),
            u("003E"),
            u("0026"),
            u("0027"),
            u("002B"),
            u("0060"),
            u("0022"),
            u("0001")
        );
        assert!(s.contains(&expected), "{s}\n{expected}");
        let back: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(back, v);
        assert!(s.starts_with("{\n  \"a\": "));
    }

    #[test]
    fn astral_characters_use_surrogate_pairs() {
        let s = to_indented(&"\u{1F600}").unwrap();
        assert_eq!(s, format!("\"{}{}\"", u("D83D"), u("DE00")));
    }
}
