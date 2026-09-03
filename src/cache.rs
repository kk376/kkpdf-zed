//! Memory-budgeted LRU cache for rasterized PDF page bitmaps.
//!
//! Stores rendered RGBA framebuffers indexed by `(page_index, zoom_bucket, dark_mode)`.
//! Automatically evicts oldest unused pages when memory consumption exceeds the configured budget.

use std::collections::HashMap;
use std::sync::Arc;

/// Default maximum memory budget: 256 Megabytes.
pub const DEFAULT_MEMORY_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Cache lookup key capturing page index, discrete zoom bucket, and dark-mode setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub page_index: usize,
    pub zoom_bucket: u32,
    pub dark_mode: bool,
}

impl CacheKey {
    /// Creates a new cache key with zoom discretized to 2 decimal places (e.g. 1.25 -> 125).
    pub fn new(page_index: usize, zoom_factor: f32, dark_mode: bool) -> Self {
        let zoom_bucket = (zoom_factor * 100.0).round().max(1.0) as u32;
        Self {
            page_index,
            zoom_bucket,
            dark_mode,
        }
    }
}

/// A fully rasterized PDF page holding raw RGBA pixel data.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub page_index: usize,
    pub width: u32,
    pub height: u32,
    pub zoom_factor: f32,
    pub dark_mode: bool,
    pub rgba_buffer: Arc<Vec<u8>>,
}

impl RenderedPage {
    /// Constructs a new RenderedPage.
    pub fn new(
        page_index: usize,
        width: u32,
        height: u32,
        zoom_factor: f32,
        dark_mode: bool,
        rgba_buffer: Vec<u8>,
    ) -> Self {
        Self {
            page_index,
            width,
            height,
            zoom_factor,
            dark_mode,
            rgba_buffer: Arc::new(rgba_buffer),
        }
    }

    /// Returns the exact memory footprint of this bitmap in bytes (width * height * 4).
    #[inline]
    pub fn byte_size(&self) -> usize {
        (self.width as usize) * (self.height as usize) * 4
    }
}

/// Internal node in the intrusive doubly-linked LRU chain.
#[derive(Debug)]
struct LruNode {
    key: CacheKey,
    page: RenderedPage,
    prev: Option<usize>,
    next: Option<usize>,
}

/// A Least-Recently-Used (LRU) cache bounded by byte size and item limits with O(1) updates and lookups.
#[derive(Debug)]
pub struct PageLruCache {
    max_memory_bytes: usize,
    current_memory_bytes: usize,
    max_pages: Option<usize>,
    entries: HashMap<CacheKey, usize>,
    nodes: Vec<Option<LruNode>>,
    free_slots: Vec<usize>,
    head: Option<usize>, // least recently used (oldest)
    tail: Option<usize>, // most recently used (newest)
}

impl PageLruCache {
    /// Creates a new cache with the specified memory budget in bytes.
    pub fn new(max_memory_bytes: usize) -> Self {
        Self {
            max_memory_bytes: max_memory_bytes.max(1024 * 1024), // Minimum 1MB
            current_memory_bytes: 0,
            max_pages: None,
            entries: HashMap::new(),
            nodes: Vec::new(),
            free_slots: Vec::new(),
            head: None,
            tail: None,
        }
    }

    /// Configures an optional hard ceiling on the maximum number of cached pages.
    pub fn with_max_pages(mut self, max_pages: usize) -> Self {
        self.max_pages = Some(max_pages);
        self
    }

    /// Retrieves a cached page, refreshing its LRU position in O(1) time.
    pub fn get(&mut self, key: &CacheKey) -> Option<RenderedPage> {
        if let Some(&idx) = self.entries.get(key) {
            self.touch(idx);
            self.nodes[idx].as_ref().map(|n| n.page.clone())
        } else {
            None
        }
    }

    /// Inserts a newly rasterized page into the cache in O(1) time, evicting older pages if needed.
    pub fn insert(&mut self, key: CacheKey, page: RenderedPage) {
        let page_size = page.byte_size();

        // If replacing existing entry, remove old size and node first
        if let Some(old_idx) = self.entries.remove(&key) {
            let (_, old_page) = self.free_node(old_idx);
            self.current_memory_bytes = self
                .current_memory_bytes
                .saturating_sub(old_page.byte_size());
        }

        // Evict until new page fits within budget
        self.evict_for_space(page_size);

        // If page is larger than max budget itself, we do not cache it
        if page_size > self.max_memory_bytes {
            log::warn!(
                "Rendered page size ({page_size} bytes) exceeds total cache budget ({max} bytes); skipping cache insertion",
                max = self.max_memory_bytes
            );
            return;
        }

        let idx = self.alloc_node(key, page);
        self.attach_tail(idx);
        self.entries.insert(key, idx);
        self.current_memory_bytes += page_size;
    }

