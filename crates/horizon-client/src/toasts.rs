//! `ToastManager`'s slide, shared by the game's toasts and the title
//! screens' system toasts.
use crate::hud::{HudAtlas, HudBatch};
use std::{collections::VecDeque, time::Instant};

/// `ToastManager.ToastInstance`: slides a toast in and out over 600 ms,
/// eased by the square, and counts how long it has been fully shown.
#[derive(Default)]
pub struct Slide {
    /// When it first updated; the times below are milliseconds since.
    pub origin: Option<Instant>,
    begun: bool,
    /// `animationStartTime`.
    start: i64,
    /// `becameFullyVisibleAt`.
    fully_at: i64,
    show: bool,
    /// `visiblePortion`: 0 off screen, 1 fully in.
    pub portion: f32,
    /// `fullyVisibleFor`.
    pub fully_for: i64,
}
impl Slide {
    pub const MS: i64 = 600;
    /// `ToastInstance.update` at `now`; `wants_show` is the toast's own
    /// update. The visibility it switched to (the sound to play), and whether
    /// it has finished sliding out.
    pub fn update(
        &mut self,
        now: i64,
        wants_show: impl FnOnce(i64) -> bool,
    ) -> (Option<bool>, bool) {
        let was = self.show;
        if !self.begun {
            self.begun = true;
            self.start = now;
            self.show = true;
        }
        if self.show && now - self.start <= Self::MS {
            self.fully_at = now;
        }
        self.fully_for = now - self.fully_at;
        let progress = ((now - self.start) as f32 / Self::MS as f32).clamp(0.0, 1.0);
        let progress = progress * progress;
        self.portion = if self.show { progress } else { 1.0 - progress };
        let wanted = wants_show(self.fully_for);
        if wanted != self.show {
            self.start = now - ((1.0 - self.portion) * Self::MS as f32) as i64;
            self.show = wanted;
        }
        let finished = !self.show && now - self.start > Self::MS;
        ((self.show != was).then_some(self.show), finished)
    }
    /// `update` at the time since the first one.
    pub fn update_now(&mut self, wants_show: impl FnOnce(i64) -> bool) -> (Option<bool>, bool) {
        let origin = *self.origin.get_or_insert_with(Instant::now);
        self.update(origin.elapsed().as_millis() as i64, wants_show)
    }
}

