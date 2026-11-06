fn xaero_rs_world_primitives(
    game: &crate::game::Game<'_>,
    partial: f32,
    camera: [f64; 3],
    primitives: &mut horizon_client::world_primitives::WorldPrimitives,
) {
    crate::game::umbx_xaero::world_primitives(game, partial, camera, primitives);
}
