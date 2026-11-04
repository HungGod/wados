//! Command-line argument parsing for the WadBrowser binary.

use crate::NEW_TAB_URI;

#[derive(Debug, Clone, Default)]
pub struct LaunchArgs {
    pub private: bool,
    pub urls: Vec<String>,
    pub restore: Option<bool>,
    pub verbose: bool,
}

impl LaunchArgs {
    pub fn parse<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out = Self::default();
        let mut iter = args.into_iter().peekable();
        // skip argv[0]
        let _ = iter.next();

        while let Some(arg) = iter.next() {
            let arg = arg.as_ref();
            match arg {
                "--private" | "-p" => out.private = true,
                "--verbose" | "-v" => out.verbose = true,
                "--no-restore" => out.restore = Some(false),
                "--restore" => out.restore = Some(true),
                "--help" | "-h" => {
                    // Caller may print help; we just leave urls empty.
                    out.urls.clear();
                    out.urls.push("__help__".into());
                    return out;
                }
                "--" => {
                    for rest in iter.by_ref() {
                        push_url(&mut out.urls, rest.as_ref());
                    }
                }
                s if s.starts_with('-') => {
                    // Unknown flag — ignore for forward compatibility.
                }
                s => push_url(&mut out.urls, s),
            }
        }
        out
    }

    pub fn initial_urls(&self, fallback_new_tab: bool) -> Vec<String> {
        if self.urls.iter().any(|u| u == "__help__") {
            return vec![];
        }
        if self.urls.is_empty() && fallback_new_tab { vec![NEW_TAB_URI.into()] } else { self.urls.clone() }
    }
}

fn push_url(urls: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    urls.push(trimmed.to_string());
}

pub fn help_text() -> &'static str {
    const TEXT: &str = "WadBrowser — WadOS's web browser\n\n\
     Usage: wadbrowser [OPTIONS] [URL...]\n\n\
     Options:\n\
       -p, --private      Open a private window\n\
       -v, --verbose      More logging\n\
           --restore      Force session restore\n\
           --no-restore   Skip session restore\n\
       -h, --help         Show this help\n\n\
     Based on Cactus: https://github.com/yohannes-mesay/cactus-browser (MPL-2.0)\n";
    TEXT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_private_and_urls() {
        let args = LaunchArgs::parse(["wadbrowser", "--private", "https://example.com", "rust lang"]);
        assert!(args.private);
        assert_eq!(args.urls.len(), 2);
        assert_eq!(args.urls[0], "https://example.com");
    }

    #[test]
    fn parses_no_restore() {
        let args = LaunchArgs::parse(["wadbrowser", "--no-restore"]);
        assert_eq!(args.restore, Some(false));
    }
}
