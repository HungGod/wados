//! Netscape-bookmark-file import/export (compatible with Firefox/Chrome exports).

use crate::store::{Bookmark, Store};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

pub fn export_netscape_html(bookmarks: &[Bookmark]) -> String {
    let mut body = String::from(
        r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<!-- This is an automatically generated file.
     It will be read and overwritten.
     DO NOT EDIT! -->
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<DL><p>
"#,
    );

    let mut folders: Vec<String> = bookmarks.iter().map(|b| b.folder.clone()).collect();
    folders.sort();
    folders.dedup();

    for folder in folders {
        body.push_str(&format!("    <DT><H3>{}</H3>\n    <DL><p>\n", esc(&folder)));
        for b in bookmarks.iter().filter(|b| b.folder == folder) {
            body.push_str(&format!("        <DT><A HREF=\"{}\">{}</A>\n", esc_attr(&b.url), esc(&b.title)));
        }
        body.push_str("    </DL><p>\n");
    }
    body.push_str("</DL><p>\n");
    body
}

pub fn export_to_file(bookmarks: &[Bookmark], path: &Path) -> Result<()> {
    let html = export_netscape_html(bookmarks);
    fs::write(path, html).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Very small HTML bookmark importer — enough for common export files.
pub fn import_netscape_html(html: &str) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    let mut folder = "Imported".to_string();
    for raw_line in html.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("<DT><H3") {
            if let Some(start) = rest.find('>') {
                let name = rest[start + 1..].trim_end_matches("</H3>").trim().to_string();
                if !name.is_empty() {
                    folder = unescape(&name);
                }
            }
        } else if (line.contains("<A HREF=\"") || line.contains("<A HREF='"))
            && let Some((title, url)) = parse_anchor(line)
        {
            out.push((title, url, folder.clone()));
        }
    }
    if out.is_empty() {
        bail!("no bookmarks found in file");
    }
    Ok(out)
}

pub fn import_file_into_store(store: &Store, path: &Path) -> Result<usize> {
    let html = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let items = import_netscape_html(&html)?;
    let mut n = 0;
    for (title, url, folder) in items {
        store.add_bookmark(&title, &url, &folder)?;
        n += 1;
    }
    Ok(n)
}

fn parse_anchor(line: &str) -> Option<(String, String)> {
    let href_key = if line.contains("HREF=\"") { "HREF=\"" } else { "HREF='" };
    let quote = if href_key.ends_with('"') { '"' } else { '\'' };
    let start = line.find(href_key)? + href_key.len();
    let rest = &line[start..];
    let end = rest.find(quote)?;
    let url = unescape(&rest[..end]);
    let after = &rest[end + 1..];
    let title_start = after.find('>')? + 1;
    let title_end = after[title_start..].find("</A>")? + title_start;
    let title = unescape(after[title_start..title_end].trim());
    if url.is_empty() {
        return None;
    }
    let title = if title.is_empty() { url.clone() } else { title };
    Some((title, url))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn esc_attr(s: &str) -> String {
    esc(s).replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_minimal_export_import() {
        let bookmarks = vec![Bookmark {
            id: 1,
            title: "Example".into(),
            url: "https://example.com/".into(),
            folder: "Bar".into(),
            position: 0,
            created_at: chrono::Utc::now(),
        }];
        let html = export_netscape_html(&bookmarks);
        let parsed = import_netscape_html(&html).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "Example");
        assert_eq!(parsed[0].1, "https://example.com/");
    }
}
