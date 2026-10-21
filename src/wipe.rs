// SPDX-License-Identifier: GPL-3.0-or-later

//! Overwrite secrets before their memory is given back.
//!
//! A plain `drop` or `clear` only frees memory, and the old bytes stay in it until something else reuses the
//! space. These helpers write zeros with volatile stores, which the compiler may not optimise away, and then
//! fence, so the writes are really done before the memory is freed.
//!
//! What this can and cannot reach is described in the README ("Memory hygiene").

use std::sync::atomic::{compiler_fence, Ordering};

/// Zero every byte of the slice.
pub fn bytes(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: `b` is a valid, aligned, exclusive reference to a byte.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}

/// Zero a string's whole buffer, including the spare capacity past its length, then empty it.
pub fn string(s: &mut String) {
    let cap = s.capacity();
    // Fill up to the capacity with spaces first, so the whole buffer is addressable as initialised bytes.
    s.clear();
    s.extend(std::iter::repeat(' ').take(cap));
    // SAFETY: zero bytes are valid UTF-8, so the string stays valid.
    bytes(unsafe { s.as_bytes_mut() });
    s.clear();
}

/// Forget a list of words by overwriting every pointer in it (they point into the program's read-only text,
/// but *which* words were drawn is the secret), then empty it.
pub fn words(list: &mut Vec<&'static str>) {
    for w in list.iter_mut() {
        // SAFETY: `w` is a valid, aligned, exclusive reference to a `&'static str`.
        unsafe { std::ptr::write_volatile(w, "") };
    }
    compiler_fence(Ordering::SeqCst);
    list.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_become_zero() {
        let mut v = vec![0xAAu8; 64];
        bytes(&mut v);
        assert!(v.iter().all(|&b| b == 0));
    }

    #[test]
    fn string_buffer_is_zeroed_not_just_cleared() {
        let mut s = String::with_capacity(32);
        s.push_str("secret12345");
        let (ptr, cap) = (s.as_ptr(), s.capacity());
        string(&mut s);
        assert!(s.is_empty());
        // The allocation is still ours (the string was not dropped), so reading it is fine.
        let raw = unsafe { std::slice::from_raw_parts(ptr, cap.min(32)) };
        assert!(raw.iter().all(|&b| b == 0), "{raw:?}");
    }

    #[test]
    fn word_pointers_are_overwritten() {
        let mut v: Vec<&'static str> = vec!["abacus", "zoom", "arousal"];
        let ptr = v.as_mut_ptr();
        words(&mut v);
        assert!(v.is_empty());
        // The vector's buffer still holds three slots; every one now points at the empty string.
        let slots = unsafe { std::slice::from_raw_parts(ptr, 3) };
        assert!(slots.iter().all(|s| s.is_empty()));
    }

    #[test]
    fn wiping_nothing_is_fine() {
        string(&mut String::new());
        words(&mut Vec::new());
        bytes(&mut []);
    }
}
