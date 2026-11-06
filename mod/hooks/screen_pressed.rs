fn xaero_rs_screen_pressed(
    game: &crate::game::Game<'_>,
    screen: &crate::mod_api::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) -> core::ops::ControlFlow<()> {
    crate::game::umbx_xaero::screen_pressed(game, screen, button, canceled);
    core::ops::ControlFlow::Continue(())
}
