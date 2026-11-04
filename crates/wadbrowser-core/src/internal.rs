//! Built-in `cactus://` pages rendered as data URIs / HTML strings.

use crate::prefs::Preferences;
use crate::project::{CACTUS_COMMIT, CACTUS_LICENSE, CACTUS_REPO, CACTUS_VERSION};
use crate::store::{Bookmark, HistoryEntry};

pub fn is_internal(uri: &str) -> bool {
    uri.starts_with("cactus://") || uri.starts_with("about:")
}

pub fn map_about(uri: &str) -> Option<&'static str> {
    match uri {
        "about:blank" => Some("cactus://newtab"),
        "about:home" => Some("cactus://newtab"),
        "about:preferences" | "about:settings" => Some("cactus://settings"),
        _ => None,
    }
}

pub fn newtab_html(
    bookmarks: &[Bookmark],
    history: &[HistoryEntry],
    blocked_count: u64,
    cosmetic_count: u64,
) -> String {
    let mut bookmarks_html = String::new();
    for b in bookmarks.iter().take(8) {
        bookmarks_html.push_str(&format!(
            r#"<a class="chip" href="{}"><span>{}</span></a>"#,
            esc_attr(&b.url),
            esc_html(&b.title)
        ));
    }
    if bookmarks_html.is_empty() {
        bookmarks_html = r#"<p class="muted">Pin sites with Ctrl+D</p>"#.into();
    }

    let mut history_html = String::new();
    for h in history.iter().take(8) {
        history_html.push_str(&format!(
            r#"<a class="row" href="{}"><strong>{}</strong><span>{}</span></a>"#,
            esc_attr(&h.url),
            esc_html(&h.title),
            esc_html(&h.url)
        ));
    }
    if history_html.is_empty() {
        history_html = r#"<p class="muted">Your recent pages will show up here</p>"#.into();
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>New Tab</title>
<style>
  :root {{
    --bg0: #0f1a14;
    --bg1: #173024;
    --ink: #e7f2ea;
    --muted: #9bb5a6;
    --accent: #3ecf8e;
    --accent2: #c6f54c;
    --card: rgba(255,255,255,0.04);
  }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0; min-height: 100vh;
    font-family: "Segoe UI", "Ubuntu", "Cantarell", sans-serif;
    color: var(--ink);
    background:
      radial-gradient(1200px 600px at 10% -10%, #1f4d38 0%, transparent 55%),
      radial-gradient(900px 500px at 100% 0%, #2a3d18 0%, transparent 50%),
      linear-gradient(160deg, var(--bg0), var(--bg1));
  }}
  main {{
    max-width: 720px; margin: 0 auto; padding: 12vh 24px 48px;
  }}
  .brand {{
    font-size: clamp(2.8rem, 8vw, 4.5rem);
    font-weight: 750; letter-spacing: -0.04em; margin: 0 0 8px;
  }}
  .brand span {{ color: var(--accent); }}
  .tag {{
    color: var(--muted); margin: 0 0 32px; font-size: 1.05rem;
  }}
  form {{
    display: flex; gap: 8px; margin-bottom: 40px;
  }}
  input {{
    flex: 1; border: 1px solid rgba(255,255,255,0.12);
    background: var(--card); color: var(--ink);
    border-radius: 14px; padding: 14px 18px; font-size: 1rem;
    outline: none;
  }}
  input:focus {{ border-color: var(--accent); box-shadow: 0 0 0 3px rgba(62,207,142,0.2); }}
  button {{
    border: 0; border-radius: 14px; padding: 0 20px;
    background: linear-gradient(135deg, var(--accent), #2aa86f);
    color: #062015; font-weight: 700; cursor: pointer;
  }}
  h2 {{ font-size: 0.85rem; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted); }}
  .chips {{ display: flex; flex-wrap: wrap; gap: 8px; margin-bottom: 28px; }}
  .chip {{
    display: inline-flex; padding: 10px 14px; border-radius: 999px;
    background: var(--card); color: var(--ink); text-decoration: none;
    border: 1px solid rgba(255,255,255,0.08);
  }}
  .chip:hover {{ border-color: var(--accent2); }}
  .list {{ display: grid; gap: 6px; }}
  .row {{
    display: grid; gap: 2px; padding: 12px 14px; border-radius: 12px;
    background: var(--card); color: inherit; text-decoration: none;
    border: 1px solid transparent;
  }}
  .row:hover {{ border-color: rgba(62,207,142,0.35); }}
  .row span {{ color: var(--muted); font-size: 0.85rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
  .muted {{ color: var(--muted); }}
  .privacy-strip {{ font-size: 0.9rem; margin: -16px 0 28px; }}
  .privacy-strip a {{ color: var(--accent); }}
  .site-foot {{
    margin-top: 48px; padding-top: 20px; border-top: 1px solid rgba(255,255,255,0.08);
    font-size: 0.88rem; color: var(--muted); display: flex; flex-wrap: wrap; gap: 8px 16px;
  }}
  .site-foot a {{ color: var(--accent); text-decoration: none; }}
  .site-foot a:hover {{ text-decoration: underline; }}
</style>
</head>
<body>
<main>
  <h1 class="brand">Wad<span>Browser</span></h1>
  <p class="tag">Native Linux browsing. Quiet by default.</p>
  <p class="privacy-strip"><a href="cactus://privacy">Privacy this session</a>: {blocked} blocked requests · {cosmetic} cosmetic rules applied</p>
  <form action="https://duckduckgo.com/" method="get" role="search">
    <input name="q" autofocus placeholder="Search or enter address" autocomplete="off"/>
    <button type="submit">Go</button>
  </form>
  <h2>Bookmarks</h2>
  <div class="chips">{bookmarks_html}</div>
  <h2>Recent</h2>
  <div class="list">{history_html}</div>
  {site_footer}
</main>
<script>
document.querySelector('form').addEventListener('submit', (e) => {{
  const q = new FormData(e.target).get('q');
  if (!q) {{ e.preventDefault(); return; }}
  // Prefer letting the browser chrome handle search; still works standalone.
}});
</script>
</body>
</html>"#,
        blocked = blocked_count,
        cosmetic = cosmetic_count,
        site_footer = site_footer_html(),
    )
}

/// Session privacy dashboard (`cactus://privacy`).
pub fn privacy_html(
    prefs: &Preferences,
    blocked: u64,
    allowed: u64,
    cosmetic: u64,
    filter_engine_ready: bool,
    allowlist: &[String],
) -> String {
    let mut allowlist_rows = String::new();
    for origin in allowlist {
        allowlist_rows.push_str(&format!(
            r#"<div class="row"><strong>{}</strong><span>Tracking protection disabled</span></div>"#,
            esc_html(origin)
        ));
    }
    if allowlist_rows.is_empty() {
        allowlist_rows = r#"<p class="muted">No sites on your allowlist.</p>"#.into();
    }

    let body = format!(
        r#"
<p class="muted">Counters reflect this browser session only. Nothing is sent to WadBrowser or third parties.</p>
<div class="metrics">
  <div class="metric"><span class="num">{blocked}</span><span class="lbl">Network blocks</span></div>
  <div class="metric"><span class="num">{allowed}</span><span class="lbl">Allowed requests</span></div>
  <div class="metric"><span class="num">{cosmetic}</span><span class="lbl">Cosmetic rules</span></div>
</div>
<div class="card">
  <h2>Protection</h2>
  <label><span>Block ads &amp; trackers</span><span>{block}</span></label>
  <label><span>Cosmetic filtering</span><span>{cosmetic_on}</span></label>
  <label><span>Filter lists loaded</span><span>{engine}</span></label>
  <label><span>HTTPS-First</span><span>{https}</span></label>
  <label><span>Do Not Track</span><span>{dnt}</span></label>
  <label><span>Global Privacy Control</span><span>{gpc}</span></label>
</div>
<h2>Allowlist</h2>
<div class="list">{allowlist_rows}</div>
<p class="muted">Allow a site from the app menu when blocking breaks a page you trust. Stored in <code>~/.local/share/wadbrowser/wadbrowser.db</code>.</p>
<p><a href="cactus://settings">Open settings</a> · <a href="cactus://newtab">New tab</a> · <a href="cactus://about">About WadBrowser</a></p>
{site_footer}
"#,
        blocked = blocked,
        allowed = allowed,
        cosmetic = cosmetic,
        block = prefs.block_ads_trackers,
        cosmetic_on = prefs.block_cosmetic_filters,
        engine = if filter_engine_ready { "yes" } else { "no" },
        https = prefs.https_first,
        dnt = prefs.send_dnt,
        gpc = prefs.send_gpc,
        site_footer = site_footer_html(),
    );

    privacy_shell_page(&body)
}

fn privacy_shell_page(body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"/><title>Privacy</title>
<style>
body{{margin:0;font-family:Ubuntu,Cantarell,sans-serif;background:#0f1411;color:#e8efe9}}
main{{max-width:800px;margin:32px auto;padding:0 20px}}
h1{{letter-spacing:-0.03em;font-size:1.75rem}}
h2{{font-size:0.95rem;margin:0 0 12px;font-weight:600}}
.muted{{color:#93a59a;font-size:0.92rem;line-height:1.55}}
.metrics{{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:12px;margin:20px 0 28px}}
.metric{{background:rgba(255,255,255,0.04);border:1px solid rgba(255,255,255,0.08);border-radius:12px;padding:16px;text-align:center}}
.metric .num{{display:block;font-size:1.75rem;font-weight:700;color:#3ecf8e;letter-spacing:-0.02em}}
.metric .lbl{{font-size:0.78rem;color:#93a59a}}
.card{{background:rgba(255,255,255,0.04);border:1px solid rgba(255,255,255,0.08);border-radius:12px;padding:16px 18px;margin:16px 0 24px}}
.card label{{display:flex;justify-content:space-between;padding:8px 0;border-bottom:1px solid rgba(255,255,255,0.06);font-size:0.92rem}}
.card label:last-child{{border:0}}
.list{{display:grid;gap:8px;margin-bottom:16px}}
.row{{display:grid;gap:2px;padding:12px 14px;border-radius:12px;background:rgba(255,255,255,0.04);border:1px solid rgba(255,255,255,0.06)}}
.row span{{color:#93a59a;font-size:0.85rem}}
a{{color:#3ecf8e}}
code{{background:#1b2420;padding:2px 6px;border-radius:4px;font-size:0.85em}}
.site-foot{{margin-top:28px;padding-top:16px;border-top:1px solid rgba(255,255,255,0.08);font-size:0.88rem;color:#93a59a;display:flex;flex-wrap:wrap;gap:8px 14px}}
.site-foot a{{color:#3ecf8e;text-decoration:none}}
</style></head>
<body><main><h1>Privacy dashboard</h1>{body}</main></body></html>"#,
        body = body,
    )
}

pub fn settings_html(prefs: &Preferences) -> String {
    let engine = match &prefs.search_engine {
        crate::prefs::SearchEngine::DuckDuckGo => "DuckDuckGo",
        crate::prefs::SearchEngine::Startpage => "Startpage",
        crate::prefs::SearchEngine::Wikipedia => "Wikipedia",
        crate::prefs::SearchEngine::Custom { name, .. } => name.as_str(),
    };
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"/><title>Settings</title>
<style>
body{{font-family:Ubuntu,Cantarell,sans-serif;margin:0;background:#111;color:#eee}}
main{{max-width:720px;margin:40px auto;padding:0 20px}}
h1{{letter-spacing:-0.03em}}
.card{{background:#1a1a1a;border:1px solid #333;border-radius:12px;padding:16px 18px;margin:12px 0}}
label{{display:flex;justify-content:space-between;padding:8px 0;border-bottom:1px solid #2a2a2a}}
label:last-child{{border:0}}
.muted{{color:#999}}
code{{background:#222;padding:2px 6px;border-radius:4px}}
.site-foot{{margin-top:28px;padding-top:16px;border-top:1px solid #333;font-size:0.88rem;color:#999;display:flex;flex-wrap:wrap;gap:8px 14px}}
.site-foot a{{color:#3ecf8e;text-decoration:none}}
</style></head>
<body><main>
<h1>Settings</h1>
<p class="muted">Preferences also live in <code>~/.config/wadbrowser/prefs.toml</code>. Toggle privacy options from the app menu for immediate effect.</p>
<div class="card">
<label><span>Homepage</span><span>{homepage}</span></label>
<label><span>Search engine</span><span>{engine}</span></label>
<label><span>Restore session</span><span>{restore}</span></label>
<label><span>HTTPS-First</span><span>{https}</span></label>
<label><span>Block ads &amp; trackers</span><span>{block}</span></label>
<label><span>Send DNT</span><span>{dnt}</span></label>
<label><span>Send GPC</span><span>{gpc}</span></label>
</div>
<div class="card">
<p>Shortcuts: Ctrl+T new tab · Ctrl+W close · Ctrl+L focus URL · Ctrl+Shift+N private window · Ctrl+Shift+P privacy dashboard · Ctrl+D bookmark · Ctrl+H history · Ctrl+J downloads · Ctrl+F find</p>
</div>
{site_footer}
</main></body></html>"#,
        homepage = esc_html(&prefs.homepage),
        engine = esc_html(engine),
        restore = prefs.restore_session,
        https = prefs.https_first,
        block = prefs.block_ads_trackers,
        dnt = prefs.send_dnt,
        gpc = prefs.send_gpc,
        site_footer = site_footer_html(),
    )
}

pub fn history_html(entries: &[HistoryEntry]) -> String {
    let mut rows = String::new();
    for e in entries {
        rows.push_str(&format!(
            r#"<a class="row" href="{url}"><strong>{title}</strong><span>{url}</span></a>"#,
            url = esc_attr(&e.url),
            title = esc_html(if e.title.is_empty() { &e.url } else { &e.title }),
        ));
    }
    if rows.is_empty() {
        rows = r#"<p class="muted">No history yet.</p>"#.into();
    }
    shell_page("History", &format!(r#"<div class="list">{rows}</div>"#))
}

pub fn bookmarks_html(entries: &[Bookmark]) -> String {
    let mut rows = String::new();
    for e in entries {
        rows.push_str(&format!(
            r#"<a class="row" href="{url}"><strong>{title}</strong><span>{url}</span></a>"#,
            url = esc_attr(&e.url),
            title = esc_html(&e.title),
        ));
    }
    if rows.is_empty() {
        rows = r#"<p class="muted">No bookmarks yet. Press Ctrl+D to add one.</p>"#.into();
    }
    shell_page("Bookmarks", &format!(r#"<div class="list">{rows}</div>"#))
}

pub fn downloads_html(entries: &[crate::store::DownloadRecord]) -> String {
    let mut rows = String::new();
    for e in entries {
        rows.push_str(&format!(
            r#"<div class="row"><strong>{}</strong><span>{} · {}</span></div>"#,
            esc_html(&e.filename),
            esc_html(&e.url),
            esc_html(&e.status),
        ));
    }
    if rows.is_empty() {
        rows = r#"<p class="muted">No downloads yet.</p>"#.into();
    }
    shell_page("Downloads", &format!(r#"<div class="list">{rows}</div>"#))
}

/// `cactus://about`: WadBrowser, what it's based on, its WebKit and its licence.
pub fn about_html(webkit_version: &str) -> String {
    shell_page(
        crate::APP_NAME,
        &format!(
            r#"
<p>{name} {version}, the web browser of WadOS.</p>
<p>Based on <a href="{repo}">Cactus</a> {cactus_version} (commit <code>{commit}</code>).</p>
<p class="muted">WebKitGTK {webkit}</p>
<p class="muted">Licence: {license} (Mozilla Public License 2.0)</p>
"#,
            name = crate::APP_NAME,
            version = crate::APP_VERSION,
            repo = CACTUS_REPO,
            cactus_version = CACTUS_VERSION,
            commit = CACTUS_COMMIT,
            webkit = esc_html(webkit_version),
            license = CACTUS_LICENSE,
        ),
    )
}

pub fn error_html(title: &str, message: &str, uri: &str) -> String {
    shell_page(
        title,
        &format!(
            r#"
<p>{msg}</p>
<p class="muted">Failed URL: <code>{uri}</code></p>
<p><a href="{uri}">Try again</a> · <a href="cactus://newtab">New tab</a></p>
"#,
            msg = esc_html(message),
            uri = esc_attr(uri),
        ),
    )
}

fn shell_page(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"/><title>{title}</title>
<style>
body{{margin:0;font-family:Ubuntu,Cantarell,sans-serif;background:#0f1411;color:#e8efe9}}
main{{max-width:800px;margin:32px auto;padding:0 20px}}
h1{{letter-spacing:-0.03em}}
.muted{{color:#93a59a}}
.list{{display:grid;gap:8px}}
.row{{display:grid;gap:2px;padding:12px 14px;border-radius:12px;background:rgba(255,255,255,0.04);color:inherit;text-decoration:none;border:1px solid rgba(255,255,255,0.06)}}
.row:hover{{border-color:#3ecf8e}}
.row span{{color:#93a59a;font-size:0.85rem;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}
a{{color:#3ecf8e}}
code{{background:#1b2420;padding:2px 6px;border-radius:4px}}
.site-foot{{margin-top:28px;padding-top:16px;border-top:1px solid rgba(255,255,255,0.08);font-size:0.88rem;color:#93a59a;display:flex;flex-wrap:wrap;gap:8px 14px}}
.site-foot a{{color:#3ecf8e;text-decoration:none}}
</style></head>
<body><main><h1>{title}</h1>{body}{site_footer}</main></body></html>"#,
        title = esc_html(title),
        body = body,
        site_footer = site_footer_html(),
    )
}

fn site_footer_html() -> String {
    r#"<nav class="site-foot" aria-label="Project">
<a href="cactus://about">About</a>
<a href="cactus://privacy">Privacy</a>
</nav>"#
        .to_string()
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn esc_attr(s: &str) -> String {
    esc_html(s).replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::Preferences;

    #[test]
    fn privacy_page_includes_session_counts() {
        let html = privacy_html(&Preferences::default(), 42, 1000, 7, true, &[]);
        assert!(html.contains("Privacy dashboard"));
        assert!(html.contains("42"));
        assert!(html.contains("1000"));
        assert!(html.contains("Filter lists loaded"));
    }

    #[test]
    fn about_page_credits_cactus() {
        let html = about_html("2.54.2");
        assert!(html.contains("WadBrowser"));
        assert!(html.contains("github.com/yohannes-mesay/cactus-browser"));
        assert!(html.contains("8ae5a66"));
        assert!(html.contains("2.54.2"));
        assert!(html.contains("MPL-2.0"));
        assert!(!html.contains("cactus-browser.tech"));
    }
}
