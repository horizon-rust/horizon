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
    time::Instant,
};

const PANEL_WIDTH: f32 = 220.0;
/// `CommonLinks.PRIVACY_AND_ONLINE_SETTINGS`, behind `MICROSOFT_ACCOUNT_LINK`.
pub const PRIVACY_AND_ONLINE_URL: &str = "https://aka.ms/MinecraftJavaXboxPrivacyAndSafety";
/// `FriendsOverlayScreen.TAB_BUTTON_WIDTH`/`HEIGHT`, and the 7 above the panel.
const TAB_WIDTH: f32 = 110.0;
const TAB_HEIGHT: f32 = 20.0;
const TAB_GAP: f32 = 7.0;
/// `AddFriendWidget`: the input row (20) with 3 above, the profile row (9) with 6, the
/// separator (2) with 4.
const ADD_HEIGHT: f32 = 44.0;
/// `AbstractFriendsEntryContainerWidget`'s entries.
const ENTRY_HEIGHT: f32 = 28.0;
/// `PendingTab`'s group headers: a `FocusableTextWidget` (9 + 4 padding twice).
const HEADER_HEIGHT: f32 = 17.0;
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
    /// The entry button showing loading after a press (`switchToLoadingAfterPress`).
    action_button: Option<u8>,
    /// A failure toast (`SystemToast`, `FRIEND_SYSTEM_NOTIFICATION`).
    toast: Option<crate::toasts::SystemToast>,
    /// `FriendToast`s: request sent/received, accepted, a friend added.
    friend_toasts: Vec<crate::toasts::FriendToast>,
    /// The lists the friend toasts last compared against.
    seen: Option<Seen>,
    /// The friend ids and the order the last populate gave them: presence
    /// updates change the status text but do not resort the rows.
    sort_cache: RefCell<Option<(Vec<u128>, Vec<u128>)>>,
    /// The scroller drag: the press's y, the offset then, its thumb and the
    /// list height (`AbstractScrollArea.scrolling`).
    scroll_drag: Cell<Option<(f32, f32, f32, f32)>>,
    /// The account link's boxes as last drawn (`MICROSOFT_ACCOUNT_LINK`), in the
    /// footer and the empty state.
    links: RefCell<Vec<[f32; 4]>>,
    /// The footer link's boxes: its click plays the button sound.
    footer_links: RefCell<Vec<[f32; 4]>>,
    /// The account link was clicked (`PrivacyConfirmLinkScreen.confirmLinkNow`).
    link_clicked: bool,
    /// The footer link was the one clicked (`createManageAccountFooter`).
    link_sound: bool,
    /// The profile name's box as last drawn (`PlainTextButton`).
    profile_box: Cell<[f32; 4]>,
    /// The profile name was clicked: copy it.
    profile_clicked: bool,
    /// Turn Off on `FriendsListConfirmScreen`: the overlay closes.
    cancelled: bool,
    /// `PopupScreen`'s and `FriendsListConfirmScreen`'s button boxes as drawn.
    confirm_buttons: Cell<[[f32; 4]; 2]>,
    enable_buttons: Cell<[[f32; 4]; 2]>,
    started: Instant,
    faces: RefCell<Faces>,
}

