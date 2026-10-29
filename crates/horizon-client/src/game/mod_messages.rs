//! Private program delivery over the existing play connection.
use super::{Game, lan::LanHost};
use horizon_protocol::{
    crypto::uuid_string,
    packets::{
        configuration::CustomPayload,
        play::{Clientbound as C, Serverbound as S},
    },
    program_messages as wire,
};
#[derive(Default)]
pub(super) struct MessageState {
    world: Option<u64>,
    generation: Option<u64>,
    instance: Option<u64>,
    advertised: Option<Vec<wire::Subscription>>,
    reset: bool,
}
impl Game<'_> {
    pub(super) fn clear_mod_messages(&mut self) {
        let _ = self.mods.mods.set_message_context(None);
        self.mods.messages = MessageState::default();
    }
    pub(super) fn sync_mod_messages(&mut self) {
        let active = if self.remote.is_some() {
            self.player.is_some()
        } else {
            !self.world_root.as_os_str().is_empty()
                && (self.player.is_some() || !self.options.execution_role.is_client())
        };
        if !active {
            if self.mods.messages.world.is_some() {
                self.clear_mod_messages();
            }
            return;
        }
        if self.mods.messages.world.is_none() {
            self.mods.messages.world = horizon_mods::messages::new_epoch().ok();
        }
        let Ok(instance) = self.mods.mods.message_instance() else {
            self.clear_mod_messages();
            return;
        };
        if self.mods.messages.instance != Some(instance) {
            if self.mods.messages.instance.is_some() && self.remote.is_none() {
                self.mods.messages.reset = true;
            }
            self.mods.messages.instance = Some(instance);
            self.mods.messages.generation = None;
            self.mods.messages.advertised = None;
        }
        let generation = self.mods.mods.generation();
        if self.mods.messages.generation != Some(generation) {
            if self.mods.messages.generation.is_some() {
                let _ = self.mods.mods.set_message_context(None);
                if self.remote.is_none() {
                    self.mods.messages.reset = true;
                }
            }
            self.mods.messages.generation = Some(generation);
            self.mods.messages.advertised = None;
        }
        if let Err(error) = self.mods.mods.set_message_context(self.mods.messages.world) {
            log::warn!("Horizon: program messages unavailable: {error}");
            return;
        }
        if self.remote.is_none()
            && self.options.execution_role.is_client()
            && self.lan_acting.is_none()
        {
            let player = self.player_uuid_string();
            self.mods.mods.set_message_local_player(Some(player));
        } else if !self.options.execution_role.is_client() || self.remote.is_some() {
            self.mods.mods.set_message_local_player(None);
        }
        if !self.mods.messages.reset {
            if let Some(host) = &self.lan {
                for g in host.hub.guests.iter().filter(|g| g.placed) {
                    let subscriptions = g.program_subscriptions.iter().map(subscription).collect();
                    let _ = self
                        .mods
                        .mods
                        .set_message_routes(&uuid_string(g.uuid), subscriptions);
                }
            }
        }
        if self.remote.as_ref().is_some_and(|r| r.quiet_commands) {
            let subscriptions: Vec<_> = self
                .mods
                .mods
                .message_subscriptions()
                .into_iter()
                .map(|s| wire::Subscription {
                    receiver: s.receiver,
                    epoch: s.epoch,
                    sender: s.sender,
                    channel: s.channel,
                })
                .collect();
            if self.mods.messages.advertised.as_ref() != Some(&subscriptions) {
                self.send(S::CustomPayload(CustomPayload {
                    channel: wire::SUBSCRIBE_CHANNEL.into(),
                    data: wire::encode_subscriptions(&subscriptions),
                }));
                self.mods.messages.advertised = Some(subscriptions);
            }
        }
    }
    pub(super) fn receive_program_subscriptions(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        data: &[u8],
    ) -> Result<(), String> {
        self.sync_mod_messages();
        if self.mods.messages.reset {
            return Err("program subscription reset pending".into());
        }
        let values = wire::decode_subscriptions(data).map_err(|e| e.to_string())?;
        let guest = host
            .hub
            .guest_mut(conn)
            .filter(|g| g.placed && g.brand == "horizon")
            .ok_or("program subscriptions require an active Horizon connection")?;
        self.mods.mods.set_message_routes(
            &uuid_string(guest.uuid),
            values.iter().map(subscription).collect(),
        )?;
        guest.program_subscriptions = values;
        Ok(())
    }
    pub(super) fn receive_program_delivery(&mut self, data: &[u8]) -> Result<(), String> {
        self.sync_mod_messages();
        let value = wire::decode_delivery(data).map_err(|e| e.to_string())?;
        self.mods.mods.receive_program_message(
            &value.receiver,
            value.epoch,
            &value.sender,
            &value.channel,
            &value.payload,
        )?;
        Ok(())
    }
    pub(super) fn reset_program_delivery(&mut self) {
        let _ = self.mods.mods.set_message_context(None);
        let _ = self.mods.mods.set_message_context(self.mods.messages.world);
        self.mods.messages.advertised = None;
    }
    pub(super) fn route_program_messages(&mut self, mut host: Option<&mut LanHost>) {
        self.sync_mod_messages();
        if self.mods.messages.reset {
            if let Some(host) = host.as_deref_mut().or(self.lan.as_mut()) {
                for guest in &mut host.hub.guests {
                    if !guest.program_subscriptions.is_empty() {
                        guest.send(&C::CustomPayload(CustomPayload {
                            channel: wire::RESET_CHANNEL.into(),
                            data: Vec::new(),
                        }));
                        guest.program_subscriptions.clear();
                        let _ = self
                            .mods
                            .mods
                            .set_message_routes(&uuid_string(guest.uuid), Vec::new());
                    }
                }
                self.mods.messages.reset = false;
            } else if self.options.execution_role.is_client() && self.lan_acting.is_none() {
                self.mods.messages.reset = false;
            }
        }
        // The outer as_guest call supplies the authoritative hub after restoring its actor.
        if self.lan_acting.is_some() && host.is_none() && self.lan.is_none() {
            return;
        }
        for delivery in self.mods.mods.drain_program_messages() {
            if self.mods.messages.world != Some(delivery.world) {
                continue;
            }
            if self.remote.is_none()
                && self.options.execution_role.is_client()
                && self.player_uuid_string() == delivery.player
            {
                let _ = self.mods.mods.receive_program_message(
                    &delivery.receiver,
                    delivery.receiver_epoch,
                    &delivery.sender,
                    &delivery.channel,
                    &delivery.payload,
                );
                continue;
            }
            let wire = wire::Delivery {
                receiver: delivery.receiver,
                epoch: delivery.receiver_epoch,
                sender: delivery.sender,
                channel: delivery.channel,
                payload: delivery.payload.to_vec(),
            };
            let packet = C::CustomPayload(CustomPayload {
                channel: wire::DELIVERY_CHANNEL.into(),
                data: wire::encode_delivery(&wire),
            });
            if let Some(host) = host.as_deref_mut().or(self.lan.as_mut()) {
                if let Some(guest) = host
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.placed && uuid_string(g.uuid) == delivery.player)
                {
                    if guest.sender.pending_bytes() <= 2 << 20
                        && guest.program_subscriptions.iter().any(|s| {
                            s.receiver == wire.receiver
                                && s.epoch == wire.epoch
                                && s.sender == wire.sender
                                && s.channel == wire.channel
                        })
                    {
                        guest.send(&packet);
                    }
                }
            }
        }
    }
}
fn subscription(value: &wire::Subscription) -> horizon_mods::messages::Subscription {
    horizon_mods::messages::Subscription {
        receiver: value.receiver.clone(),
        epoch: value.epoch,
        sender: value.sender.clone(),
        channel: value.channel.clone(),
    }
}