    /// Evicts oldest entries until at least `needed_bytes` is available.
    fn evict_for_space(&mut self, needed_bytes: usize) {
        // Enforce page count limit if configured
        if let Some(max_pages) = self.max_pages {
            while self.entries.len() >= max_pages {
                if !self.evict_oldest() {
                    break;
                }
            }
        }

        // Enforce memory budget
        while self.current_memory_bytes + needed_bytes > self.max_memory_bytes {
            if !self.evict_oldest() {
                break;
            }
        }
    }

    /// Evicts the single least recently used item in O(1) time. Returns false if cache was empty.
    fn evict_oldest(&mut self) -> bool {
        if let Some(oldest_idx) = self.head {
            let (oldest_key, evicted_page) = self.free_node(oldest_idx);
            self.entries.remove(&oldest_key);
            self.current_memory_bytes = self
                .current_memory_bytes
                .saturating_sub(evicted_page.byte_size());
            log::debug!(
                "Evicted page {page_idx} (zoom: {zoom}) to free {bytes} bytes (current memory: {cur} / {max})",
                page_idx = oldest_key.page_index,
                zoom = oldest_key.zoom_bucket,
                bytes = evicted_page.byte_size(),
                cur = self.current_memory_bytes,
                max = self.max_memory_bytes
            );
            return true;
        }
        false
    }

    /// Unlinks a node from the doubly-linked list in O(1) time.
    fn unlink(&mut self, idx: usize) {
        let (prev, next) = match self.nodes.get(idx).and_then(|n| n.as_ref()) {
            Some(node) => (node.prev, node.next),
            None => return,
        };

        if let Some(p) = prev {
            if let Some(Some(p_node)) = self.nodes.get_mut(p) {
                p_node.next = next;
            }
        } else {
            self.head = next;
        }

        if let Some(n) = next {
            if let Some(Some(n_node)) = self.nodes.get_mut(n) {
                n_node.prev = prev;
            }
        } else {
            self.tail = prev;
        }

        if let Some(Some(node)) = self.nodes.get_mut(idx) {
            node.prev = None;
            node.next = None;
        }
    }

    /// Attaches an existing node to the tail (most recently used) in O(1) time.
    fn attach_tail(&mut self, idx: usize) {
        let old_tail = self.tail;
        if let Some(t) = old_tail {
            if let Some(Some(t_node)) = self.nodes.get_mut(t) {
                t_node.next = Some(idx);
            }
            if let Some(Some(node)) = self.nodes.get_mut(idx) {
                node.prev = Some(t);
                node.next = None;
            }
            self.tail = Some(idx);
        } else {
            if let Some(Some(node)) = self.nodes.get_mut(idx) {
                node.prev = None;
                node.next = None;
            }
            self.head = Some(idx);
            self.tail = Some(idx);
        }
    }

    /// Refreshes the position of a node, moving it to the tail (MRU) in O(1) time.
    fn touch(&mut self, idx: usize) {
        if self.tail == Some(idx) {
            return;
        }
        self.unlink(idx);
        self.attach_tail(idx);
    }

    /// Allocates or reuses a node slot in O(1) time.
    fn alloc_node(&mut self, key: CacheKey, page: RenderedPage) -> usize {
        let new_node = LruNode {
            key,
            page,
            prev: None,
            next: None,
        };

        if let Some(slot) = self.free_slots.pop() {
            self.nodes[slot] = Some(new_node);
            slot
        } else {
            let slot = self.nodes.len();
            self.nodes.push(Some(new_node));
            slot
        }
    }

    /// Frees a node slot, unlinking it and returning its payload in O(1) time.
    fn free_node(&mut self, idx: usize) -> (CacheKey, RenderedPage) {
        self.unlink(idx);
        let node = self
            .nodes
            .get_mut(idx)
            .and_then(|n| n.take())
            .expect("node must exist when freed");
        self.free_slots.push(idx);
        (node.key, node.page)
    }

