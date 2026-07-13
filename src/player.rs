use glam::Vec2;
use sdl3::keyboard::{Keycode, Scancode};

use crate::{camera::Camera, input::FrameInput, terrain::HeightField};

const PLAYER_EYE_HEIGHT: f32 = 1.0;
const PLAYER_MOVE_SPEED: f32 = 5.0;
const PLAYER_MIN_EYE_HEIGHT: f32 = 1.0;
const PLAYER_MAX_EYE_HEIGHT: f32 = 120.0;
const PLAYER_EYE_HEIGHT_SCROLL_STEP: f32 = 0.5;
const PLAYER_MIN_MOVE_SPEED: f32 = 5.0;
const PLAYER_MAX_MOVE_SPEED: f32 = 500.0;
const PLAYER_MOVE_SPEED_SCROLL_STEP: f32 = 1.0;
const PLAYER_GRAVITY: f32 = 240.0;
const PLAYER_JUMP_SPEED: f32 = 105.0;
const PLAYER_MAX_FALL_SPEED: f32 = 260.0;
const PLAYER_GROUND_SNAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    Freecam,
    Gravity,
}

struct PlayerPhysics {
    vertical_velocity: f32,
    on_ground: bool,
    eye_height: f32,
    move_speed: f32,
}

impl PlayerPhysics {
    fn new() -> Self {
        Self {
            vertical_velocity: 0.0,
            on_ground: false,
            eye_height: PLAYER_EYE_HEIGHT,
            move_speed: PLAYER_MOVE_SPEED,
        }
    }
}

pub struct PlayerController {
    mode: CameraMode,
    physics: PlayerPhysics,
}

impl PlayerController {
    pub fn new() -> Self {
        Self {
            mode: CameraMode::Freecam,
            physics: PlayerPhysics::new(),
        }
    }

    pub fn mode(&self) -> CameraMode {
        self.mode
    }

    pub fn is_gravity_mode(&self) -> bool {
        self.mode == CameraMode::Gravity
    }

    pub fn toggle_mode(&mut self, camera: &mut Camera, terrain_height: f32) {
        self.mode = match self.mode {
            CameraMode::Freecam => {
                enable_gravity_mode(camera, &mut self.physics, terrain_height);
                CameraMode::Gravity
            }
            CameraMode::Gravity => CameraMode::Freecam,
        };
    }

    pub fn tick(
        &mut self,
        input: &FrameInput,
        camera: &mut Camera,
        collision_height: &HeightField,
        dt: f32,
    ) {
        if self.mode == CameraMode::Gravity && input.wheel_delta() != 0.0 {
            let adjust_move_speed =
                input.scancode_held(Scancode::LShift) || input.scancode_held(Scancode::RShift);
            apply_gravity_wheel_adjustment(
                camera,
                &mut self.physics,
                collision_height,
                input.wheel_delta(),
                adjust_move_speed,
            );
        }

        match self.mode {
            CameraMode::Freecam => update_freecam(input, camera, dt),
            CameraMode::Gravity => update_gravity_camera(
                input,
                camera,
                &mut self.physics,
                collision_height,
                dt,
                input.key_pressed(Keycode::Space),
            ),
        }
    }
}

fn update_camera_look(input: &FrameInput, camera: &mut Camera, dt: f32) {
    let turn_speed = 1.85;
    let pitch_speed = 1.35;
    let mouse_sensitivity = 0.0024;
    let mouse_delta = input.mouse_delta();

    camera.yaw += mouse_delta[0] * mouse_sensitivity;
    camera.pitch -= mouse_delta[1] * mouse_sensitivity;

    if input.scancode_held(Scancode::Q) || input.scancode_held(Scancode::Left) {
        camera.yaw -= turn_speed * dt;
    }
    if input.scancode_held(Scancode::E) || input.scancode_held(Scancode::Right) {
        camera.yaw += turn_speed * dt;
    }
    if input.scancode_held(Scancode::Up) {
        camera.pitch += pitch_speed * dt;
    }
    if input.scancode_held(Scancode::Down) {
        camera.pitch -= pitch_speed * dt;
    }

    camera.pitch = camera.pitch.clamp(-1.45, 1.45);
}

fn update_freecam(input: &FrameInput, camera: &mut Camera, dt: f32) {
    update_camera_look(input, camera, dt);

    let move_speed = 135.0;
    let height_speed = 80.0;

    let (forward, right) = horizontal_camera_axes(camera.yaw);
    let mut movement = Vec2::ZERO;

    if input.scancode_held(Scancode::W) {
        movement += forward;
    }
    if input.scancode_held(Scancode::S) {
        movement -= forward;
    }
    if input.scancode_held(Scancode::D) {
        movement += right;
    }
    if input.scancode_held(Scancode::A) {
        movement -= right;
    }

    if movement.length_squared() > 0.0 {
        let movement = movement.normalize() * move_speed * dt;
        camera.x += movement.x;
        camera.y += movement.y;
    }

    if input.scancode_held(Scancode::R) {
        camera.height += height_speed * dt;
    }
    if input.scancode_held(Scancode::F) {
        camera.height -= height_speed * dt;
    }

    camera.height = camera.height.clamp(20.0, 520.0);
    camera.vertical_fov = camera.vertical_fov.clamp(0.5, 1.4);
    camera.max_distance = camera.max_distance.max(120.0);
}

fn enable_gravity_mode(camera: &mut Camera, physics: &mut PlayerPhysics, terrain_height: f32) {
    let ground_height = terrain_height + physics.eye_height;
    if camera.height < ground_height {
        camera.height = ground_height;
    }
    physics.vertical_velocity = 0.0;
    physics.on_ground = camera.height <= ground_height + PLAYER_GROUND_SNAP;
}

