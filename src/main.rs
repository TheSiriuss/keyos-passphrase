// SPDX-License-Identifier: GPL-3.0-or-later

//! Diceware passphrase generator for Passport Prime.
//!
//! Two ways to get a passphrase from the EFF large wordlist: draw the words with the
//! device's hardware RNG, or roll your own dice and tap the results. Nothing is saved,
//! the words are never logged, and what the app keeps of them is overwritten when it lets go.

mod glyphs;
mod passphrase;
mod pixel;
mod theme;
mod wipe;

use slint_keyos_platform::app_ui2;
use slint_keyos_platform::gui_server_api::InputMessage;
use slint_keyos_platform::slint::{
    ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel, Weak,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

app_ui2!("Passphrase");

/// The word counts offered by the segmented control, in order.
const COUNTS: [usize; 4] = [4, 5, 6, 8];

/// What the screen shows. Every callback changes this, then `Ctx::render` copies it to the UI.
/// Everything secret in here is overwritten by `clear` and when the session is dropped.
struct Session {
    words: Vec<&'static str>,
    /// Dice mode: the five dice of each finished word, so the last word can be undone.
    keys: Vec<String>,
    /// Dice mode: the dice tapped so far for the word being rolled.
    rolls: String,
    /// The hardware RNG failed on the last draw.
    rng_failed: bool,
    /// "Show words" is on. While it is off the UI is not given the words at all, only empty rows.
    shown: bool,
}

impl Default for Session {
    fn default() -> Self {
        Session { words: Vec::new(), keys: Vec::new(), rolls: String::new(), rng_failed: false, shown: true }
    }
}

impl Session {
    /// Forget the words and dice, overwriting them. Keeps the "show words" choice.
    fn clear(&mut self) {
        wipe::words(&mut self.words);
        for key in self.keys.iter_mut() {
            wipe::string(key);
        }
        self.keys.clear();
        wipe::string(&mut self.rolls);
        self.rng_failed = false;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.clear();
    }
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

/// A frame of the pixel-in effect as an image: white, with the coverage as alpha (the UI tints it).
fn to_image(b: &pixel::Bitmap) -> Image {
    let mut rgba = Vec::with_capacity(b.alpha.len() * 4);
    for &a in &b.alpha {
        rgba.extend_from_slice(&[255, 255, 255, a]);
    }
    let image =
        Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, b.width as u32, b.height as u32));
    wipe::bytes(&mut rgba);
    image
}

/// The row for a word. While the words are hidden the UI gets an empty row: no text, no frames.
fn make_word(text: &str, shown: bool) -> Word {
    if !shown {
        return Word::default();
    }
    Word { text: text.into(), frames: model(pixel::frames(text).iter().map(to_image).collect()) }
}

