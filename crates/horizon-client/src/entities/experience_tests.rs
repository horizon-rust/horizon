use super::*;

#[test]
fn death_reward_uses_levels_and_caps_the_recoverable_points() {
    for (level, reward) in [(0, 0), (1, 7), (14, 98), (15, 100), (30, 100)] {
        let xp = Experience {
            level,
            progress: 0.75,
            total: 10_000,
            score: 3,
        };
        assert_eq!(xp.death_reward(false), reward);
        assert_eq!(xp.death_reward(true), 0);
    }
}

#[test]
fn respawn_clears_all_xp_unless_it_is_kept() {
    let mut xp = Experience::default();
    xp.give(155);
    let before = (xp.level, xp.progress, xp.total, xp.score);
    xp.respawn(true);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), before);
    xp.respawn(false);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), (0, 0.0, 0, 0));
    xp.give(7);
    assert_eq!((xp.level, xp.progress, xp.total, xp.score), (1, 0.0, 7, 7));
}
