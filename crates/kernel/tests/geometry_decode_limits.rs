//! Malformed counts must be rejected before they amplify a tiny input into a
//! large allocation. Moderate counts keep these regressions safe on old builds.

use kernel::{DecodeError, Object};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct ObservedAllocator;

thread_local! {
    static LARGEST_ALLOCATION: Cell<Option<usize>> = const { Cell::new(None) };
}

fn observe_allocation(size: usize) {
    let _ = LARGEST_ALLOCATION.try_with(|largest| {
        if let Some(previous) = largest.get() {
            largest.set(Some(previous.max(size)));
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
