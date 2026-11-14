//! `/say` and `/me` (SayCommand, EmoteCommands) and `/msg` (MsgCommand): the
//! lines they send beyond the command source's own feedback.
use super::{Arg, Context, Game, R, Target};
use horizon_client::text::{self, Text};

impl Game<'_> {
    /// `SayCommand.register`: resolve the source name and broadcast unsigned announcements.
    pub(super) fn cmd_say(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let raw = match ctx.get(if id == "me" { "action" } else { "message" }) {
            Some(Arg::Str(s)) => s.clone(),
            _ => String::new(),
        };
        let message = self.expand_selectors(&raw);
        // CommandSourceStack.getDisplayName: the executing entity's name.
        let name = match self.command_source {
            Some(source) => match source.entity {
                Some(target) => self.target_name(target),
                None => source
                    .block
                    .and_then(|pos| match self.world.block_entities.get(&pos) {
                        Some(horizon_client::containers::BlockEntity::CommandBlock(block)) => {
                            Some(block.name())
                        }
                        _ => None,
                    })
                    .or_else(|| {
                        source.cart.and_then(|id| {
                            self.carts
                                .iter()
                                .find(|cart| cart.id == id)
                                .map(|cart| cart.command.name())
                        })
                    })
                    .unwrap_or_else(|| self.server_name()),
            },
            None => self.player_display_name(),
        };
        // A player's command whose output is captured (a function, `execute …
        // run say`, the command harness) gets the announcement with that
        // output, in order; everyone else gets the broadcast.
        let captured_for = self.chat_capture.as_ref().and(self.lan_acting);
        if id == "say" || id == "me" {
            // SayCommand and EmoteCommands broadcast independently of the
            // command source's feedback: to every player, and logged.
            let (chat_type, key) = if id == "say" {
                ("say_command", "chat.type.announcement")
            } else {
                ("emote_command", "chat.type.emote")
            };
            let line = Text::translate(key, &[name.clone(), message.clone()]);
            if let Some(host) = &self.lan {
                use horizon_client::lan::data::component_nbt;
                host.hub.broadcast_source_chat(
                    chat_type,
                    component_nbt(&name.component()),
                    component_nbt(&message.component()),
                    captured_for,
                );
            }
            if self.lan.is_some() && self.options.dedicated.is_some() {
                self.lan_log_event(horizon_client::lan::hub::HostEvent::Chat {
                    line: line.plain(),
                    secure: false,
                });
            }
            if self.lan.is_some() && self.chat_capture.is_some() {
                // The broadcast skipped the acting player: their copy goes with
                // the captured output. Without one (the console), the broadcast
                // reached everyone and the console logged it above.
                if captured_for.is_some()
                    && let Some(capture) = self.chat_capture.as_mut()
                {
                    capture.push(line);
                }
                return Ok(1);
            }
            self.player_message(line);
            return Ok(1);
        }
        // MsgCommand.sendMessage: the outgoing line to the source, the incoming
        // line to each target player.
        let targets = self.entities(ctx, "targets", true)?;
        for t in &targets {
            let target = self.target_name(*t);
            self.system_message(
                Text::translate(
                    "commands.message.display.outgoing",
                    &[target, message.clone()],
                )
                .with_color(text::GRAY)
                .italic(),
            );
            let incoming = Text::translate(
                "commands.message.display.incoming",
                &[name.clone(), message.clone()],
            )
            .with_color(text::GRAY)
            .italic();
            // A guest target hears it on its own connection; the player in
            // place (`Target::Player`, the guest whose turn it is included)
            // gets it with its command's output, as `MsgCommand` sends it to
            // the source and to the target.
            let conn = match *t {
                Target::Guest(conn) => Some(conn),
                _ => None,
            };
            let Some(conn) = conn else {
                self.player_message(incoming);
                continue;
            };
            if let Some(host) = &self.lan {
                use horizon_client::lan::data::component_nbt;
                if let Some(k) = host.hub.guests.iter().position(|g| g.conn == conn) {
                    host.hub.send_source_chat(
                        k,
                        "msg_command_incoming",
                        component_nbt(&name.component()),
                        None,
                        component_nbt(&message.component()),
                    );
                }
            }
        }
        Ok(targets.len() as i32)
    }
}
