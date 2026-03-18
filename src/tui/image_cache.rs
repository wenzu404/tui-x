use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Manages async image downloading and protocol encoding.
pub struct ImageCache {
    picker: Picker,
    /// Ready-to-render images keyed by URL.
    cache: HashMap<String, StatefulProtocol>,
    /// URLs currently being fetched.
    pending: std::collections::HashSet<String>,
    /// Channel to receive downloaded images.
    rx: mpsc::UnboundedReceiver<(String, Vec<u8>)>,
    tx: mpsc::UnboundedSender<(String, Vec<u8>)>,
    http: Arc<reqwest::Client>,
}

impl ImageCache {
    pub fn new(picker: Picker, http: Arc<reqwest::Client>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            picker,
            cache: HashMap::new(),
            pending: std::collections::HashSet::new(),
            rx,
            tx,
            http,
        }
    }

    /// Request an image to be fetched. Returns immediately.
    /// The image will be available via `get()` after it's downloaded and decoded.
    pub fn request(&mut self, url: &str) {
        if self.cache.contains_key(url) || self.pending.contains(url) {
            return;
        }

        self.pending.insert(url.to_string());
        let tx = self.tx.clone();
        let http = self.http.clone();
        let url_owned = url.to_string();

        tokio::spawn(async move {
            let Ok(resp) = http.get(&url_owned).send().await else {
                return;
            };
            let Ok(bytes) = resp.bytes().await else {
                return;
            };
            let _ = tx.send((url_owned, bytes.to_vec()));
        });
    }

    /// Process downloaded images (call every frame).
    pub fn drain(&mut self) {
        while let Ok((url, bytes)) = self.rx.try_recv() {
            self.pending.remove(&url);
            if let Ok(dyn_img) = image::load_from_memory(&bytes) {
                let protocol = self.picker.new_resize_protocol(dyn_img);
                self.cache.insert(url, protocol);
            }
        }
    }

    /// Get a ready-to-render image protocol. Returns None if not yet loaded.
    pub fn get(&mut self, url: &str) -> Option<&mut StatefulProtocol> {
        self.cache.get_mut(url)
    }

    /// Check if an image URL is cached and ready.
    pub fn is_ready(&self, url: &str) -> bool {
        self.cache.contains_key(url)
    }

    /// Request a thumbnail variant of a Twitter image URL (smaller for TUI).
    pub fn request_thumbnail(&mut self, url: &str) {
        // Twitter image URLs support size suffixes: ?format=jpg&name=small
        let thumb_url = if url.contains("pbs.twimg.com") {
            if url.contains("?") {
                format!("{url}&name=small")
            } else {
                format!("{url}?name=small")
            }
        } else {
            url.to_string()
        };
        self.request(&thumb_url);
    }

    /// Get thumbnail URL for a Twitter image.
    pub fn thumb_url(url: &str) -> String {
        if url.contains("pbs.twimg.com") {
            if url.contains("?") {
                format!("{url}&name=small")
            } else {
                format!("{url}?name=small")
            }
        } else {
            url.to_string()
        }
    }
}
