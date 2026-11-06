fn xaero_rs_screen_key(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    key: &str,
    repeat: bool,
    canceled: &mut bool,
) -> core::ops::ControlFlow<()> {
    crate::game::umbx_xaero::screen_key(game, screen, key, repeat, canceled);
    core::ops::ControlFlow::Continue(())
}
