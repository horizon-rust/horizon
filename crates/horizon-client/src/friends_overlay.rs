//! `FriendsOverlayScreen`: the account's friends and pending requests over the title or
//! the pause menu, from the service `social` keeps (`MinecraftServicesFriendsService`).
//! Vanilla's layout: a 220-wide panel centred in the screen, two 110-wide tabs above
//! it, the add-friend row at its top, then the friends or the requests, scrolled.
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    ui,
};
use horizon_client_render::social::{self, Friend, PresenceStatus, State};
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const PANEL_WIDTH: f32 = 220.0;
/// `FriendsOverlayScreen.TAB_BUTTON_WIDTH`/`HEIGHT`, and the 7 above the panel.
const TAB_WIDTH: f32 = 110.0;
const TAB_HEIGHT: f32 = 20.0;
const TAB_GAP: f32 = 7.0;
/// `AddFriendWidget`: the input row (20) with 3 above, the profile row (9) with 6, the
/// separator (2) with 4.
const ADD_HEIGHT: f32 = 44.0;
/// `AbstractFriendsEntryContainerWidget`'s entries.
const ENTRY_HEIGHT: f32 = 28.0;
const GREY: u32 = 0xFFA0_A0A0;
const GREEN: u32 = 0xFF00_FF00;

/// The field this screen edits (copy and paste).
const ADD_FRIEND: crate::clipboard::FieldId = ("friends.add", 0);

/// What a worker hands back.
type Job<T> = Arc<Mutex<Option<T>>>;

fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Job<T> {
    let job = Arc::new(Mutex::new(None));
    let out = Arc::clone(&job);
    if let Err(e) = std::thread::Builder::new()
        .name("Friends".into())
        .spawn(move || {
            let result = work();
            if let Ok(mut slot) = out.lock() {
                *slot = Some(result);
            }
        })
    {
        log::warn!("Horizon: could not start a friends worker: {e}");
    }
    job
}

fn take<T>(job: &Job<T>) -> Option<T> {
    job.lock().ok()?.take()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Friends,
    Requests,
}