    /// Clears all entries from the cache and resets memory tracking.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.nodes.clear();
        self.free_slots.clear();
        self.head = None;
        self.tail = None;
        self.current_memory_bytes = 0;
    }

    /// Number of pages currently cached.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the cache contains no pages.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Current memory usage in bytes.
    #[inline]
    pub fn memory_usage(&self) -> usize {
        self.current_memory_bytes
    }

    /// Maximum memory budget in bytes.
    #[inline]
    pub fn max_memory(&self) -> usize {
        self.max_memory_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_discretization() {
        let key1 = CacheKey::new(0, 1.004, false);
        let key2 = CacheKey::new(0, 1.001, false);
        let key3 = CacheKey::new(0, 1.250, false);
        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
        assert_eq!(key1.zoom_bucket, 100);
        assert_eq!(key3.zoom_bucket, 125);
    }

    #[test]
    fn test_lru_eviction_on_memory_budget() {
        // 1 MB cache budget
        let mut cache = PageLruCache::new(1024 * 1024);

        // Create 3 mock pages of 400 KB each (100x1000 RGBA = 400,000 bytes)
        let page_size_bytes = 400_000;
        let buf = vec![0u8; page_size_bytes];

        let page0 = RenderedPage::new(0, 100, 1000, 1.0, false, buf.clone());
        let page1 = RenderedPage::new(1, 100, 1000, 1.0, false, buf.clone());
        let page2 = RenderedPage::new(2, 100, 1000, 1.0, false, buf);

        let key0 = CacheKey::new(0, 1.0, false);
        let key1 = CacheKey::new(1, 1.0, false);
        let key2 = CacheKey::new(2, 1.0, false);

        // Insert page 0 (400 KB) -> total: 400 KB
        cache.insert(key0, page0);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.memory_usage(), 400_000);

        // Insert page 1 (400 KB) -> total: 800 KB
        cache.insert(key1, page1);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.memory_usage(), 800_000);

        // Insert page 2 (400 KB) -> 800 + 400 = 1200 KB > 1024 KB
        // Page 0 should be evicted!
        cache.insert(key2, page2);
        assert_eq!(cache.len(), 2);
        assert!(
            cache.get(&key0).is_none(),
            "Page 0 should have been evicted"
        );
        assert!(cache.get(&key1).is_some(), "Page 1 should still exist");
        assert!(cache.get(&key2).is_some(), "Page 2 should still exist");
    }

    #[test]
    fn test_lru_access_refresh() {
        let mut cache = PageLruCache::new(1024 * 1024);
        let buf = vec![0u8; 400_000];

        let key0 = CacheKey::new(0, 1.0, false);
        let key1 = CacheKey::new(1, 1.0, false);
        let key2 = CacheKey::new(2, 1.0, false);

        cache.insert(
            key0,
            RenderedPage::new(0, 100, 1000, 1.0, false, buf.clone()),
        );
        cache.insert(
            key1,
            RenderedPage::new(1, 100, 1000, 1.0, false, buf.clone()),
        );

        // Access page 0, making page 1 the oldest
        let _ = cache.get(&key0);

        // Insert page 2 -> page 1 should be evicted instead of page 0!
        cache.insert(key2, RenderedPage::new(2, 100, 1000, 1.0, false, buf));

        assert!(
            cache.get(&key0).is_some(),
            "Page 0 was accessed and should survive"
        );
        assert!(
            cache.get(&key1).is_none(),
            "Page 1 was oldest and should be evicted"
        );
        assert!(cache.get(&key2).is_some(), "Page 2 is newly inserted");
    }

    #[test]
    fn test_lru_key_replacement_and_max_pages() {
        let mut cache = PageLruCache::new(10 * 1024 * 1024).with_max_pages(2);
        let key = CacheKey::new(0, 1.0, false);
        let page_a = RenderedPage::new(0, 10, 10, 1.0, false, vec![0u8; 400]);
        let page_b = RenderedPage::new(0, 10, 10, 1.0, false, vec![1u8; 400]);

        cache.insert(key, page_a);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.memory_usage(), 400);

        // Overwrite key
        cache.insert(key, page_b);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.memory_usage(), 400);

        // Add 2 more keys to trigger max_pages limit
        let key1 = CacheKey::new(1, 1.0, false);
        let page1 = RenderedPage::new(1, 10, 10, 1.0, false, vec![0u8; 400]);
        cache.insert(key1, page1);
        assert_eq!(cache.len(), 2);

        let key2 = CacheKey::new(2, 1.0, false);
        let page2 = RenderedPage::new(2, 10, 10, 1.0, false, vec![0u8; 400]);
        cache.insert(key2, page2);
        assert_eq!(cache.len(), 2);

        // Key 0 was oldest, should be evicted
        assert!(cache.get(&key).is_none());
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_some());
    }
}
