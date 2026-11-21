//! Receiving-provider credit follows ordered events and decoder placeholders.
use super::super::raw_backlog::BundlePart;
use super::*;

impl Remote {
    pub(super) fn receive_credited_backlog(&mut self, events: &mut VecDeque<Received>) {
        // Chunk packets start decoding now; the frame that reaches one in
        // order inserts its column.
        for receipt in self.session.poll() {
            let (event, credit) = receipt.into_parts();
            events.push_back(self.received(event, credit));
        }
    }

    /// Turn one delivered event into backlog slots. A closed bundle becomes a
    /// single `Received::Bundle`: its chunks are queued on the decoder, and the
    /// whole group is released by the frame only when all of them can decode.
    fn received(
        &self,
        event: Event,
        credit: horizon_client::net::ingress::ReceiveLease<Event>,
    ) -> Received {
        match event {
            Event::Bundle(bundle) => {
                let mut parts = Vec::with_capacity(bundle.len());
                for event in Vec::from(bundle) {
                    match event {
                        Event::Packet(p) if matches!(*p, C::LevelChunkWithLight(_)) => {
                            match self.decoder.submit(self.chunk_format.clone(), p) {
                                None => parts.push(BundlePart::Chunk(self.chunk_format.clone())),
                                Some(p) => parts.push(BundlePart::Event(Event::Packet(p))),
                            }
                        }
                        event => parts.push(BundlePart::Event(event)),
                    }
                }
                Received::Bundle(parts, credit)
            }
            Event::Packet(p) if matches!(*p, C::LevelChunkWithLight(_)) => {
                match self.decoder.submit(self.chunk_format.clone(), p) {
                    None => Received::Chunk(self.chunk_format.clone(), Some(credit)),
                    Some(p) => Received::Event(Event::Packet(p), Some(credit)),
                }
            }
            event => Received::Event(event, Some(credit)),
        }
    }
}