/// `SystemToast`: a yellow title over a white message (or the title alone,
/// lower down) on the system background. It stays until its display time
/// has passed since its last change, or until it is hidden.
pub struct SystemToast {
    pub slide: Slide,
    /// `ToastInstance.firstSlotIndex`: its 32-pixel slot once shown.
    pub slot: Option<usize>,
    title: String,
    message: Option<String>,
    /// `changed`: the next update takes its time as `lastChanged`.
    changed: bool,
    last_changed: i64,
    force_hide: bool,
    /// `SystemToastId.displayTime`, in milliseconds.
    display_time: f64,
    /// The title's colour (`ChatFormatting.YELLOW` unless the text is styled).
    title_color: u32,
}
impl SystemToast {
    /// `SystemToastId.displayTime`'s default, in milliseconds.
    const DISPLAY_TIME: f64 = 5000.0;
    /// `SystemToastId.LOW_DISK_SPACE`'s and `UNSECURE_SERVER_WARNING`'s display time.
    pub const LONG_DISPLAY_TIME: f64 = 10000.0;
    pub fn new(title: String, message: Option<String>) -> Self {
        Self {
            slide: Slide::default(),
            slot: None,
            title,
            message,
            changed: false,
            last_changed: 0,
            force_hide: false,
            display_time: Self::DISPLAY_TIME,
            title_color: crate::hud::rgba(255, 255, 0, 255),
        }
    }
    /// A toast of an id with its own display time (`new SystemToastId(displayTime)`).
    pub fn with_display_time(mut self, ms: f64) -> Self {
        self.display_time = ms;
        self
    }
    /// A title with its own colour (`Component.withStyle(ChatFormatting.RED)`).
    pub fn with_title_color(mut self, color: u32) -> Self {
        self.title_color = color;
        self
    }
    /// `reset`: new text, and the display time starts again.
    pub fn reset(&mut self, title: String, message: Option<String>) {
        self.title = title;
        self.message = message;
        self.changed = true;
    }
    pub fn display_time(&self) -> f64 {
        self.display_time
    }
    /// `addOrUpdate` on a toast that is still shown: `reset` to `other`'s text.
    pub fn reset_from(&mut self, other: SystemToast) {
        self.title_color = other.title_color;
        self.reset(other.title, other.message);
    }
    pub fn force_hide(&mut self) {
        self.force_hide = true;
    }
    /// `ToastInstance.update` with `SystemToast.update`, the display time
    /// scaled by the Notification Time option: the visibility it switched
    /// to (the sound to play), and whether it has slid out.
    pub fn update(&mut self, multiplier: f64) -> (Option<bool>, bool) {
        self.slide.update_now(|fully| {
            if std::mem::take(&mut self.changed) {
                self.last_changed = fully;
            }
            !self.force_hide
                && ((fully - self.last_changed) as f64) < self.display_time * multiplier
        })
    }
    /// `extractRenderState` at `y`, in from the right edge by the visible
    /// portion (`Toast.xPos`).
    pub fn draw(&self, atlas: &HudAtlas, batch: &mut HudBatch, y: f32) {
        let (title, message) = self.lines(atlas);
        let width = title
            .iter()
            .chain(&message)
            .map(|line| HudBatch::text_width(atlas, line))
            .fold(0.0, f32::max)
            .ceil()
            .max(160.0)
            + 30.0;
        let height = Self::height_of(&title, &message);
        let x = batch.width - width * self.slide.portion;
        crate::ui::blit_sprite(
            batch,
            atlas,
            "gui/sprites/toast/system",
            x,
            y,
            width,
            height,
            u32::MAX,
        );
        let yellow = self.title_color;
        if message.is_empty() {
            for (i, line) in title.iter().enumerate() {
                batch.text(
                    atlas,
                    line,
                    x + 18.0,
                    y + 12.0 + i as f32 * 12.0,
                    yellow,
                    false,
                );
            }
        } else {
            for (i, line) in title.iter().enumerate() {
                batch.text(
                    atlas,
                    line,
                    x + 18.0,
                    y + 7.0 + i as f32 * 12.0,
                    yellow,
                    false,
                );
            }
            let top = 7.0 + title.len() as f32 * 12.0;
            for (i, line) in message.iter().enumerate() {
                batch.text(
                    atlas,
                    line,
                    x + 18.0,
                    y + top + i as f32 * 12.0,
                    u32::MAX,
                    false,
                );
            }
        }
    }

    /// `SystemToast.splitToLength`: 200 pixels a line.
    fn lines(&self, atlas: &HudAtlas) -> (Vec<String>, Vec<String>) {
        let title = crate::multiplayer::wrap(&self.title, 200.0, atlas);
        let message = self
            .message
            .as_deref()
            .map(|message| crate::multiplayer::wrap(message, 200.0, atlas))
            .unwrap_or_default();
        (title, message)
    }

    /// `SystemToast.height`: 20 + the title's extra lines + at least one
    /// message line, 12 each.
    fn height_of(title: &[String], message: &[String]) -> f32 {
        20.0 + (title.len().max(1) - 1) as f32 * 12.0 + message.len().max(1) as f32 * 12.0
    }

    /// The toast's height at the font it draws in.
    pub fn height(&self, atlas: &HudAtlas) -> f32 {
        let (title, message) = self.lines(atlas);
        Self::height_of(&title, &message)
    }
}
/// SystemToast's 32-pixel background keeps the notice icon and borders intact.
pub fn background(atlas: &HudAtlas, batch: &mut HudBatch, x: f32, y: f32, width: f32) {
    let Some(rect) = atlas.sprites.get("gui/sprites/toast/system").copied() else {
        return;
    };
    let source_x = [0.0, 17.0, rect.w as f32 - 4.0, rect.w as f32];
    let target_x = [x, x + 17.0, x + width - 4.0, x + width];
    for col in 0..3 {
        for (sy, sh, y, h) in [
            (0.0, 28.0, y, 28.0),
            (rect.h as f32 - 4.0, 4.0, y + 28.0, 4.0),
        ] {
            let mut piece = rect;
            let du = (rect.u1 - rect.u0) / rect.w as f32;
            let dv = (rect.v1 - rect.v0) / rect.h as f32;
            piece.u0 = rect.u0 + source_x[col] * du;
            piece.u1 = rect.u0 + source_x[col + 1] * du;
            piece.v0 = rect.v0 + sy * dv;
            piece.v1 = rect.v0 + (sy + sh) * dv;
            batch.rect_uv(
                target_x[col],
                y,
                target_x[col + 1] - target_x[col],
                h,
                piece,
                u32::MAX,
            );
        }
    }
}

