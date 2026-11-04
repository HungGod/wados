use std::path::{Path, PathBuf};

/// Build a non-colliding destination path under the downloads directory.
pub fn unique_download_path(dir: &Path, filename: &str) -> PathBuf {
    let safe = sanitize_filename(filename);
    let mut path = dir.join(&safe);
    if !path.exists() {
        return path;
    }
    let stem = Path::new(&safe).file_stem().and_then(|s| s.to_str()).unwrap_or("download");
    let ext = Path::new(&safe).extension().and_then(|s| s.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    for i in 1..10_000 {
        path = dir.join(format!("{stem}-{i}{ext}"));
        if !path.exists() {
            break;
        }
    }
    path
}

pub fn sanitize_filename(name: &str) -> String {
    let trimmed = name.trim();
    let base = if trimmed.is_empty() { "download" } else { trimmed };
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    if cleaned == "." || cleaned == ".." { "download".into() } else { cleaned }
}

pub fn filename_from_url(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments().and_then(|mut s| s.next_back()).map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .map(|s| sanitize_filename(&s))
        .unwrap_or_else(|| "download".into())
}

pub fn file_uri_for_path(path: &Path) -> String {
    format!("file://{}", path.display())
}

pub fn path_from_destination(dest: &str) -> PathBuf {
    if let Some(rest) = dest.strip_prefix("file://") { PathBuf::from(rest) } else { PathBuf::from(dest) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_path_separators() {
        assert_eq!(sanitize_filename("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_filename(".."), "download");
    }

    #[test]
    fn filename_from_typical_url() {
        assert_eq!(filename_from_url("https://example.com/files/report.pdf?x=1"), "report.pdf");
    }
}
