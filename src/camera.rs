use glam::Vec2;

pub(crate) struct Camera {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) height: f32,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) vertical_fov: f32,
    pub(crate) max_distance: f32,
}

pub(crate) fn terrain_full_map_distance(terrain_size: [f32; 2]) -> f32 {
    Vec2::from_array(terrain_size).length()
}