/// `FriendToast`: a 160-wide `friends/toast_background`, the friend's 20x20
/// face at (6, 6) when one is known, and up to two lines of white text; it
/// slides like every toast and stays five seconds (times Notification Time).
pub struct FriendToast {
    pub slide: Slide,
    message: String,
    /// The friend whose face it shows (`ResolvableProfile`), when one is known.
    pub friend: Option<u128>,
}

impl FriendToast {
    const WIDTH: f32 = 160.0;
    const FACE_SIZE: f32 = 20.0;
    const TEXT_LEFT_WITH_FACE: f32 = 30.0;
    const TEXT_LEFT_NO_FACE: f32 = 7.0;
    const PADDING_TOP: f32 = 7.0;
    const PADDING_BOTTOM: f32 = 3.0;
    const LINE_SPACING: f32 = 11.0;

    pub fn new(message: String, friend: Option<u128>) -> Self {
        Self {
            slide: Slide::default(),
            message,
            friend,
        }
    }

    /// `FriendToast.update`: five seconds (128, `Notification Time` scaled).
    pub fn update(&mut self, multiplier: f64) -> (Option<bool>, bool) {
        self.slide
            .update_now(|fully| (fully as f64) < 5000.0 * multiplier)
    }

    fn lines(&self, atlas: &HudAtlas) -> Vec<String> {
        let text_left = if self.friend.is_some() {
            Self::TEXT_LEFT_WITH_FACE
        } else {
            Self::TEXT_LEFT_NO_FACE
        };
        crate::multiplayer::wrap(&self.message, Self::WIDTH - text_left - 4.0, atlas)
    }

    /// `FriendToast.height`: 7 + `max(lines, 2) * 11` + 3.
    pub fn height(&self, atlas: &HudAtlas) -> f32 {
        let lines = self.lines(atlas).len().max(2) as f32;
        Self::PADDING_TOP + lines * Self::LINE_SPACING + Self::PADDING_BOTTOM
    }

    /// `extractRenderState`: the face when known, the lines centred in the
    /// content's two-line height.
    pub fn draw(&self, atlas: &HudAtlas, batch: &mut HudBatch, y: f32, face: Option<&str>) {
        if self.slide.portion <= 0.0 {
            return;
        }
        let height = self.height(atlas);
        let x = batch.width - Self::WIDTH * self.slide.portion;
        crate::ui::blit_sprite(
            batch,
            atlas,
            "gui/sprites/friends/toast_background",
            x,
            y,
            Self::WIDTH,
            height,
            u32::MAX,
        );
        let face = face.filter(|_| self.friend.is_some());
        // `FriendToast.skinProfile` decides the text inset, even before the
        // asynchronous skin has downloaded and a face can be drawn.
        let text_left = if self.friend.is_some() {
            Self::TEXT_LEFT_WITH_FACE
        } else {
            Self::TEXT_LEFT_NO_FACE
        };
        if let Some(sprite) = face {
            crate::ui::blit_sprite(
                batch,
                atlas,
                sprite,
                x + 6.0,
                y + 6.0,
                Self::FACE_SIZE,
                Self::FACE_SIZE,
                u32::MAX,
            );
        } else if let Some(friend) = self.friend {
            let skin = crate::tab_list::default_skin(friend);
            for u in [8.0, 40.0] {
                crate::ui::blit_sprite_region(
                    batch,
                    atlas,
                    skin,
                    [u, 8.0, 8.0, 8.0],
                    x + 6.0,
                    y + 6.0,
                    Self::FACE_SIZE,
                    Self::FACE_SIZE,
                    u32::MAX,
                );
            }
        }
        let lines = self.lines(atlas);
        let content = (lines.len().max(2) as f32) * Self::LINE_SPACING;
        let top = Self::PADDING_TOP + (content - lines.len() as f32 * Self::LINE_SPACING) / 2.0;
        for (i, line) in lines.iter().enumerate() {
            batch.text(
                atlas,
                line,
                x + text_left,
                y + top + i as f32 * Self::LINE_SPACING,
                u32::MAX,
                false,
            );
        }
    }
}

