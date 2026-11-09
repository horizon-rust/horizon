//! Work-site restocking and the trading-player refresh event.
use super::{Mob, MobEvent, MobWorld};

/// `WorkAtPoi.start`'s restock.
pub(super) fn restock_if_due(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let day = w.day_time.div_euclid(24000);
    if let Some(t) = m.extra.trade.as_mut() {
        let before = t.offers.clone();
        let before_restock = (t.last_restock, t.restocks_today);
        let restock = t.should_restock(w.game_time, day);
        if restock {
            t.restock(w.game_time);
        }
        // shouldRestock also catches up missed demand on a new day.
        if restock || t.offers != before || (t.last_restock, t.restocks_today) != before_restock {
            events.push(MobEvent::VillagerRestocked { mob: m.id });
        }
    }
}
