fn xaero_rs_screen_scrolled(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    scroll: [f64; 2],
    canceled: &mut bool,
) -> core::ops::ControlFlow<()> {
    crate::game::umbx_xaero::screen_scrolled(game, screen, scroll, canceled);
    core::ops::ControlFlow::Continue(())
}