/// The friends overlay's `ToastManager` queues, using the vanilla five 32-pixel
/// slots. A toast waiting for a slot is not updated, so it does not spend time.
const MAX_WAITING: usize = 64;

#[derive(Default)]
pub struct FriendsToasts {
    waiting: VecDeque<FriendToastEntry>,
    displayed: Vec<DisplayedFriendToast>,
    occupied: u32,
}

enum FriendToastEntry {
    System(SystemToast),
    Friend(FriendToast),
}

struct DisplayedFriendToast {
    toast: FriendToastEntry,
    slot: usize,
    slots: usize,
}

impl FriendsToasts {
    /// `FRIEND_SYSTEM_NOTIFICATION` has one token: reset displayed or pending.
    pub fn add_system(&mut self, title: String) {
        for entry in &mut self.displayed {
            if let FriendToastEntry::System(toast) = &mut entry.toast {
                toast.reset(title, None);
                return;
            }
        }
        for entry in &mut self.waiting {
            if let FriendToastEntry::System(toast) = entry {
                toast.reset(title, None);
                return;
            }
        }
        if self.waiting.len() >= MAX_WAITING {
            self.trim_waiting_for_new_friend();
        }
        self.waiting
            .push_back(FriendToastEntry::System(SystemToast::new(title, None)));
    }

    /// Friend notifications never merge. Keep at most 64 pending entries; if
    /// full, discard the oldest pending friend notification (never a system token).
    pub fn add_friend(&mut self, message: String, friend: Option<u128>) {
        self.trim_waiting_for_new_friend();
        self.waiting
            .push_back(FriendToastEntry::Friend(FriendToast::new(message, friend)));
    }

    fn trim_waiting_for_new_friend(&mut self) {
        if self.waiting.len() < MAX_WAITING {
            return;
        }
        if let Some(index) = self
            .waiting
            .iter()
            .position(|entry| matches!(entry, FriendToastEntry::Friend(_)))
        {
            self.waiting.remove(index);
        }
    }

    /// Update displayed entries, release their slots, then assign waiting entries.
    /// Return the first visibility transition this frame; a freshly assigned
    /// ToastInstance becomes visible at its next update (vanilla's order).
    pub fn update(&mut self, atlas: &HudAtlas, multiplier: f64, used: u32) -> Option<bool> {
        let mut sound = None;
        let mut retained = Vec::with_capacity(self.displayed.len());
        for mut entry in self.displayed.drain(..) {
            let (changed, finished) = match &mut entry.toast {
                FriendToastEntry::System(toast) => toast.update(multiplier),
                FriendToastEntry::Friend(toast) => toast.update(multiplier),
            };
            if sound.is_none() {
                sound = changed;
            }
            if finished {
                self.occupied &= !slot_mask(entry.slot, entry.slots);
            } else {
                retained.push(entry);
            }
        }
        self.displayed = retained;
        let mut available = self.occupied | used;
        let mut scanned = 0;
        while scanned < self.waiting.len() {
            let entry = self.waiting.pop_front().expect("length was checked");
            let slots = entry.slots(atlas);
            if let Some(slot) = first_free_slots(available, slots) {
                let mask = slot_mask(slot, slots);
                self.occupied |= mask;
                available |= mask;
                self.displayed.push(DisplayedFriendToast {
                    toast: entry,
                    slot,
                    slots,
                });
                scanned = 0;
            } else {
                self.waiting.push_back(entry);
                scanned += 1;
            }
        }
        sound
    }

