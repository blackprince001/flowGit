//! GitHub avatars and Markdown images, fetched once on a worker thread and kept
//! in memory. Until an image arrives (or when avatars are off) views draw
//! initials or the image's alt text instead.
use gpui::{Global, Image, ImageFormat};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// Where a Markdown image lives.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Url(String),
    /// A file in a repository, by relative path from a README.
    Repo { slug: String, reference: String, path: String },
}

enum Slot {
    Loading,
    Ready(Arc<Image>),
    Missing,
}

#[derive(Clone, Default)]
pub struct Avatars {
    slots: Arc<Mutex<HashMap<String, Slot>>>,
    /// Set when an image lands, so the workspace knows to repaint.
    pub dirty: Arc<AtomicBool>,
}

impl Global for Avatars {}

fn format(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [0xFF, 0xD8, ..] => Some(ImageFormat::Jpeg),
        [b'G', b'I', b'F', ..] => Some(ImageFormat::Gif),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some(ImageFormat::Webp),
        // Badges (shields.io and the like) are SVG.
        _ if is_svg(bytes) => Some(ImageFormat::Svg),
        _ => None,
    }
}
fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

/// Images GitHub hosts may sit behind sign-in (private repositories, PR
/// attachments); only those hosts are ever sent the token.
fn github_host(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    host == "github.com" || host.ends_with(".githubusercontent.com")
}
fn http_get(url: &str) -> Option<Vec<u8>> {
    static TOKEN: OnceLock<Option<String>> = OnceLock::new();
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("flowgit-desktop")
        .build()
        .ok()?;
    let mut req = client.get(url);
    if github_host(url)
        && let Some(token) = TOKEN.get_or_init(crate::gh::token)
    {
        req = req.header("Authorization", format!("token {token}"));
    }
    let bytes = req.send().and_then(|r| r.error_for_status()).and_then(|r| r.bytes()).ok()?;
    Some(bytes.to_vec())
}
/// Images larger than this are left as their alt text.
const MAX_IMAGE: usize = 20 << 20;

impl Avatars {
    /// The avatar for `login`, starting a download the first time it is asked for.
    pub fn get(&self, login: &str) -> Option<Arc<Image>> {
        if login.is_empty() {
            return None;
        }
        let mut slots = self.slots.lock().ok()?;
        match slots.get(login) {
            Some(Slot::Ready(img)) => return Some(img.clone()),
            Some(_) => return None,
            None => {}
        }
        slots.insert(login.to_string(), Slot::Loading);
        drop(slots);
        let (slots, dirty, login) = (self.slots.clone(), self.dirty.clone(), login.to_string());
        std::thread::spawn(move || {
            let url = format!("https://avatars.githubusercontent.com/{login}?s=80");
            let image = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("flowgit-desktop")
                .build()
                .and_then(|c| c.get(url).send())
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.bytes())
                .ok()
                .and_then(|b| format(&b).map(|f| Arc::new(Image::from_bytes(f, b.to_vec()))));
            if let Ok(mut s) = slots.lock() {
                s.insert(login, image.map(Slot::Ready).unwrap_or(Slot::Missing));
            }
            dirty.store(true, Ordering::SeqCst);
        });
        None
    }
    /// A Markdown image, starting a download the first time it is asked for.
    /// `Err(())` means it could not be loaded (alt text is shown instead).
    pub fn image(&self, src: &Source) -> Result<Option<Arc<Image>>, ()> {
        let key = format!("img:{src:?}");
        let mut slots = self.slots.lock().map_err(|_| ())?;
        match slots.get(&key) {
            Some(Slot::Ready(img)) => return Ok(Some(img.clone())),
            Some(Slot::Loading) => return Ok(None),
            Some(Slot::Missing) => return Err(()),
            None => {}
        }
        slots.insert(key.clone(), Slot::Loading);
        drop(slots);
        let (slots, dirty, src) = (self.slots.clone(), self.dirty.clone(), src.clone());
        std::thread::spawn(move || {
            let bytes = match &src {
                Source::Url(url) if url.starts_with("https://") || url.starts_with("http://") => http_get(url),
                Source::Url(_) => None,
                Source::Repo { slug, reference, path } => crate::gh::raw_file(slug, path, reference).ok(),
            };
            let image = bytes
                .filter(|b| b.len() <= MAX_IMAGE)
                .and_then(|b| format(&b).map(|f| Arc::new(Image::from_bytes(f, b))));
            if let Ok(mut s) = slots.lock() {
                s.insert(key, image.map(Slot::Ready).unwrap_or(Slot::Missing));
            }
            dirty.store(true, Ordering::SeqCst);
        });
        Ok(None)
    }
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sniffs_image_formats() {
        use super::format;
        use gpui::ImageFormat;
        assert_eq!(format(&[0x89, b'P', b'N', b'G', 0]), Some(ImageFormat::Png));
        assert_eq!(format(&[0xFF, 0xD8, 0xFF]), Some(ImageFormat::Jpeg));
        assert_eq!(format(b"<html>"), None);
        assert_eq!(format(b"<svg xmlns=\"x\"></svg>"), Some(ImageFormat::Svg));
        assert_eq!(format(b"<?xml version=\"1.0\"?>\n<svg></svg>"), Some(ImageFormat::Svg));
    }
    #[test]
    fn token_only_goes_to_github() {
        use super::github_host;
        assert!(github_host("https://github.com/user-attachments/assets/x"));
        assert!(github_host("https://raw.githubusercontent.com/a/b/main/x.png"));
        assert!(!github_host("https://img.shields.io/badge/x"));
        assert!(!github_host("https://github.com.evil.example/x"));
        assert!(!github_host("http://github.com/x"));
    }
}
