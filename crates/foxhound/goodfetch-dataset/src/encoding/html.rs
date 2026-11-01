//! An HTML table: one UTF-8 HTML document named after the Dataset, holding
//! one table, its header row in `thead` and one row per Record in `tbody`.
//! Each cell's text is escaped so an HTML parser gives it back: `&`, `<`,
//! `>`, `"` and `'` as character references, and a carriage return as one,
//! since a parser reads a bare one as a line feed.

use super::columns::Cell;

/// The writer of one HTML table Export's rows.
#[derive(Clone, Debug)]
pub(super) struct HtmlTable {
    pub(super) title: String,
}

impl HtmlTable {
    pub(super) fn begin(&self, names: &[String], out: &mut Vec<u8>) {
        let mut text =
            String::from("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>");
        escape(&self.title, &mut text);
        text.push_str("</title>\n</head>\n<body>\n<table>\n<thead>\n<tr>");
        for name in names {
            text.push_str("<th>");
            escape(name, &mut text);
            text.push_str("</th>");
        }
        text.push_str("</tr>\n</thead>\n<tbody>\n");
        out.extend_from_slice(text.as_bytes());
    }

    pub(super) fn row(cells: &[Cell], out: &mut Vec<u8>) {
        let mut text = String::from("<tr>");
        for cell in cells {
            text.push_str("<td>");
            escape(cell.text(), &mut text);
            text.push_str("</td>");
        }
        text.push_str("</tr>\n");
        out.extend_from_slice(text.as_bytes());
    }

    pub(super) fn end(out: &mut Vec<u8>) {
        out.extend_from_slice(b"</tbody>\n</table>\n</body>\n</html>\n");
    }
}

fn escape(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\r' => out.push_str("&#13;"),
            other => out.push(other),
        }
    }
}
