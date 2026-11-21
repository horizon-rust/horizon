use super::{HudAtlas, HudBatch, Outcome, PlayerInfo, Screen, SubScreen, Text, player_uuid};
use crate::{ui::Scroll, widgets};
use std::cell::{Cell, RefCell};
use winit::keyboard::KeyCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerAction {
    Op(u128),
    Deop(u128),
    Remove(u128),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerColumn {
    Operator,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PlayerFocus {
    uuid: u128,
    column: PlayerColumn,
}

impl PlayerAction {
    fn focus(self) -> PlayerFocus {
        match self {
            Self::Op(uuid) | Self::Deop(uuid) => PlayerFocus {
                uuid,
                column: PlayerColumn::Operator,
            },
            Self::Remove(uuid) => PlayerFocus {
                uuid,
                column: PlayerColumn::Remove,
            },
        }
    }
}

impl PlayerColumn {
    fn action(self, player: &PlayerInfo, uuid: u128) -> PlayerAction {
        match self {
            Self::Operator if player.operator => PlayerAction::Deop(uuid),
            Self::Operator => PlayerAction::Op(uuid),
            Self::Remove => PlayerAction::Remove(uuid),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListFocus {
    Header,
    Player(PlayerFocus),
}

#[derive(Clone, Copy)]
enum Position {
    Header,
    Row(usize, PlayerColumn),
}

#[cfg(test)]
#[path = "players_list_tests.rs"]
mod tests;

const ROW: f32 = 36.0;
const HEADER: f32 = 17.0;
const PADDING: f32 = 2.0;
const ROW_WIDTH: f32 = 300.0;
const BUTTON_HEIGHT: f32 = 20.0;
const LABELS: [&str; 3] = [
    "mco.configure.world.invites.normal.tooltip",
    "mco.configure.world.invites.ops.tooltip",
    "mco.configure.world.invites.remove.tooltip",
];
const ICONS: [&str; 3] = [
    "gui/sprites/player_list/make_operator",
    "gui/sprites/player_list/remove_operator",
    "gui/sprites/player_list/remove_player",
];

#[derive(Clone, Debug, Default)]
pub(super) struct PlayersList {
    widths: Cell<Option<[f32; 3]>>,
    heading_width: Cell<Option<f32>>,
    rendered: RefCell<Option<RenderedPlayers>>,
    focus: Option<(i64, ListFocus)>,
    #[cfg(test)]
    offline_faces: bool,
}

/// The frame's clipped hit regions, invalid once the list scrolls, resizes, or changes realm.
#[derive(Clone, Debug)]
struct RenderedPlayers {
    realm: i64,
    viewport: Viewport,
    amount: f32,
    heading: Option<[f32; 4]>,
    controls: Vec<([f32; 4], usize, PlayerAction)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewport {
    width: f32,
    top: f32,
    bottom: f32,
    row_left: f32,
    content: f32,
}

impl Viewport {
    fn new((w, h): (f32, f32), rows: usize) -> Option<Self> {
        let (width, h) = (w.ceil(), h.ceil());
        // GridLayoutTab gives the list the content height minus 36. Its scissor
        // spans the full GUI width; 300 only constrains where each entry starts.
        let (top, bottom) = (25.0, h - 68.0);
        (bottom > top).then_some(Self {
            width,
            top,
            bottom,
            row_left: (width / 2.0).floor() - ROW_WIDTH / 2.0,
            content: PADDING * 2.0 + HEADER + rows as f32 * ROW,
        })
    }

    fn bar_x(self) -> f32 {
        self.row_left + ROW_WIDTH + 8.0
    }

    fn amount(self, mut scroll: Scroll) -> f32 {
        scroll.clamp(self.top, self.bottom, self.content);
        scroll.amount.trunc()
    }

    fn row_y(self, row: usize, scroll: Scroll) -> f32 {
        self.top + PADDING + HEADER + row as f32 * ROW - self.amount(scroll)
    }

    fn heading_rect(self, text_width: f32, scroll: Scroll) -> [f32; 4] {
        let width = text_width + 8.0;
        [
            self.row_left + ROW_WIDTH / 2.0 - (width / 2.0).floor(),
            self.top + PADDING - self.amount(scroll),
            width,
            HEADER,
        ]
    }

    fn reveal(self, y: f32, height: f32, scroll: &mut Scroll) {
        scroll.clamp(self.top, self.bottom, self.content);
        if y < self.top + PADDING {
            scroll.amount += y - self.top - PADDING;
        } else if y + height > self.bottom - PADDING {
            scroll.amount += y + height - self.bottom + PADDING;
        }
        scroll.clamp(self.top, self.bottom, self.content);
    }

    fn contains(self, (x, y): (f32, f32)) -> bool {
        (0.0..self.width).contains(&x) && (self.top..self.bottom).contains(&y)
    }

    fn clip(self, [x, y, w, h]: [f32; 4]) -> Option<[f32; 4]> {
        let (x0, y0) = (x.max(0.0), y.max(self.top));
        let (x1, y1) = ((x + w).min(self.width), (y + h).min(self.bottom));
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
    }

    fn controls(self, y: f32, operator: bool, widths: [f32; 3]) -> [Control; 2] {
        let op = usize::from(operator);
        let [op_w, remove_w] = [widths[op], widths[2]];
        let y = y + 8.0;
        let remove_x = self.row_left + ROW_WIDTH - PADDING - remove_w;
        [
            Control {
                column: PlayerColumn::Operator,
                style: op,
                rect: [remove_x - op_w - 8.0, y, op_w, BUTTON_HEIGHT],
            },
            Control {
                column: PlayerColumn::Remove,
                style: 2,
                rect: [remove_x, y, remove_w, BUTTON_HEIGHT],
            },
        ]
    }
}

#[derive(Clone, Copy)]
struct Control {
    column: PlayerColumn,
    style: usize,
    rect: [f32; 4],
}

impl Control {
    fn draw(self, batch: &mut HudBatch, atlas: &HudAtlas, active: bool, highlighted: bool) {
        let [x, y, w, h] = self.rect;
        let sprite = if !active {
            "gui/sprites/widget/button_disabled"
        } else if highlighted {
            "gui/sprites/widget/button_highlighted"
        } else {
            "gui/sprites/widget/button"
        };
        crate::ui::nine_slice(batch, atlas, sprite, x, y, w, h, u32::MAX);
        // SpriteIconButton.TextAndIcon uses an 8×7 sprite and clamps the label
        // against its reserved room; the icon never replaces the label.
        crate::ui::scrolling_text_around_at(
            batch,
            atlas,
            &Text::translate(LABELS[self.style], &[]),
            x + (w / 2.0).floor(),
            x + 2.0,
            x + w - 12.0,
            y,
            y + h,
            y + 6.0,
            crate::hud_overlays::now_ms(),
            if active { u32::MAX } else { 0xFFA0_A0A0 },
        );
        batch.sprite_scaled(
            atlas,
            ICONS[self.style],
            x + w - 10.0,
            y + 7.0,
            8.0,
            7.0,
            u32::MAX,
        );
    }
}

impl Screen {
    // The configure screen attaches its tab widgets only once all four requests succeed.
    fn player_rows(&self) -> Option<(i64, &[PlayerInfo], Scroll, bool)> {
        if !self.players_ready() {
            return None;
        }
        let SubScreen::Configure {
            server,
            detail: Some(Ok(detail)),
            players_scroll,
            switching_slot,
            member_job,
            ..
        } = &self.subscreen
        else {
            return None;
        };
        Some((
            server.id,
            &detail.players,
            *players_scroll,
            member_job.is_none() && switching_slot.is_none(),
        ))
    }

    pub(crate) fn players_tab_ready(&self) -> bool {
        self.players_ready()
    }

    pub(crate) fn clear_players_focus(&mut self) {
        self.players_list.focus = None;
    }

    fn hits(cursor: (f32, f32), [x, y, w, h]: [f32; 4]) -> bool {
        crate::ui::hovered(cursor, x, y, w, h)
    }

    fn rendered_matches(
        rendered: &RenderedPlayers,
        realm: i64,
        view: Viewport,
        scroll: Scroll,
    ) -> bool {
        rendered.realm == realm
            && rendered.viewport == view
            && rendered.amount == view.amount(scroll)
    }

    pub(crate) fn player_control_at(
        &self,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> Option<PlayerAction> {
        let (realm, players, scroll, active) = self.player_rows()?;
        let view = Viewport::new(size, players.len())?;
        let rendered = self.players_list.rendered.borrow();
        let frame = rendered.as_ref()?;
        if !active || !Self::rendered_matches(frame, realm, view, scroll) {
            return None;
        }
        let (_, row, action) = *frame
            .controls
            .iter()
            .find(|(rect, _, _)| Self::hits(cursor, *rect))?;
        let player = players.get(row)?;
        let id = player_uuid(player)?;
        // Never reinterpret a visible Op as Deop (or aim it at a replacement row) if a
        // reply changed the DTO since the last draw. The business handler rechecks too.
        (action.focus().column.action(player, id) == action).then_some(action)
    }

    pub(crate) fn focus_player_at(
        &mut self,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> Option<[f32; 4]> {
        let (realm, players, scroll, _) = self.player_rows()?;
        let view = Viewport::new(size, players.len())?;
        let (focus, y, height) = if let Some(action) = self.player_control_at(cursor, size) {
            let focus = action.focus();
            let row = players
                .iter()
                .position(|player| player_uuid(player) == Some(focus.uuid))?;
            (ListFocus::Player(focus), view.row_y(row, scroll), ROW)
        } else {
            let rendered = self.players_list.rendered.borrow();
            let frame = rendered.as_ref()?;
            if !Self::rendered_matches(frame, realm, view, scroll)
                || !Self::hits(cursor, frame.heading?)
            {
                return None;
            }
            (
                ListFocus::Header,
                view.top + PADDING - view.amount(scroll),
                HEADER,
            )
        };
        self.players_list.focus = Some((realm, focus));
        if let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen {
            view.reveal(y, height, players_scroll);
        }
        self.focused_player_rect(size)
    }

    pub(crate) fn focused_player_rect(&self, size: (f32, f32)) -> Option<[f32; 4]> {
        let (_, players, scroll, active) = self.player_rows()?;
        let view = Viewport::new(size, players.len())?;
        match self.current_list_focus()? {
            ListFocus::Header => {
                view.clip(view.heading_rect(self.players_list.heading_width.get()?, scroll))
            }
            ListFocus::Player(focus) if active => {
                let (row, player) = players
                    .iter()
                    .enumerate()
                    .find(|(_, player)| player_uuid(player) == Some(focus.uuid))?;
                let control = view
                    .controls(
                        view.row_y(row, scroll),
                        player.operator,
                        self.players_list.widths.get()?,
                    )
                    .into_iter()
                    .find(|control| control.column == focus.column)?;
                view.clip(control.rect)
            }
            _ => None,
        }
    }

    pub(super) fn press_players(
        &mut self,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> Option<(Outcome, bool)> {
        let (_, players, _, _) = self.player_rows()?;
        let view = Viewport::new(size, players.len())?;
        if let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen
            && view.bottom - view.top >= 40.0
            && players_scroll.press(cursor, view.bar_x(), view.top, view.bottom, view.content)
        {
            return Some((Outcome::None, false));
        }
        let action = self.player_control_at(cursor, size);
        if self.focus_player_at(cursor, size).is_some() {
            return Some((
                action.map_or(Outcome::None, |action| self.click_player_action(action)),
                true,
            ));
        }
        self.clear_players_focus();
        None
    }

    pub(super) fn drag_players(&mut self, y: f32, size: (f32, f32)) -> bool {
        let Some((_, players, scroll, _)) = self.player_rows() else {
            return false;
        };
        if !scroll.held() {
            return false;
        }
        if let Some(view) = Viewport::new(size, players.len())
            && view.bottom - view.top >= 40.0
            && let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen
        {
            players_scroll.drag(y, view.top, view.bottom, view.content);
        }
        true
    }

    pub(super) fn wheel_players(
        &mut self,
        delta: f32,
        cursor: (f32, f32),
        size: (f32, f32),
    ) -> bool {
        let Some((_, players, _, _)) = self.player_rows() else {
            return false;
        };
        if let Some(view) = Viewport::new(size, players.len())
            && view.contains(cursor)
            && let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen
        {
            players_scroll.wheel(delta, ROW / 2.0, view.top, view.bottom, view.content);
        }
        true
    }

    fn current_list_focus(&self) -> Option<ListFocus> {
        let (realm, players, _, _) = self.player_rows()?;
        let (_, focus) = self.players_list.focus.filter(|(id, _)| *id == realm)?;
        match focus {
            ListFocus::Player(focus)
                if !players
                    .iter()
                    .any(|player| player_uuid(player) == Some(focus.uuid)) =>
            {
                None
            }
            _ => Some(focus),
        }
    }

    pub(crate) fn players_list_focused(&self) -> bool {
        self.current_list_focus().is_some()
    }

    pub(crate) fn focused_player_column(&self) -> Option<PlayerColumn> {
        let ListFocus::Player(focus) = self.current_list_focus()? else {
            return None;
        };
        Some(focus.column)
    }

    fn displayed_player_action(&self) -> Option<PlayerAction> {
        let (realm, players, _, active) = self.player_rows()?;
        let ListFocus::Player(focus) = self.current_list_focus()? else {
            return None;
        };
        let player = players
            .iter()
            .find(|player| player_uuid(player) == Some(focus.uuid))?;
        let action = focus.column.action(player, focus.uuid);
        // A reply can change Op into Deop before the next frame. Do not press
        // an action the player has not seen. Match identity, not row index, so
        // roster reordering still keeps keyboard focus on the same UUID.
        let displayed = self
            .players_list
            .rendered
            .borrow()
            .as_ref()
            .is_some_and(|frame| {
                frame.realm == realm && frame.controls.iter().any(|(_, _, shown)| *shown == action)
            });
        (active && displayed).then_some(action)
    }

    pub(crate) fn activate_player_focus(&mut self) -> Option<Outcome> {
        let action = self.displayed_player_action()?;
        Some(self.click_player_action(action))
    }

    /// `ContainerObjectSelectionList.nextFocusPath`: vertical arrows keep the child
    /// column even when its label width (and thus its x position) changes.
    pub(crate) fn move_player_focus(&mut self, key: KeyCode, size: (f32, f32)) -> bool {
        use {
            PlayerColumn::{Operator, Remove},
            Position::{Header, Row},
        };
        self.move_players(size, |rows, focus| match (key, focus?) {
            (KeyCode::ArrowLeft, Row(i, Remove)) => Some(Row(i, Operator)),
            (KeyCode::ArrowRight, Row(i, Operator)) => Some(Row(i, Remove)),
            (KeyCode::ArrowUp, Row(0, _)) => Some(Header),
            (KeyCode::ArrowUp, Row(i, column)) => Some(Row(i - 1, column)),
            (KeyCode::ArrowDown, Header) if rows != 0 => Some(Row(0, Operator)),
            (KeyCode::ArrowDown, Row(i, column)) if i + 1 < rows => Some(Row(i + 1, column)),
            _ => None,
        })
    }

    pub(crate) fn tab_player_focus(&mut self, reverse: bool, size: (f32, f32)) -> bool {
        use {
            PlayerColumn::{Operator, Remove},
            Position::{Header, Row},
        };
        self.move_players(size, |rows, focus| match (reverse, focus) {
            (false, None) => Some(Header),
            (false, Some(Header)) if rows != 0 => Some(Row(0, Operator)),
            (false, Some(Row(i, Operator))) => Some(Row(i, Remove)),
            (false, Some(Row(i, Remove))) if i + 1 < rows => Some(Row(i + 1, Operator)),
            (true, Some(Row(i, Remove))) => Some(Row(i, Operator)),
            (true, Some(Row(i, Operator))) => {
                Some(i.checked_sub(1).map_or(Header, |i| Row(i, Remove)))
            }
            _ => None,
        })
    }

    pub(crate) fn enter_player_focus(
        &mut self,
        from_bottom: bool,
        column: PlayerColumn,
        size: (f32, f32),
    ) -> bool {
        self.move_players(size, |rows, _| {
            Some(if from_bottom && rows != 0 {
                Position::Row(rows - 1, column)
            } else {
                Position::Header
            })
        })
    }

    fn move_players(
        &mut self,
        size: (f32, f32),
        next: impl FnOnce(usize, Option<Position>) -> Option<Position>,
    ) -> bool {
        let Some((realm, players, scroll, active)) = self.player_rows() else {
            return false;
        };
        let Some(view) = Viewport::new(size, players.len()) else {
            return false;
        };
        let rows: Vec<_> = players
            .iter()
            .enumerate()
            .filter_map(|(i, player)| Some((i, player_uuid(player)?)))
            .collect();
        let focus = match self.current_list_focus() {
            Some(ListFocus::Header) => Some(Position::Header),
            Some(ListFocus::Player(focus)) if active => rows
                .iter()
                .position(|&(_, id)| id == focus.uuid)
                .map(|i| Position::Row(i, focus.column)),
            // Disabled member controls still remember their focus while a request runs.
            Some(ListFocus::Player(_)) => return false,
            None => None,
        };
        let Some(next) = next(if active { rows.len() } else { 0 }, focus) else {
            return false;
        };
        let (focus, y, height) = match next {
            Position::Header => (
                ListFocus::Header,
                view.top + PADDING - view.amount(scroll),
                HEADER,
            ),
            Position::Row(i, column) => {
                let (row, uuid) = rows[i];
                (
                    ListFocus::Player(PlayerFocus { uuid, column }),
                    view.row_y(row, scroll),
                    ROW,
                )
            }
        };
        self.players_list.focus = Some((realm, focus));
        if let SubScreen::Configure { players_scroll, .. } = &mut self.subscreen {
            view.reveal(y, height, players_scroll);
        }
        true
    }

    pub(super) fn draw_players(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        size: (f32, f32),
        show_focus: bool,
    ) {
        let Some((realm, players, scroll, active)) = self.player_rows() else {
            return;
        };
        let Some(view) = Viewport::new(size, players.len()) else {
            return;
        };
        let focus = self
            .players_list
            .focus
            .filter(|(id, _)| show_focus && *id == realm)
            .map(|(_, focus)| focus);
        let labels = LABELS.map(crate::lang::get);
        let widths = labels
            .each_ref()
            .map(|text| 16.0 + HudBatch::text_width(atlas, text));
        self.players_list.widths.set(Some(widths));

        // HeaderEntry is the first scrolling entry, not a separate static label.
        let first = batch.vertices.len();
        let heading = Text::translate(
            "mco.configure.world.invited.number",
            &[Text::literal(players.len().to_string())],
        )
        .underlined();
        let width = atlas.font.width_text(&heading.resolved());
        self.players_list.heading_width.set(Some(width));
        let heading_rect @ [x, y, w, h] = view.heading_rect(width, scroll);
        let mut rendered = RenderedPlayers {
            realm,
            viewport: view,
            amount: view.amount(scroll),
            heading: view.clip(heading_rect),
            controls: Vec::new(),
        };
        if let Some(rect) = rendered.heading {
            let focused = focus == Some(ListFocus::Header);
            widgets::record(&heading.plain(), rect, true);
            widgets::narratable(|| {
                widgets::Narratable::text(heading.plain(), rect).hovered(
                    focused || crate::ui::hovered(cursor, rect[0], rect[1], rect[2], rect[3]),
                )
            });
            if focused {
                batch.fill(atlas, x, y, w, h, u32::MAX);
                batch.fill(atlas, x + 1.0, y + 1.0, w - 2.0, h - 2.0, 0xFF00_0000);
            }
            batch.text_styled(atlas, &heading, x + 4.0, y + 4.0, u32::MAX, true);
        }
        for (i, player) in players.iter().enumerate() {
            let y = view.row_y(i, scroll);
            if y + ROW <= view.top {
                continue;
            }
            if y >= view.bottom {
                break;
            }
            let face_uuid = player.uuid.as_deref();
            // The CPU draw fixture exercises the same widgets without starting profile requests.
            #[cfg(test)]
            let face_uuid = face_uuid.filter(|_| !self.players_list.offline_faces);
            let face = self.face_sprite(batch, atlas, face_uuid, i);
            batch.sprite_scaled(
                atlas,
                face.as_deref().unwrap_or("misc/unknown_server"),
                view.row_left + PADDING,
                y + PADDING,
                32.0,
                32.0,
                u32::MAX,
            );
            let color = if !player.accepted {
                0xFFA0_A0A0
            } else if player.online {
                0xFF00_FF00
            } else {
                u32::MAX
            };
            batch.text(
                atlas,
                &player.name,
                view.row_left + 42.0,
                y + 14.0,
                color,
                true,
            );
            let id = player_uuid(player);
            for control in view.controls(y, player.operator, widths) {
                let Some(rect @ [x, y, w, h]) = view.clip(control.rect) else {
                    continue;
                };
                if let Some(uuid) = id.filter(|_| active) {
                    rendered
                        .controls
                        .push((rect, i, control.column.action(player, uuid)));
                }
                let enabled = active && id.is_some();
                let hover = crate::ui::hovered(cursor, x, y, w, h);
                let focused = id.is_some_and(|uuid| {
                    focus
                        == Some(ListFocus::Player(PlayerFocus {
                            uuid,
                            column: control.column,
                        }))
                });
                // Registration uses the same clipped bounds as pointer hit testing.
                widgets::record(&labels[control.style], rect, enabled);
                widgets::widget_cursor(hover, enabled);
                widgets::narratable(|| {
                    let named = crate::lang::tr("mco.invited.player.narration", &[&player.name]);
                    widgets::Narratable::button(
                        widgets::join_for_narration(&[&named, &labels[control.style]]),
                        rect,
                        enabled,
                    )
                    .hovered(hover || focused)
                });
                control.draw(batch, atlas, enabled, hover || focused);
            }
        }
        self.players_list.rendered.replace(Some(rendered));
        batch.clip_since(first, 0.0, view.top, view.width, view.bottom);
        if view.bottom - view.top >= 40.0 {
            crate::ui::scrollbar(
                batch,
                atlas,
                view.bar_x(),
                view.top,
                view.bottom,
                view.content,
                view.amount(scroll),
            );
        }
    }
}