/// A row's button: the title's `ButtonSpec` plus the sprite it draws.
pub type FriendButton = (f32, f32, f32, String, bool, u8, Option<(&'static str, f32)>);

pub struct Screen {
    tab: Tab,
    add: String,
    add_focused: bool,
    caret: RefCell<crate::ui::Caret>,
    offset: Cell<f32>,
    /// A friend whose removal is asked about (`FriendEntry.confirmRemoveFriend`).
    confirm: Option<Friend>,
    /// `FriendsListConfirmScreen`, before the list is turned on from the title.
    enable_confirm: bool,
    sending: Option<Job<social::ResultCode>>,
    action: Option<Job<social::ResultCode>>,
    /// A failure toast (`SystemToast`), and when it started.
    toast: Option<(String, Instant)>,
    started: Instant,
    faces: RefCell<Faces>,
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Screen {
    /// The title's `Mode` clones itself for its error screen; the faces are looked up
    /// again by the copy.
    fn clone(&self) -> Self {
        Self {
            tab: self.tab,
            add: self.add.clone(),
            add_focused: self.add_focused,
            caret: RefCell::new(self.caret.borrow().clone()),
            offset: Cell::new(self.offset.get()),
            confirm: self.confirm.clone(),
            enable_confirm: self.enable_confirm,
            sending: self.sending.clone(),
            action: self.action.clone(),
            toast: self.toast.clone(),
            started: self.started,
            faces: RefCell::new(Faces::new()),
        }
    }
}

impl std::fmt::Debug for Screen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("friends::Screen")
            .field("tab", &self.tab)
            .finish_non_exhaustive()
    }
}

impl Screen {
    pub fn new() -> Self {
        Self {
            tab: Tab::Friends,
            add: String::new(),
            add_focused: true,
            caret: RefCell::new(crate::ui::Caret::default()),
            offset: Cell::new(0.0),
            confirm: None,
            enable_confirm: false,
            sending: None,
            action: None,
            toast: None,
            started: Instant::now(),
            faces: RefCell::new(Faces::new()),
        }
    }

    /// `OnlineOptionsScreen.confirmFriendsListEnabled`: the list turns on only after the
    /// player agreed (`FriendsListConfirmScreen`).
    pub fn new_gated() -> Self {
        let mut screen = Self::new();
        screen.enable_confirm = !social::is_enabled();
        screen.add_focused = false;
        screen
    }

    pub fn tick(&mut self) {
        social::set_foreground(true);
        if let Some(result) = self.sending.as_ref().and_then(take) {
            self.sending = None;
            if result == social::ResultCode::Success {
                self.add.clear();
            } else {
                self.failed(result);
            }
        }
        if let Some(result) = self.action.as_ref().and_then(take) {
            self.action = None;
            if result != social::ResultCode::Success {
                self.failed(result);
            }
        }
        if let Some((_, at)) = &self.toast
            && at.elapsed() > Duration::from_secs(5)
        {
            self.toast = None;
        }
    }

    fn failed(&mut self, code: social::ResultCode) {
        let key = match code {
            social::ResultCode::TooManyRequests => "gui.friends.error.rateLimited.message",
            social::ResultCode::UnknownProfile => "gui.friends.error.user_may_lack_active_profile",
            social::ResultCode::Unauthorized => "gui.friends.error.unauthorized",
            social::ResultCode::Forbidden => "gui.friends.error.forbidden.message",
            social::ResultCode::ServiceNotAvailable | social::ResultCode::TemporaryUnavailable => {
                "gui.friends.error.unavailable.message"
            }
            _ => "gui.friends.error.failed.message",
        };
        self.toast = Some((crate::lang::get(key), Instant::now()));
    }

    fn lists(&self) -> social::FriendData {
        social::data()
    }

    /// The friends sorted as `FriendEntry.presenceStatusSortOrder`.
    fn friends(&self) -> Vec<Friend> {
        let presence = social::presence();
        let mut friends = self.lists().friends;
        friends.sort_by_key(|friend| {
            presence
                .iter()
                .find(|p| p.id == friend.id)
                .map_or(PresenceStatus::Offline, |p| p.status)
                .sort_order()
        });
        friends
    }

    fn panel((w, h): (f32, f32)) -> (f32, f32, f32) {
        let height = (h - 80.0).max(60.0);
        ((w - PANEL_WIDTH) / 2.0, (h - height) / 2.0, height)
    }

    fn field_rect((w, h): (f32, f32)) -> [f32; 4] {
        let (x, y, _) = Self::panel((w, h));
        // `AddFriendWidget`: width - 20 - 3 - 16, 8 from the panel's left and 3 down.
        [x + 8.0, y + 3.0, PANEL_WIDTH - 20.0 - 3.0 - 16.0, 20.0]
    }

    /// The add-friend field's box, for the caret's press (`focused_rect`).
    pub fn focused_box(&self, (w, h): (f32, f32)) -> Option<[f32; 4]> {
        (self.add_focused && !self.enable_confirm && self.confirm.is_none())
            .then(|| Self::field_rect((w, h)))
    }

    pub fn focused_text(&mut self) -> Option<(crate::clipboard::FieldId, &mut String, usize)> {
        if !self.add_focused || self.confirm.is_some() || self.enable_confirm {
            return None;
        }
        Some((ADD_FRIEND, &mut self.add, 16))
    }

    /// Whether the add-friend box takes text (`EditBox.isFocused`).
    pub fn add_focused(&self) -> bool {
        self.add_focused && self.confirm.is_none() && !self.enable_confirm
    }

    /// The panel and its tabs under the pointer (`FriendsOverlayScreen.mouseClicked`'s
    /// rectangle); a click outside closes the overlay.
    pub fn contains(&self, cursor: (f32, f32), (w, h): (f32, f32)) -> bool {
        let (x, y, height) = Self::panel((w.ceil(), h.ceil()));
        let top = y - TAB_GAP - TAB_HEIGHT;
        ui::hovered(
            cursor,
            x - 8.0,
            top,
            PANEL_WIDTH + 16.0,
            height + 16.0 + 8.0,
        )
    }

    /// `EditBox.charTyped` for the add-friend box.
    pub fn type_text(&mut self, text: &str) {
        if !self.add_focused() {
            return;
        }
        self.caret
            .borrow_mut()
            .edit(ADD_FRIEND, &mut self.add, 16, |edit| edit.insert_text(text));
    }

    /// `EditBox.keyPressed` for the add-friend box.
    pub fn key(&mut self, key: winit::keyboard::KeyCode, ctrl: bool, shift: bool) -> bool {
        if !self.add_focused() {
            return false;
        }
        self.caret
            .borrow_mut()
            .edit(ADD_FRIEND, &mut self.add, 16, |edit| {
                edit.key(key, ctrl, shift)
            })
    }

    /// A press in the add-friend box puts the caret under the pointer
    /// (`EditBox.onClick`).
    pub fn press_field(
        &mut self,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        (w, h): (f32, f32),
        shift: bool,
    ) {
        let rect = Self::field_rect((w.ceil(), h.ceil()));
        if !ui::hovered(cursor, rect[0], rect[1], rect[2], rect[3]) || !self.add_focused() {
            return;
        }
        self.caret.borrow_mut().press(
            atlas,
            (ADD_FRIEND, &mut self.add, 16),
            rect,
            cursor.0,
            shift,
        );
    }

    pub fn scroll(&mut self, delta: f32, size: (f32, f32)) {
        if self.enable_confirm || self.confirm.is_some() {
            return;
        }
        self.offset.set(self.offset.get() - delta * 20.0);
        self.clamp_scroll(size);
    }

    fn content_height(&self) -> f32 {
        let rows = match self.tab {
            Tab::Friends => self.friends().len() + 1,
            Tab::Requests => {
                let data = self.lists();
                data.incoming.len()
                    + data.outgoing.len()
                    + usize::from(!data.incoming.is_empty())
                    + usize::from(!data.outgoing.is_empty())
            }
        };
        rows as f32 * ENTRY_HEIGHT + 12.0
    }

    pub fn clamp_scroll(&mut self, size: (f32, f32)) {
        let (_, _, height) = Self::panel(size);
        let max = (self.content_height() - (height - ADD_HEIGHT)).max(0.0);
        self.offset.set(self.offset.get().clamp(0.0, max));
    }

    /// The overlay's buttons, in the title's list.
    pub fn buttons(&self, (w, h): (f32, f32)) -> Vec<FriendButton> {
        if self.enable_confirm {
            let (cx, y) = (w / 2.0, h / 2.0 + 12.0);
            return vec![
                (
                    cx - 155.0,
                    y,
                    150.0,
                    crate::lang::get("options.friendsList.confirm.turnOn"),
                    true,
                    20,
                    None,
                ),
                (
                    cx + 5.0,
                    y,
                    150.0,
                    crate::lang::get("options.friendsList.confirm.turnOff"),
                    true,
                    21,
                    None,
                ),
            ];
        }
        if self.confirm.is_some() {
            let (cx, y) = (w / 2.0, h / 2.0 + 12.0);
            return vec![
                (
                    cx - 155.0,
                    y,
                    150.0,
                    crate::lang::get("gui.remove"),
                    true,
                    30,
                    None,
                ),
                (
                    cx + 5.0,
                    y,
                    150.0,
                    crate::lang::get("gui.cancel"),
                    true,
                    31,
                    None,
                ),
            ];
        }
        let (x, y, height) = Self::panel((w, h));
        let mut buttons = vec![
            (
                x,
                y - TAB_GAP - TAB_HEIGHT,
                TAB_WIDTH,
                crate::lang::get("gui.friends.tab_friends"),
                true,
                1,
                None,
            ),
            (
                x + TAB_WIDTH,
                y - TAB_GAP - TAB_HEIGHT,
                TAB_WIDTH,
                crate::lang::tr(
                    "gui.friends.requests_count",
                    &[&self.lists().incoming.len() as &dyn std::fmt::Display],
                ),
                true,
                2,
                None,
            ),
            (
                x + PANEL_WIDTH - 8.0 - 20.0,
                y + 3.0,
                20.0,
                String::new(),
                social::state() == State::Success
                    && !self.add.trim().is_empty()
                    && self.sending.is_none(),
                3,
                Some(("gui/sprites/friends/send_request", 15.0)),
            ),
        ];
        let offset = self.offset.get();
        let top = y + ADD_HEIGHT;
        let bottom = y + height;
        if self.tab == Tab::Friends {
            for (i, _) in self.friends().iter().enumerate() {
                let Ok(id) = u8::try_from(40 + i) else {
                    break;
                };
                let row = top + i as f32 * ENTRY_HEIGHT - offset;
                if row + ENTRY_HEIGHT <= top || row >= bottom {
                    continue;
                }
                buttons.push((
                    x + PANEL_WIDTH - 8.0 - 20.0,
                    row + 4.0,
                    20.0,
                    String::new(),
                    self.action.is_none(),
                    id,
                    Some(("gui/sprites/friends/remove", 14.0)),
                ));
            }
        } else {
            let data = self.lists();
            let mut row = 0usize;
            let mut slot = 0usize;
            let at = |row: &mut usize| {
                let y = top + *row as f32 * ENTRY_HEIGHT - offset;
                *row += 1;
                y
            };
            if !data.incoming.is_empty() {
                let _ = at(&mut row);
            }
            for _ in &data.incoming {
                let row_y = at(&mut row);
                if row_y + ENTRY_HEIGHT > top
                    && row_y < bottom
                    && let Some(base) = request_id(slot, 0)
                {
                    buttons.push((
                        x + PANEL_WIDTH - 8.0 - 20.0 - 4.0 - 20.0,
                        row_y + 4.0,
                        20.0,
                        String::new(),
                        self.action.is_none(),
                        base,
                        Some(("gui/sprites/friends/accept", 18.0)),
                    ));
                    buttons.push((
                        x + PANEL_WIDTH - 8.0 - 20.0,
                        row_y + 4.0,
                        20.0,
                        String::new(),
                        self.action.is_none(),
                        base + 1,
                        Some(("gui/sprites/friends/reject", 18.0)),
                    ));
                }
                slot += 1;
            }
            if !data.outgoing.is_empty() {
                let _ = at(&mut row);
            }
            for _ in &data.outgoing {
                let row_y = at(&mut row);
                if row_y + ENTRY_HEIGHT > top
                    && row_y < bottom
                    && let Some(base) = request_id(slot, 0)
                {
                    buttons.push((
                        x + PANEL_WIDTH - 8.0 - 20.0,
                        row_y + 4.0,
                        20.0,
                        String::new(),
                        self.action.is_none(),
                        base,
                        Some(("gui/sprites/friends/cancel", 16.0)),
                    ));
                }
                slot += 1;
            }
        }
        buttons
    }

    pub fn click(&mut self, hit: Option<u8>, cursor: (f32, f32), size: (f32, f32)) {
        if let Some(id) = hit {
            match id {
                1 => {
                    self.tab = Tab::Friends;
                    self.offset.set(0.0);
                }
                2 => {
                    self.tab = Tab::Requests;
                    self.offset.set(0.0);
                }
                3 => self.send(),
                20 => {
                    self.enable_confirm = false;
                    self.action = Some(spawn(|| social::update_settings(true, true)));
                }
                21 => self.enable_confirm = false,
                30 => {
                    if let Some(friend) = self.confirm.take() {
                        self.action = Some(spawn(move || social::remove(friend.id)));
                    }
                }
                31 => self.confirm = None,
                id if (40..100).contains(&id) => {
                    if let Some(friend) = self.friends().get(usize::from(id - 40)).cloned() {
                        self.confirm = Some(friend);
                    }
                }
                id if id >= 100 => {
                    let slot = usize::from((id - 100) / 2);
                    let data = self.lists();
                    let target = data
                        .incoming
                        .get(slot)
                        .cloned()
                        .map(|friend| (friend, (id - 100) % 2 == 0))
                        .or_else(|| {
                            data.outgoing
                                .get(slot.checked_sub(data.incoming.len())?)
                                .cloned()
                                .map(|friend| (friend, true))
                        });
                    if let Some((friend, accept)) = target {
                        let job = if accept && data.incoming.len() > slot {
                            spawn(move || social::accept(friend.id))
                        } else {
                            spawn(move || social::remove(friend.id))
                        };
                        self.action = Some(job);
                    }
                }
                _ => {}
            }
            return;
        }
        let rect = Self::field_rect((size.0.ceil(), size.1.ceil()));
        self.add_focused = ui::hovered(cursor, rect[0], rect[1], rect[2], rect[3])
            && !self.enable_confirm
            && self.confirm.is_none();
    }

    fn send(&mut self) {
        let name = self.add.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let problem = if horizon_client_render::auth::current()
            .as_ref()
            .is_some_and(|s| s.name.eq_ignore_ascii_case(&name))
        {
            Some("gui.friends.validation.cannot_add_self")
        } else if self
            .lists()
            .friends
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(&name))
        {
            Some("gui.friends.validation.already_friend")
        } else if self
            .lists()
            .outgoing
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(&name))
        {
            Some("gui.friends.validation.already_outgoing")
        } else if self
            .lists()
            .incoming
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(&name))
        {
            Some("gui.friends.validation.already_incoming")
        } else {
            None
        };
        if let Some(key) = problem {
            self.toast = Some((crate::lang::get(key), Instant::now()));
            return;
        }
        self.sending = Some(spawn(move || social::send_request(&name)));
    }

    /// `FriendsOverlayScreen.keyPressed`: Escape closes, unless a question is open.
    /// True when the overlay should close.
    pub fn escape(&mut self) -> bool {
        if self.enable_confirm {
            self.enable_confirm = false;
            return true;
        }
        if self.confirm.take().is_some() {
            return false;
        }
        true
    }

    pub fn enter(&mut self) {
        if !self.enable_confirm
            && self.confirm.is_none()
            && !self.add.trim().is_empty()
            && social::state() == State::Success
        {
            self.send();
        }
    }

    pub fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        (w, h): (f32, f32),
    ) {
        let (x, y, height) = Self::panel((w, h));
        // The screen behind stays, dimmed (`extractBlurredBackground`).
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 140));
        if self.enable_confirm {
            self.draw_enable_confirm(atlas, batch, (w, h));
            return;
        }
        ui::nine_slice(
            batch,
            atlas,
            "gui/sprites/friends/background",
            x - 8.0,
            y - 8.0,
            PANEL_WIDTH + 16.0,
            height + 16.0 + 1.0,
            u32::MAX,
        );
        // The selected tab's underline (`FriendsOverlayTabButton.renderFocusUnderline`).
        let label = if self.tab == Tab::Friends {
            crate::lang::get("gui.friends.tab_friends")
        } else {
            crate::lang::tr(
                "gui.friends.requests_count",
                &[&self.lists().incoming.len() as &dyn std::fmt::Display],
            )
        };
        let width = HudBatch::text_width(atlas, &label).min(TAB_WIDTH - 4.0);
        let tab = if self.tab == Tab::Friends {
            0.0
        } else {
            TAB_WIDTH
        };
        batch.fill(
            atlas,
            x + tab + (TAB_WIDTH - width) / 2.0,
            y - TAB_GAP - 2.0,
            width,
            1.0,
            u32::MAX,
        );

        let [fx, fy, fw, fh] = Self::field_rect((w, h));
        self.caret.borrow_mut().draw(
            batch,
            atlas,
            (ADD_FRIEND, &self.add, 16),
            [fx, fy, fw, fh],
            self.add_focused,
            self.started,
        );
        if !self.add_focused && self.add.is_empty() {
            batch.text(
                atlas,
                &crate::lang::get("gui.friends.enter_nickname"),
                fx + 5.0,
                fy + 6.0,
                GREY,
                false,
            );
        }
        // The profile row (`AddFriendWidget.createProfileRow`).
        let name = horizon_client_render::auth::current()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let label = crate::lang::get("gui.friends.my_profile_name");
        let label_w = HudBatch::text_width(atlas, &label);
        batch.text(atlas, &label, x + 8.0, fy + fh + 6.0, GREY, false);
        batch.text(
            atlas,
            &name,
            x + 8.0 + label_w + 2.0,
            fy + fh + 6.0,
            u32::MAX,
            false,
        );
        crate::ui::tiled_sized(
            batch,
            atlas,
            "gui/sprites/friends/list_separator_top",
            x,
            y + ADD_HEIGHT - 2.0,
            PANEL_WIDTH,
            2.0,
            [32.0, 2.0],
            u32::MAX,
        );
        let top = y + ADD_HEIGHT;
        let bottom = y + height;
        match self.tab {
            Tab::Friends => self.draw_friends(atlas, batch, (x, top, bottom)),
            Tab::Requests => self.draw_requests(atlas, batch, (x, top, bottom)),
        }
        if self.confirm.is_some() {
            self.draw_confirm(atlas, batch, (w, h));
        }
        if let Some((message, _)) = &self.toast {
            let width = (HudBatch::text_width(atlas, message) + 16.0).max(160.0);
            ui::nine_slice(
                batch,
                atlas,
                "gui/sprites/friends/toast_background",
                w - width - 4.0,
                4.0,
                width,
                24.0,
                u32::MAX,
            );
            batch.text(atlas, message, w - width + 4.0, 10.0, u32::MAX, false);
        }
        let _ = cursor;
    }

    fn draw_friends(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (x, top, bottom): (f32, f32, f32),
    ) {
        let state = social::state();
        if state != State::Success {
            centered(
                batch,
                atlas,
                &state_text(state, true),
                x + PANEL_WIDTH / 2.0,
                top + 12.0,
                GREY,
            );
            return;
        }
        let friends = self.friends();
        let offset = self.offset.get();
        let first = batch.vertices.len();
        if friends.is_empty() {
            let cx = x + PANEL_WIDTH / 2.0;
            let mid = (top + bottom) / 2.0;
            ui::blit_sprite(
                batch,
                atlas,
                "gui/sprites/friends/illustrations_00",
                cx - 64.0,
                mid - 40.0,
                128.0,
                48.0,
                u32::MAX,
            );
            centered(
                batch,
                atlas,
                &crate::lang::get("gui.friends.empty_state"),
                cx,
                mid + 12.0,
                GREY,
            );
        }
        for (i, friend) in friends.iter().enumerate() {
            let row = top + i as f32 * ENTRY_HEIGHT - offset;
            if row + ENTRY_HEIGHT <= top || row >= bottom {
                continue;
            }
            self.draw_entry(atlas, batch, x, row, friend, false);
        }
        // `FriendsTab.createManageAccountFooter`.
        let footer = top + friends.len() as f32 * ENTRY_HEIGHT - offset + 4.0;
        if footer > top && footer < bottom {
            centered(
                batch,
                atlas,
                &crate::lang::get("gui.friends.manage_account_footer"),
                x + PANEL_WIDTH / 2.0,
                footer,
                GREY,
            );
        }
        batch.clip_since(first, x, top, PANEL_WIDTH, bottom);
    }

    fn draw_requests(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (x, top, bottom): (f32, f32, f32),
    ) {
        let state = social::state();
        if state != State::Success {
            centered(
                batch,
                atlas,
                &state_text(state, false),
                x + PANEL_WIDTH / 2.0,
                top + 12.0,
                GREY,
            );
            return;
        }
        let data = self.lists();
        if data.incoming.is_empty() && data.outgoing.is_empty() {
            centered(
                batch,
                atlas,
                &crate::lang::get("gui.friends.pending.empty"),
                x + PANEL_WIDTH / 2.0,
                (top + bottom) / 2.0,
                GREY,
            );
            return;
        }
        let offset = self.offset.get();
        let first = batch.vertices.len();
        let mut row = 0usize;
        if !data.incoming.is_empty() {
            let y = top - offset;
            if y + ENTRY_HEIGHT > top && y < bottom {
                centered(
                    batch,
                    atlas,
                    &crate::lang::get("gui.friends.pending.received"),
                    x + PANEL_WIDTH / 2.0,
                    y + 8.0,
                    u32::MAX,
                );
            }
            row += 1;
            for friend in &data.incoming {
                let y = top + row as f32 * ENTRY_HEIGHT - offset;
                row += 1;
                if y + ENTRY_HEIGHT <= top || y >= bottom {
                    continue;
                }
                self.draw_entry(atlas, batch, x, y, friend, true);
            }
        }
        if !data.outgoing.is_empty() {
            let y = top + row as f32 * ENTRY_HEIGHT - offset;
            if y + ENTRY_HEIGHT > top && y < bottom {
                centered(
                    batch,
                    atlas,
                    &crate::lang::get("gui.friends.pending.sent"),
                    x + PANEL_WIDTH / 2.0,
                    y + 8.0,
                    u32::MAX,
                );
            }
            row += 1;
            for friend in &data.outgoing {
                let y = top + row as f32 * ENTRY_HEIGHT - offset;
                row += 1;
                if y + ENTRY_HEIGHT <= top || y >= bottom {
                    continue;
                }
                self.draw_entry(atlas, batch, x, y, friend, true);
            }
        }
        batch.clip_since(first, x, top, PANEL_WIDTH, bottom);
    }

    fn draw_entry(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        friend: &Friend,
        request: bool,
    ) {
        // `PlayerFaceWidget`: 24x24 at the entry's left, vertically centred.
        if let Some(sprite) = self.faces.borrow_mut().face(batch, atlas, friend.id) {
            ui::blit_sprite(
                batch,
                atlas,
                &sprite,
                x + 8.0,
                y + 2.0,
                24.0,
                24.0,
                u32::MAX,
            );
        } else {
            batch.fill(atlas, x + 8.0, y + 2.0, 24.0, 24.0, 0xFF30_3030);
        }
        let text_x = x + 8.0 + 24.0 + 4.0;
        // `AbstractFriendsEntryContainerWidget`: a name with a status is three-thirds
        // down the entry; a request's name alone is centred.
        let name_y = if request { y + 9.0 } else { y + 6.0 };
        batch.text(atlas, &friend.name, text_x, name_y, u32::MAX, true);
        if !request {
            let (text, color) = match social::presence_of(friend.id) {
                Some(status) if status != PresenceStatus::Offline => {
                    (crate::lang::get(status.key()), GREEN)
                }
                _ => (
                    crate::lang::get("gui.friends.presence.status.offline"),
                    GREY,
                ),
            };
            batch.text(atlas, &text, text_x, y + 17.0, color, false);
        }
    }

    fn draw_enable_confirm(&self, atlas: &HudAtlas, batch: &mut HudBatch, (w, h): (f32, f32)) {
        let width = 316.0_f32.min(w - 16.0);
        let height = 150.0_f32.min(h - 16.0);
        let (x, y) = ((w - width) / 2.0, (h - height) / 2.0);
        ui::nine_slice(
            batch,
            atlas,
            "gui/sprites/friends/background_dark",
            x,
            y,
            width,
            height,
            u32::MAX,
        );
        centered(
            batch,
            atlas,
            &crate::lang::get("options.friendsList.confirm.title"),
            w / 2.0,
            y + 18.0,
            u32::MAX,
        );
        for (i, line) in crate::multiplayer::wrap(
            &crate::lang::get("options.friendsList.confirm.message"),
            width - 20.0,
            atlas,
        )
        .iter()
        .enumerate()
        {
            batch.text(
                atlas,
                line,
                x + 10.0,
                y + 40.0 + i as f32 * 9.0,
                u32::MAX,
                false,
            );
        }
    }

    fn draw_confirm(&self, atlas: &HudAtlas, batch: &mut HudBatch, (w, h): (f32, f32)) {
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 120));
        let width = 300.0_f32.min(w - 16.0);
        let height = 120.0_f32.min(h - 16.0);
        let (x, y) = ((w - width) / 2.0, (h - height) / 2.0);
        ui::nine_slice(
            batch,
            atlas,
            "gui/sprites/friends/background_dark",
            x,
            y,
            width,
            height,
            u32::MAX,
        );
        centered(
            batch,
            atlas,
            &crate::lang::get("gui.friends.confirm_title"),
            w / 2.0,
            y + 18.0,
            u32::MAX,
        );
        centered(
            batch,
            atlas,
            &crate::lang::get("gui.friends.confirm_unfriend"),
            w / 2.0,
            y + 40.0,
            u32::MAX,
        );
    }
}