/// The last lists friend toasts were made from.
#[derive(Clone)]
struct Seen {
    friends: Vec<u128>,
    incoming: Vec<u128>,
    outgoing: Vec<u128>,
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
            action_button: self.action_button,
            toast: None,
            friend_toasts: Vec::new(),
            seen: self.seen.clone(),
            sort_cache: RefCell::new(self.sort_cache.borrow().clone()),
            scroll_drag: Cell::new(None),
            links: RefCell::new(Vec::new()),
            footer_links: RefCell::new(Vec::new()),
            link_clicked: false,
            link_sound: false,
            profile_box: Cell::new([-1000.0; 4]),
            profile_clicked: false,
            cancelled: false,
            confirm_buttons: Cell::new([[-1000.0; 4]; 2]),
            enable_buttons: Cell::new([[-1000.0; 4]; 2]),
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
            // `FriendsOverlayScreen.init` starts with no focused widget: the tab
            // bar takes the focus, so the edit box shows its hint until clicked.
            add_focused: false,
            caret: RefCell::new(crate::ui::Caret::default()),
            offset: Cell::new(0.0),
            confirm: None,
            enable_confirm: false,
            sending: None,
            action: None,
            action_button: None,
            toast: None,
            friend_toasts: Vec::new(),
            seen: None,
            sort_cache: RefCell::new(None),
            scroll_drag: Cell::new(None),
            links: RefCell::new(Vec::new()),
            footer_links: RefCell::new(Vec::new()),
            link_clicked: false,
            link_sound: false,
            profile_box: Cell::new([-1000.0; 4]),
            profile_clicked: false,
            cancelled: false,
            confirm_buttons: Cell::new([[-1000.0; 4]; 2]),
            enable_buttons: Cell::new([[-1000.0; 4]; 2]),
            started: Instant::now(),
            faces: RefCell::new(Faces::new()),
        }
    }

    /// `OnlineOptionsScreen.confirmFriendsListEnabled`: the list turns on only after the
    /// player agreed (`FriendsListConfirmScreen`).
    pub fn new_gated() -> Self {
        let mut screen = Self::new();
        screen.enable_confirm = !social::is_friend_list_enabled();
        screen.add_focused = false;
        screen
    }

    /// `multiplier` is Notification Time (`Options.notificationDisplayTime`).
    pub fn tick(&mut self, multiplier: f64) {
        social::set_foreground(true);
        if let Some(result) = self.sending.as_ref().and_then(take) {
            self.sending = None;
            let name = self.add.trim().to_owned();
            if result == social::ResultCode::Success {
                self.add.clear();
                self.add_focused = true;
                // `FriendToast.showFriendRequestSent`: no face.
                self.friend_toasts.push(crate::toasts::FriendToast::new(
                    crate::lang::tr(
                        "gui.friends.toast.request_sent.message",
                        &[&name as &dyn std::fmt::Display],
                    ),
                    None,
                ));
            } else {
                self.add_focused = true;
                self.failed(result);
            }
        }
        if let Some(result) = self.action.as_ref().and_then(take) {
            self.action = None;
            self.action_button = None;
            if result != social::ResultCode::Success {
                self.failed(result);
                // `onActionFailed`: refreshLists() reads the lists again.
                social::refresh_lists();
            }
        }
        self.poll_friend_toasts();
        // `SystemToast.update`: the id's display time scaled by Notification Time.
        if let Some(toast) = &mut self.toast {
            let (_, finished) = toast.update(multiplier);
            if finished {
                self.toast = None;
            }
        }
        self.friend_toasts.retain_mut(|toast| {
            let (_, finished) = toast.update(multiplier);
            !finished
        });
    }

    /// `PlayerSocialManager`'s pushes as `FriendToast`s: a new incoming request,
    /// a friend accepted from either side. The first successful read only
    /// records the lists.
    fn poll_friend_toasts(&mut self) {
        if social::friend_list_state() != State::Success {
            return;
        }
        let data = self.lists();
        let now = Seen {
            friends: data.friends.iter().map(|friend| friend.id).collect(),
            incoming: data.incoming.iter().map(|friend| friend.id).collect(),
            outgoing: data.outgoing.iter().map(|friend| friend.id).collect(),
        };
        if let Some(previous) = &self.seen {
            for friend in &data.incoming {
                if !previous.incoming.contains(&friend.id) {
                    self.friend_toasts.push(crate::toasts::FriendToast::new(
                        crate::lang::tr(
                            "gui.friends.toast.request_received.message",
                            &[&friend.name as &dyn std::fmt::Display],
                        ),
                        Some(friend.id),
                    ));
                }
            }
            for friend in &data.friends {
                if previous.friends.contains(&friend.id) {
                    continue;
                }
                let key = if previous.outgoing.contains(&friend.id) {
                    "gui.friends.toast.friend_added.message"
                } else if previous.incoming.contains(&friend.id) {
                    "gui.friends.toast.request_accepted.message"
                } else {
                    continue;
                };
                self.friend_toasts.push(crate::toasts::FriendToast::new(
                    crate::lang::tr(key, &[&friend.name as &dyn std::fmt::Display]),
                    Some(friend.id),
                ));
            }
        }
        self.seen = Some(now);
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
        self.show_system_toast(crate::lang::get(key));
    }

    /// `SystemToast.addOrUpdate(..., FRIEND_SYSTEM_NOTIFICATION, title, null)`: the
    /// yellow title on the system background, over the screen's own widgets.
    fn show_system_toast(&mut self, title: String) {
        self.toast = Some(crate::toasts::SystemToast::new(title, None));
    }

    fn lists(&self) -> social::FriendData {
        social::data()
    }

    /// The friends sorted as `FriendEntry.presenceStatusSortOrder` when the list
    /// was populated. A presence update only changes the status text
    /// (`applyPresenceUpdate`), so the order is cached until the friend ids do.
    fn friends(&self) -> Vec<Friend> {
        let mut friends = self.lists().friends;
        let ids: Vec<u128> = friends.iter().map(|friend| friend.id).collect();
        let mut cache = self.sort_cache.borrow_mut();
        let sorted = match cache.as_ref() {
            Some((input, sorted)) if *input == ids => sorted.clone(),
            _ => {
                let presence = social::presence();
                friends.sort_by_key(|friend| {
                    presence
                        .iter()
                        .find(|p| p.id == friend.id)
                        .map_or(PresenceStatus::Offline, |p| p.status)
                        .sort_order()
                });
                let sorted: Vec<u128> = friends.iter().map(|friend| friend.id).collect();
                *cache = Some((ids.clone(), sorted.clone()));
                sorted
            }
        };
        if sorted != ids {
            let mut by_id: std::collections::HashMap<u128, Friend> = friends
                .into_iter()
                .map(|friend| (friend.id, friend))
                .collect();
            friends = sorted.iter().filter_map(|id| by_id.remove(id)).collect();
        }
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
        self.add_focused().then(|| Self::field_rect((w, h)))
    }

    pub fn focused_text(&mut self) -> Option<(crate::clipboard::FieldId, &mut String, usize)> {
        if !self.add_focused
            || self.confirm.is_some()
            || self.enable_confirm
            || self.sending.is_some()
        {
            return None;
        }
        Some((ADD_FRIEND, &mut self.add, 32))
    }

    /// Whether the add-friend box takes text (`EditBox.isFocused`).
    pub fn add_focused(&self) -> bool {
        self.add_focused
            && self.confirm.is_none()
            && !self.enable_confirm
            && self.sending.is_none()
            // `FriendsTab.showLoading`/`showError`: `AddFriendWidget.SENDING` and
            // `DISABLED` take the edit box out of the screen.
            && social::friend_list_state() == State::Success
    }

    /// The add button shows its loading sprite (`AddFriendWidget.SENDING`): while
    /// the request is in flight or the list itself is loading.
    pub fn loading(&self) -> bool {
        self.sending.is_some() || social::friend_list_state() == State::Loading
    }

    /// The request is pending (`AddFriendWidget.State.SENDING`).
    pub fn sending(&self) -> bool {
        self.sending.is_some()
    }

    /// A tab's title (0 Friends, 1 Requests), for `TabNavigationBar` narration.
    pub fn tab_title(&self, index: usize) -> String {
        if index == 0 {
            crate::lang::get("gui.friends.tab_friends")
        } else {
            crate::lang::tr(
                "gui.friends.requests_count",
                &[&self.lists().incoming.len() as &dyn std::fmt::Display],
            )
        }
    }

    /// The selected tab's button id (1 Friends, 2 Requests).
    pub fn selected_tab(&self) -> u8 {
        match self.tab {
            Tab::Friends => 1,
            Tab::Requests => 2,
        }
    }

    /// `TabNavigationBar.keyPressed`: with Ctrl held, a digit picks a tab and
    /// Tab steps to the other one (with two tabs Shift+Tab lands there too).
    /// `None` for a key the bar leaves alone; otherwise whether the tab changed.
    pub fn tab_key(
        &mut self,
        key: winit::keyboard::KeyCode,
        ctrl: bool,
        _shift: bool,
    ) -> Option<bool> {
        use winit::keyboard::KeyCode;
        if !ctrl {
            return None;
        }
        let target = match key {
            KeyCode::Digit1 | KeyCode::Numpad1 => Tab::Friends,
            KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9
            | KeyCode::Digit0
            | KeyCode::Numpad0
            | KeyCode::Numpad2
            | KeyCode::Numpad3
            | KeyCode::Numpad4
            | KeyCode::Numpad5
            | KeyCode::Numpad6
            | KeyCode::Numpad7
            | KeyCode::Numpad8
            | KeyCode::Numpad9 => Tab::Requests,
            KeyCode::Tab => match self.tab {
                Tab::Friends => Tab::Requests,
                Tab::Requests => Tab::Friends,
            },
            _ => return None,
        };
        let changed = self.tab != target;
        if changed {
            self.tab = target;
            self.offset.set(0.0);
        }
        Some(changed)
    }

    /// A confirm screen covers the overlay (`PopupScreen`/`ConfirmScreen`).
    pub fn question_open(&self) -> bool {
        self.enable_confirm || self.confirm.is_some()
    }

    /// `AbstractScrollArea.updateScrolling`: a press on the scroller starts a drag.
    pub fn begin_scroll_drag(&mut self, cursor: (f32, f32), size: (f32, f32)) -> bool {
        if self.question_open() || self.sending.is_some() {
            return false;
        }
        let (x, y, height) = Self::panel(size);
        let top = y + ADD_HEIGHT;
        let bottom = y + height;
        let Some((_, thumb)) =
            scroller(self.content_height(), bottom - top, self.offset.get(), top)
        else {
            return false;
        };
        let bar_x = x + PANEL_WIDTH - 6.0;
        if cursor.0 >= bar_x && cursor.0 <= bar_x + 6.0 && cursor.1 >= top && cursor.1 < bottom {
            self.scroll_drag
                .set(Some((cursor.1, self.offset.get(), thumb, bottom - top)));
            true
        } else {
            false
        }
    }

    /// `AbstractScrollArea.mouseDragged` on the scroller.
    pub fn drag_scroll(&mut self, cursor: (f32, f32)) {
        let Some((start_y, start_offset, thumb, height)) = self.scroll_drag.get() else {
            return;
        };
        let content = self.content_height();
        let max = (content - height).max(0.0);
        let scale = (max / (height - thumb).max(1.0)).max(1.0);
        let offset = start_offset + (cursor.1 - start_y) * scale;
        self.offset.set(offset.clamp(0.0, max));
    }

    pub fn end_scroll_drag(&mut self) {
        self.scroll_drag.set(None);
    }

    /// The entry button showing `switchToLoadingAfterPress`.
    pub fn action_loading(&self, id: u8) -> bool {
        self.action.is_some() && self.action_button == Some(id)
    }

    /// The `PopupScreen`/`FriendsListConfirmScreen` button boxes as drawn.
    pub fn button_box(&self, id: u8) -> Option<[f32; 4]> {
        match id {
            20 | 21 => Some(self.enable_buttons.get()[usize::from(id - 20)]),
            30 | 31 => Some(self.confirm_buttons.get()[usize::from(id - 30)]),
            _ => None,
        }
    }

    /// `PlainTextButton` on the profile row: the name clicked, to copy.
    pub fn take_profile_copy(&mut self) -> Option<String> {
        std::mem::take(&mut self.profile_clicked).then(|| {
            horizon_client_render::auth::current()
                .map(|session| session.name.clone())
                .unwrap_or_default()
        })
    }

    /// `FriendsListConfirmScreen` answered Turn Off: return to the screen behind.
    pub fn take_cancelled(&mut self) -> bool {
        std::mem::take(&mut self.cancelled)
    }

    /// The footer link's click plays the button sound (`createManageAccountFooter`).
    pub fn take_link_sound(&mut self) -> bool {
        std::mem::take(&mut self.link_sound)
    }

    /// An icon button's tooltip (`SpriteIconButton`'s `Tooltip`).
    pub fn button_tooltip(&self, id: u8) -> Option<&'static str> {
        match id {
            3 if self.loading() => Some("gui.friends.button.loading"),
            3 if self.add.trim().is_empty() => Some("gui.friends.empty_nickname"),
            3 => Some("gui.friends.send_request"),
            id if (40..100).contains(&id) => Some("gui.friends.unfriend"),
            id if id >= 100 => {
                let slot = usize::from((id - 100) / 2);
                let data = self.lists();
                if slot < data.incoming.len() {
                    Some(if (id - 100) % 2 == 0 {
                        "gui.friends.accept"
                    } else {
                        "gui.friends.decline"
                    })
                } else if slot < data.incoming.len() + data.outgoing.len() {
                    Some("gui.friends.cancel_request")
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// The panel and its tabs under the pointer (`FriendsOverlayScreen.mouseClicked`'s
    /// rectangle); a click outside closes the overlay.
    pub fn contains(&self, cursor: (f32, f32), (w, h): (f32, f32)) -> bool {
        let (x, y, height) = Self::panel((w.ceil(), h.ceil()));
        let top = y - TAB_GAP - TAB_HEIGHT;
        // `FriendsOverlayScreen.mouseClicked`: from the tab bar's top to the
        // background's bottom edge (`layout.y + height + 16`).
        ui::hovered(
            cursor,
            x - 8.0,
            top,
            PANEL_WIDTH + 16.0,
            height + TAB_GAP + TAB_HEIGHT + 16.0,
        )
    }

    /// `EditBox.charTyped` for the add-friend box.
    pub fn type_text(&mut self, text: &str) {
        if !self.add_focused() || self.sending.is_some() {
            return;
        }
        self.caret
            .borrow_mut()
            .edit(ADD_FRIEND, &mut self.add, 32, |edit| edit.insert_text(text));
    }

    /// `EditBox.keyPressed` for the add-friend box.
    pub fn key(&mut self, key: winit::keyboard::KeyCode, ctrl: bool, shift: bool) -> bool {
        if !self.add_focused() || self.sending.is_some() {
            return false;
        }
        self.caret
            .borrow_mut()
            .edit(ADD_FRIEND, &mut self.add, 32, |edit| {
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
            (ADD_FRIEND, &mut self.add, 32),
            rect,
            cursor.0,
            shift,
        );
    }

    pub fn scroll(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
        if self.question_open() {
            return;
        }
        let (x, y, height) = Self::panel(size);
        let top = y + ADD_HEIGHT;
        // `AbstractScrollArea.mouseScrolled` only reaches the list under the
        // pointer; the add-friend row and the tabs are outside it.
        if !ui::hovered(cursor, x, top, PANEL_WIDTH, y + height - top) {
            return;
        }
        // `scrollY * scrollRate`, 10 here.
        self.offset.set(self.offset.get() - delta * 10.0);
        self.clamp_scroll(size);
    }

    fn content_height(&self) -> f32 {
        let data = self.lists();
        match self.tab {
            Tab::Friends if data.friends.is_empty() => 0.0,
            // `FriendsTab.updateEntries`: the entries, then the account footer
            // (a one-line `MultiLineTextWidget` at the list's 204-wide content).
            Tab::Friends => data.friends.len() as f32 * ENTRY_HEIGHT + HEADER_HEIGHT,
            // `PendingTab.updateEntries`: each group's 9-high header over its
            // 28-high entries.
            Tab::Requests => {
                let mut height = 0.0;
                if !data.incoming.is_empty() {
                    height += HEADER_HEIGHT;
                }
                height += data.incoming.len() as f32 * ENTRY_HEIGHT;
                if !data.outgoing.is_empty() {
                    height += HEADER_HEIGHT;
                }
                height + data.outgoing.len() as f32 * ENTRY_HEIGHT
            }
        }
    }

    pub fn clamp_scroll(&mut self, size: (f32, f32)) {
        let (_, _, height) = Self::panel(size);
        let max = (self.content_height() - (height - ADD_HEIGHT)).max(0.0);
        self.offset.set(self.offset.get().clamp(0.0, max));
    }

    /// The overlay's buttons, in the title's list.
    pub fn buttons(&self, (w, h): (f32, f32)) -> Vec<FriendButton> {
        if self.enable_confirm {
            let [on, off] = self.enable_buttons.get();
            return vec![
                (
                    on[0],
                    on[1],
                    on[2],
                    crate::lang::get("options.friendsList.confirm.turnOn"),
                    on[2] > 0.0,
                    20,
                    None,
                ),
                (
                    off[0],
                    off[1],
                    off[2],
                    crate::lang::get("options.friendsList.confirm.turnOff"),
                    true,
                    21,
                    None,
                ),
            ];
        }
        if self.confirm.is_some() {
            let [remove, cancel] = self.confirm_buttons.get();
            return vec![
                (
                    remove[0],
                    remove[1],
                    remove[2],
                    crate::lang::get("gui.remove"),
                    true,
                    30,
                    None,
                ),
                (
                    cancel[0],
                    cancel[1],
                    cancel[2],
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
                social::friend_list_state() == State::Success
                    && !self.add.trim().is_empty()
                    && self.sending.is_none(),
                3,
                Some(("friends/send_request", 15.0)),
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
                    Some(("friends/remove", 13.0)),
                ));
            }
        } else {
            // `PendingTab.updateEntries`: 9-high group headers, 28-high entries.
            let data = self.lists();
            let rows = pending_rows(data.incoming.len(), data.outgoing.len(), top, offset);
            let mut slot = 0usize;
            for row_y in rows.incoming {
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
                        Some(("friends/accept", 18.0)),
                    ));
                    buttons.push((
                        x + PANEL_WIDTH - 8.0 - 20.0,
                        row_y + 4.0,
                        20.0,
                        String::new(),
                        self.action.is_none(),
                        base + 1,
                        Some(("friends/reject", 18.0)),
                    ));
                }
                slot += 1;
            }
            for row_y in rows.outgoing {
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
                        Some(("friends/cancel", 16.0)),
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
                    // `applyFriendSettings(true, true)`: the flags move only when the
                    // service accepts the change (`OnlineOptionsScreen`).
                    self.action = Some(spawn(|| {
                        let code = social::update_friend_settings(true, true);
                        if code == social::ResultCode::Success {
                            social::set_friend_list_enabled(true);
                            social::set_allow_friend_requests(true);
                        }
                        code
                    }));
                }
                21 => {
                    // `FriendsListConfirmScreen` answered no: `lastScreen`.
                    self.enable_confirm = false;
                    self.cancelled = true;
                }
                30 => {
                    if let Some(friend) = self.confirm.take() {
                        // `FriendEntry.confirmRemoveFriend`: that row's remove
                        // button shows loading while the action runs.
                        self.action_button = self
                            .friends()
                            .iter()
                            .position(|f| f.id == friend.id)
                            .and_then(|index| u8::try_from(40 + index).ok());
                        self.action = Some(spawn(move || social::remove_friend(friend.id)));
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
                            spawn(move || social::accept_incoming_friend_request(friend.id))
                        } else {
                            spawn(move || social::remove_friend(friend.id))
                        };
                        // `IncomingEntry`/`OutgoingEntry` `switchToLoadingAfterPress`.
                        self.action_button = Some(id);
                        self.action = Some(job);
                    }
                }
                _ => {}
            }
            return;
        }
        // `MICROSOFT_ACCOUNT_LINK`'s click handler: the account page (through
        // `PrivacyConfirmLinkScreen.confirmLinkNow` in vanilla). The enable
        // question's message has the link too.
        if self.confirm.is_none()
            && self.links.borrow().iter().any(|r| {
                cursor.0 >= r[0] && cursor.0 <= r[2] && cursor.1 >= r[1] && cursor.1 <= r[3]
            })
        {
            self.link_clicked = true;
            self.link_sound = self.footer_links.borrow().iter().any(|r| {
                cursor.0 >= r[0] && cursor.0 <= r[2] && cursor.1 >= r[1] && cursor.1 <= r[3]
            });
            return;
        }
        // A question covers the overlay: a miss outside it does nothing, as
        // `PopupScreen`/`ConfirmScreen` consume it.
        if self.enable_confirm || self.confirm.is_some() {
            return;
        }
        // `PlainTextButton` on the profile row.
        let [px, py, pw, ph] = self.profile_box.get();
        if cursor.0 >= px && cursor.0 < px + pw && cursor.1 >= py && cursor.1 < py + ph {
            self.profile_clicked = true;
            return;
        }
        let rect = Self::field_rect((size.0.ceil(), size.1.ceil()));
        self.add_focused = ui::hovered(cursor, rect[0], rect[1], rect[2], rect[3])
            && self.sending.is_none()
            && social::friend_list_state() == State::Success;
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
            Some(crate::lang::get("gui.friends.validation.cannot_add_self"))
        } else if self
            .lists()
            .friends
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(&name))
        {
            Some(crate::lang::tr(
                "gui.friends.validation.already_friend",
                &[&name as &dyn std::fmt::Display],
            ))
        } else if self
            .lists()
            .outgoing
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(&name))
        {
            Some(crate::lang::tr(
                "gui.friends.validation.already_outgoing",
                &[&name as &dyn std::fmt::Display],
            ))
        } else if self
            .lists()
            .incoming
            .iter()
            .any(|f| f.name.eq_ignore_ascii_case(&name))
        {
            Some(crate::lang::tr(
                "gui.friends.validation.already_incoming",
                &[&name as &dyn std::fmt::Display],
            ))
        } else {
            None
        };
        if let Some(message) = problem {
            self.show_system_toast(message);
            return;
        }
        self.add_focused = false;
        self.sending = Some(spawn(move || social::send_friend_request(&name)));
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

    /// `AddFriendWidget`'s `EditBox.keyPressed`: Enter sends when the box is
    /// focused and active, and only then plays the button sound.
    pub fn enter(&mut self) -> bool {
        if self.add_focused() && !self.add.trim().is_empty() {
            self.send();
            true
        } else {
            false
        }
    }

    pub fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        size: (f32, f32),
    ) {
        self.draw_impl(atlas, batch, cursor, size, true);
    }

    /// [`Self::draw`] over a background drawn by the caller (the title screen or
    /// the pause menu, as `FriendsOverlayScreen.backgroundScreen`): no extra dim.
    pub fn draw_over(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        size: (f32, f32),
    ) {
        self.draw_impl(atlas, batch, cursor, size, false);
    }

    fn draw_impl(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        cursor: (f32, f32),
        (w, h): (f32, f32),
        dim: bool,
    ) {
        self.links.borrow_mut().clear();
        self.footer_links.borrow_mut().clear();
        let (x, y, height) = Self::panel((w, h));
        if dim {
            batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 140));
        }
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
        // The selected tab's underline (`FriendsOverlayTabButton.renderFocusUnderline`):
        // min(message width, width - 4) centred, 2 above the tab's bottom edge.
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
            (ADD_FRIEND, &self.add, 32),
            [fx, fy, fw, fh],
            self.add_focused(),
            self.started,
        );
        if !self.add_focused() && self.add.is_empty() {
            batch.text(
                atlas,
                &crate::lang::get("gui.friends.enter_nickname"),
                fx + 4.0,
                fy + 6.0,
                0xFF40_4040,
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
        let name_w = HudBatch::text_width(atlas, &name);
        // `PlainTextButton`: the name's own width and 9 height, copying it.
        self.profile_box
            .set([x + 8.0 + label_w, fy + fh + 6.0, name_w, 9.0]);
        batch.text(
            atlas,
            &name,
            x + 8.0 + label_w,
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
        } else {
            // `PlainTextButton.setTooltip(COPY_TO_CLIPBOARD)`.
            let [px, py, pw, ph] = self.profile_box.get();
            if pw > 0.0
                && cursor.0 >= px
                && cursor.0 < px + pw
                && cursor.1 >= py
                && cursor.1 < py + ph
            {
                ui::tooltip(
                    batch,
                    atlas,
                    &[crate::text::Text::translate(
                        "gui.friends.copy_to_clipboard",
                        &[],
                    )],
                    cursor,
                );
            }
        }
    }

    /// `ToastManager.extractRenderState`: the failure toast over every widget the
    /// screen draws, so a tab button or the panel never covers it.
    pub fn draw_toast(&self, atlas: &HudAtlas, batch: &mut HudBatch) {
        self.draw_toast_at(atlas, batch, 0.0);
    }

    /// The toasts from `y` down: other toasts' slots come first in the game.
    pub fn draw_toast_at(&self, atlas: &HudAtlas, batch: &mut HudBatch, y: f32) {
        let mut y = y;
        if let Some(toast) = &self.toast {
            toast.draw(atlas, batch, y);
            y += toast.height(atlas);
        }
        for toast in &self.friend_toasts {
            let face = toast
                .friend
                .and_then(|id| self.faces.borrow_mut().face(batch, atlas, id));
            toast.draw(atlas, batch, y, face.as_deref());
            y += toast.height(atlas);
        }
    }

    fn draw_friends(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (x, top, bottom): (f32, f32, f32),
    ) {
        let state = social::friend_list_state();
        if state != State::Success {
            self.draw_status(
                atlas,
                batch,
                state_text(state, true),
                (x, top, bottom),
                state == State::Loading,
            );
            return;
        }
        let friends = self.friends();
        let offset = self.offset.get();
        let first = batch.vertices.len();
        if friends.is_empty() {
            // `showEmpty`: illustration (48), 8, the centred text, the whole
            // block centred in the list's height.
            let cx = x + PANEL_WIDTH / 2.0;
            let empty = empty_state_text();
            let link = account_link_text();
            let lines: Vec<&str> = empty.lines().collect();
            let block_h = 48.0 + 8.0 + lines.len() as f32 * 9.0;
            let block_top = top + ((bottom - top - block_h) / 2.0).floor();
            ui::blit_sprite(
                batch,
                atlas,
                "gui/sprites/friends/illustrations_00",
                cx - 64.0,
                block_top,
                128.0,
                48.0,
                u32::MAX,
            );
            // `EMPTY_STATE`: the two sentences with the account link composed in,
            // the link underlined and clickable like `MICROSOFT_ACCOUNT_LINK`.
            for (i, line) in lines.iter().enumerate() {
                self.centered_with_link(
                    batch,
                    atlas,
                    line,
                    cx,
                    block_top + 56.0 + i as f32 * 9.0,
                    &link,
                    false,
                );
            }
        }
        for (i, friend) in friends.iter().enumerate() {
            let row = top + i as f32 * ENTRY_HEIGHT - offset;
            if row + ENTRY_HEIGHT <= top || row >= bottom {
                continue;
            }
            self.draw_entry(atlas, batch, x, row, friend, false);
        }
        // `FriendsTab.updateEntries` adds `createManageAccountFooter` after the
        // entries only; the empty state (`showEmpty`) has no footer.
        let footer = top + friends.len() as f32 * ENTRY_HEIGHT - offset;
        if !friends.is_empty() && footer > top && footer < bottom {
            // `MANAGE_ACCOUNT_FOOTER`: one centred paragraph, wrapped at the list's
            // width (`MultiLineTextWidget`), with the account link composed in.
            let footer_text = manage_account_text();
            let link = account_link_text();
            for (i, line) in crate::multiplayer::wrap(&footer_text, PANEL_WIDTH - 16.0, atlas)
                .iter()
                .enumerate()
            {
                self.centered_with_link(
                    batch,
                    atlas,
                    line,
                    x + PANEL_WIDTH / 2.0,
                    footer + 4.0 + i as f32 * 9.0,
                    &link,
                    true,
                );
            }
        }
        self.draw_scrollbar(atlas, batch, (x, top, bottom));
        batch.clip_since(first, x, top, x + PANEL_WIDTH, bottom);
    }

    /// `showLoading`/`showError`: `LoadingDotsWidget` (message, then the `O o o`
    /// line 300 ms a frame) or the grey `MultiLineTextWidget`, centred in the list.
    fn draw_status(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        message: String,
        (x, top, bottom): (f32, f32, f32),
        loading: bool,
    ) {
        let cx = x + PANEL_WIDTH / 2.0;
        if loading {
            let y = top + ((bottom - top - 28.0) / 2.0).floor();
            let width = HudBatch::text_width(atlas, &message);
            batch.text(
                atlas,
                &message,
                cx - (width / 2.0).floor(),
                y + 2.0,
                u32::MAX,
                false,
            );
            let dots = ["O o o", "o O o", "o o O", "o O o"]
                [(self.started.elapsed().as_millis() / 300 % 4) as usize];
            let width = HudBatch::text_width(atlas, dots);
            batch.text(
                atlas,
                dots,
                cx - (width / 2.0).floor(),
                y + 17.0,
                0xFF80_8080,
                false,
            );
        } else {
            let lines = crate::multiplayer::wrap(&message, PANEL_WIDTH - 16.0, atlas);
            let y = top + ((bottom - top - lines.len() as f32 * 9.0) / 2.0).floor();
            for (i, line) in lines.iter().enumerate() {
                let width = HudBatch::text_width(atlas, line);
                batch.text(
                    atlas,
                    line,
                    cx - (width / 2.0).floor(),
                    y + i as f32 * 9.0,
                    GREY,
                    false,
                );
            }
        }
    }

    fn draw_requests(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (x, top, bottom): (f32, f32, f32),
    ) {
        let state = social::friend_list_state();
        if state != State::Success {
            self.draw_status(
                atlas,
                batch,
                state_text(state, false),
                (x, top, bottom),
                state == State::Loading,
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
                top + ((bottom - top - 9.0) / 2.0).floor(),
                GREY,
            );
            return;
        }
        let offset = self.offset.get();
        let first = batch.vertices.len();
        let rows = pending_rows(data.incoming.len(), data.outgoing.len(), top, offset);
        if let Some(y) = rows.received_header
            && y + HEADER_HEIGHT > top
            && y < bottom
        {
            // `PendingTab.RECEIVED_HEADER`: `ChatFormatting.BOLD, UNDERLINE`,
            // a 9-high line centred in the list's content width.
            centered_styled(
                batch,
                atlas,
                &pending_header("gui.friends.pending.received"),
                x + PANEL_WIDTH / 2.0,
                y + 4.0,
                u32::MAX,
            );
        }
        for (friend, y) in data.incoming.iter().zip(rows.incoming.iter().copied()) {
            if y + ENTRY_HEIGHT <= top || y >= bottom {
                continue;
            }
            self.draw_entry(atlas, batch, x, y, friend, true);
        }
        if let Some(y) = rows.sent_header
            && y + HEADER_HEIGHT > top
            && y < bottom
        {
            // `PendingTab.SENT_HEADER`: `ChatFormatting.BOLD, UNDERLINE`.
            centered_styled(
                batch,
                atlas,
                &pending_header("gui.friends.pending.sent"),
                x + PANEL_WIDTH / 2.0,
                y + 4.0,
                u32::MAX,
            );
        }
        for (friend, y) in data.outgoing.iter().zip(rows.outgoing.iter().copied()) {
            if y + ENTRY_HEIGHT <= top || y >= bottom {
                continue;
            }
            self.draw_entry(atlas, batch, x, y, friend, true);
        }
        self.draw_scrollbar(atlas, batch, (x, top, bottom));
        batch.clip_since(first, x, top, x + PANEL_WIDTH, bottom);
    }

    /// `ScrollableLayout.ReserveStrategy.BOTH`'s scroller, drawn after the
    /// clipped rows (`AbstractScrollArea.extractScrollbar`).
    fn draw_scrollbar(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        (x, top, bottom): (f32, f32, f32),
    ) {
        let Some((y, height)) =
            scroller(self.content_height(), bottom - top, self.offset.get(), top)
        else {
            return;
        };
        let bar_x = x + PANEL_WIDTH - 6.0;
        ui::blit_sprite(
            batch,
            atlas,
            "gui/sprites/widget/scroller_background",
            bar_x,
            top,
            6.0,
            bottom - top,
            u32::MAX,
        );
        ui::blit_sprite(
            batch,
            atlas,
            "gui/sprites/widget/scroller",
            bar_x,
            y,
            6.0,
            height,
            u32::MAX,
        );
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
        // `AbstractFriendsEntryContainerWidget`: `height / 3 - nameHeight / 2`
        // with a status (28 / 3 - 9 / 2 = 5), `height / 2 - nameHeight / 2` (10)
        // for a request's name alone.
        let name_y = if request { y + 10.0 } else { y + 5.0 };
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
            // `FriendEntry.statusWidget`: max width to 2px left of the remove
            // button, `TextOverflow.SCROLLING`. The text is 1 lower
            // (`defaultScrollingHelper`'s `textTop`), and past the width it
            // slides: `period = max(overflow * 0.5, 3)`, an eased triangle.
            let status_x = text_x;
            let status_w = (x + 8.0 + PANEL_WIDTH - 16.0 - 20.0) - status_x - 2.0;
            let text_w = HudBatch::text_width(atlas, &text);
            if text_w <= status_w {
                batch.text(atlas, &text, status_x, y + 16.0, color, false);
            } else {
                let left = status_x + 2.0;
                let right = status_x + status_w - 2.0;
                let overflow = text_w - (right - left);
                let period = (overflow * 0.5).max(3.0) as f64;
                let time = self.started.elapsed().as_secs_f64();
                let alpha = (std::f64::consts::FRAC_PI_2
                    * (std::f64::consts::TAU * time / period).cos())
                .sin()
                    / 2.0
                    + 0.5;
                let pos = (alpha * f64::from(overflow)) as f32;
                let start = batch.vertices.len();
                batch.text(atlas, &text, left - pos.floor(), y + 17.0, color, false);
                batch.clip_since(start, left, y + 16.0, right, y + 25.0);
            }
        }
    }

    /// A centred line with the account link underlined; its box is kept so a click can
    /// reach `PRIVACY_AND_ONLINE_URL` (`MICROSOFT_ACCOUNT_LINK`'s click event).
    fn centered_with_link(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        line: &str,
        cx: f32,
        y: f32,
        link: &str,
        footer: bool,
    ) {
        let width = HudBatch::text_width(atlas, line);
        let x = cx - (width / 2.0).floor();
        self.draw_link_line(batch, atlas, line, x, y, link, footer);
    }

    /// A grey line left at `x`, its account link underlined and clickable;
    /// `footer` marks the manage-account footer, whose click has a sound.
    fn draw_link_line(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        line: &str,
        x: f32,
        y: f32,
        link: &str,
        footer: bool,
    ) {
        batch.text(atlas, line, x, y, GREY, true);
        if let Some(at) = line.find(link) {
            let link_x = x + HudBatch::text_width(atlas, &line[..at]);
            let link_w = HudBatch::text_width(atlas, link);
            batch.fill(atlas, link_x, y + 9.0, link_w, 1.0, GREY);
            let box_ = [link_x, y, link_x + link_w, y + 10.0];
            self.links.borrow_mut().push(box_);
            if footer {
                self.footer_links.borrow_mut().push(box_);
            }
        }
    }

    /// The account link was clicked: the URL to confirm and open.
    pub fn take_link_click(&mut self) -> Option<&'static str> {
        std::mem::take(&mut self.link_clicked).then_some(PRIVACY_AND_ONLINE_URL)
    }

    /// `FriendsListConfirmScreen`/`ConfirmScreen`: the title (9), then the message
    /// container (5 above and below), then the button row 16 below; both buttons
    /// are `Button`'s default 150 wide with 4 between them. The dark sprite frames
    /// the layout 18 out (`PANEL_PADDING + BG_BORDER_WIDTH`).
    fn draw_enable_confirm(&self, atlas: &HudAtlas, batch: &mut HudBatch, (w, h): (f32, f32)) {
        let title = crate::lang::get("options.friendsList.confirm.title");
        let link = account_link_text();
        let message = crate::lang::tr("options.friendsList.confirm.message", &[&link]);
        let message_lines = crate::multiplayer::wrap(&message, (w - 180.0).max(20.0), atlas);
        let title_w = HudBatch::text_width(atlas, &title);
        let message_w = message_lines
            .iter()
            .map(|line| HudBatch::text_width(atlas, line))
            .fold(0.0, f32::max);
        let message_h = message_lines.len() as f32 * 9.0;
        let content_w = (w - 160.0).max(304.0);
        let content_h = 9.0 + 8.0 + 10.0 + message_h + 8.0 + 16.0 + 20.0;
        let lx = ((w - content_w) / 2.0).floor();
        let ly = ((h - content_h) / 2.0).floor();
        ui::blit_sprite(
            batch,
            atlas,
            "gui/sprites/friends/background_dark",
            lx - 18.0,
            ly - 18.0,
            content_w + 37.0,
            content_h + 37.0,
            u32::MAX,
        );
        let title_x = ((w - title_w) / 2.0).floor();
        batch.text(atlas, &title, title_x, ly, u32::MAX, true);
        let block_x = ((w - message_w) / 2.0).floor();
        for (i, line) in message_lines.iter().enumerate() {
            self.draw_link_line(
                batch,
                atlas,
                line,
                block_x,
                ly + 22.0 + i as f32 * 9.0,
                &link,
                false,
            );
        }
        // `Button.builder(...).build()`: 150 by 20, 4 apart, 16 below.
        let bx = ((w - 304.0) / 2.0).floor();
        let by = ly + 9.0 + 8.0 + 10.0 + message_h + 8.0 + 16.0;
        self.enable_buttons
            .set([[bx, by, 150.0, 20.0], [bx + 154.0, by, 150.0, 20.0]]);
    }

    /// `FriendEntry.confirmRemoveFriend`'s `PopupScreen`: `popup/background`
    /// 250 wide, the bold title and the message centred in its 214-wide
    /// content, and two 104-wide buttons 6 apart.
    fn draw_confirm(&self, atlas: &HudAtlas, batch: &mut HudBatch, (w, h): (f32, f32)) {
        batch.fill_gradient(
            atlas,
            (0.0, 0.0, w, h),
            rgba(16, 16, 16, 192),
            rgba(16, 16, 16, 208),
        );
        let title = crate::text::Text::translate("gui.friends.confirm_title", &[]).bold();
        let message = crate::text::Text::translate("gui.friends.confirm_unfriend", &[]);
        let title_lines = atlas.font.split(&title, 214.0);
        let message_lines = atlas.font.split(&message, 214.0);
        let title_h = title_lines.len() as f32 * 9.0;
        let message_h = message_lines.len() as f32 * 9.0;
        let content_h = title_h + 12.0 + message_h + 12.0 + 20.0;
        let lx = ((w - 214.0) / 2.0).floor();
        let ly = ((h - content_h) / 2.0).floor();
        ui::blit_sprite(
            batch,
            atlas,
            "gui/sprites/popup/background",
            lx - 18.0,
            ly - 18.0,
            250.0,
            content_h + 36.0,
            u32::MAX,
        );
        let mut y = ly;
        for line in &title_lines {
            let x = ((w - atlas.font.width_text(line)) / 2.0).floor();
            batch.text_styled(atlas, line, x, y, u32::MAX, true);
            y += 9.0;
        }
        y += 12.0;
        for line in &message_lines {
            let x = ((w - atlas.font.width_text(line)) / 2.0).floor();
            batch.text_styled(atlas, line, x, y, u32::MAX, true);
            y += 9.0;
        }
        y += 12.0;
        let button_w = (214.0 - 6.0) / 2.0;
        let bx = lx;
        self.confirm_buttons.set([
            [bx, y, button_w, 20.0],
            [bx + button_w + 6.0, y, button_w, 20.0],
        ]);
    }
}

