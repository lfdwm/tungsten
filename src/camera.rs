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
