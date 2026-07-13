use std::{error::Error, fs, io, path::PathBuf};

use sdl3::gpu::PresentMode;
use serde::Deserialize;

pub const CONFIG_PATH: &str = "config.toml";
const DEFAULT_WORLDMAP_PATH: &str = "assets/worldmaps/continent/manifest.toml";
const DEFAULT_START_X: f32 = 250.0;
const DEFAULT_START_Y: f32 = 330.0;
const DEFAULT_START_HEIGHT: f32 = 150.0;
const DEFAULT_TILE_CACHE_RADIUS: u32 = 1;
const MAX_TILE_CACHE_RADIUS: u32 = 2;
const DEFAULT_RAY_ITERATION_COUNT: u32 = 700;
const MAX_RAY_ITERATION_COUNT: u32 = 4096;
const DEFAULT_NEAR_DDA_DISTANCE: f32 = 512.0;
const DEFAULT_NEAR_DDA_MAX_STEPS: u32 = 1024;
const MAX_NEAR_DDA_MAX_STEPS: u32 = 4096;
const DEFAULT_HEIGHT_LOD_BLEND_START: f32 = 125.0;
const DEFAULT_HEIGHT_LOD_BLEND_END: f32 = 300.0;
const DEFAULT_NORMAL_DETAIL_BLEND_START: f32 = 500.0;
const DEFAULT_NORMAL_DETAIL_BLEND_END: f32 = 1000.0;
const DEFAULT_PERFORMANCE_RENDER_SCALE: f32 = 0.5;
const DEFAULT_PRESENT_MODE: AppPresentMode = AppPresentMode::Vsync;
const DEFAULT_MAX_FRAMERATE: f32 = 0.0;
const DEFAULT_RENDER_DEBUG_VISUALS: bool = false;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub worldmap: PathBuf,
    pub tile_cache_radius: u32,
    pub ray_iteration_count: u32,
    pub performance_render_scale: f32,
    pub present_mode: AppPresentMode,
    pub max_framerate: f32,
    pub render_debug_visuals: bool,
    pub near_dda_distance: f32,
    pub near_dda_max_steps: u32,
    pub start_x: f32,
    pub start_y: f32,
    pub start_height: f32,
    pub normal_detail_blend_start: f32,
    pub normal_detail_blend_end: f32,
    pub height_lod_blend_start: f32,
    pub height_lod_blend_end: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum AppPresentMode {
    #[serde(rename = "vsync", alias = "v-sync")]
    Vsync,
    #[serde(rename = "immediate")]
    Immediate,
    #[serde(rename = "mailbox")]
    Mailbox,
}

impl AppPresentMode {
    pub fn to_sdl(self) -> PresentMode {
        match self {
            Self::Vsync => PresentMode::Vsync,
            Self::Immediate => PresentMode::Immediate,
            Self::Mailbox => PresentMode::Mailbox,
        }
    }

    pub fn as_config_value(self) -> &'static str {
        match self {
            Self::Vsync => "vsync",
            Self::Immediate => "immediate",
            Self::Mailbox => "mailbox",
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            worldmap: PathBuf::from(DEFAULT_WORLDMAP_PATH),
            tile_cache_radius: DEFAULT_TILE_CACHE_RADIUS,
            ray_iteration_count: DEFAULT_RAY_ITERATION_COUNT,
            performance_render_scale: DEFAULT_PERFORMANCE_RENDER_SCALE,
            present_mode: DEFAULT_PRESENT_MODE,
            max_framerate: DEFAULT_MAX_FRAMERATE,
            render_debug_visuals: DEFAULT_RENDER_DEBUG_VISUALS,
            near_dda_distance: DEFAULT_NEAR_DDA_DISTANCE,
            near_dda_max_steps: DEFAULT_NEAR_DDA_MAX_STEPS,
            start_x: DEFAULT_START_X,
            start_y: DEFAULT_START_Y,
            start_height: DEFAULT_START_HEIGHT,
            normal_detail_blend_start: DEFAULT_NORMAL_DETAIL_BLEND_START,
            normal_detail_blend_end: DEFAULT_NORMAL_DETAIL_BLEND_END,
            height_lod_blend_start: DEFAULT_HEIGHT_LOD_BLEND_START,
            height_lod_blend_end: DEFAULT_HEIGHT_LOD_BLEND_END,
        }
    }
}