/// A request row's button ids: 100 + slot * 2, accept/cancel at +0 and decline at +1.
fn request_id(slot: usize, offset: u8) -> Option<u8> {
    u8::try_from(100usize + slot * 2 + usize::from(offset)).ok()
}

fn state_text(state: State, friends: bool) -> String {
    crate::lang::get(match state {
        State::Loading if friends => "gui.friends.loading_friends",
        State::Loading => "gui.friends.loading_requests",
        State::UpgradeNeeded => "gui.friends.error.upgrade_needed",
        State::ConnectionIssue => "gui.friends.error.connection_issue",
        State::UserMayLackActiveProfile => "gui.friends.error.user_may_lack_active_profile",
        State::Unauthorized => "gui.friends.error.unauthorized",
        State::TemporaryUnavailable => "gui.friends.error.temporary_unavailable",
        State::GenericError => "gui.friends.error.generic",
        State::Success => "gui.friends.loading_friends",
    })
}

fn centered(batch: &mut HudBatch, atlas: &HudAtlas, text: &str, cx: f32, y: f32, color: u32) {
    let w = HudBatch::text_width(atlas, text);
    batch.text(atlas, text, cx - (w / 2.0).floor(), y, color, true);
}

/// The players' faces: `PlayerFaceWidget`'s 8x8 head in a `horizon/world-iconN` slot,
/// as the Realms screens draw one.
struct Faces {
    profiles: Arc<Mutex<horizon_client_render::skins::Resolver>>,
    downloads: Arc<Mutex<horizon_client_render::skins::Downloader>>,
    images: std::collections::HashMap<String, Vec<u8>>,
    slots: std::collections::HashMap<u128, usize>,
}

