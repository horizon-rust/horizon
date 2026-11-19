/// With teams but no holder on one, no mob has a team (the answer the per-entity pass gave); once
/// the player joins a team, the pass still finds it.
#[test]
#[ignore = "daily"]
fn a_team_without_members_gives_the_mobs_none() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        assert!(game.mob_teams().mobs.is_empty() && game.mob_teams().player.is_none());
        game.score.board.add_team("x");
        let teams = game.mob_teams();
        assert!(teams.mobs.is_empty());
        assert!(teams.player.is_none());
        let name = game.acting_player_name().to_owned();
        game.score.board.add_to_team(&name, "x");
        let teams = game.mob_teams();
        assert_eq!(teams.player.map(|t| t.name.as_str()), Some("x"));
    });
}
