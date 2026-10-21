// SPDX-License-Identifier: GPL-3.0-or-later

//! The pixel-in effect for a new word: the word is first shown as coarse blocks that get finer step by
//! step until the real text takes over.
//!
//! Slint cannot pixelate text, so the blocky frames are made here. A word is composed from the glyph masks
//! in `glyphs.rs` (Montserrat Medium, rasterised once at development time by tools/gen-glyphs), then
//! averaged over square cells. Nothing here needs a font rasteriser at run time.

use crate::glyphs::{ASCENT, GLYPHS, KERN, LINE_HEIGHT};

/// Empty margin around the word so the coarsest cells do not get cut off at the edges.
const PAD: usize = 4;

/// Cell sizes of the frames, coarsest first. The real text follows the last one.
pub const BLOCKS: [usize; 4] = [11, 7, 4, 2];

/// How much brighter each frame's averaged cells are drawn. A thin stroke covers only a part of a big
/// cell, so its average is faint; the gain keeps the blocks readable.
const GAIN: [f32; 4] = [3.2, 2.6, 2.0, 1.4];

/// A greyscale coverage image: 0 = nothing, 255 = full ink.
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub width: usize,
    pub height: usize,
    pub alpha: Vec<u8>,
}

/// A bitmap is a picture of a secret word, so it is wiped when it goes away.
impl Drop for Bitmap {
    fn drop(&mut self) {
        crate::wipe::bytes(&mut self.alpha);
    }
}

fn glyph(ch: char) -> Option<&'static crate::glyphs::Glyph> {
    GLYPHS.iter().find(|g| g.ch == ch)
}

fn kern(a: char, b: char) -> f32 {
    KERN.iter().find(|(l, r, _)| *l == a && *r == b).map_or(0.0, |(_, _, k)| *k)
}

/// The word drawn at full size. Characters without a glyph are skipped.
pub fn render(word: &str) -> Bitmap {
    let chars: Vec<char> = word.chars().filter(|c| glyph(*c).is_some()).collect();
    let mut pen = 0.0f32;
    let mut pens = Vec::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        pens.push(pen);
        pen += glyph(c).unwrap().advance;
        if let Some(&next) = chars.get(i + 1) {
            pen += kern(c, next);
        }
    }
    let width = pen.ceil() as usize + 2 * PAD;
    let height = LINE_HEIGHT as usize + 2 * PAD;
    let mut alpha = vec![0u8; width * height];
    for (&c, &x0) in chars.iter().zip(&pens) {
        let g = glyph(c).unwrap();
        let gx = (x0 + g.left as f32).round() as i32 + PAD as i32;
        let gy = ASCENT - g.top + PAD as i32;
        for row in 0..g.height {
            for col in 0..g.width {
                let (x, y) = (gx + col as i32, gy + row as i32);
                if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
                    continue;
                }
                let px = &mut alpha[y as usize * width + x as usize];
                *px = px.saturating_add(g.mask[row * g.width + col]);
            }
        }
    }
    Bitmap { width, height, alpha }
}

/// The bitmap averaged over `block` x `block` cells, each cell painted with its average (times `gain`).
/// Same size as the input, so every frame can be shown at the same place.
pub fn pixelate(b: &Bitmap, block: usize, gain: f32) -> Bitmap {
    let block = block.max(1);
    let mut out = vec![0u8; b.alpha.len()];
    for cy in (0..b.height).step_by(block) {
        for cx in (0..b.width).step_by(block) {
            let (x1, y1) = ((cx + block).min(b.width), (cy + block).min(b.height));
            let mut sum = 0u32;
            for y in cy..y1 {
                for x in cx..x1 {
                    sum += b.alpha[y * b.width + x] as u32;
                }
            }
            // divide by the full cell area, so a partial cell at the edge is not boosted
            let v = ((sum as f32 / (block * block) as f32) * gain).round().min(255.0) as u8;
            for y in cy..y1 {
                out[y * b.width + cx..y * b.width + x1].fill(v);
            }
        }
    }
    Bitmap { width: b.width, height: b.height, alpha: out }
}

/// The frames of the effect for `word`, coarsest first.
pub fn frames(word: &str) -> Vec<Bitmap> {
    let full = render(word);
    BLOCKS.iter().zip(GAIN).map(|(&block, gain)| pixelate(&full, block, gain)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn ink(b: &Bitmap) -> u64 {
        b.alpha.iter().map(|&a| a as u64).sum()
    }

    #[test]
    fn words_get_wider_with_more_letters() {
        assert!(render("zoom").width < render("abdominal").width);
        assert_eq!(render("zoom").height, LINE_HEIGHT as usize + 2 * PAD);
    }

    #[test]
    fn ink_is_inside_the_bitmap_and_not_empty() {
        for w in ["abacus", "t-shirt", "yo-yo", "drop-down", "zoom"] {
            let b = render(w);
            assert!(ink(&b) > 5_000, "{w}: {}", ink(&b));
            assert_eq!(b.alpha.len(), b.width * b.height);
            // the margin stays clear: nothing in the outermost pixel ring
            for x in 0..b.width {
                assert_eq!(b.alpha[x], 0);
                assert_eq!(b.alpha[(b.height - 1) * b.width + x], 0);
            }
        }
    }

    #[test]
    fn every_character_of_the_wordlist_has_a_glyph() {
        let list = include_str!("eff_large_wordlist.txt");
        for line in list.lines() {
            let word = line.split('\t').nth(1).unwrap();
            for c in word.chars() {
                assert!(glyph(c).is_some(), "no glyph for {c:?} in {word}");
            }
        }
    }

    #[test]
    fn block_of_one_changes_nothing() {
        let b = render("abacus");
        assert_eq!(pixelate(&b, 1, 1.0), b);
    }

    #[test]
    fn coarser_blocks_have_fewer_distinct_cells() {
        let b = render("headroom");
        let cells = |block: usize| -> usize {
            let p = pixelate(&b, block, 1.0);
            (0..p.height)
                .step_by(block)
                .flat_map(|y| (0..p.width).step_by(block).map(move |x| (x, y)))
                .map(|(x, y)| p.alpha[y * p.width + x])
                .collect::<HashSet<_>>()
                .len()
        };
        assert!(cells(11) < cells(4) && cells(4) <= cells(2));
    }

    #[test]
    fn every_cell_is_one_flat_value() {
        let b = render("breeching");
        let p = pixelate(&b, 7, 1.0);
        for cy in (0..p.height).step_by(7) {
            for cx in (0..p.width).step_by(7) {
                let v = p.alpha[cy * p.width + cx];
                for y in cy..(cy + 7).min(p.height) {
                    for x in cx..(cx + 7).min(p.width) {
                        assert_eq!(p.alpha[y * p.width + x], v);
                    }
                }
            }
        }
    }

    #[test]
    fn frames_keep_the_size_and_a_visible_amount_of_ink() {
        let full = render("ladybug");
        let fs = frames("ladybug");
        assert_eq!(fs.len(), BLOCKS.len());
        for f in &fs {
            assert_eq!((f.width, f.height), (full.width, full.height));
            // the gain keeps even the coarsest frame from being a faint smudge
            assert!(ink(f) * 3 > ink(&full), "{} vs {}", ink(f), ink(&full));
        }
    }

    #[test]
    fn unknown_characters_are_skipped_not_fatal() {
        assert_eq!(render("ab!c").width, render("abc").width);
        assert_eq!(render("").alpha.iter().map(|&a| a as u32).sum::<u32>(), 0);
    }
}
