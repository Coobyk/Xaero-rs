fn xaero_rs_screen_released(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    button: i32,
    canceled: &mut bool,
) -> core::ops::ControlFlow<()> {
    crate::game::umbx_xaero::screen_released(game, screen, button, canceled);
    core::ops::ControlFlow::Continue(())
}