/// A request row's button ids: 100 + slot * 2, accept/cancel at +0 and decline at +1.
/// `PendingTab.updateEntries`'s rows: a 9-high header over each non-empty
/// group's 28-high entries, from the list top less the scroll offset.
struct PendingRows {
    received_header: Option<f32>,
    incoming: Vec<f32>,
    sent_header: Option<f32>,
    outgoing: Vec<f32>,
}

fn pending_rows(incoming: usize, outgoing: usize, top: f32, offset: f32) -> PendingRows {
    let mut y = top - offset;
    let mut rows = PendingRows {
        received_header: None,
        incoming: Vec::new(),
        sent_header: None,
        outgoing: Vec::new(),
    };
    if incoming > 0 {
        rows.received_header = Some(y);
        y += HEADER_HEIGHT;
    }
    for _ in 0..incoming {
        rows.incoming.push(y);
        y += ENTRY_HEIGHT;
    }
    if outgoing > 0 {
        rows.sent_header = Some(y);
        y += HEADER_HEIGHT;
    }
    for _ in 0..outgoing {
        rows.outgoing.push(y);
        y += ENTRY_HEIGHT;
    }
    rows
}

/// `AbstractScrollArea`: the scroller's y and height in a `height`-tall view of
/// `content`, when the content overflows it. The thumb is `height²/content`,
/// at least 32 (the settings' `SCROLLBAR_MIN_HEIGHT`), less than `height - 8`.
fn scroller(content: f32, height: f32, offset: f32, top: f32) -> Option<(f32, f32)> {
    if content <= height || height <= 0.0 {
        return None;
    }
    let computed = (height * height / content).floor();
    let thumb = if computed < 32.0 {
        32.0
    } else if computed > height - 8.0 {
        height - 8.0
    } else {
        computed
    };
    let max = content - height;
    let y = top + (offset * (height - thumb) / max).max(0.0);
    Some((y, thumb))
}

