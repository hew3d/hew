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
//!
//! The byte budget is the only limit on how much the store holds: every
//! buffer is charged at least [`MIN_DROP_CHARGE`], which bounds the entry
//! count too.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The least any buffer is charged against the byte budget, however small.
/// A stored drop costs memory besides its payload: the token string, its map
/// slot, the allocator's own bookkeeping — a few hundred bytes. Charging at
/// least a page covers that many times over, so the budget alone bounds the
/// entry count (`max_total_bytes / MIN_DROP_CHARGE`: 65,536 at the default
/// 256 MiB) and no separate, shared entry limit is needed. It is kept small
/// because a larger floor would let fewer requests fill the relay.
pub const MIN_DROP_CHARGE: usize = 4096;

/// What a buffer of `capacity` bytes costs against the byte budget. An
/// unallocated buffer costs nothing.
pub fn charge(capacity: usize) -> usize {
    if capacity == 0 {
        0
    } else {
        capacity.max(MIN_DROP_CHARGE)
    }
}

/// Body frames and refused uploads must not each scan the entire map.
const SWEEP_INTERVAL: Duration = Duration::from_millis(100);

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

/// Owns an outgoing buffer's quota until the transport drops its last copy.
/// `Bytes::from_owner` keeps this owner alive even after the response body
/// hands a frame to hyper, so slow readers cannot free quota prematurely.
pub struct ResponseBytes {
    bytes: Vec<u8>,
    store: Arc<Store>,
}

impl AsRef<[u8]> for ResponseBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for ResponseBytes {
    fn drop(&mut self) {
        let capacity = self.bytes.capacity();
        drop(std::mem::take(&mut self.bytes));
        self.store.lock().total -= charge(capacity);
    }
}

struct Inner {
    drops: HashMap<String, Entry>,
    /// Charges for stored drops AND buffers still owned by GET responses.
    total: usize,
    /// Bytes claimed by uploads still streaming in but not yet stored. Both
    /// `total` and `reserved` count against `max_total_bytes`, so N
    /// concurrent in-flight PUT bodies can never collectively buffer past
    /// the global cap — the memory-exhaustion hole a per-request cap alone
    /// leaves open (audit sec-relay). Grown by `reserve` as body frames
    /// arrive, released by `Reservation`'s drop, and converted into `total`
    /// by `put_reserved` on success.
    reserved: usize,
    next_sweep: Instant,
}