impl Faces {
    fn new() -> Self {
        let root = std::env::current_dir()
            .unwrap_or_default()
            .join("assets/skins");
        Self {
            profiles: Arc::new(Mutex::new(horizon_client_render::skins::Resolver::new())),
            downloads: Arc::new(Mutex::new(horizon_client_render::skins::Downloader::new(
                root,
            ))),
            images: std::collections::HashMap::new(),
            slots: std::collections::HashMap::new(),
        }
    }

    fn face(&mut self, batch: &mut HudBatch, atlas: &HudAtlas, id: u128) -> Option<String> {
        let slot = match self.slots.get(&id) {
            Some(slot) => *slot,
            None => {
                let slot = self.slots.len() % 32;
                self.slots.insert(id, slot);
                slot
            }
        };
        let (value, signature) = {
            let mut profiles = self.profiles.lock().ok()?;
            let horizon_client_render::skins::Resolve::Done(Some(profile)) =
                profiles.state(&horizon_client_render::skins::NameOrId::Id(id))
            else {
                return None;
            };
            profile.textures()?
        };
        let textures = horizon_client_render::skins::unpack(
            &value,
            signature,
            &horizon_client_render::skins::TEXTURE_URIS,
        );
        let url = textures.skin?;
        let location = horizon_client_render::skins::skin_location(
            &horizon_client_render::skins::texture_hash(&url)?,
        );
        let mut downloads = self.downloads.lock().ok()?;
        if downloads.state(horizon_client_render::skins::Kind::Skin, &url)
            != horizon_client_render::skins::Download::Ready
        {
            return None;
        }
        let _ = downloads.take_ready();
        let face = match self.images.get(&location) {
            Some(face) => face.clone(),
            None => {
                let skin = downloads.image(&location)?;
                let face = face_from_skin(skin)?;
                self.images.insert(location, face.clone());
                face
            }
        };
        drop(downloads);
        let sprite = format!("horizon/world-icon{slot}");
        let rect = atlas.sprites.get(&sprite)?;
        batch.patches.push(crate::font::Patch {
            x: (rect.u0 * atlas.width as f32).round() as u32,
            y: (rect.v0 * atlas.height as f32).round() as u32,
            width: 64,
            height: 64,
            rgba: face,
        });
        Some(sprite)
    }
}