    pub fn occupied_slots(&self) -> u32 {
        self.occupied
    }

    /// The caller controls global hidden/F1 state, as with vanilla ToastManager.
    pub fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        mut face: impl FnMut(&mut HudBatch, &HudAtlas, u128) -> Option<String>,
    ) {
        for entry in &self.displayed {
            match &entry.toast {
                FriendToastEntry::System(toast) => {
                    toast.draw(atlas, batch, entry.slot as f32 * toast.height(atlas));
                }
                FriendToastEntry::Friend(toast) => {
                    let face = toast.friend.and_then(|id| face(batch, atlas, id));
                    toast.draw(
                        atlas,
                        batch,
                        entry.slot as f32 * toast.height(atlas),
                        face.as_deref(),
                    );
                }
            }
        }
    }
}

impl FriendToastEntry {
    fn slots(&self, atlas: &HudAtlas) -> usize {
        let height = match self {
            Self::System(toast) => toast.height(atlas),
            Self::Friend(toast) => toast.height(atlas),
        };
        ((height / 32.0).ceil() as usize).max(1)
    }
}

fn slot_mask(slot: usize, slots: usize) -> u32 {
    ((1u32 << slots) - 1) << slot
}

fn first_free_slots(occupied: u32, slots: usize) -> Option<usize> {
    if slots == 0 || slots > 5 {
        return None;
    }
    (0..=5 - slots).find(|start| occupied & slot_mask(*start, slots) == 0)
}

/// `NowPlayingToast`'s `musicNoteColorTick`, `lastMusicNoteColorChange` and
/// `musicNoteColor`: the notes change colour every 25 ms, while a song plays.
pub struct MusicNotes {
    tick: i32,
    last_change: Option<Instant>,
    /// Opaque ARGB, white until the first change.
    pub color: u32,
}
impl Default for MusicNotes {
    fn default() -> Self {
        Self {
            tick: 0,
            last_change: None,
            color: u32::MAX,
        }
    }
}
impl MusicNotes {
    /// `NowPlayingToast.tickMusicNotes`, for a song that is playing.
    pub fn tick(&mut self) {
        if self
            .last_change
            .is_none_or(|at| at.elapsed().as_millis() > 25)
        {
            self.tick += 1;
            self.last_change = Some(Instant::now());
            self.color = music_note_color(self.tick);
        }
    }
}
/// `ColorLerper.getLerpedColor(Type.MUSIC_NOTE, tick)` as opaque ARGB: the
/// dye colours of `MUSIC_NOTE_COLORS` at 125 % (white is 0xE6E6E6), one per
/// 30 ticks, lerped in between.
fn music_note_color(tick: i32) -> u32 {
    // DyeColor ids of WHITE, LIGHT_GRAY, LIGHT_BLUE, BLUE, CYAN, GREEN, LIME,
    // YELLOW, ORANGE, PINK, RED and MAGENTA.
    const DYES: [usize; 12] = [0, 8, 3, 11, 9, 13, 5, 4, 1, 6, 14, 2];
    let rgb = |dye: usize| -> [i32; 3] {
        if dye == 0 {
            return [0xE6; 3];
        }
        let src = crate::mobs::render::DYE[dye];
        [16, 8, 0].map(|shift| (((src >> shift & 255) as f32 * 1.25).floor() as i32).clamp(0, 255))
    };
    let value = (tick / 30) as usize;
    let (from, to) = (
        rgb(DYES[value % DYES.len()]),
        rgb(DYES[(value + 1) % DYES.len()]),
    );
    let sub = (tick % 30) as f32 / 30.0;
    // ARGB.srgbLerp: Mth.lerpInt per channel.
    let lerp = |i: usize| (from[i] + (sub * (to[i] - from[i]) as f32).floor() as i32) as u32;
    0xFF00_0000 | lerp(0) << 16 | lerp(1) << 8 | lerp(2)
}