impl Game<'_> {
    pub(super) fn poll_remote_events(&mut self) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let mut events = std::mem::take(&mut r.backlog);
        r.receive_credited_backlog(&mut events);
        let overdue = r
            .backlog_since
            .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
        let mut chunks = Duration::ZERO;
        // A run of ForgetLevelChunk (thousands when the server's view moves far: a
        // teleport at 32 chunks) is forgotten at once, before whatever follows it.
        let mut forgotten = horizon_client::fast_hash::FxHashSet::default();
        let mut forgotten_credits = Vec::new();
        // C1..CN below belong to one closed bundle; while it is non-zero the
        // frame may not stop at a chunk budget or reorder its items.
        let mut group_left = 0usize;
        while let Some(item) = events.pop_front() {
            let item = match item {
                Received::Bundle(parts, credit) => {
                    let chunk_count = parts
                        .iter()
                        .filter(|part| matches!(part, BundlePart::Chunk(_)))
                        .count();
                    let ready = self
                        .remote
                        .as_ref()
                        .map_or(0, |r| r.decoder.ready_run(chunk_count));
                    if ready < chunk_count {
                        events.push_front(Received::Bundle(parts, credit));
                        break;
                    }
                    let last = parts.len().saturating_sub(1);
                    let mut credit = Some(credit);
                    group_left = parts.len();
                    for (index, part) in parts.into_iter().enumerate().rev() {
                        let lease = if index == last { credit.take() } else { None };
                        events.push_front(match part {
                            BundlePart::Event(event) => Received::Event(event, lease),
                            BundlePart::Chunk(format) => Received::Chunk(format, lease),
                        });
                    }
                    continue;
                }
                other => other,
            };
            let in_bundle = group_left > 0;
            if in_bundle {
                group_left -= 1;
            }
            if let Received::Event(Event::Packet(packet), _) = &item
                && let C::ForgetLevelChunk(p) = &**packet
            {
                self.observe_client_packet(packet);
                forgotten.insert((p.pos[0], p.pos[1]));
                forgotten_credits.push(item.into_event_credit());
                continue;
            }
            horizon_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
            forgotten_credits.clear();
            let chunk = matches!(item, Received::Chunk(_, _));
            if chunk && !in_bundle && !overdue && chunks >= CHUNK_PACKETS_PER_FRAME {
                events.push_front(item);
                break;
            }
            let started = chunk.then(Instant::now);
            let (event, _credit) = match item {
                Received::Event(event, credit) => (event, credit),
                Received::Chunk(format, credit) => {
                    // A column still decoding waits for the next frame, with
                    // what came after it, overdue or not.
                    let Some(next) = self.remote.as_ref().map(|r| r.decoder.next()) else {
                        break;
                    };
                    let (packet, column) = match next {
                        Next::Ready(decoded) => decoded,
                        Next::Decoding => {
                            debug_assert!(
                                !in_bundle,
                                "a bundle was released before all its chunks could decode"
                            );
                            events.push_front(Received::Chunk(format, credit));
                            break;
                        }
                        Next::Lost => {
                            log::warn!("Horizon: the chunk decoder stopped; a column is missing");
                            continue;
                        }
                    };
                    self.observe_client_packet(&packet);
                    if let C::LevelChunkWithLight(p) = *packet {
                        horizon_client::spans::time("pr_chunk", || {
                            self.remote_chunk(&p, Some((&format, column)))
                        });
                    }
                    chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
                    continue;
                }
                Received::Bundle(..) => {
                    unreachable!("bundles are expanded before this match")
                }
            };
            match event {
                Event::Status(s) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.status = s;
                    }
                }
                Event::Joined {
                    registries,
                    name,
                    uuid,
                } => {
                    self.set_registries(registries);
                    self.extras_joined();
                    self.close_dialogs();
                    if let Some(r) = self.remote.as_mut() {
                        r.joined = true;
                        r.profile = uuid;
                        r.profile_name = name.clone();
                        r.status = horizon_client::lang::get_or(
                            "multiplayer.downloadingTerrain",
                            "Loading terrain...",
                        );
                    }
                    self.begin_book_negotiation();
                }
                Event::Features(features) => {
                    if let Some(remote) = &mut self.remote {
                        remote.features = features.into();
                    }
                }
                Event::Brand(brand) => {
                    if let Some(remote) = &mut self.remote {
                        remote.brand = brand;
                    }
                }
                Event::QuietCommands => {
                    if let Some(remote) = &mut self.remote {
                        remote.quiet_commands = true;
                    }
                }
                Event::CustomBooks => {
                    if let Some(remote) = &mut self.remote {
                        remote.custom_books = true;
                    }
                }
                Event::FarTerrain(offer) => {
                    if let Some(remote) = &mut self.remote {
                        remote.far_terrain = Some(offer);
                    }
                }
                Event::Reconfiguring => {
                    self.reset_book_view();
                    // handleConfigurationStart: the held-back chat shows, then
                    // clearClientLevel opens ServerReconfigScreen over no level.
                    self.flush_chat_queue();
                    self.send_chat_acknowledgement();
                    self.close_dialogs();
                    // The server configuring us now (a proxy's next one)
                    // offers its own far terrain, or none, and announces its
                    // own features again.
                    if let Some(remote) = &mut self.remote {
                        remote.viewed.clear();
                        remote.far_terrain = None;
                        remote.quiet_commands = false;
                        remote.custom_books = false;
                    }
                    self.clear_lod();
                    if let Some(remote) = &self.remote {
                        remote.session.reset_far_terrain();
                    }
                    self.remove_local_player_body();
                    self.set_screen(Screen::Playing);
                    self.capture(false);
                    // clearClientLevel drops the client level now, so its
                    // unload comes before the configuration screen, not
                    // after the next login's load.
                    self.forget_all_remote_columns();
                    if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
                        self.native_client_level_unload(&old);
                    }
                    self.native_lifecycle.begin_close();
                    self.native_lifecycle_world_closed();
                    let ticks = self.ticks;
                    if let Some(r) = self.remote.as_mut() {
                        r.reconfiguring = Some(ticks);
                        r.chat_tree_received = false;
                    }
                }
                Event::Reconfigured(registries) => {
                    // handleConfigurationStart → clearClientLevel, and the next
                    // ClientPacketListener starts with no players or scoreboard,
                    // no waypoints (its own ClientWaypointManager: a proxy's next
                    // server never untracks the last one's), and its new
                    // ClientSuggestionProvider with no custom chat completions.
                    // clearClientLevel clears the narrator.
                    self.clear_narrator();
                    self.hud_disconnected();
                    self.score.board = Default::default();
                    self.clear_waypoints();
                    if let Some(r) = self.remote.as_mut() {
                        r.attachments.clear();
                        r.players.clear();
                        r.tab_players.clear();
                        r.listed.clear();
                        r.signatures = Default::default();
                        r.custom_completions.clear();
                    }
                    self.ui.registries.custom_completions.clear();
                    self.set_registries(registries);
                }
                // handleTransfer: "Cannot transfer to server from
                // singleplayer" (no ServerData), so the world just closes.
                Event::Transfer { host, port } => {
                    if let Some(r) = self.remote.as_mut()
                        && r.session.address != "local"
                    {
                        r.transfer = Some(format!("{host}:{port}"));
                    }
                }
                Event::PackPush(p) => self.handle_pack_push(p),
                Event::PackPop(id) => self.handle_pack_pop(id),
                Event::ServerLinks(links) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.server_links = crate::game::server_links::trusted(links);
                    }
                }
                Event::ShowDialog(dialog) => self.show_dialog(&dialog),
                Event::ClearDialog => self.clear_dialog(),
                Event::CodeOfConduct(text) => self.handle_code_of_conduct(text),
                Event::DecodeError(e) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.decode_errors += 1;
                        if r.decode_errors <= 20 {
                            log::warn!("Horizon: skipped undecodable packet {e}");
                        }
                    }
                }
                Event::Disconnected {
                    reason,
                    key,
                    from_server,
                } => {
                    self.reset_book_view();
                    log::info!("Horizon: disconnected: {reason}");
                    // ClientboundTransferPacket left a target: connect there
                    // instead of showing the disconnect, the stored cookies
                    // carried over (ClientCommonPacketListenerImpl.
                    // handleTransfer → ConnectScreen.startConnecting).
                    let transfer = self.remote.as_mut().and_then(|r| {
                        r.transfer.take().map(|address| {
                            (address, r.session.name.clone(), r.session.cookies.clone())
                        })
                    });
                    if let Some((address, name, cookies)) = transfer {
                        self.leave_server(None)?;
                        self.connect_server_with_cookies(&address, &name, Some(cookies))?;
                        return Ok(());
                    }
                    let retry = self.reconnect_after(&reason, key.as_deref(), from_server);
                    self.leave_server(Some(Text::literal(reason)))?;
                    self.offer_reconnect(retry);
                    return Ok(());
                }
                Event::RawPayload(mut incoming) => {
                    if !self.receive_client_payload(&mut incoming)? {
                        // A bundled payload's main root can defer (a busy guest).
                        // It stays at the head, so the rest of the group follows
                        // it in order once the root commits.
                        events.push_front(Received::Event(Event::RawPayload(incoming), _credit));
                        break;
                    }
                }
                Event::Packet(packet) => match *packet {
                    C::CustomPayload(p) if p.channel == horizon_client::perf_net::CHANNEL => {
                        self.remote_perf_reply(&p.data)
                    }
                    packet => self.handle_packet(packet)?,
                },
                Event::Bundle(_) => {
                    unreachable!("bundles are not transported inside bundles")
                }
            }
            chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.remote.is_none() {
                break;
            }
        }
        if self.remote.is_some() {
            horizon_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
        }
        if let Some(r) = self.remote.as_mut() {
            r.backlog_since = if events.is_empty() {
                None
            } else {
                Some(r.backlog_since.unwrap_or_else(Instant::now))
            };
            r.backlog = events;
        }
        self.send_suggestion_request();
        Ok(())
    }
}
