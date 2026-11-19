//! `/random` (RandomCommand) and `/teammsg` (TeamMsgCommand).
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{
    clock::SprintReport,
    text::{self, Text},
};
use horizon_core::java_number::format_fixed;
use horizon_core::random::RandomSource;

pub(super) fn millis(nanos: u64) -> String {
    format_fixed(f64::from(nanos as f32 / 1_000_000.0), 1)
}

impl Game<'_> {
    /// `/stopwatch` (StopwatchCommand).
    pub(super) fn cmd_stopwatch(&mut self, id: &str, ctx: &Context) -> R<i32> {
        horizon_gameplay::commands::misc::stopwatch(self, id, ctx)
    }
    /// `/tick` (TickCommand) over the game clock.
    pub(super) fn cmd_tick(&mut self, id: &str, ctx: &Context) -> R<i32> {
        horizon_gameplay::commands::misc::tick(self, id, ctx)
    }
    /// The report `/tick sprint` prints when a sprint ends. Each caller then
    /// runs `on_tick_rate_changed`, as `ServerTickRateManager.finishTickSprint` does.
    pub(in crate::game) fn sprint_report(&mut self, report: &SprintReport) {
        self.feedback(Text::translate(
            "commands.tick.sprint.report",
            &[
                Text::literal(report.ticks_per_second.to_string()),
                Text::literal(report.milliseconds_per_tick.clone()),
            ],
        ));
    }
    pub(super) fn cmd_random(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let seed = self.level.seed;
        if id == "random.reset" {
            let sequence = match ctx.get("sequence") {
                Some(Arg::Resource(s)) => Some(s.clone()),
                _ => None,
            };
            let int = |name: &str| match ctx.get(name) {
                Some(Arg::Int(v)) => Some(*v),
                _ => None,
            };
            let flag = |name: &str| !matches!(ctx.get(name), Some(Arg::Bool(false)));
            let name_text = |s: &str| Text::literal(s.to_owned());
            return match (sequence, int("seed")) {
                (None, None) => {
                    let n = self.rand_sequences.clear();
                    self.feedback(Text::translate(
                        "commands.random.reset.all.success",
                        &[Text::literal(n.to_string())],
                    ));
                    Ok(n as i32)
                }
                (None, Some(salt)) => {
                    self.rand_sequences.set_defaults(
                        salt,
                        flag("includeWorldSeed"),
                        flag("includeSequenceId"),
                    );
                    let n = self.rand_sequences.clear();
                    self.feedback(Text::translate(
                        "commands.random.reset.all.success",
                        &[Text::literal(n.to_string())],
                    ));
                    Ok(n as i32)
                }
                (Some(s), None) => {
                    self.rand_sequences.reset(&s, seed);
                    self.feedback(Text::translate(
                        "commands.random.reset.success",
                        &[name_text(&s)],
                    ));
                    Ok(1)
                }
                (Some(s), Some(salt)) => {
                    self.rand_sequences.reset_with(
                        &s,
                        seed,
                        salt,
                        flag("includeWorldSeed"),
                        flag("includeSequenceId"),
                    );
                    self.feedback(Text::translate(
                        "commands.random.reset.success",
                        &[name_text(&s)],
                    ));
                    Ok(1)
                }
            };
        }
        let Some(Arg::IntRange((lo, hi))) = ctx.get("range").cloned() else {
            return Ok(0);
        };
        let (min, max) = (lo.unwrap_or(i32::MIN), hi.unwrap_or(i32::MAX));
        let span = i64::from(max) - i64::from(min);
        if span == 0 {
            return Err(err("commands.random.error.range_too_small", &[]));
        }
        if span >= i64::from(i32::MAX) {
            return Err(err("commands.random.error.range_too_large", &[]));
        }
        let value = match ctx.get("sequence") {
            Some(Arg::Resource(s)) => {
                let s = s.clone();
                self.rand_sequences
                    .get(&s, seed)
                    .next_int_between_inclusive(min, max)
            }
            _ => self.world.random.next_int_between_inclusive(min, max),
        }
        .unwrap_or(min);
        if id == "random.roll" {
            let line = Text::translate(
                "commands.random.roll",
                &[
                    self.source_display_name(),
                    Text::literal(value.to_string()),
                    Text::literal(min.to_string()),
                    Text::literal(max.to_string()),
                ],
            );
            // PlayerList.broadcastSystemMessage: every player hears the roll,
            // the source's own copy with the captured output it has (`/say`'s).
            let captured = self.chat_capture.as_ref().and(self.lan_acting);
            if let Some(host) = &self.lan {
                let content = horizon_client::lan::data::component_nbt(&line.component());
                match captured {
                    Some(conn) => host.hub.system_message_except(content, conn),
                    None => host.hub.system_message(content),
                }
            }
            // MinecraftServer.sendSystemMessage: the server's log, unless the
            // console's captured reply below is what logs it (`console_reply`).
            if self.options.dedicated.is_some()
                && (self.lan_acting.is_some() || self.chat_capture.is_none())
            {
                super::super::dedicated::log("INFO", &format!("System chat: {}", line.plain()));
            }
            self.system_message(line);
        } else {
            self.feedback(Text::translate(
                "commands.random.sample.success",
                &[Text::literal(value.to_string())],
            ));
        }
        Ok(value)
    }
    /// `/teammsg`: the sender and every player on its team hear the message.
    pub(super) fn cmd_teammsg(&mut self, ctx: &Context) -> R<i32> {
        let entity = match self.command_source {
            Some(s) => s.entity,
            None => Some(Target::Player),
        };
        let Some(entity) = entity else {
            return Err(err("permissions.requires.entity", &[]));
        };
        let Some(holder) = self.holder_name(entity) else {
            return Err(err("permissions.requires.entity", &[]));
        };
        let Some(team) = self.score.board.players_team(&holder).cloned() else {
            return Err(err("commands.teammsg.failed.noteam", &[]));
        };
        let raw = match ctx.get("message") {
            Some(Arg::Str(m)) => m.clone(),
            _ => String::new(),
        };
        let message = self.expand_selectors(&raw);
        let hover = text::Style {
            hover: Some(std::sync::Arc::new(Text::translate(
                "chat.type.team.hover",
                &[],
            ))),
            click: Some(text::ClickEvent::SuggestCommand("/teammsg ".into())),
            ..text::Style::default()
        };
        let team_name = team.formatted_display_name().with_style(&hover);
        let sender = self.target_name(entity);
        let line = |sent: bool| {
            Text::translate(
                if sent {
                    "chat.type.team.sent"
                } else {
                    "chat.type.team.text"
                },
                &[team_name.clone(), sender.clone(), message.clone()],
            )
        };
        // TeamMsgCommand.receivers: every player on the team, and the sender.
        let board = &self.score.board;
        let same_team = |name: &str| {
            board
                .players_team(name)
                .is_some_and(|t| t.name == team.name)
        };
        // The player in place (`Target::Player`), unless a server has none.
        let local = self.player.is_some()
            && (entity == Target::Player || same_team(self.acting_player_name()));
        // The guests who hear it, and whether the line is theirs to send. The
        // guest whose turn it is is `Target::Player`, not a guest here.
        let guests: Vec<(usize, bool)> = self
            .lan
            .iter()
            .flat_map(|host| host.hub.guests.iter().enumerate())
            .filter(|(_, g)| g.placed && Some(g.conn) != self.lan_acting)
            .filter(|(_, g)| entity == Target::Guest(g.conn) || same_team(&g.name))
            .map(|(k, g)| (k, entity == Target::Guest(g.conn)))
            .collect();
        let receivers = guests.len() as i32 + i32::from(local);
        // A chat line: the player in place hears it with its command's output
        // (`player_message` keeps a guest's own line for its turn's capture).
        if local {
            self.player_message(line(entity == Target::Player));
        }
        if let Some(host) = &self.lan {
            use horizon_client::lan::data::component_nbt;
            let team = component_nbt(&team_name.component());
            let name = component_nbt(&sender.component());
            let text = component_nbt(&message.component());
            let send = |k: usize, sent: bool| {
                host.hub.send_source_chat(
                    k,
                    if sent {
                        "team_msg_command_outgoing"
                    } else {
                        "team_msg_command_incoming"
                    },
                    name.clone(),
                    Some(team.clone()),
                    text.clone(),
                );
            };
            for (k, sent) in guests {
                send(k, sent);
            }
        }
        Ok(receivers)
    }
}

#[cfg(test)]
#[path = "tick_status_broadcast_tests.rs"]
mod tick_status_broadcast_tests;

#[cfg(test)]
mod random_command_tests;
