//! Copy-on-write storage for an [`crate::topo::Object`]'s element tables.
//!
//! Every undo snapshot, transaction bracket and strong-guarantee scratch
//! copy of an Object is a `clone()`. On a dense imported mesh a deep clone
//! is hundreds of megabytes, so each table sits behind a [`Shared`]: a
//! clone shares the table, and the first write through either copy gives
//! that copy a private table of its own (`Arc::make_mut`). Value semantics
//! are unchanged — a clone still behaves as an independent deep copy — but
//! a step that touches one table of one object costs that table, not the
//! document.
//!
//! The memory budget for the undo stack ([`crate::Document::history_memory`])
//! counts each table once by its address ([`Shared::addr`]).

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use slotmap::{Key, SlotMap};

/// A copy-on-write value: cheap to clone, private on first write.
#[derive(Default)]
pub struct Shared<T>(Arc<T>);

impl<T> Shared<T> {
    /// Wraps `value` as the only owner of its table.
    pub fn new(value: T) -> Shared<T> {
        Shared(Arc::new(value))
    }

    /// The table's address: two copies that still share it agree, a copy
    /// that has written since does not. Identity only — never dereferenced.
    pub fn addr(&self) -> usize {
        Arc::as_ptr(&self.0) as *const () as usize
    }

    /// Whether `self` and `other` still share one table.
    pub fn ptr_eq(&self, other: &Shared<T>) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Shared<T> {
        Shared(Arc::clone(&self.0))
    }
}

impl<T> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a, K: Key, V> IntoIterator for &'a Shared<SlotMap<K, V>> {
    type Item = (K, &'a V);
    type IntoIter = slotmap::basic::Iter<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a, K: Key, V: Clone> IntoIterator for &'a mut Shared<SlotMap<K, V>> {
    type Item = (K, &'a mut V);
    type IntoIter = slotmap::basic::IterMut<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        Arc::make_mut(&mut self.0).iter_mut()
    }
}

/// An estimate of the heap a table's slot array holds at its capacity:
/// each slot is the value plus slotmap's version word, padded to the
/// value's alignment. O(1); heap the values own themselves is the caller's
/// to add.
pub(crate) fn slotmap_heap_bytes<K: Key, V>(map: &SlotMap<K, V>) -> usize {
    let align = std::mem::align_of::<V>().max(4);
    let slot = (std::mem::size_of::<V>() + 4).div_ceil(align) * align;
    map.capacity() * slot
}
