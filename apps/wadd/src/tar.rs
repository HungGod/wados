//! A build's context as a tar (ustar, with pax records for long names):
//! its Dockerfile, and what its COPY steps take in. Every entry's time is 0
//! and its owner root, so the same files make the same bytes, and podman's
//! cache knows them again.

use std::path::Path;

/// Parent folders are added as their entries are.
#[derive(Default)]
pub struct Tar {
    out: Vec<u8>,
    dirs: Vec<String>,
    /// Bytes of files so far.
    pub bytes: u64,
}

const DIR: u8 = b'5';
const FILE: u8 = b'0';
const SYMLINK: u8 = b'2';
const PAX: u8 = b'x';

fn put(h: &mut [u8; 512], at: usize, s: &[u8]) {
    let end = (at + s.len()).min(512);
    h[at..end].copy_from_slice(&s[..end - at]);
}

fn octal(n: u64, width: usize) -> String {
    format!("{n:0>w$o}\0", w = width - 1)
}

/// A name that fits ustar's 100 bytes, or its 155 of prefix and 100 of name.
fn split(name: &str) -> Option<(&str, &str)> {
    if name.len() <= 100 {
        return Some(("", name));
    }
    let cut = name.as_bytes()[..name.len().min(156)].iter().rposition(|b| *b == b'/')?;
    let (prefix, base) = (&name[..cut], &name[cut + 1..]);
    (base.len() <= 100 && !base.is_empty()).then_some((prefix, base))
}

/// One pax record: `<len> key=value\n`, its length counting itself.
fn pax_record(key: &str, value: &str) -> String {
    let body = format!(" {key}={value}\n");
    let mut len = body.len() + 1;
    while format!("{len}").len() + body.len() != len {
        len += 1;
    }
    format!("{len}{body}")
}

impl Tar {
    pub fn new() -> Self {
        Self::default()
    }

    fn header(&mut self, name: &str, mode: u32, size: u64, kind: u8, link: &str) {
        let mut pax = String::new();
        let (prefix, base) = match split(name) {
            Some(p) => p,
            None => {
                pax.push_str(&pax_record("path", name));
                let mut at = name.len().saturating_sub(100);
                while !name.is_char_boundary(at) {
                    at += 1;
                }
                ("", &name[at..])
            }
        };
        if link.len() > 100 {
            pax.push_str(&pax_record("linkpath", link));
        }
        if !pax.is_empty() {
            self.raw(&format!("PaxHeaders/{}", base.trim_end_matches('/')), 0o644, pax.len() as u64, PAX, "", "");
            self.data(pax.as_bytes());
        }
        let link = if link.len() > 100 { &link[..0] } else { link };
        self.raw(base, mode, size, kind, link, prefix);
    }

    fn raw(&mut self, base: &str, mode: u32, size: u64, kind: u8, link: &str, prefix: &str) {
        let mut h = [0u8; 512];
        // A name cut to fit at a character's edge (pax has the whole of it).
        let mut b = base.len().min(100);
        while !base.is_char_boundary(b) {
            b -= 1;
        }
        put(&mut h, 0, &base.as_bytes()[..b]);
        put(&mut h, 100, octal(u64::from(mode & 0o7777), 8).as_bytes());
        put(&mut h, 108, octal(0, 8).as_bytes());
        put(&mut h, 116, octal(0, 8).as_bytes());
        put(&mut h, 124, octal(size, 12).as_bytes());
        put(&mut h, 136, octal(0, 12).as_bytes());
        put(&mut h, 148, b"        ");
        h[156] = kind;
        put(&mut h, 157, link.as_bytes());
        put(&mut h, 257, b"ustar\0");
        put(&mut h, 263, b"00");
        put(&mut h, 345, prefix.as_bytes());
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        put(&mut h, 148, format!("{sum:06o}\0 ").as_bytes());
        self.out.extend_from_slice(&h);
    }

    fn data(&mut self, bytes: &[u8]) {
        self.out.extend_from_slice(bytes);
        let pad = (512 - bytes.len() % 512) % 512;
        self.out.resize(self.out.len() + pad, 0);
    }

    fn parents(&mut self, path: &str) {
        let segs: Vec<&str> = path.split('/').collect();
        for i in 1..segs.len() {
            let d = segs[..i].join("/");
            if !self.dirs.contains(&d) {
                self.header(&format!("{d}/"), 0o755, 0, DIR, "");
                self.dirs.push(d);
            }
        }
    }

    fn check(path: &str) -> Result<(), String> {
        let bad = path.is_empty()
            || path.starts_with('/')
            || path.split('/').any(|s| s == ".." || s == ".")
            || path.chars().any(char::is_control);
        if bad { Err(format!("not a path a build takes: {path:?}")) } else { Ok(()) }
    }

    pub fn dir(&mut self, path: &str, mode: u32) -> Result<(), String> {
        Self::check(path)?;
        self.parents(path);
        if !self.dirs.iter().any(|d| d == path) {
            self.header(&format!("{path}/"), mode, 0, DIR, "");
            self.dirs.push(path.to_string());
        }
        Ok(())
    }