/// `NowPlayingToast` in its `ToastManager.ToastInstance`: it slides in from
/// the left edge at the top of the screen and goes out after five seconds
/// (scaled by the Notification Time option).
#[derive(Default)]
pub struct NowPlayingToast {
    slide: Slide,
    /// `updateToast`: `showToast` sets it, `onFinishedRendering` clears it.
    update_toast: bool,
    multiplier: f64,
    /// `wantedVisibility` is SHOW.
    wanted: bool,
    finished: bool,
}
impl NowPlayingToast {
    /// `ToastManager.showNowPlayingToast`: `resetToast`, then `showToast`.
    pub fn show(&mut self, multiplier: f64) {
        self.slide = Slide::default();
        self.finished = false;
        self.update_toast = true;
        self.multiplier = multiplier;
        self.wanted = true;
    }
    /// `ToastManager.hideNowPlayingToast`.
    pub fn hide(&mut self) {
        self.wanted = false;
    }
    /// `ToastInstance.update` with `NowPlayingToast.update`. `ToastManager`
    /// plays no visibility sound for this toast.
    pub fn update(&mut self, song_playing: bool, notes: &mut MusicNotes) {
        let (update_toast, multiplier) = (self.update_toast, self.multiplier);
        let wanted = &mut self.wanted;
        let (_, finished) = self.slide.update_now(|fully| {
            if update_toast {
                *wanted = (fully as f64) < 5000.0 * multiplier;
                if song_playing {
                    notes.tick();
                }
            }
            *wanted
        });
        if finished && !self.finished {
            self.update_toast = false;
        }
        self.finished = finished;
    }
    /// `ToastInstance.extractRenderState` with `Toast.xPos`: nothing once it
    /// has slid out.
    pub fn draw(&self, atlas: &HudAtlas, batch: &mut HudBatch, song: &str, notes: &MusicNotes) {
        if self.finished {
            return;
        }
        let width = Self::width(atlas, song);
        draw_now_playing(
            atlas,
            batch,
            width * self.slide.portion - width,
            song,
            notes.color,
        );
    }
    /// `NowPlayingToast.getWidth`.
    fn width(atlas: &HudAtlas, song: &str) -> f32 {
        30.0 + HudBatch::text_width(atlas, song).ceil() + 7.0
    }
}
/// `NowPlayingToast.extractToast` at `x` on the top row: the background, the
/// music notes in `notes` (ARGB) and the song's name in light grey.
pub fn draw_now_playing(atlas: &HudAtlas, batch: &mut HudBatch, x: f32, song: &str, notes: u32) {
    let width = NowPlayingToast::width(atlas, song);
    // `toast/now_playing`: a nine slice of 160 by 32 with a border of 4, here 30 high.
    if let Some(rect) = atlas.sprites.get("gui/sprites/toast/now_playing").copied() {
        let (du, dv) = (
            (rect.u1 - rect.u0) / rect.w as f32,
            (rect.v1 - rect.v0) / rect.h as f32,
        );
        let source_x = [0.0, 4.0, rect.w as f32 - 4.0, rect.w as f32];
        let source_y = [0.0, 4.0, rect.h as f32 - 4.0, rect.h as f32];
        let target_x = [x, x + 4.0, x + width - 4.0, x + width];
        let target_y = [0.0, 4.0, 26.0, 30.0];
        for row in 0..3 {
            for col in 0..3 {
                let piece = crate::hud::Rect {
                    u0: rect.u0 + source_x[col] * du,
                    u1: rect.u0 + source_x[col + 1] * du,
                    v0: rect.v0 + source_y[row] * dv,
                    v1: rect.v0 + source_y[row + 1] * dv,
                    ..rect
                };
                batch.rect_uv(
                    target_x[col],
                    target_y[row],
                    target_x[col + 1] - target_x[col],
                    target_y[row + 1] - target_y[row],
                    piece,
                    u32::MAX,
                );
            }
        }
    }
    if let Some(rect) = atlas.sprites.get("gui/sprites/icon/music_notes").copied() {
        batch.rect_uv(
            x + 7.0,
            7.0,
            16.0,
            16.0,
            rect,
            crate::mobs::render::abgr(notes & 0x00FF_FFFF),
        );
    }
    // DyeColor.LIGHT_GRAY.getTextColor, at 15 - 9 / 2.
    batch.text(atlas, song, x + 30.0, 11.0, 0xFFD3_D3D3, true);
}