impl AppConfig {
    pub fn load(path: &str) -> Result<Self, Box<dyn Error>> {
        match fs::read_to_string(path) {
            Ok(contents) => {
                Self::parse(&contents).map_err(|error| format!("{path}: {error}").into())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("failed to read {path}: {error}").into()),
        }
    }

    fn parse(contents: &str) -> Result<Self, String> {
        let config: Self = toml::from_str(contents).map_err(|error| error.to_string())?;
        config.validate()
    }

    fn validate(self) -> Result<Self, String> {
        validate_finite_values(&[
            ("performance_render_scale", self.performance_render_scale),
            ("max_framerate", self.max_framerate),
            ("near_dda_distance", self.near_dda_distance),
            ("start_x", self.start_x),
            ("start_y", self.start_y),
            ("start_height", self.start_height),
            ("normal_detail_blend_start", self.normal_detail_blend_start),
            ("normal_detail_blend_end", self.normal_detail_blend_end),
            ("height_lod_blend_start", self.height_lod_blend_start),
            ("height_lod_blend_end", self.height_lod_blend_end),
        ])?;
        if self.worldmap.as_os_str().is_empty() {
            return Err("`worldmap` must not be empty".to_owned());
        }
        if self.tile_cache_radius == 0 || self.tile_cache_radius > MAX_TILE_CACHE_RADIUS {
            return Err(format!(
                "`tile_cache_radius` must be between 1 and {MAX_TILE_CACHE_RADIUS}"
            ));
        }
        if !(1..=MAX_RAY_ITERATION_COUNT).contains(&self.ray_iteration_count) {
            return Err(format!(
                "`ray_iteration_count` must be between 1 and {MAX_RAY_ITERATION_COUNT}"
            ));
        }
        if !(self.performance_render_scale > 0.0 && self.performance_render_scale <= 1.0) {
            return Err(
                "`performance_render_scale` must be greater than 0.0 and no more than 1.0"
                    .to_owned(),
            );
        }
        if self.max_framerate < 0.0 {
            return Err("`max_framerate` must be non-negative; use 0.0 for unlimited".to_owned());
        }
        if self.near_dda_distance <= 0.0 {
            return Err("`near_dda_distance` must be greater than 0.0".to_owned());
        }
        if !(1..=MAX_NEAR_DDA_MAX_STEPS).contains(&self.near_dda_max_steps) {
            return Err(format!(
                "`near_dda_max_steps` must be between 1 and {MAX_NEAR_DDA_MAX_STEPS}"
            ));
        }
        if self.start_x < 0.0 || self.start_y < 0.0 || self.start_height < 0.0 {
            return Err("`start_x`, `start_y`, and `start_height` must be non-negative".to_owned());
        }
        validate_blend_range(
            "height_lod",
            self.height_lod_blend_start,
            self.height_lod_blend_end,
        )?;
        validate_blend_range(
            "normal_detail",
            self.normal_detail_blend_start,
            self.normal_detail_blend_end,
        )?;

        Ok(self)
    }
}

fn validate_finite_values(values: &[(&str, f32)]) -> Result<(), String> {
    for (key, value) in values {
        if !value.is_finite() {
            return Err(format!("`{key}` must be finite"));
        }
    }

    Ok(())
}