    pub fn file(&mut self, path: &str, mode: u32, bytes: &[u8]) -> Result<(), String> {
        Self::check(path)?;
        self.parents(path);
        self.header(path, mode, bytes.len() as u64, FILE, "");
        self.data(bytes);
        self.bytes += bytes.len() as u64;
        Ok(())
    }

    pub fn symlink(&mut self, path: &str, target: &str) -> Result<(), String> {
        Self::check(path)?;
        if target.chars().any(char::is_control) {
            return Err(format!("{path}: a link to {target:?}"));
        }
        self.parents(path);
        self.header(path, 0o777, 0, SYMLINK, target);
        Ok(())
    }

    /// The archive, ended.
    pub fn finish(mut self) -> Vec<u8> {
        self.out.resize(self.out.len() + 1024, 0);
        self.out
    }
}

/// What a COPY takes from this machine: a file, a folder (all of it, links
/// as links: never followed), at `to` in the context. Refused past
/// `max_bytes` in all (`tar.bytes` counts what's in already), and anything
/// `allowed` refuses (the owner check on a machine).
pub fn add_path(
    tar: &mut Tar,
    from: &Path,
    to: &str,
    max_bytes: u64,
    allowed: &dyn Fn(&std::fs::Metadata) -> bool,
) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta = std::fs::symlink_metadata(from).map_err(|e| format!("{}: {e}", from.display()))?;
    if !allowed(&meta) {
        return Err(format!("{} isn't yours", from.display()));
    }
    let mode = meta.permissions().mode() & 0o7777;
    let kind = meta.file_type();
    if kind.is_symlink() {
        let target = std::fs::read_link(from).map_err(|e| format!("{}: {e}", from.display()))?;
        return tar.symlink(to, &target.to_string_lossy());
    }
    if kind.is_file() {
        if tar.bytes + meta.size() > max_bytes {
            return Err(too_big(max_bytes));
        }
        let bytes = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
        if tar.bytes + bytes.len() as u64 > max_bytes {
            return Err(too_big(max_bytes));
        }
        return tar.file(to, mode, &bytes);
    }
    if kind.is_dir() {
        tar.dir(to, mode)?;
        let mut names: Vec<_> = std::fs::read_dir(from)
            .map_err(|e| format!("{}: {e}", from.display()))?
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        names.sort();
        for n in names {
            let Some(name) = n.to_str() else {
                return Err(format!("{}: a name that isn't UTF-8", from.join(&n).display()));
            };
            add_path(tar, &from.join(name), &format!("{to}/{name}"), max_bytes, allowed)?;
        }
        return Ok(());
    }
    // Sockets, pipes, devices: nothing to copy.
    tracing::debug!("{}: not a file, a folder or a link; left out", from.display());
    Ok(())
}

fn too_big(max: u64) -> String {
    format!("that's more than the {} MB a build takes from this machine", max >> 20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_come_back_as_they_went_in() {
        let mut t = Tar::new();
        t.file("Dockerfile", 0o644, b"FROM x\n").unwrap();
        let long = format!("copy/1/{}/deep.txt", "a".repeat(150));
        t.file(&long, 0o755, b"hi").unwrap();
        t.symlink("copy/2/link", "../elsewhere").unwrap();
        let bytes = t.finish();
        assert_eq!(bytes.len() % 512, 0);
        let back = crate::fake::untar(&bytes);
        assert_eq!(back["Dockerfile"], b"FROM x\n");
        assert_eq!(back[&long], b"hi");
        assert_eq!(back["copy/2/link"], b"-> ../elsewhere");
        assert!(back.contains_key("copy/1/"), "{:?}", back.keys().collect::<Vec<_>>());
        assert!(t_bad("../x") && t_bad("/abs") && t_bad("a/./b"));
    }

    fn t_bad(p: &str) -> bool {
        Tar::new().file(p, 0o644, b"").is_err()
    }

    #[test]
    fn a_folder_is_copied_whole_within_its_cap() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("notes/sub")).unwrap();
        std::fs::write(d.path().join("notes/a.md"), "a").unwrap();
        std::fs::write(d.path().join("notes/sub/b.md"), "bb").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", d.path().join("notes/p")).unwrap();
        let mut t = Tar::new();
        add_path(&mut t, &d.path().join("notes"), "copy/1/notes", 1024, &|_| true).unwrap();
        assert_eq!(t.bytes, 3);
        let back = crate::fake::untar(&t.finish());
        assert_eq!(back["copy/1/notes/sub/b.md"], b"bb");
        // A link stays a link: what it points at isn't read.
        assert_eq!(back["copy/1/notes/p"], b"-> /etc/passwd");
        let mut small = Tar::new();
        assert!(add_path(&mut small, &d.path().join("notes"), "c", 2, &|_| true).unwrap_err().contains("more than"));
    }
}