fn request_id(slot: usize, offset: u8) -> Option<u8> {
    u8::try_from(100usize + slot * 2 + usize::from(offset)).ok()
}

/// `FriendsTab.MICROSOFT_ACCOUNT_LINK`: the account link's text.
fn account_link_text() -> String {
    crate::lang::get("gui.friends.empty_state.link")
}

/// `FriendsTab.MANAGE_ACCOUNT_FOOTER`: the footer with the account link composed in.
fn manage_account_text() -> String {
    let link = account_link_text();
    crate::lang::tr("gui.friends.manage_account_footer", &[&link])
}

/// `FriendsTab.EMPTY_STATE`: the empty list's sentence with the account link.
fn empty_state_text() -> String {
    let link = account_link_text();
    crate::lang::tr("gui.friends.empty_state", &[&link])
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

/// A centred styled line (`PendingTab`'s bold, underlined group headers).
fn centered_styled(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &crate::text::Text,
    cx: f32,
    y: f32,
    color: u32,
) {
    let w = atlas.font.width_text(text);
    batch.text_styled(atlas, text, cx - (w / 2.0).floor(), y, color, true);
}

/// `PendingTab.RECEIVED_HEADER`/`SENT_HEADER`: the label in BOLD and UNDERLINE.
fn pending_header(key: &str) -> crate::text::Text {
    crate::text::Text::literal(crate::lang::get(key))
        .bold()
        .underlined()
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

    /// `PendingTab.updateEntries`: each group's 9-high header sits above 28-high
    /// entries, and the whole list moves up by the scroll offset.
    #[test]
    fn pending_headers_are_nine_high_and_entries_twenty_eight() {
        let rows = pending_rows(2, 1, 100.0, 0.0);
        assert_eq!(rows.received_header, Some(100.0));
        assert_eq!(rows.incoming, vec![109.0, 137.0]);
        assert_eq!(rows.sent_header, Some(165.0));
        assert_eq!(rows.outgoing, vec![174.0]);
        let scrolled = pending_rows(1, 1, 100.0, 10.0);
        assert_eq!(scrolled.received_header, Some(90.0));
        assert_eq!(scrolled.incoming, vec![99.0]);
        assert_eq!(scrolled.sent_header, Some(127.0));
        assert_eq!(scrolled.outgoing, vec![136.0]);
    }

    /// `FriendsOverlayScreen.mouseClicked`: the overlay reaches from the tab
    /// bar's top to the background's bottom (`layout.y + height + 16`).
    #[test]
    fn the_overlay_area_reaches_the_backgrounds_bottom() {
        let screen = Screen::new();
        assert!(screen.contains((427.0, 14.0), (854.0, 480.0)));
        assert!(screen.contains((427.0, 450.0), (854.0, 480.0)));
        assert!(!screen.contains((427.0, 12.0), (854.0, 480.0)));
        assert!(!screen.contains((427.0, 457.0), (854.0, 480.0)));
    }

    /// `AbstractScrollArea`: no scroller without overflow; otherwise a thumb of
    /// `height²/content` (at least 32) that reaches the bottom at max scroll.
    #[test]
    fn the_scroller_appears_only_when_the_list_overflows() {
        assert_eq!(scroller(100.0, 100.0, 0.0, 40.0), None);
        assert_eq!(scroller(200.0, 100.0, 0.0, 40.0), Some((40.0, 50.0)));
        assert_eq!(scroller(200.0, 100.0, 100.0, 40.0), Some((90.0, 50.0)));
        // Smaller content gives a thumb above the 32 minimum.
        assert_eq!(scroller(1000.0, 100.0, 0.0, 40.0), Some((40.0, 32.0)));
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

    /// `MICROSOFT_ACCOUNT_LINK`: a click on the drawn link answers the privacy page.
    #[test]
    fn the_account_link_is_clickable() {
        let mut screen = Screen::new();
        screen.links.borrow_mut().push([10.0, 20.0, 60.0, 30.0]);
        screen.click(None, (20.0, 25.0), (854.0, 480.0));
        assert_eq!(screen.take_link_click(), Some(PRIVACY_AND_ONLINE_URL));
        assert_eq!(screen.take_link_click(), None);
        // A click elsewhere keeps editing the add-friend field.
        screen.click(None, (400.0, 400.0), (854.0, 480.0));
        assert_eq!(screen.take_link_click(), None);
    }
}