fn validate_blend_range(name: &str, start: f32, end: f32) -> Result<(), String> {
    if start < 0.0 || end < 0.0 {
        return Err(format!(
            "`{name}_blend_start` and `{name}_blend_end` must be non-negative"
        ));
    }
    if start >= end {
        return Err(format!(
            "`{name}_blend_start` must be less than `{name}_blend_end`"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_overrides_and_keeps_defaults() {
        let config = AppConfig::parse(
            r#"
            worldmap = "assets/worldmaps/test/manifest.toml"
            tile_cache_radius = 1
            ray_iteration_count = 200
            performance_render_scale = 0.4
            present_mode = "mailbox"
            max_framerate = 120.0
            render_debug_visuals = true
            near_dda_distance = 96.0
            near_dda_max_steps = 128
            start_x = 123.0
            start_y = 456.0
            start_height = 78.0
            height_lod_blend_start = 175.0
            # normal detail values intentionally omitted
            "#,
        )
        .unwrap();

        assert_eq!(
            config.worldmap,
            PathBuf::from("assets/worldmaps/test/manifest.toml")
        );
        assert_eq!(config.tile_cache_radius, 1);
        assert_eq!(config.ray_iteration_count, 200);
        assert_eq!(config.performance_render_scale, 0.4);
        assert_eq!(config.present_mode, AppPresentMode::Mailbox);
        assert_eq!(config.max_framerate, 120.0);
        assert!(config.render_debug_visuals);
        assert_eq!(config.near_dda_distance, 96.0);
        assert_eq!(config.near_dda_max_steps, 128);
        assert_eq!(config.start_x, 123.0);
        assert_eq!(config.start_y, 456.0);
        assert_eq!(config.start_height, 78.0);
        assert_eq!(config.height_lod_blend_start, 175.0);
        assert_eq!(
            config.normal_detail_blend_start,
            DEFAULT_NORMAL_DETAIL_BLEND_START
        );
        assert_eq!(
            config.normal_detail_blend_end,
            DEFAULT_NORMAL_DETAIL_BLEND_END
        );
    }

    #[test]
    fn parses_present_mode_values() {
        assert_eq!(
            AppConfig::parse("present_mode = \"vsync\"")
                .unwrap()
                .present_mode,
            AppPresentMode::Vsync
        );
        assert_eq!(
            AppConfig::parse("present_mode = \"v-sync\"")
                .unwrap()
                .present_mode,
            AppPresentMode::Vsync
        );
        assert_eq!(
            AppConfig::parse("present_mode = 'immediate'")
                .unwrap()
                .present_mode,
            AppPresentMode::Immediate
        );
        assert_eq!(
            AppConfig::parse("present_mode = \"mailbox\"")
                .unwrap()
                .present_mode,
            AppPresentMode::Mailbox
        );
    }

    #[test]
    fn rejects_unquoted_config_strings() {
        let error = AppConfig::parse("present_mode = vsync").unwrap_err();

        assert!(error.contains("invalid") || error.contains("expected"));
    }

    #[test]
    fn rejects_invalid_config_ranges() {
        let error = AppConfig::parse(
            r#"
            ray_iteration_count = 0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("ray_iteration_count"));

        let error = AppConfig::parse(
            r#"
            height_lod_blend_start = 300.0
            height_lod_blend_end = 125.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("height_lod_blend_start"));

        let error = AppConfig::parse(
            r#"
            start_height = -1.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("start_height"));

        let error = AppConfig::parse(
            r#"
            present_mode = "triple"
            "#,
        )
        .unwrap_err();
        assert!(error.contains("triple"));

        let error = AppConfig::parse(
            r#"
            near_dda_distance = 0.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("near_dda_distance"));

        let error = AppConfig::parse(
            r#"
            max_framerate = -1.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("max_framerate"));

        let error = AppConfig::parse(
            r#"
            near_dda_max_steps = 0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("near_dda_max_steps"));

        let error = AppConfig::parse(
            r#"
            render_debug_visuals = maybe
            "#,
        )
        .unwrap_err();
        assert!(error.contains("invalid") || error.contains("expected"));

        let error = AppConfig::parse(
            r#"
            unknown_key = true
            "#,
        )
        .unwrap_err();
        assert!(error.contains("unknown_key"));
        assert!(error.contains("unknown field"));

        let error = AppConfig::parse(
            r#"
            tile_cache_radius = 99
            "#,
        )
        .unwrap_err();
        assert!(error.contains("tile_cache_radius"));

        let error = AppConfig::parse(
            r#"
            worldmap = ""
            "#,
        )
        .unwrap_err();
        assert!(error.contains("worldmap"));
    }

    #[test]
    fn rejects_non_finite_config_values() {
        let error = AppConfig::parse(
            r#"
            performance_render_scale = nan
            "#,
        )
        .unwrap_err();

        assert!(error.contains("performance_render_scale"));
        assert!(error.contains("finite"));
    }
}
