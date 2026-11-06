fn xaero_rs_screen_char(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    c: char,
    canceled: &mut bool,
) -> core::ops::ControlFlow<()> {
    crate::game::umbx_xaero::screen_char(game, screen, c, canceled);
    core::ops::ControlFlow::Continue(())
}
