//! In-memory drop store: `Mutex<HashMap<Token, Drop>>`, bounded by a total
//! byte cap, expiring by TTL. No disk, no SQLite — a drop lives ten minutes
//! and a restart mid-handoff costs one rescan (docs/design/self-hosting-
//! relay.md §5).
//!
//! One-shot is atomic under the lock: `take` removes the entry in the same
//! critical section that reads it, which is the property the Worker gets from
//! a single-threaded Durable Object. `peek` never mutates a live entry;
//! `delete` is idempotent. Expiry is checked on every read AND swept by a
//! background task (`sweep`) so an unread drop does not hold memory until
//! someone asks about it — the memory cap is what makes the sweep matter.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A valid token is exactly what `generate_token` produces: 22 base64url
/// characters (16 random bytes, unpadded). Same grammar as the Worker.
pub fn is_valid_token(token: &str) -> bool {
    token.len() == 22
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Base64url (RFC 4648 §5), no padding — 16 bytes → 22 characters.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).map_or(0, |b| u32::from(*b));
        let b2 = chunk.get(2).map_or(0, |b| u32::from(*b));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 63) as usize] as char);
        }
    }
    out
}

/// A fresh drop token: 128 random bits, base64url.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS randomness unavailable");
    base64url(&bytes)
}

struct Entry {
    bytes: Vec<u8>,
    expires_at: Instant,
}

struct Inner {
    drops: HashMap<String, Entry>,
    /// Sum of `bytes.len()` over `drops` — maintained on every insert/remove
    /// so `remaining` is O(1).
    total: usize,
}

/// Why a `put` was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum PutError {
    /// Storing this drop would exceed the total memory cap. Fail closed:
    /// the caller answers `503 {"error":"relay full"}` + `Retry-After`.
    Full,
}

pub struct Store {
    inner: Mutex<Inner>,
    max_total_bytes: usize,
    ttl: Duration,
}

impl Store {
    pub fn new(max_total_bytes: usize, ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner {
                drops: HashMap::new(),
                total: 0,
            }),
            max_total_bytes,
            ttl,
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // A poisoned mutex means a panic mid-mutation on another thread; the
        // map is still a valid HashMap (every mutation here is a single insert
        // or remove plus a total adjustment), so recovering is safe and beats
        // taking the whole relay down for a bug in one request.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Bytes still available under the cap — what a declared `Content-Length`
    /// is checked against before a body is read.
    pub fn remaining(&self) -> usize {
        let inner = self.lock();
        self.max_total_bytes.saturating_sub(inner.total)
    }

    /// Stores `bytes` under a fresh token; `Err(Full)` if it would exceed
    /// the cap. Expired entries are swept first so a full-looking store that
    /// is only holding dead drops still accepts.
    pub fn put(&self, bytes: Vec<u8>) -> Result<String, PutError> {
        let now = Instant::now();
        let mut inner = self.lock();
        Self::sweep_locked(&mut inner, now);
        if inner.total.saturating_add(bytes.len()) > self.max_total_bytes {
            return Err(PutError::Full);
        }
        // Loop on the practically-impossible 128-bit collision rather than
        // silently overwriting a live drop.
        let token = loop {
            let t = generate_token();
            if !inner.drops.contains_key(&t) {
                break t;
            }
        };
        inner.total += bytes.len();
        inner.drops.insert(
            token.clone(),
            Entry {
                bytes,
                expires_at: now + self.ttl,
            },
        );
        Ok(token)
    }

    /// One-shot read: removes and returns the drop, or `None` if it is
    /// unknown, already taken, or expired (an expired entry is dropped on
    /// the way out). Atomic under the lock — of two concurrent `take`s for
    /// one token exactly one gets `Some`.
    pub fn take(&self, token: &str) -> Option<Vec<u8>> {
        let now = Instant::now();
        let mut inner = self.lock();
        let entry = inner.drops.remove(token)?;
        inner.total -= entry.bytes.len();
        if entry.expires_at <= now {
            return None;
        }
        Some(entry.bytes)
    }

    /// Non-consuming existence check: `true` iff a live, unexpired drop is
    /// behind `token`. Never removes a live entry (an expired one is swept).
    pub fn peek(&self, token: &str) -> bool {
        let now = Instant::now();
        let mut inner = self.lock();
        match inner.drops.get(token) {
            None => false,
            Some(entry) if entry.expires_at <= now => {
                let dead = inner.drops.remove(token).expect("just found");
                inner.total -= dead.bytes.len();
                false
            }
            Some(_) => true,
        }
    }

    /// Deletes if present — idempotent, no answer either way.
    pub fn delete(&self, token: &str) {
        let mut inner = self.lock();
        if let Some(entry) = inner.drops.remove(token) {
            inner.total -= entry.bytes.len();
        }
    }

