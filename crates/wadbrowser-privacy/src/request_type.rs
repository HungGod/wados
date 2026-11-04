//! Map coarse browser resource hints to adblock request types.

/// Best-effort classification used when WebKit does not expose a rich type.
pub fn classify(url: &str, mime_hint: Option<&str>) -> &'static str {
    let lower = url.to_ascii_lowercase();
    if let Some(mime) = mime_hint {
        let m = mime.to_ascii_lowercase();
        if m.starts_with("image/") {
            return "image";
        }
        if m.starts_with("font/") || m.contains("font") {
            return "font";
        }
        if m.starts_with("text/css") {
            return "stylesheet";
        }
        if m.contains("javascript") || m.contains("ecmascript") {
            return "script";
        }
        if m.starts_with("video/") || m.starts_with("audio/") {
            return "media";
        }
        if m.starts_with("text/html") {
            return "subdocument";
        }
    }

    if lower.contains(".js") || lower.contains("/js/") {
        return "script";
    }
    if lower.contains(".css") {
        return "stylesheet";
    }
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
        || lower.ends_with(".svg")
        || lower.ends_with(".ico")
    {
        return "image";
    }
    if lower.ends_with(".woff") || lower.ends_with(".woff2") || lower.ends_with(".ttf") {
        return "font";
    }
    if lower.ends_with(".mp4") || lower.ends_with(".webm") || lower.ends_with(".mp3") {
        return "media";
    }
    if lower.contains("/ad") || lower.contains("doubleclick") || lower.contains("googlesyndication") {
        return "xmlhttprequest";
    }
    "other"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_scripts_and_images() {
        assert_eq!(classify("https://cdn.example/app.js", None), "script");
        assert_eq!(classify("https://cdn.example/a.png", None), "image");
        assert_eq!(classify("https://x.test/x", Some("text/css; charset=utf-8")), "stylesheet");
    }
}
