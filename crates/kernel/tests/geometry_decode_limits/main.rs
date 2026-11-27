//! Malformed counts must be rejected before they amplify a tiny input into a
//! large allocation. Moderate counts keep these regressions safe on old builds.
//!
//! A test binary of its own (see `crates/kernel/Cargo.toml`): the counting
//! global allocator below would otherwise sit under every kernel spec.

use kernel::{DecodeError, Object};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct ObservedAllocator;

thread_local! {
    static LARGEST_ALLOCATION: Cell<Option<usize>> = const { Cell::new(None) };
    /// While `Some((threshold, count))`, counts allocations of at least
    /// `threshold` bytes.
    static LARGE_ALLOCATIONS: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

fn observe_allocation(size: usize) {
    let _ = LARGEST_ALLOCATION.try_with(|largest| {
        if let Some(previous) = largest.get() {
            largest.set(Some(previous.max(size)));
        }
    });
    let _ = LARGE_ALLOCATIONS.try_with(|large| {
        if let Some((threshold, count)) = large.get()
            && size >= threshold
        {
            large.set(Some((threshold, count + 1)));
        }
    });
}

// SAFETY: every allocation operation is delegated unchanged to System. The
// thread-local observer only records an integer and never allocates itself.
unsafe impl GlobalAlloc for ObservedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        observe_allocation(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        observe_allocation(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        observe_allocation(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: ObservedAllocator = ObservedAllocator;

fn header() -> Vec<u8> {
    let mut bytes = b"HEWG".to_vec();
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.push(0); // open surface
    bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // no material
    bytes
}

fn triangle_vertices() -> Vec<u8> {
    let mut bytes = header();
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    for coordinate in [0.0_f64, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bytes.extend_from_slice(&coordinate.to_le_bytes());
    }
    bytes
}

fn triangle_outer_loop() -> Vec<u8> {
    let mut bytes = triangle_vertices();
    bytes.extend_from_slice(&1_u32.to_le_bytes()); // one face
    bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // no face material
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    for index in [0_u32, 1, 2] {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes
}

fn assert_bounded_truncation(bytes: &[u8]) {
    LARGEST_ALLOCATION.with(|largest| largest.set(Some(0)));
    let result = Object::decode(bytes, &|_| None);
    let largest = LARGEST_ALLOCATION.with(|largest| largest.replace(None).unwrap());
    assert_eq!(result.err(), Some(DecodeError::Truncated));
    // Valid prefixes here contain at most three vertices and one face. A
    // 4-KiB allocation allowance is deliberately generous for that geometry;
    // reserving thousands of absent elements exceeds it on both wasm32/native.
    assert!(
        largest <= 4096,
        "tiny input caused a {largest}-byte allocation"
    );
}

#[test]
fn rejects_impossible_vertex_count_before_allocating_positions() {
    let mut bytes = header();
    bytes.extend_from_slice(&4096_u32.to_le_bytes());
    assert_bounded_truncation(&bytes);
}

#[test]
fn rejects_impossible_face_count_before_allocating_face_state() {
    let mut bytes = header();
    bytes.extend_from_slice(&0_u32.to_le_bytes()); // no vertices
    bytes.extend_from_slice(&4096_u32.to_le_bytes());
    assert_bounded_truncation(&bytes);
}

#[test]
fn rejects_impossible_outer_count_before_allocating_indices() {
    let mut bytes = triangle_vertices();
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    bytes.extend_from_slice(&4096_u32.to_le_bytes());
    // Satisfy the enclosing face's 24-byte minimum so this exercises the
    // outer-loop count check, rather than being rejected by the face count.
    bytes.extend_from_slice(&[0; 16]);
    assert_bounded_truncation(&bytes);
}

#[test]
fn rejects_impossible_hole_count_before_allocating_holes() {
    let mut bytes = triangle_outer_loop();
    bytes.extend_from_slice(&4096_u32.to_le_bytes());
    assert_bounded_truncation(&bytes);
}

#[test]
fn rejects_impossible_hole_vertex_count_before_allocating_indices() {
    let mut bytes = triangle_outer_loop();
    bytes.extend_from_slice(&1_u32.to_le_bytes()); // one hole
    bytes.extend_from_slice(&4096_u32.to_le_bytes());
    // One hole's minimum is its count and three indices. Keep that outer
    // guard satisfied so only the inflated hole vertex count rejects this.
    bytes.extend_from_slice(&[0; 12]);
    assert_bounded_truncation(&bytes);
}

/// How many allocations of at least `threshold` bytes decoding `bytes`
/// makes, with the decode result.
fn decode_counting(bytes: &[u8], threshold: usize) -> (Result<Object, DecodeError>, usize) {
    LARGE_ALLOCATIONS.with(|large| large.set(Some((threshold, 0))));
    let result = Object::decode(bytes, &|_| None);
    let (_, count) = LARGE_ALLOCATIONS.with(|large| large.replace(None).unwrap());
    (result, count)
}

// ── Minimum item sizes ──────────────────────────────────────────────────────
//
// Each count is checked against the bytes after it at its item's minimum
// encoded size. A count one past what those bytes can hold is refused
// before anything is reserved for it; a count that exactly fits is
// reserved for. Together these pin each minimum from both sides, and pin
// the comparison as "more than fits", not "at least what fits".

/// Items' worth of bytes behind each count below; far above every
/// allocation the small fixed geometry in front of it makes.
const ITEMS: usize = 4096;

fn assert_minimum_item_bytes(prefix: impl Fn(u32) -> Vec<u8>, minimum: usize, fill: u8) {
    let padded = |count: usize| {
        let mut bytes = prefix(count as u32);
        bytes.resize(bytes.len() + minimum * ITEMS, fill);
        bytes
    };
    // Reserving `count` items of any type takes at least `count` bytes.
    let (result, reserved) = decode_counting(&padded(ITEMS + 1), ITEMS);
    assert_eq!(result.err(), Some(DecodeError::Truncated));
    assert_eq!(
        reserved, 0,
        "a count one past {minimum}-byte items in the remaining bytes was reserved for"
    );
    let (result, reserved) = decode_counting(&padded(ITEMS), ITEMS);
    assert!(result.is_err(), "the padding is not a whole object");
    assert!(
        reserved > 0,
        "a count that exactly fits {minimum}-byte items was refused before reserving"
    );
}

#[test]
fn a_vertex_needs_at_least_24_bytes() {
    let prefix = |count: u32| {
        let mut bytes = header();
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes
    };
    assert_minimum_item_bytes(prefix, 24, 0);
}

#[test]
fn a_face_needs_at_least_24_bytes() {
    let prefix = |count: u32| {
        let mut bytes = header();
        bytes.extend_from_slice(&0_u32.to_le_bytes()); // no vertices
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes
    };
    // 0xFF fill reads as "no material", so an accepted face goes on to
    // its own counts.
    assert_minimum_item_bytes(prefix, 24, 0xFF);
}

#[test]
fn a_loop_index_needs_at_least_4_bytes() {
    let prefix = |count: u32| {
        let mut bytes = triangle_vertices();
        bytes.extend_from_slice(&1_u32.to_le_bytes()); // one face
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // no face material
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes
    };
    assert_minimum_item_bytes(prefix, 4, 0);
}

#[test]
fn a_hole_needs_at_least_16_bytes() {
    let prefix = |count: u32| {
        let mut bytes = triangle_outer_loop();
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes
    };
    assert_minimum_item_bytes(prefix, 16, 0);
}

#[test]
fn a_hole_index_needs_at_least_4_bytes() {
    let prefix = |count: u32| {
        let mut bytes = triangle_outer_loop();
        bytes.extend_from_slice(&1_u32.to_le_bytes()); // one hole
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes
    };
    assert_minimum_item_bytes(prefix, 4, 0);
}

// ── Per-edge blocks ─────────────────────────────────────────────────────────
//
// These blocks repeat once per loop edge, so their lengths come from loop
// counts the buffer already paid for in index bytes. The decoder must still
// check that the block itself fits what remains before reserving for it.

/// The geometry version whose layout the per-edge fixtures write. Fixed,
/// not the current version, so a later version cannot shift the layout
/// under them; every reader keeps reading v11.
const FIXTURE_VERSION: u32 = 11;

/// A long loop: every edge then owes an entry in each per-edge block.
const LONG_LOOP: usize = 4096;

/// The per-edge blocks of a face, in the order a v11 buffer writes them,
/// with the error the decoder reports for an entry flag of 2.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Block {
    Soft,
    Curves,
    Polygons,
    Dictionaries,
    Crease,
}

impl Block {
    fn bad_entry_flag(self) -> &'static str {
        match self {
            Block::Soft => "soft edge flag must be 0 or 1",
            Block::Curves => "edge curve flag must be 0 or 1",
            Block::Polygons => "claim flag must be 0 or 1",
            Block::Dictionaries => "attribute dictionary flag must be 0 or 1",
            Block::Crease => "crease flag must be 0 or 1",
        }
    }
}

/// Indices cycling over the three triangle vertices, `len` of them. The
/// repeated triangle still spans a plane, so decoding reaches the blocks.
fn cycling_loop(len: usize) -> Vec<u32> {
    (0..len as u32).map(|i| i % 3).collect()
}

/// A v11 buffer holding one face, stopped right before `block`; every
/// block in front of it is written as empty.
fn face_before(block: Block, outer: &[u32], holes: &[Vec<u32>]) -> Vec<u8> {
    let mut bytes = b"HEWG".to_vec();
    bytes.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
    bytes.push(0); // open surface
    bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // no material
    bytes.push(0); // not imported (v3)
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    for coordinate in [0.0_f64, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bytes.extend_from_slice(&coordinate.to_le_bytes());
    }
    bytes.extend_from_slice(&1_u32.to_le_bytes()); // one face
    bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // no face material
    bytes.push(0); // no UV frame (v2)
    bytes.push(0); // no surface (v4)
    bytes.extend_from_slice(&(outer.len() as u32).to_le_bytes());
    for index in outer {
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    bytes.extend_from_slice(&(holes.len() as u32).to_le_bytes());
    for hole in holes {
        bytes.extend_from_slice(&(hole.len() as u32).to_le_bytes());
        for index in hole {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
    }
    bytes.push(0); // refit plane (v11)
    if block > Block::Soft {
        bytes.push(0); // no soft edges (v6)
    }
    if block > Block::Curves {
        bytes.push(0); // no edge curves (v5)
    }
    if block > Block::Polygons {
        bytes.push(0); // no polygon claims (v7)
        bytes.push(0); // no back paint (v8)
        bytes.push(0); // no face dictionary (v9)
    }
    if block > Block::Dictionaries {
        bytes.push(0); // no edge dictionaries (v9)
    }
    bytes
}

/// Announcing `block` with nothing behind it must allocate no more than
/// stopping before the announcement: the long loop's own buffers are
/// reserved in both, and nothing is reserved for the block.
///
/// `outer` edges come before `holes` in each block; when the long loop is
/// a hole, `lead` is the block's flag and the short outer loop's entries.
fn assert_announced_block_allocates_nothing(block: Block, long_loop_is_hole: bool) {
    let (outer, holes, lead): (Vec<u32>, Vec<Vec<u32>>, &[u8]) = if long_loop_is_hole {
        (vec![0, 1, 2], vec![cycling_loop(LONG_LOOP)], &[1, 0, 0, 0])
    } else {
        (cycling_loop(LONG_LOOP), Vec::new(), &[1])
    };
    let stopped = face_before(block, &outer, &holes);

    // Control: with the long loop's entries present (the first one bad),
    // decoding reaches this block and fails on that entry, so the two
    // truncated runs below stop where they are meant to.
    let mut reached = stopped.clone();
    reached.extend_from_slice(lead);
    reached.push(2);
    reached.resize(reached.len() + LONG_LOOP - 1, 0);
    match Object::decode(&reached, &|_| None) {
        Err(DecodeError::Corrupt { what, .. }) => assert_eq!(what, block.bad_entry_flag()),
        other => panic!("the fixture does not reach the block: {:?}", other.err()),
    }

    let mut announced = stopped.clone();
    announced.extend_from_slice(lead);
    // A one-byte entry per edge of the long loop is LONG_LOOP bytes.
    let (result, before) = decode_counting(&stopped, LONG_LOOP);
    assert_eq!(result.err(), Some(DecodeError::Truncated));
    let (result, after) = decode_counting(&announced, LONG_LOOP);
    assert_eq!(result.err(), Some(DecodeError::Truncated));
    assert_eq!(
        after, before,
        "an announced block the buffer cannot hold was reserved for"
    );
}

#[test]
fn rejects_outer_soft_flags_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Soft, false);
}

#[test]
fn rejects_hole_soft_flags_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Soft, true);
}

#[test]
fn rejects_outer_edge_curves_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Curves, false);
}

#[test]
fn rejects_hole_edge_curves_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Curves, true);
}

#[test]
fn rejects_outer_polygon_claims_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Polygons, false);
}

#[test]
fn rejects_hole_polygon_claims_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Polygons, true);
}

#[test]
fn rejects_outer_edge_dictionaries_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Dictionaries, false);
}

#[test]
fn rejects_hole_edge_dictionaries_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Dictionaries, true);
}

#[test]
fn rejects_outer_crease_flags_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Crease, false);
}

#[test]
fn rejects_hole_crease_flags_the_buffer_cannot_hold() {
    assert_announced_block_allocates_nothing(Block::Crease, true);
}
