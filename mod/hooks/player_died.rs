fn xaero_rs_player_died(game: &mut crate::game::Game<'_>, player: u128, cause: Option<&str>) {
    crate::game::umbx_xaero::player_died(game, player, cause);
}
