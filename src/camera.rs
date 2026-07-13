use glam::{Vec2, Vec3};

pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub height: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub vertical_fov: f32,
    pub max_distance: f32,
}

pub fn terrain_full_map_distance(terrain_size: [f32; 2]) -> f32 {
    Vec2::from_array(terrain_size).length()
}

#[derive(Clone, Copy, Debug)]
pub struct CameraRayBasis {
    pub forward: Vec3,
    pub right_scaled: Vec3,
    pub up_scaled: Vec3,
}

#[derive(Clone, Copy, Debug)]
pub struct CameraRay {
    pub origin: Vec3,
    pub direction: Vec3,
}

pub fn camera_ray_basis(camera: &Camera, width: u32, height: u32) -> CameraRayBasis {
    let (sin_yaw, cos_yaw) = camera.yaw.sin_cos();
    let (sin_pitch, cos_pitch) = camera.pitch.sin_cos();
    let forward_flat = Vec3::new(sin_yaw, 0.0, -cos_yaw);
    let right = Vec3::new(cos_yaw, 0.0, sin_yaw);
    let forward = (forward_flat * cos_pitch + Vec3::Y * sin_pitch).normalize();
    let up = (Vec3::Y * cos_pitch - forward_flat * sin_pitch).normalize();
    let aspect = width as f32 / (height as f32).max(1.0);
    let tan_half_fov = (camera.vertical_fov * 0.5).tan();

    CameraRayBasis {
        forward,
        right_scaled: right * aspect * tan_half_fov,
        up_scaled: up * tan_half_fov,
    }
}

pub fn camera_screen_ray(
    camera: &Camera,
    width: u32,
    height: u32,
    screen_pos: [f32; 2],
) -> CameraRay {
    let basis = camera_ray_basis(camera, width, height);
    let ndc_x = (screen_pos[0] / (width as f32).max(1.0)) * 2.0 - 1.0;
    let ndc_y = 1.0 - (screen_pos[1] / (height as f32).max(1.0)) * 2.0;
    let direction =
        (basis.forward + basis.right_scaled * ndc_x + basis.up_scaled * ndc_y).normalize_or_zero();

    CameraRay {
        origin: Vec3::new(camera.x, camera.height, camera.y),
        direction,
    }
}

pub fn camera_project_world_to_screen(
    camera: &Camera,
    width: u32,
    height: u32,
    world_pos: Vec3,
) -> Option<[f32; 2]> {
    let basis = camera_ray_basis(camera, width, height);
    let origin = Vec3::new(camera.x, camera.height, camera.y);
    let delta = world_pos - origin;
    let view_depth = delta.dot(basis.forward);
    if view_depth <= 0.001 {
        return None;
    }

    let right_len_sq = basis.right_scaled.length_squared().max(0.0001);
    let up_len_sq = basis.up_scaled.length_squared().max(0.0001);
    let clip_x = delta.dot(basis.right_scaled) / (view_depth * right_len_sq);
    let clip_y = delta.dot(basis.up_scaled) / (view_depth * up_len_sq);

    Some([
        (clip_x + 1.0) * 0.5 * width as f32,
        (1.0 - clip_y) * 0.5 * height as f32,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_screen_ray_points_back_to_screen() {
        let camera = Camera {
            x: 4.0,
            y: 8.0,
            height: 20.0,
            yaw: 0.4,
            pitch: -0.2,
            vertical_fov: 1.05,
            max_distance: 1000.0,
        };
        let screen_pos = [830.0, 290.0];
        let ray = camera_screen_ray(&camera, 1280, 720, screen_pos);
        let world_pos = ray.origin + ray.direction * 150.0;

        let projected = camera_project_world_to_screen(&camera, 1280, 720, world_pos).unwrap();

        assert!((projected[0] - screen_pos[0]).abs() < 0.01);
        assert!((projected[1] - screen_pos[1]).abs() < 0.01);
    }
}