fn update_gravity_camera(
    input: &FrameInput,
    camera: &mut Camera,
    physics: &mut PlayerPhysics,
    collision_height: &HeightField,
    dt: f32,
    jump_requested: bool,
) {
    update_camera_look(input, camera, dt);
    update_player_horizontal_movement(input, camera, collision_height, physics.move_speed, dt);

    let ground_height = player_ground_height(camera, physics, collision_height);
    if physics.on_ground && !jump_requested {
        if camera.height < ground_height || camera.height - ground_height <= PLAYER_GROUND_SNAP {
            camera.height = ground_height;
            physics.vertical_velocity = 0.0;
            physics.on_ground = true;
        } else {
            physics.on_ground = false;
        }
    }

    if jump_requested && physics.on_ground {
        physics.vertical_velocity = PLAYER_JUMP_SPEED;
        physics.on_ground = false;
    }

    if !physics.on_ground {
        physics.vertical_velocity =
            (physics.vertical_velocity - PLAYER_GRAVITY * dt).max(-PLAYER_MAX_FALL_SPEED);
        camera.height += physics.vertical_velocity * dt;
    }

    collide_player_with_terrain(camera, physics, collision_height);
    camera.vertical_fov = camera.vertical_fov.clamp(0.5, 1.4);
    camera.max_distance = camera.max_distance.max(120.0);
}

fn apply_gravity_wheel_adjustment(
    camera: &mut Camera,
    physics: &mut PlayerPhysics,
    collision_height: &HeightField,
    wheel_delta: f32,
    adjust_move_speed: bool,
) {
    let changed = if adjust_move_speed {
        let previous = physics.move_speed;
        physics.move_speed = (physics.move_speed + wheel_delta * PLAYER_MOVE_SPEED_SCROLL_STEP)
            .clamp(PLAYER_MIN_MOVE_SPEED, PLAYER_MAX_MOVE_SPEED);
        physics.move_speed != previous
    } else {
        let previous = physics.eye_height;
        physics.eye_height = (physics.eye_height + wheel_delta * PLAYER_EYE_HEIGHT_SCROLL_STEP)
            .clamp(PLAYER_MIN_EYE_HEIGHT, PLAYER_MAX_EYE_HEIGHT);
        let delta = physics.eye_height - previous;
        if delta != 0.0 {
            camera.height += delta;
            let ground_height = player_ground_height(camera, physics, collision_height);
            if camera.height < ground_height || physics.on_ground {
                camera.height = ground_height;
                physics.vertical_velocity = 0.0;
                physics.on_ground = true;
            }
        }
        physics.eye_height != previous
    };

    if changed {
        println!(
            "gravity camera height: {:.1}, movement speed: {:.1}",
            physics.eye_height, physics.move_speed
        );
    }
}

fn update_player_horizontal_movement(
    input: &FrameInput,
    camera: &mut Camera,
    collision_height: &HeightField,
    move_speed: f32,
    dt: f32,
) {
    let (forward, right) = horizontal_camera_axes(camera.yaw);
    let mut movement = Vec2::ZERO;

    if input.scancode_held(Scancode::W) {
        movement += forward;
    }
    if input.scancode_held(Scancode::S) {
        movement -= forward;
    }
    if input.scancode_held(Scancode::D) {
        movement += right;
    }
    if input.scancode_held(Scancode::A) {
        movement -= right;
    }

    if movement.length_squared() > 0.0 {
        let movement = movement.normalize() * move_speed * dt;
        camera.x += movement.x;
        camera.y += movement.y;
        camera.x = camera.x.clamp(0.0, collision_height.terrain_size[0]);
        camera.y = camera.y.clamp(0.0, collision_height.terrain_size[1]);
    }
}

fn horizontal_camera_axes(yaw: f32) -> (Vec2, Vec2) {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    (Vec2::new(sin_yaw, -cos_yaw), Vec2::new(cos_yaw, sin_yaw))
}

fn collide_player_with_terrain(
    camera: &mut Camera,
    physics: &mut PlayerPhysics,
    collision_height: &HeightField,
) {
    let ground_height = player_ground_height(camera, physics, collision_height);
    if camera.height <= ground_height {
        camera.height = ground_height;
        if physics.vertical_velocity < 0.0 {
            physics.vertical_velocity = 0.0;
        }
        physics.on_ground = true;
    } else {
        physics.on_ground = false;
    }
}

fn player_ground_height(
    camera: &Camera,
    physics: &PlayerPhysics,
    collision_height: &HeightField,
) -> f32 {
    collision_height.height_at(camera.x, camera.y) + physics.eye_height
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            x: 10.0,
            y: 20.0,
            height: 30.0,
            yaw: 0.0,
            pitch: 0.0,
            vertical_fov: 1.05,
            max_distance: 200.0,
        }
    }

    fn assert_f32_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be near {expected}"
        );
    }

    #[test]
    fn toggles_between_freecam_and_gravity_modes() {
        let mut controller = PlayerController::new();
        let mut camera = camera();

        assert_eq!(controller.mode(), CameraMode::Freecam);
        controller.toggle_mode(&mut camera, 40.0);
        assert_eq!(controller.mode(), CameraMode::Gravity);
        assert_f32_near(camera.height, 41.0);

        controller.toggle_mode(&mut camera, 40.0);
        assert_eq!(controller.mode(), CameraMode::Freecam);
    }

    #[test]
    fn freecam_moves_forward_from_held_input() {
        let input = FrameInput::for_test(&[], &[Scancode::W], [0.0, 0.0], 0.0, false);
        let mut camera = camera();

        update_freecam(&input, &mut camera, 1.0);

        assert_f32_near(camera.x, 10.0);
        assert_f32_near(camera.y, -115.0);
    }
}