/// Make the rows match `wanted`, touching as few as possible: rows that already show the right word stay,
/// so only a word that is really new plays its effect. `rebuild` replaces every row.
fn sync_words(rows: &VecModel<Word>, wanted: &[&'static str], rebuild: bool, shown: bool) {
    let keep = if rebuild {
        0
    } else if !shown {
        // hidden rows carry no text to compare; "Show words" refreshes every row from the session anyway
        rows.row_count().min(wanted.len())
    } else {
        (0..rows.row_count().min(wanted.len()))
            .take_while(|&i| rows.row_data(i).is_some_and(|row| row.text.as_str() == wanted[i]))
            .count()
    };
    while rows.row_count() > keep {
        rows.remove(rows.row_count() - 1);
    }
    for &word in &wanted[keep..] {
        rows.push(make_word(word, shown));
    }
}

/// Everything a callback needs.
struct Ctx {
    ui: Weak<AppWindow>,
    session: RefCell<Session>,
    rows: Rc<VecModel<Word>>,
    /// When the system hid the app, while it is hidden.
    hidden_at: Cell<Option<Instant>>,
}

impl Ctx {
    fn target(ui: &AppWindow) -> usize {
        COUNTS[(ui.global::<PassphraseState>().get_count_index() as usize).min(COUNTS.len() - 1)]
    }

    fn render(&self, rebuild: bool) {
        let Some(ui) = self.ui.upgrade() else { return };
        let s = self.session.borrow();
        let state = ui.global::<PassphraseState>();
        let dice_mode = state.get_mode() == 1;
        let target = Self::target(&ui);
        let finished = dice_mode && s.words.len() == target;

        sync_words(&self.rows, &s.words, rebuild, s.shown);
        state.set_finished(finished);
        state.set_bits(passphrase::bits(s.words.len()) as f32);

        let status = if dice_mode {
            if finished {
                "Done. The words are not saved anywhere.".to_string()
            } else {
                format!("Word {} of {}: tap the five dice you rolled", s.words.len() + 1, target)
            }
        } else if s.rng_failed {
            "The hardware RNG did not answer. Nothing was generated.".to_string()
        } else if s.words.is_empty() {
            String::new()
        } else {
            "Drawn from the hardware RNG. Not saved anywhere.".to_string()
        };
        state.set_status(status.into());
        state.set_slots(model(
            (0..passphrase::DICE)
                .map(|i| s.rolls.get(i..i + 1).unwrap_or("").into())
                .collect::<Vec<SharedString>>(),
        ));
    }

    fn generate(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        {
            let mut s = self.session.borrow_mut();
            wipe::words(&mut s.words);
            match passphrase::generate(Self::target(&ui), passphrase::hardware_u32) {
                Some(words) => {
                    s.words = words;
                    s.rng_failed = false;
                }
                None => s.rng_failed = true,
            }
            // a fresh passphrase is shown: the user asked for it
            s.shown = true;
        }
        ui.global::<PassphraseState>().set_revealed(true);
        self.render(true);
    }

    fn die(&self, pips: i32) {
        let Some(ui) = self.ui.upgrade() else { return };
        {
            let mut s = self.session.borrow_mut();
            if !(1..=6).contains(&pips) || s.words.len() >= Self::target(&ui) {
                return;
            }
            s.rolls.push(char::from(b'0' + pips as u8));
            if let Some(word) = passphrase::word_for_rolls(&s.rolls) {
                let key = std::mem::take(&mut s.rolls);
                s.words.push(word);
                s.keys.push(key);
            }
        }
        self.render(false);
    }

    fn undo(&self) {
        {
            let mut s = self.session.borrow_mut();
            if s.rolls.pop().is_none() {
                // Nothing typed for the current word: take the last finished word back, minus its last die.
                if let (Some(_), Some(mut key)) = (s.words.pop(), s.keys.pop()) {
                    key.pop();
                    s.rolls = key;
                }
            }
        }
        self.render(false);
    }

    /// The UI fades the old words out first, then asks for this: after "Start over", after a mode or
    /// word-count change, and when the idle timeout wipes the session.
    fn restart(&self) {
        self.session.borrow_mut().clear();
        self.render(false);
    }

    /// The system hid the app (another app or the launcher took the foreground; a locking screen is expected to
    /// do the same). Cover the words at once, so that the UI keeps no copy of them while nobody is looking, and
    /// note the time. They stay covered when the app comes back; "Show words" hands them over again.
    fn hidden(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.hidden_at.get().is_none() {
            self.hidden_at.set(Some(Instant::now()));
        }
        ui.invoke_hide_words();
    }

    /// The app is shown again. The platform runs no timers while it is hidden, so the idle timeout could not
    /// count that time; the UI is told how long it was, and wipes if the passphrase has been left that long.
    fn shown_again(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if let Some(since) = self.hidden_at.take() {
            ui.invoke_came_back(since.elapsed().as_secs().min(i32::MAX as u64) as i32);
        }
    }

    /// "Show words" was switched, by the user or by the idle timeout. While it is off, the UI holds no
    /// copy of the words; switching it on hands them over again.
    fn set_shown(&self, on: bool) {
        let mut s = self.session.borrow_mut();
        s.shown = on;
        for (i, &word) in s.words.iter().enumerate() {
            if i < self.rows.row_count() {
                self.rows.set_row_data(i, make_word(word, on));
            }
        }
    }
}

fn app_main(cx: AppContext, ui: AppWindow) {
    log_server::init_wait(env!("CARGO_CRATE_NAME")).unwrap();
    log::set_max_level(log::LevelFilter::Info);

    theme::init(&ui);

    let rows = Rc::new(VecModel::<Word>::default());
    let state = ui.global::<PassphraseState>();
    state.set_words(ModelRc::from(rows.clone()));

    let ctx = Rc::new(Ctx {
        ui: ui.as_weak(),
        session: RefCell::new(Session::default()),
        rows,
        hidden_at: Cell::new(None),
    });
    ctx.render(false);

    state.on_generate({
        let ctx = ctx.clone();
        move || ctx.generate()
    });
    state.on_die({
        let ctx = ctx.clone();
        move |pips| ctx.die(pips)
    });
    state.on_undo({
        let ctx = ctx.clone();
        move || ctx.undo()
    });
    state.on_restart({
        let ctx = ctx.clone();
        move || ctx.restart()
    });
    state.on_reveal_changed({
        let ctx = ctx.clone();
        move |on| ctx.set_shown(on)
    });
    cx.set_input_handler({
        let ctx = ctx.clone();
        move |input| match input.msg {
            InputMessage::Hidden => ctx.hidden(),
            InputMessage::Visible => ctx.shown_again(),
            _ => {}
        }
    });

    ui.run().expect("UI running");
}
