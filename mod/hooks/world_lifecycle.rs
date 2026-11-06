fn xaero_rs_world_lifecycle(
    game: &mut crate::game::Game<'_>,
    context: &crate::mod_api::NativeLifecycleContext,
    phase: crate::mod_api::WorldPhase,
    error_text: &str,
) {
    crate::game::umbx_xaero::world_lifecycle(game, context, &phase, error_text);
}
