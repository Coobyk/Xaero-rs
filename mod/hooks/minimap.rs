fn xaero_rs_minimap(game: &crate::game::Game<'_>, minimap: &mut Option<crate::game::native_hooks::NativeMinimap>) {
    crate::game::umbx_xaero::minimap(game, minimap);
}
