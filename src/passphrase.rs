// SPDX-License-Identifier: GPL-3.0-or-later

//! Diceware passphrases from the EFF large wordlist (7776 words, 5 dice per word).
//!
//! The wordlist is the one published by the Electronic Frontier Foundation,
//! <https://www.eff.org/dice>, licensed CC BY 3.0 US. It is embedded unchanged.
//!
//! Nothing here keeps a passphrase: callers get the words back and own them.

use std::sync::OnceLock;

const RAW: &str = include_str!("eff_large_wordlist.txt");

/// Number of words in the list: 6^5, one per roll of five dice.
pub const WORDS: usize = 7776;

/// Dice per word.
pub const DICE: usize = 5;

/// (dice digits, word), sorted by digits so `index == base-6 value of the digits - 11111`.
fn list() -> &'static [(&'static str, &'static str)] {
    static LIST: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    LIST.get_or_init(|| {
        RAW.lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(digits, word)| (digits.trim(), word.trim()))
            .collect()
    })
}

/// The word at `index` (0..WORDS).
pub fn word_at(index: usize) -> &'static str {
    list()[index].1
}

/// The word for five dice results such as "16543". `None` unless there are exactly
/// five digits, each between 1 and 6.
pub fn word_for_rolls(rolls: &str) -> Option<&'static str> {
    if rolls.len() != DICE || !rolls.bytes().all(|b| (b'1'..=b'6').contains(&b)) {
        return None;
    }
    let index = rolls.bytes().fold(0usize, |acc, b| acc * 6 + (b - b'1') as usize);
    Some(word_at(index))
}

/// Entropy in bits of `words` words drawn uniformly from the list.
pub fn bits(words: usize) -> f64 {
    words as f64 * (WORDS as f64).log2()
}

/// A uniform index in 0..WORDS from a source of random 32-bit values. Values from the
/// incomplete last block of the 32-bit range are rejected, so there is no modulo bias.
/// `None` if the source fails.
pub fn pick_index(next_u32: &mut impl FnMut() -> Option<u32>) -> Option<usize> {
    let words = WORDS as u32;
    let limit = u32::MAX - (u32::MAX % words);
    loop {
        let x = next_u32()?;
        if x < limit {
            return Some((x % words) as usize);
        }
    }
}

/// `count` independent words. `None` if the random source fails.
pub fn generate(count: usize, mut next_u32: impl FnMut() -> Option<u32>) -> Option<Vec<&'static str>> {
    (0..count).map(|_| pick_index(&mut next_u32).map(word_at)).collect()
}

/// 32 random bits from the device's hardware RNG (the TRNG server on KeyOS, the OS RNG in the simulator).
pub fn hardware_u32() -> Option<u32> {
    let mut bytes = [0u8; 4];
    let ok = getrandom::getrandom(&mut bytes).is_ok();
    let value = u32::from_le_bytes(bytes);
    crate::wipe::bytes(&mut bytes);
    ok.then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn list_is_the_full_eff_list() {
        assert_eq!(list().len(), WORDS);
        let words: HashSet<_> = list().iter().map(|(_, w)| *w).collect();
        assert_eq!(words.len(), WORDS, "words must be unique");
        let keys: HashSet<_> = list().iter().map(|(k, _)| *k).collect();
        assert_eq!(keys.len(), WORDS, "dice keys must be unique");
    }

    #[test]
    fn dice_key_matches_position() {
        // The list is ordered 11111, 11112, ... 66666, so rolling is the same as indexing.
        for (i, (key, word)) in list().iter().enumerate() {
            assert_eq!(key.len(), DICE);
            assert_eq!(word_for_rolls(key), Some(*word), "key {key}");
            assert_eq!(word_at(i), *word);
        }
        assert_eq!(word_for_rolls("11111"), Some("abacus"));
        assert_eq!(word_for_rolls("66666"), Some("zoom"));
    }

    #[test]
    fn bad_rolls_are_rejected() {
        for bad in ["", "1111", "111111", "11117", "11110", "1111a", "11 11", "１１１１１"] {
            assert_eq!(word_for_rolls(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn entropy_per_word() {
        assert!((bits(1) - 12.924_812_503_605_78).abs() < 1e-9);
        assert!((bits(6) - 77.548_875_021_634_69).abs() < 1e-9);
        assert_eq!(bits(0), 0.0);
    }

    #[test]
    fn biased_tail_is_rejected() {
        let limit = u32::MAX - (u32::MAX % WORDS as u32);
        assert_eq!(limit % WORDS as u32, 0);
        let mut feed = [u32::MAX, limit, limit - 1].into_iter();
        let mut src = move || feed.next();
        assert_eq!(pick_index(&mut src), Some(WORDS - 1));
    }

    #[test]
    fn failing_source_gives_none() {
        assert_eq!(pick_index(&mut || None), None);
        assert_eq!(generate(3, || None), None);
        let mut n = 0u32;
        // works for two words, then the source dies
        let r = generate(3, || {
            n += 1;
            (n <= 2).then_some(n * 1000)
        });
        assert_eq!(r, None);
    }

    #[test]
    fn indices_are_uniform() {
        // xorshift32 as a stand-in source; every word should come up about 100 times in 777,600 draws.
        let mut s = 0x1234_5678u32;
        let mut src = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            Some(s)
        };
        let mut counts = vec![0u32; WORDS];
        for _ in 0..WORDS * 100 {
            counts[pick_index(&mut src).unwrap()] += 1;
        }
        let (min, max) = (*counts.iter().min().unwrap(), *counts.iter().max().unwrap());
        assert!(min > 50 && max < 160, "min {min}, max {max}");
    }

    #[test]
    fn generate_returns_requested_count_of_listed_words() {
        let mut s = 7u32;
        let words = generate(6, || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            Some(s)
        })
        .unwrap();
        assert_eq!(words.len(), 6);
        let all: HashSet<_> = list().iter().map(|(_, w)| *w).collect();
        assert!(words.iter().all(|w| all.contains(w)));
    }
}