/// Why a `put` was refused. Test-only: production uploads go through
/// [`Store::reserve`] + [`Store::put_reserved`], which hold space for the
/// whole stream and so cannot hit the cap at store time.
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub enum PutError {
    /// Storing this drop would exceed the byte budget. Fail closed:
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
                reserved: 0,
                next_sweep: Instant::now(),
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
        let mut inner = self.lock();
        self.sweep_if_due(&mut inner);
        self.max_total_bytes
            .saturating_sub(inner.total)
            .saturating_sub(inner.reserved)
    }

    /// Claim `n` more in-flight bytes against the global budget for a body
    /// currently streaming in. Returns `false` (claim nothing) when the
    /// stored plus already-reserved bytes leave no room — the caller then
    /// answers 503 and aborts the upload. Sweeps expired drops when due.
    /// Pair every successful reserve with exactly one release (via
    /// [`Reservation`]) or a [`Store::put_reserved`] that consumes it.
    pub fn reserve(&self, n: usize) -> bool {
        let mut inner = self.lock();
        self.sweep_if_due(&mut inner);
        let total = inner
            .total
            .checked_add(inner.reserved)
            .and_then(|v| v.checked_add(n));
        if total.is_none_or(|total| total > self.max_total_bytes) {
            return false;
        }
        inner.reserved += n;
        true
    }

    /// Release `n` reserved bytes without storing them (an aborted or failed
    /// upload). Idempotent-safe via saturating subtraction.
    pub fn release(&self, n: usize) {
        let mut inner = self.lock();
        inner.reserved = inner.reserved.saturating_sub(n);
    }

    /// Store `bytes` whose charge was reserved via [`Store::reserve`]:
    /// give back spare capacity, move `reserved_bytes` out of the
    /// reservation, charge what the buffer really holds to `total`, then
    /// insert under a fresh token. Cannot fail on the cap (at least that
    /// much was held the whole time the body streamed), so — unlike
    /// [`Store::put`] — it returns the token directly.
    pub fn put_reserved(&self, mut bytes: Vec<u8>, reserved_bytes: usize) -> String {
        // Outside the lock: shrinking may copy the whole buffer.
        bytes.shrink_to_fit();
        let now = Instant::now();
        let mut inner = self.lock();
        inner.reserved = inner.reserved.saturating_sub(reserved_bytes);
        let token = loop {
            let t = generate_token();
            if !inner.drops.contains_key(&t) {
                break t;
            }
        };
        inner.total += charge(bytes.capacity());
        inner.drops.insert(
            token.clone(),
            Entry {
                bytes,
                expires_at: now + self.ttl,
            },
        );
        token
    }

    /// Stores `bytes` under a fresh token; `Err(Full)` if it would exceed
    /// the cap. Test-only convenience that bundles the cap check and the
    /// insert — production splits these into [`Store::reserve`] (before the
    /// body streams) and [`Store::put_reserved`] (after) so concurrent
    /// in-flight bodies share the budget.
    #[cfg(test)]
    pub fn put(&self, bytes: Vec<u8>) -> Result<String, PutError> {
        let held = charge(bytes.capacity());
        if !self.reserve(held) {
            return Err(PutError::Full);
        }
        Ok(self.put_reserved(bytes, held))
    }

    /// One-shot read that retains the buffer's charge for the
    /// lifetime of the returned transport owner, rather than freeing quota
    /// while a slow reader still holds the allocation.
    pub fn take_for_response(self: &Arc<Self>, token: &str) -> Option<ResponseBytes> {
        let now = Instant::now();
        let mut inner = self.lock();
        let entry = inner.drops.remove(token)?;
        if entry.expires_at <= now {
            inner.total -= charge(entry.bytes.capacity());
            return None;
        }
        Some(ResponseBytes {
            bytes: entry.bytes,
            store: Arc::clone(self),
        })
    }

    /// Test-only convenience for consuming and immediately releasing a drop.
    /// Removes and returns the drop, or `None` if it is
    /// unknown, already taken, or expired (an expired entry is dropped on
    /// the way out). Atomic under the lock — of two concurrent `take`s for
    /// one token exactly one gets `Some`.
    #[cfg(test)]
    pub fn take(&self, token: &str) -> Option<Vec<u8>> {
        let now = Instant::now();
        let mut inner = self.lock();
        let entry = inner.drops.remove(token)?;
        inner.total -= charge(entry.bytes.capacity());
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
                inner.total -= charge(dead.bytes.capacity());
                false
            }
            Some(_) => true,
        }
    }

    /// Deletes if present — idempotent, no answer either way.
    pub fn delete(&self, token: &str) {
        let mut inner = self.lock();
        if let Some(entry) = inner.drops.remove(token) {
            inner.total -= charge(entry.bytes.capacity());
        }
    }

    /// Drops every expired entry. Called by the background sweeper and at
    /// the top of `put`.
    pub fn sweep(&self) {
        let mut inner = self.lock();
        let now = Instant::now();
        Self::sweep_locked(&mut inner, now);
        inner.next_sweep = now + self.ttl.min(SWEEP_INTERVAL);
    }

    fn sweep_if_due(&self, inner: &mut Inner) {
        let now = Instant::now();
        if now >= inner.next_sweep {
            Self::sweep_locked(inner, now);
            inner.next_sweep = now + self.ttl.min(SWEEP_INTERVAL);
        }
    }

    fn sweep_locked(inner: &mut Inner, now: Instant) {
        let mut freed = 0usize;
        inner.drops.retain(|_, entry| {
            if entry.expires_at <= now {
                freed += charge(entry.bytes.capacity());
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

    /// Budgets and sizes below are in units of the minimum charge, so that
    /// they are charged at face value.
    const U: usize = MIN_DROP_CHARGE;

    #[test]
    fn put_take_is_one_shot() {
        let store = Store::new(U, Duration::from_secs(60));
        let token = store.put(vec![1, 2, 3]).unwrap();
        assert!(store.peek(&token));
        assert_eq!(store.take(&token), Some(vec![1, 2, 3]));
        assert_eq!(store.take(&token), None);
        assert!(!store.peek(&token));
        assert_eq!(store.remaining(), U);
    }

    #[test]
    fn peek_does_not_consume_and_delete_is_idempotent() {
        let store = Store::new(U, Duration::from_secs(60));
        let token = store.put(vec![9]).unwrap();
        assert!(store.peek(&token));
        assert!(store.peek(&token));
        assert_eq!(store.take(&token), Some(vec![9]));
        store.delete(&token);
        store.delete(&token);
        store.delete("nope");
        assert_eq!(store.remaining(), U);
    }

    #[test]
    fn reservations_count_against_the_global_cap() {
        // Simulates concurrent in-flight uploads: their reserved bytes must
        // reduce `remaining` and block a further reservation past the cap,
        // even though nothing has been stored yet (audit sec-relay).
        let store = Store::new(10 * U, Duration::from_secs(60));
        assert!(store.reserve(6 * U));
        assert_eq!(store.remaining(), 4 * U);
        assert!(store.reserve(4 * U));
        assert_eq!(store.remaining(), 0);
        // A third in-flight body finds no room and is rejected.
        assert!(!store.reserve(1));

        // One upload completes: its reservation converts to stored bytes,
        // leaving the total unchanged and freeing nothing extra.
        let token = store.put_reserved(vec![0; 6 * U], 6 * U);
        assert_eq!(store.remaining(), 0);
        // The other aborts: releasing its reservation frees that space.
        store.release(4 * U);
        assert_eq!(store.remaining(), 4 * U);
        assert_eq!(store.take(&token).map(|v| v.len()), Some(6 * U));
        assert_eq!(store.remaining(), 10 * U);
    }

    #[test]
    fn total_cap_is_enforced_and_freed() {
        let store = Store::new(10 * U, Duration::from_secs(60));
        let a = store.put(vec![0; 6 * U]).unwrap();
        assert_eq!(store.remaining(), 4 * U);
        assert_eq!(store.put(vec![0; 5 * U]), Err(PutError::Full));
        let b = store.put(vec![0; 4 * U]).unwrap();
        assert_eq!(store.remaining(), 0);
        assert_eq!(store.put(vec![0; 1]), Err(PutError::Full));
        store.delete(&a);
        assert_eq!(store.remaining(), 6 * U);
        assert!(store.put(vec![0; 6 * U]).is_ok());
        assert_eq!(store.take(&b).map(|v| v.len()), Some(4 * U));
    }

    #[test]
    fn spare_buffer_capacity_is_given_back_before_storing() {
        let store = Store::new(8 * U, Duration::from_secs(60));
        let mut bytes = Vec::with_capacity(8 * U);
        bytes.resize(2 * U, 7);
        assert!(store.reserve(8 * U));
        let token = store.put_reserved(bytes, 8 * U);
        assert_eq!(store.remaining(), 6 * U);
        let stored = store.take(&token).unwrap();
        assert_eq!(stored.capacity(), 2 * U);
        assert_eq!(store.remaining(), 8 * U);
    }

    #[test]
    fn tiny_drops_are_charged_the_minimum_so_the_budget_bounds_their_number() {
        let store = Store::new(2000 * U, Duration::from_secs(60));
        let mut tokens = Vec::new();
        while let Ok(token) = store.put(vec![1]) {
            tokens.push(token);
            assert!(tokens.len() <= 2000, "tiny drops outran the budget");
        }
        // No other entry limit stands in the way of the budget.
        assert_eq!(tokens.len(), 2000);
        assert_eq!(store.remaining(), 0);
        store.delete(&tokens[0]);
        assert_eq!(store.remaining(), U);
        assert!(store.put(vec![1]).is_ok());
    }

    #[test]
    fn every_way_out_returns_the_whole_minimum_charge() {
        let store = Arc::new(Store::new(4 * U, Duration::from_millis(30)));
        // take_for_response: charged until the owner drops.
        let token = store.put(vec![1]).unwrap();
        let owner = store.take_for_response(&token).unwrap();
        assert_eq!(store.remaining(), 3 * U);
        drop(owner);
        assert_eq!(store.remaining(), 4 * U);
        // Expiry noticed by take_for_response, peek and the sweep.
        let a = store.put(vec![1]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert!(store.take_for_response(&a).is_none());
        assert_eq!(store.remaining(), 4 * U);
        let b = store.put(vec![1]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert!(!store.peek(&b));
        assert_eq!(store.remaining(), 4 * U);
        store.put(vec![1]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        store.sweep();
        assert_eq!(store.remaining(), 4 * U);
    }

    #[test]
    fn expiry_hides_and_frees() {
        let store = Store::new(U, Duration::from_millis(30));
        let token = store.put(vec![7; 100]).unwrap();
        assert!(store.peek(&token));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!store.peek(&token));
        assert_eq!(store.remaining(), U);
        let token2 = store.put(vec![7; 100]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(store.take(&token2), None);
        assert_eq!(store.remaining(), U);
    }

    #[test]
    fn sweep_frees_expired_without_touching_live() {
        let store = Store::new(4 * U, Duration::from_millis(30));
        let dead = store.put(vec![1; 10]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
        let live = store.put(vec![2; 10]).unwrap();
        store.sweep();
        assert_eq!(store.len(), 1);
        assert!(!store.peek(&dead));
        assert!(store.peek(&live));
        assert_eq!(store.remaining(), 3 * U);
    }

    #[test]
    fn concurrent_takes_yield_exactly_one() {
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