    /// Drops every expired entry. Called by the background sweeper and at
    /// the top of `put`.
    pub fn sweep(&self) {
        let mut inner = self.lock();
        Self::sweep_locked(&mut inner, Instant::now());
    }

    fn sweep_locked(inner: &mut Inner, now: Instant) {
        let mut freed = 0usize;
        inner.drops.retain(|_, entry| {
            if entry.expires_at <= now {
                freed += entry.bytes.len();
                false
            } else {
                true
            }
        });
        inner.total -= freed;
    }

    /// Live drop count — for tests and the shutdown log line only.
    pub fn len(&self) -> usize {
        self.lock().drops.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_grammar() {
        let t = generate_token();
        assert_eq!(t.len(), 22);
        assert!(is_valid_token(&t));
        assert_ne!(generate_token(), generate_token());
        assert!(!is_valid_token(""));
        assert!(!is_valid_token("short"));
        assert!(!is_valid_token(&"a".repeat(23)));
        assert!(!is_valid_token("../../etc/passwd"));
        assert!(!is_valid_token("has spaces xxxxxxxxxx"));
        assert!(!is_valid_token("has.dots.xxxxxxxxxxxx"));
        assert!(!is_valid_token("has+plus+xxxxxxxxxxxx"));
    }

    #[test]
    fn base64url_matches_the_worker_encoding() {
        // The Worker's toBase64Url(new Uint8Array(16).fill(255)) is 22 chars of
        // '_' except the last, which encodes 4 leftover bits ('w').
        assert_eq!(base64url(&[255u8; 16]), "_____________________w");
        assert_eq!(base64url(&[0u8; 16]), "AAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(base64url(&[]), "");
    }

    #[test]
    fn put_take_is_one_shot() {
        let store = Store::new(1024, Duration::from_secs(60));
        let token = store.put(vec![1, 2, 3]).unwrap();
        assert!(store.peek(&token));
        assert_eq!(store.take(&token), Some(vec![1, 2, 3]));
        assert_eq!(store.take(&token), None);
        assert!(!store.peek(&token));
        assert_eq!(store.remaining(), 1024);
    }

    #[test]
    fn peek_does_not_consume_and_delete_is_idempotent() {
        let store = Store::new(1024, Duration::from_secs(60));
        let token = store.put(vec![9]).unwrap();
        assert!(store.peek(&token));
        assert!(store.peek(&token));
        assert_eq!(store.take(&token), Some(vec![9]));
        store.delete(&token);
        store.delete(&token);
        store.delete("nope");
        assert_eq!(store.remaining(), 1024);
    }

    #[test]
    fn total_cap_is_enforced_and_freed() {
        let store = Store::new(10, Duration::from_secs(60));
        let a = store.put(vec![0; 6]).unwrap();
        assert_eq!(store.remaining(), 4);
        assert_eq!(store.put(vec![0; 5]), Err(PutError::Full));
        let b = store.put(vec![0; 4]).unwrap();
        assert_eq!(store.remaining(), 0);
        assert_eq!(store.put(vec![0; 1]), Err(PutError::Full));
        store.delete(&a);
        assert_eq!(store.remaining(), 6);
        assert!(store.put(vec![0; 6]).is_ok());
        assert_eq!(store.take(&b).map(|v| v.len()), Some(4));
    }

    #[test]
    fn expiry_hides_and_frees() {
        let store = Store::new(1024, Duration::from_millis(30));
        let token = store.put(vec![7; 100]).unwrap();
        assert!(store.peek(&token));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!store.peek(&token));
        assert_eq!(store.remaining(), 1024);
        let token2 = store.put(vec![7; 100]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(store.take(&token2), None);
        assert_eq!(store.remaining(), 1024);
    }

    #[test]
    fn sweep_frees_expired_without_touching_live() {
        let store = Store::new(1024, Duration::from_millis(30));
        let dead = store.put(vec![1; 10]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        let live = store.put(vec![2; 10]).unwrap();
        store.sweep();
        assert_eq!(store.len(), 1);
        assert!(!store.peek(&dead));
        assert!(store.peek(&live));
        assert_eq!(store.remaining(), 1014);
    }

    #[test]
    fn concurrent_takes_yield_exactly_one() {
        use std::sync::Arc;
        let store = Arc::new(Store::new(1 << 20, Duration::from_secs(60)));
        for _ in 0..50 {
            let token = store.put(vec![1; 64]).unwrap();
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let store = Arc::clone(&store);
                    let token = token.clone();
                    std::thread::spawn(move || store.take(&token).is_some())
                })
                .collect();
            let wins = handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|won| *won)
                .count();
            assert_eq!(wins, 1);
        }
    }
}
