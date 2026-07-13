use std::error::Error;

use sdl3::{gpu::Device, keyboard::Keycode};

use crate::{
    camera::Camera, camera_trace::CameraRecordingController, input::FrameInput,
    player::PlayerController, renderer::DebugVisualMode, terrain::TerrainMaps,
};

pub(crate) struct GameControls {
    player: PlayerController,
    camera_recording: CameraRecordingController,
    debug_visual_mode: DebugVisualMode,
    render_debug_visuals: bool,
}

impl GameControls {
    pub(crate) fn new(render_debug_visuals: bool) -> Self {
        Self {
            player: PlayerController::new(),
            camera_recording: CameraRecordingController::new(),
            debug_visual_mode: DebugVisualMode::None,
            render_debug_visuals,
        }
    }

    pub(crate) fn update(
        &mut self,
        input: &FrameInput,
        gpu: &Device,
        terrain_maps: &mut TerrainMaps,
        camera: &mut Camera,
        dt: f32,
    ) -> Result<(), Box<dyn Error>> {
        if input.key_pressed(Keycode::F3) && self.render_debug_visuals {
            self.debug_visual_mode = self.debug_visual_mode.next();
            println!(
                "debug visuals: {}\n{}",
                self.debug_visual_mode.label(),
                self.debug_visual_mode.color_key()
            );
        }

        if input.key_pressed(Keycode::F11) {
            self.camera_recording.toggle(camera)?;
        }

        if input.key_pressed(Keycode::G) {
            self.toggle_player_mode(gpu, terrain_maps, camera)?;
        }

        if self.player.is_gravity_mode() {
            terrain_maps.update_tile_cache_for_position(gpu, camera.x, camera.y)?;
        }

        self.player
            .tick(input, camera, terrain_maps.collision_height(), dt);

        Ok(())
    }

    pub(crate) fn debug_visual_mode(&self) -> DebugVisualMode {
        self.debug_visual_mode
    }

    pub(crate) fn update_after_submitted_frame(
        &mut self,
        camera: &Camera,
    ) -> Result<(), Box<dyn Error>> {
        self.camera_recording.update_after_submitted_frame(camera)
    }

    pub(crate) fn finish_recording(&mut self) -> Result<(), Box<dyn Error>> {
        self.camera_recording.finish_active()
    }

    fn toggle_player_mode(
        &mut self,
        gpu: &Device,
        terrain_maps: &mut TerrainMaps,
        camera: &mut Camera,
    ) -> Result<(), Box<dyn Error>> {
        if !self.player.is_gravity_mode() {
            terrain_maps.update_tile_cache_for_position(gpu, camera.x, camera.y)?;
        }

        let terrain_height = terrain_maps
            .collision_height()
            .height_at(camera.x, camera.y);
        self.player.toggle_mode(camera, terrain_height);
        Ok(())
    }
}
