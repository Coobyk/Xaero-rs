fn xaero_rs_minimap(game: &crate::game::Game<'_>, minimap: &mut Option<crate::mod_api::NativeMinimap>) {
    crate::game::umbx_xaero::minimap(game, minimap);
}
