fn xaero_rs_screen_render_post(
    game: &crate::game::Game<'_>,
    batch: &mut crate::hud::HudBatch,
    screen: &crate::mod_api::NativeScreenView<'_>,
) {
    crate::game::umbx_xaero::screen_render_post(game, batch, screen);
}