/// `PlayerFaceExtractor`: the head's front 8x8 texels at (8, 8), the hat layer over
/// them, scaled to a 64x64 slot.
fn face_from_skin(skin: &[u8]) -> Option<Vec<u8>> {
    if skin.len() < 64 * 64 * 4 {
        return None;
    }
    let mut out = vec![0u8; 64 * 64 * 4];
    for y in 0..8 {
        for x in 0..8 {
            let base = ((8 + y) * 64 + 8 + x) * 4;
            let hat = ((8 + y) * 64 + 40 + x) * 4;
            let mut texel = [skin[base], skin[base + 1], skin[base + 2], skin[base + 3]];
            let hat_alpha = skin[hat + 3] as u16;
            if hat_alpha > 0 {
                for channel in 0..3 {
                    texel[channel] = ((skin[hat + channel] as u16 * hat_alpha
                        + texel[channel] as u16 * (255 - hat_alpha))
                        / 255) as u8;
                }
                texel[3] = 255;
            }
            for dy in 0..8 {
                for dx in 0..8 {
                    let target = ((y * 8 + dy) * 64 + x * 8 + dx) * 4;
                    out[target..target + 4].copy_from_slice(&texel);
                }
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FriendsOverlayScreen`: the panel is 220 wide and centred, its tabs sit 7 above
    /// it, and the add-friend box is the width the row leaves.
    #[test]
    fn the_panel_is_centred_and_the_field_is_vanillas_width() {
        let (x, y, height) = Screen::panel((854.0, 480.0));
        assert_eq!((x, y, height), ((854.0 - 220.0) / 2.0, 40.0, 400.0));
        let field = Screen::field_rect((854.0, 480.0));
        assert_eq!(field, [x + 8.0, y + 3.0, 181.0, 20.0]);
    }

    /// The requests' button ids stay inside a byte and name one slot each.
    #[test]
    fn request_ids_are_unique_and_bounded() {
        assert_eq!(request_id(0, 0), Some(100));
        assert_eq!(request_id(51, 1), Some(203));
        assert_eq!(request_id(78, 0), None);
    }

    /// `OnlineOptionsScreen.confirmFriendsListEnabled`: a list that is off asks first;
    /// Escape on that question closes the overlay.
    #[test]
    fn a_disabled_list_asks_before_it_opens() {
        let mut screen = Screen::new_gated();
        assert!(screen.enable_confirm);
        assert!(screen.escape());
        // A second open shows the panel instead.
        let screen = Screen::new_gated();
        assert!(screen.contains((427.0, 240.0), (854.0, 480.0)));
    }

    /// `FriendsListConfirmScreen`'s two answers are the two buttons.
    #[test]
    fn the_enable_question_offers_turn_on_and_turn_off() {
        let mut screen = Screen::new_gated();
        screen.enable_confirm = true;
        let buttons = screen.buttons((854.0, 480.0));
        assert_eq!((buttons[0].5, buttons[1].5), (20, 21));
    }
}
