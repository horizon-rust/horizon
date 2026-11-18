//! The send pass in two steps (what each entity sends, then what each client is given), and the
//! tick's changes given as ranges of one buffer (`Body::Run`) instead of a body for each packet.

use super::*;
use crate::lan::Body;
use crate::lan::body::{Run, RunBuf};

impl Entities {
    /// Whether the tick's changes go out as ranges of one buffer.
    pub(super) fn send_runs(&self) -> bool {
        #[cfg(test)]
        {
            !self.runs_off
        }
        #[cfg(not(test))]
        {
            true
        }
    }

    /// [`Self::send_two_phase`] on several threads, a body for each packet (the send benchmark).
    #[cfg(test)]
    pub(super) fn send_on_threads(
        &mut self,
        entities: &[&EntityState],
        riding: &Riding,
        viewers: &mut [Viewer],
        active: &[usize],
    ) {
        self.send_two_phase(entities, riding, viewers, active, true, false);
    }

    /// `parallel`: on several threads. `runs`: what a client tracks of the tick's changes is
    /// given as ranges of one buffer made for all (`Body::Run`), not one body for each packet.
    pub(super) fn send_two_phase(
        &mut self,
        entities: &[&EntityState],
        riding: &Riding,
        viewers: &mut [Viewer],
        active: &[usize],
        parallel: bool,
        runs: bool,
    ) {
        /// One entity's part of the tick: its changes and its pairing, encoded.
        struct Made<'s> {
            index: usize,
            state: &'s mut ServerEntity,
            changes: Bodies,
            pairing: Option<Bodies>,
            synced: bool,
            /// Its changes' packets in the run buffers (all clients, vanilla clients).
            span: [(u32, u32); 2],
        }
        let Self {
            states,
            pairs,
            is_moved,
            synced: synced_ranges,
            flags,
            ..
        } = self;
        let mut made: Vec<Made> = states
            .of_indices_mut(active)
            .into_iter()
            .zip(active)
            .map(|(state, &index)| Made {
                index,
                state,
                changes: Bodies::default(),
                pairing: None,
                synced: false,
                span: [(0, 0); 2],
            })
            .collect();
        let (pairs, is_moved, synced_ranges) = (&*pairs, &*is_moved, &*synced_ranges);
        let phase_one = |part: &mut [Made]| {
            let mut packets = Vec::new();
            for m in part {
                let (s, e) = (&mut *m.state, entities[m.index]);
                packets.clear();
                // As in `send_in_order`: paired as it joined, its first changes go out with
                // the pairing; an idle one sends only its leash.
                m.synced = s.added
                    && e.kind != "minecraft:player"
                    && !s.arrow
                    && synced_ranges
                        .iter()
                        .any(|&(after, through)| e.id > after && e.id <= through);
                if !s.added || m.synced {
                    if e.idle && !is_moved[m.index] && !e.needs_sync {
                        s.send_leash(e, &mut packets);
                    } else {
                        s.send_changes(e, riding, &mut packets);
                    }
                }
                s.added = false;
                m.changes.encode(&packets);
                if pairs[m.index]
                    && !e.bare
                    && let Some(kind) = s.kind
                {
                    packets.clear();
                    s.pairing(e, kind, riding, &mut packets);
                    m.pairing = Some(Bodies::of(&packets));
                }
            }
        };
        if parallel {
            horizon_threads::par_chunks_mut(&mut made, 64, phase_one);
        } else {
            phase_one(&mut made);
        }
        // Nothing to send and nobody to pair with.
        made.retain(|m| !m.changes.all.is_empty() || m.pairing.is_some());
        // The tick's changes back to back, once for the clients of Horizon and once for the
        // others, and where each entity's are.
        let mut buffers = (RunBuf::default(), RunBuf::default());
        if runs {
            for m in &mut made {
                let (a, v) = (buffers.0.mark(), buffers.1.mark());
                for b in &m.changes.all {
                    buffers.0.push(b);
                }
                for b in m.changes.vanilla.as_ref().unwrap_or(&m.changes.all) {
                    buffers.1.push(b);
                }
                m.span = [(a, buffers.0.mark()), (v, buffers.1.mark())];
            }
        }
        let buffers = (Arc::new(buffers.0), Arc::new(buffers.1));
        let made = &made;
        let mut clients: Vec<(&mut Viewer, &Vec<u8>)> = viewers.iter_mut().zip(&*flags).collect();
        let phase_two = |part: &mut [(&mut Viewer, &Vec<u8>)]| {
            for (v, flags) in part {
                for m in made {
                    let f = flags[m.index];
                    if f & TRACKS != 0 {
                        if runs && v.sources.is_none() {
                            let (buf, (from, to)) = match v.horizon {
                                true => (&buffers.0, m.span[0]),
                                false => (&buffers.1, m.span[1]),
                            };
                            if from < to {
                                let joined = match v.out.last_mut() {
                                    Some(Body::Run(last)) => last.extend(buf, from, to),
                                    _ => false,
                                };
                                if !joined {
                                    v.out.push(Body::Run(Run::new(buf, from, to)));
                                }
                            }
                        } else {
                            v.out.extend(m.changes.to(v).iter().cloned());
                        }
                    } else if f & SEES != 0
                        && let Some(pairing) = &m.pairing
                    {
                        v.out.extend(pairing.to(v).iter().cloned());
                        if m.synced {
                            v.out.extend(m.changes.to(v).iter().cloned());
                        }
                        let id = entities[m.index].id;
                        if v.tracker
                            .seen
                            .insert(id, Arc::clone(&m.state.tracked))
                            .is_none()
                        {
                            v.tracker.events.push(TrackEvent {
                                start: true,
                                entity: id,
                                tracked: Arc::clone(&m.state.tracked),
                            });
                        }
                    }
                }
            }
        };
        if parallel {
            horizon_threads::par_chunks_mut(&mut clients, 4, phase_two);
        } else {
            phase_two(&mut clients);
        }
    }
}
