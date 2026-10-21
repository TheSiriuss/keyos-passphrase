// SPDX-License-Identifier: GPL-3.0-or-later
//
// Compiles the app's KeyOS-free modules as they are, so their unit tests run with a plain `cargo test`.

#[path = "../../../src/wipe.rs"]
pub mod wipe;
#[path = "../../../src/passphrase.rs"]
pub mod passphrase;
#[path = "../../../src/glyphs.rs"]
pub mod glyphs;
#[path = "../../../src/pixel.rs"]
pub mod pixel;
