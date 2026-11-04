//! Taildrive: my other machines' homes, as tailscaled serves them on the
//! tailnet's WebDAV address, read with GIO (gvfs's dav backend on the
//! machine). One function says where a machine's home is ([`home_uri`]);
//! the rest lists a folder there and opens a file with its default app.
//!
//! A machine's home is `dav://100.100.100.100:8080/<tailnet>/<machine>/home`:
//! `<tailnet>` is wad-proto's `Tailnet::tailnet` (it may have an `@`), the
//! machine the first label of its MagicDNS name (its hostname, without
//! one), each percent-encoded but for unreserved characters: wadd's own rule
//! (`taildrive.rs`'s `home_url`). wadd shares each machine's home as `home`. WADOS_TAILDRIVE puts another
//! address in place of `dav://100.100.100.100:8080` (a test's WebDAV
//! server, or a `file://` folder laid out the same way).

use gtk::{gio, glib, prelude::*};

/// Where tailscaled serves Taildrive, on every machine of the tailnet.
pub const TAILDRIVE: &str = "dav://100.100.100.100:8080";

/// What a machine's home is shared as.
pub const HOME_SHARE: &str = "home";

/// Taildrive's address here (WADOS_TAILDRIVE, else tailscaled's).
pub fn base() -> String {
    std::env::var("WADOS_TAILDRIVE")
        .ok()
        .filter(|b| !b.is_empty())
        .map(|b| b.trim_end_matches('/').to_string())
        .unwrap_or_else(|| TAILDRIVE.into())
}

/// One path segment, percent-encoded: all but unreserved characters (as wadd's `taildrive::encode`).
fn seg(s: &str) -> String {
    glib::Uri::escape_string(s, None, false).to_string()
}

/// A machine's home over Taildrive: `<base>/<tailnet>/<machine>/home`, the
/// machine being `dns_name`'s first label ('desk.tail1234.ts.net': desk), or
/// `hostname` when it has none.
pub fn home_uri(base: &str, tailnet: &str, dns_name: &str, hostname: &str) -> String {
    let machine = dns_name.split('.').next().filter(|m| !m.is_empty()).unwrap_or(hostname).to_ascii_lowercase();
    format!("{base}/{}/{}/{HOME_SHARE}", seg(tailnet), seg(&machine))
}

/// A folder in it, by its path's names from the home.
pub fn folder_uri(home: &str, path: &[String]) -> String {
    let mut u = home.to_string();
    for p in path {
        u.push('/');
        u.push_str(&seg(p));
    }
    u
}

/// One thing in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its name in the folder (what the next path segment is).
    pub name: String,
    pub display: String,
    pub folder: bool,
    pub content_type: String,
    pub size: u64,
}

impl Entry {
    /// Its glyph (a lucide name).
    pub fn icon(&self) -> &'static str {
        if self.folder {
            return "folder";
        }
        let t = self.content_type.as_str();
        if t.starts_with("image/") {
            "image"
        } else if t.starts_with("text/") || t == "application/pdf" || t.contains("document") {
            "file-text"
        } else {
            "file"
        }
    }
}

/// Folders first, then by name (as a file manager lists them); hidden ones left out.
pub fn sorted(mut list: Vec<Entry>) -> Vec<Entry> {
    list.retain(|e| !e.name.starts_with('.'));
    list.sort_by(|a, b| b.folder.cmp(&a.folder).then_with(|| a.display.to_lowercase().cmp(&b.display.to_lowercase())));
    list
}

const ATTRS: &str = "standard::name,standard::display-name,standard::type,standard::content-type,standard::size";

/// What's in the folder at `uri`, mounting Taildrive's address first when
/// gvfs hasn't (a dav location lists only once it's mounted).
pub async fn list(uri: &str) -> Result<Vec<Entry>, glib::Error> {
    let file = gio::File::for_uri(uri);
    let e = match file.enumerate_children_future(ATTRS, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT).await {
        Ok(e) => e,
        Err(err) if err.matches(gio::IOErrorEnum::NotMounted) => {
            // Taildrive asks nothing: an empty mount operation does. Another
            // listing may have mounted it meanwhile, which is as good.
            let op = gio::MountOperation::new();
            match file.mount_enclosing_volume_future(gio::MountMountFlags::NONE, Some(&op)).await {
                Ok(()) => {}
                Err(e) if e.matches(gio::IOErrorEnum::AlreadyMounted) => {}
                Err(e) => return Err(e),
            }
            file.enumerate_children_future(ATTRS, gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT).await?
        }
        Err(err) => return Err(err),
    };
    let mut out = vec![];
    loop {
        let infos = e.next_files_future(64, glib::Priority::DEFAULT).await?;
        if infos.is_empty() {
            break;
        }
        for i in infos {
            out.push(Entry {
                name: i.name().to_string_lossy().into_owned(),
                display: i.display_name().to_string(),
                folder: i.file_type() == gio::FileType::Directory,
                content_type: i.content_type().map(|t| t.to_string()).unwrap_or_default(),
                size: if i.has_attribute("standard::size") { i.size().max(0) as u64 } else { 0 },
            });
        }
    }
    Ok(sorted(out))
}

