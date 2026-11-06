fn xaero_rs_screen_dragged(
    game: &crate::game::Game<'_>,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
    button: i32,
    delta: [f32; 2],
    canceled: &mut bool,
) -> core::ops::ControlFlow<()> {
    crate::game::umbx_xaero::screen_dragged(game, screen, button, delta, canceled);
    core::ops::ControlFlow::Continue(())
}
