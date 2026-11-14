//! 1.21.4's packets in 1.21.3's layout for a 1.21.2 or 1.21.3 client (protocol 768), after 1.21.4's step:
//! - level_particles: 1.21.4 added `always_show` after `override_limiter`; it goes.
//! - player_info_update: 1.21.4 added UPDATE_HAT (bit 7, a bool last in each entry); it goes, UPDATE_LIST_ORDER
//!   (bit 6, 1.21.2's) stays.
//!
//! (Checked against the 26.3 jar's ClientboundPlayerInfoUpdatePacket.Action order and both releases' layouts.)
use super::step_1_21_1;
use crate::{
    Result, State,
    entity_data::{Value, serializer},
    ids as packet_ids,
    packets::play,
};

/// Entity data in 1.21.3's layout from 1.21.4's (`kind` is a pinned type; `None` leaves the data alone):
/// - The salmon's variant at 17 is the name `small`, `medium` or `large` (a String), not 1.21.4's id (0 to 2).
/// - The creaking's `is_tearing_down` (18) and home position (19) are 1.21.4's: they go.
pub(super) fn entity_data(kind: Option<i32>, data: &mut play::SetEntityData) {
    use crate::adapter::pinned_ids::entity_type as entity;
    match kind {
        Some(entity::SALMON) => {
            for (index, ser, value) in &mut data.values {
                if *index == 17
                    && let Value::Int(id) | Value::Id(id) = *value
                {
                    *ser = serializer::STRING;
                    *value = Value::String(
                        match id {
                            0 => "small",
                            2 => "large",
                            _ => "medium",
                        }
                        .into(),
                    );
                }
            }
        }
        Some(entity::CREAKING) => data
            .values
            .retain(|(index, _, _)| !(18..=19).contains(index)),
        _ => {}
    }
}

pub(super) fn clientbound(state: State, id: i32, payload: &[u8]) -> Result<Option<Vec<u8>>> {
    use packet_ids::play::clientbound as cb;
    if state != State::Play {
        return Ok(None);
    }
    Ok(match id {
        cb::LEVEL_PARTICLES => Some(step_1_21_1::particles(payload)?),
        cb::PLAYER_INFO_UPDATE => Some(step_1_21_1::player_info(payload, true)?),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::{Reader, Writer};

    /// 1.21.4's `set_entity_data` payload (literal bytes) through `step`, as the bytes written back.
    fn through(
        bytes: &[u8],
        kind: i32,
        step: impl Fn(Option<i32>, &mut crate::packets::play::SetEntityData),
    ) -> Vec<u8> {
        use crate::buf::{Codec, Reader, Writer};
        let mut d =
            <crate::packets::play::SetEntityData as Codec<_>>::read(&mut Reader::new(bytes))
                .unwrap();
        step(Some(kind), &mut d);
        let mut w = Writer::new();
        <crate::packets::play::SetEntityData as Codec<_>>::write(&d, &mut w);
        w.data
    }

    /// 1.21.4's salmon (entity 6: the from-bucket flag false, variant id 1) is 1.21.3's with the variant's name.
    #[test]
    fn a_salmon_s_variant_is_its_name() {
        use crate::adapter::pinned_ids::entity_type::SALMON;
        let medium = [6, 16, 8, 0, 17, 1, 1, 255];
        let mut named = vec![6, 16, 8, 0, 17, 4, 6];
        named.extend_from_slice(b"medium");
        named.push(255);
        assert_eq!(through(&medium, SALMON, entity_data), named);
        let mut large = vec![6, 16, 8, 0, 17, 4, 5];
        large.extend_from_slice(b"large");
        large.push(255);
        assert_eq!(
            through(&[6, 16, 8, 0, 17, 1, 2, 255], SALMON, entity_data),
            large
        );
        let mut small = vec![6, 17, 4, 5];
        small.extend_from_slice(b"small");
        small.push(255);
        assert_eq!(through(&[6, 17, 1, 0, 255], SALMON, entity_data), small);
    }

    /// 1.21.4's creaking (entity 7) has the tearing-down flag at 18 and the home position (none) at 19.
    #[test]
    fn a_creaking_loses_the_two_fields_1_21_4_added() {
        use crate::adapter::pinned_ids::entity_type::CREAKING;
        let newer = [7, 16, 8, 1, 17, 8, 0, 18, 8, 1, 19, 11, 0, 255];
        assert_eq!(
            through(&newer, CREAKING, entity_data),
            [7, 16, 8, 1, 17, 8, 0, 255]
        );
    }

    #[test]
    fn other_kinds_pass_through_untouched() {
        let bytes = [7, 17, 1, 1, 18, 8, 1, 255];
        assert_eq!(through(&bytes, 0, entity_data), bytes);
    }

    #[test]
    fn the_hat_goes_and_the_list_order_stays() {
        let mut w = Writer::new();
        w.u8(0x40 | 0x80 | 0x08); // listed, list order, hat
        w.varint(1);
        w.uuid(42);
        w.bool(true); // listed
        w.varint(7); // list order
        w.bool(true); // hat
        let out = clientbound(
            State::Play,
            packet_ids::play::clientbound::PLAYER_INFO_UPDATE,
            &w.data,
        )
        .unwrap()
        .unwrap();
        let mut r = Reader::new(&out);
        assert_eq!(r.u8().unwrap(), 0x48);
        assert_eq!(r.varint().unwrap(), 1);
        assert_eq!(r.uuid().unwrap(), 42);
        assert!(r.bool().unwrap());
        assert_eq!(r.varint().unwrap(), 7);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn particles_lose_always_show() {
        let out = clientbound(
            State::Play,
            packet_ids::play::clientbound::LEVEL_PARTICLES,
            &[1, 0, 9, 9],
        )
        .unwrap()
        .unwrap();
        assert_eq!(out, vec![1, 9, 9]);
    }
}