/// Why a listing didn't work, in words (Taildrive's policy, when it's that).
pub fn why(e: &glib::Error) -> String {
    if e.matches(gio::IOErrorEnum::PermissionDenied) {
        return "Taildrive refused it: the tailnet's access controls have to give this machine drive:access, and the other one drive:share".into();
    }
    if e.matches(gio::IOErrorEnum::NotSupported) {
        return "This machine can't read Taildrive (it needs gvfs's WebDAV backend)".into();
    }
    if e.matches(gio::IOErrorEnum::NotFound) {
        return "It isn't there (any more)".into();
    }
    e.message().to_string()
}

/// Opens the file at `uri` with its default app.
pub fn open(uri: &str, done: impl FnOnce(Result<(), glib::Error>) + 'static) {
    let ctx = gtk::gdk::Display::default().map(|d| d.app_launch_context());
    let uri = uri.to_string();
    glib::spawn_future_local(async move {
        done(gio::AppInfo::launch_default_for_uri_future(&uri, ctx.as_ref()).await);
    });
}

/// A size, as the spike says it.
pub fn bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let f = n as f64;
    if f < K {
        format!("{n} B")
    } else if f < K * K {
        format!("{:.0} KB", f / K)
    } else if f < K * K * K {
        if f < 10.0 * K * K { format!("{:.1} MB", f / K / K) } else { format!("{:.0} MB", f / K / K) }
    } else {
        format!("{:.2} GB", f / K / K / K)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machines_home_is_where_taildrive_serves_it() {
        assert_eq!(
            home_uri(TAILDRIVE, "tail1234.ts.net", "desk.tail1234.ts.net", "desk"),
            "dav://100.100.100.100:8080/tail1234.ts.net/desk/home"
        );
        // A personal tailnet is named by its login: its @ is encoded, as wadd's rule has it.
        assert_eq!(
            home_uri(TAILDRIVE, "you@example.com", "desk.you.ts.net", "desk"),
            "dav://100.100.100.100:8080/you%40example.com/desk/home"
        );
        // The machine is its MagicDNS name's first label (lowercase), its hostname without one.
        assert_eq!(
            home_uri(TAILDRIVE, "t", "Living-Room.t.ts.net", "x"),
            "dav://100.100.100.100:8080/t/living-room/home"
        );
        assert_eq!(home_uri(TAILDRIVE, "t", "", "desk"), "dav://100.100.100.100:8080/t/desk/home");
        let home = home_uri("file:///tmp/drive", "tail1234.ts.net", "desk.tail1234.ts.net", "desk");
        assert_eq!(home, "file:///tmp/drive/tail1234.ts.net/desk/home");
        assert_eq!(
            folder_uri(&home, &["Pictures".into(), "My sprites".into()]),
            "file:///tmp/drive/tail1234.ts.net/desk/home/Pictures/My%20sprites"
        );
    }

    #[test]
    fn folders_come_first_and_dotfiles_stay_hidden() {
        let e = |name: &str, folder: bool, t: &str| Entry {
            name: name.into(),
            display: name.into(),
            folder,
            content_type: t.into(),
            size: 0,
        };
        let s = sorted(vec![
            e("notes.txt", false, "text/plain"),
            e("Pictures", true, ""),
            e(".cache", true, ""),
            e("art.png", false, "image/png"),
            e("Documents", true, ""),
        ]);
        let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Documents", "Pictures", "art.png", "notes.txt"]);
        assert_eq!((s[0].icon(), s[2].icon(), s[3].icon()), ("folder", "image", "file-text"));
        assert_eq!(bytes(3_565_158), "3.4 MB");
    }
}
