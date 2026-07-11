use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const HEIGHT_FORMAT_R16LE: &str = "r16le";
pub const COLOR_FORMAT_RGBA8: &str = "rgba8";
pub const WATER_MESH_FORMAT_WMESH1: &str = "wmesh1";
pub const WATER_FLOW_FORMAT_RG8: &str = "rg8";
pub const MANIFEST_FILE_NAME: &str = "manifest.toml";
pub const HEIGHT_NEAR_DIR: &str = "height/near";
pub const COLOR_NEAR_DIR: &str = "color/near";
pub const WATER_MESH_DIR: &str = "water/mesh";
pub const WATER_FLOW_DIR: &str = "water/flow";
pub const PROPS_CATALOG_DIR: &str = "props/catalog";
pub const PROPS_TILES_DIR: &str = "props/tiles";

#[derive(Clone, Debug, PartialEq)]
pub struct WaterManifest {
    pub source_width: u32,
    pub source_height: u32,
    pub tile_size_x: u32,
    pub tile_size_y: u32,
    pub mesh_format: String,
    pub mesh_path: String,
    pub flow_format: String,
    pub flow_path: String,
    pub ocean_raw_height: u32,
    pub ocean_height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldmapManifest {
    pub name: String,
    pub source_width: u32,
    pub source_height: u32,
    pub horizontal_scale: f32,
    pub height_scale: f32,
    pub tile_size: u32,
    pub tile_padding: u32,
    pub tile_count_x: u32,
    pub tile_count_y: u32,
    pub height_format: String,
    pub height_near_path: String,
    pub height_far_path: String,
    pub height_far_width: u32,
    pub height_far_height: u32,
    pub color_format: String,
    pub color_near_path: String,
    pub color_far_path: String,
    pub color_far_width: u32,
    pub color_far_height: u32,
    pub props_catalog_path: String,
    pub props_tiles_path: String,
    pub water: WaterManifest,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorldmapManifestToml {
    name: String,
    source_width: u32,
    source_height: u32,
    horizontal_scale: f32,
    height_scale: f32,
    tile_size: u32,
    tile_padding: u32,
    tile_count_x: u32,
    tile_count_y: u32,
    height_format: String,
    height_near_path: String,
    height_far_path: String,
    height_far_width: u32,
    height_far_height: u32,
    color_format: String,
    color_near_path: String,
    color_far_path: String,
    color_far_width: u32,
    color_far_height: u32,
    props_catalog_path: String,
    props_tiles_path: String,
    water_source_width: u32,
    water_source_height: u32,
    water_tile_size_x: u32,
    water_tile_size_y: u32,
    water_mesh_format: String,
    water_mesh_path: String,
    water_flow_format: String,
    water_flow_path: String,
    water_ocean_raw_height: u32,
    water_ocean_height: f32,
}

impl TryFrom<WorldmapManifestToml> for WorldmapManifest {
    type Error = String;

    fn try_from(toml: WorldmapManifestToml) -> Result<Self, Self::Error> {
        Self {
            name: toml.name,
            source_width: toml.source_width,
            source_height: toml.source_height,
            horizontal_scale: toml.horizontal_scale,
            height_scale: toml.height_scale,
            tile_size: toml.tile_size,
            tile_padding: toml.tile_padding,
            tile_count_x: toml.tile_count_x,
            tile_count_y: toml.tile_count_y,
            height_format: toml.height_format,
            height_near_path: toml.height_near_path,
            height_far_path: toml.height_far_path,
            height_far_width: toml.height_far_width,
            height_far_height: toml.height_far_height,
            color_format: toml.color_format,
            color_near_path: toml.color_near_path,
            color_far_path: toml.color_far_path,
            color_far_width: toml.color_far_width,
            color_far_height: toml.color_far_height,
            props_catalog_path: toml.props_catalog_path,
            props_tiles_path: toml.props_tiles_path,
            water: WaterManifest {
                source_width: toml.water_source_width,
                source_height: toml.water_source_height,
                tile_size_x: toml.water_tile_size_x,
                tile_size_y: toml.water_tile_size_y,
                mesh_format: toml.water_mesh_format,
                mesh_path: toml.water_mesh_path,
                flow_format: toml.water_flow_format,
                flow_path: toml.water_flow_path,
                ocean_raw_height: toml.water_ocean_raw_height,
                ocean_height: toml.water_ocean_height,
            },
        }
        .validate()
    }
}

impl From<&WorldmapManifest> for WorldmapManifestToml {
    fn from(manifest: &WorldmapManifest) -> Self {
        Self {
            name: manifest.name.clone(),
            source_width: manifest.source_width,
            source_height: manifest.source_height,
            horizontal_scale: manifest.horizontal_scale,
            height_scale: manifest.height_scale,
            tile_size: manifest.tile_size,
            tile_padding: manifest.tile_padding,
            tile_count_x: manifest.tile_count_x,
            tile_count_y: manifest.tile_count_y,
            height_format: manifest.height_format.clone(),
            height_near_path: manifest.height_near_path.clone(),
            height_far_path: manifest.height_far_path.clone(),
            height_far_width: manifest.height_far_width,
            height_far_height: manifest.height_far_height,
            color_format: manifest.color_format.clone(),
            color_near_path: manifest.color_near_path.clone(),
            color_far_path: manifest.color_far_path.clone(),
            color_far_width: manifest.color_far_width,
            color_far_height: manifest.color_far_height,
            props_catalog_path: manifest.props_catalog_path.clone(),
            props_tiles_path: manifest.props_tiles_path.clone(),
            water_source_width: manifest.water.source_width,
            water_source_height: manifest.water.source_height,
            water_tile_size_x: manifest.water.tile_size_x,
            water_tile_size_y: manifest.water.tile_size_y,
            water_mesh_format: manifest.water.mesh_format.clone(),
            water_mesh_path: manifest.water.mesh_path.clone(),
            water_flow_format: manifest.water.flow_format.clone(),
            water_flow_path: manifest.water.flow_path.clone(),
            water_ocean_raw_height: manifest.water.ocean_raw_height,
            water_ocean_height: manifest.water.ocean_height,
        }
    }
}

impl WorldmapManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Self::parse(&contents).map_err(|error| format!("{}: {error}", path.display()).into())
    }

    pub fn parse(contents: &str) -> Result<Self, String> {
        let manifest: WorldmapManifestToml =
            toml::from_str(contents).map_err(|error| error.to_string())?;
        manifest.try_into()
    }

    pub fn write_to(&self, path: impl AsRef<Path>) -> Result<(), Box<dyn Error>> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, self.to_toml())?;
        Ok(())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(&WorldmapManifestToml::from(self))
            .expect("WorldmapManifestToml contains only TOML-serializable fields")
    }

    pub fn validate(self) -> Result<Self, String> {
        validate_nonzero(self.source_width, "source_width")?;
        validate_nonzero(self.source_height, "source_height")?;
        validate_nonzero(self.tile_size, "tile_size")?;
        validate_nonzero(self.tile_count_x, "tile_count_x")?;
        validate_nonzero(self.tile_count_y, "tile_count_y")?;
        validate_nonzero(self.height_far_width, "height_far_width")?;
        validate_nonzero(self.height_far_height, "height_far_height")?;
        validate_nonzero(self.color_far_width, "color_far_width")?;
        validate_nonzero(self.color_far_height, "color_far_height")?;

        if !self.horizontal_scale.is_finite() || self.horizontal_scale <= 0.0 {
            return Err("`horizontal_scale` must be finite and greater than 0.0".to_owned());
        }
        if !self.height_scale.is_finite() || self.height_scale <= 0.0 {
            return Err("`height_scale` must be finite and greater than 0.0".to_owned());
        }
        if self.height_format != HEIGHT_FORMAT_R16LE {
            return Err(format!(
                "`height_format` must be `{HEIGHT_FORMAT_R16LE}`, got `{}`",
                self.height_format
            ));
        }
        if self.color_format != COLOR_FORMAT_RGBA8 {
            return Err(format!(
                "`color_format` must be `{COLOR_FORMAT_RGBA8}`, got `{}`",
                self.color_format
            ));
        }
        if self.height_near_path.is_empty()
            || self.height_far_path.is_empty()
            || self.color_near_path.is_empty()
            || self.color_far_path.is_empty()
            || self.props_catalog_path.is_empty()
            || self.props_tiles_path.is_empty()
        {
            return Err("manifest paths must not be empty".to_owned());
        }

        if self.tile_count_x.checked_mul(self.tile_size) != Some(self.source_width) {
            return Err("`tile_count_x * tile_size` must exactly match `source_width`".to_owned());
        }
        if self.tile_count_y.checked_mul(self.tile_size) != Some(self.source_height) {
            return Err("`tile_count_y * tile_size` must exactly match `source_height`".to_owned());
        }
        if self.height_far_width > self.source_width || self.height_far_height > self.source_height
        {
            return Err(
                "far height dimensions must be no larger than source dimensions".to_owned(),
            );
        }
        if self.color_far_width > self.source_width || self.color_far_height > self.source_height {
            return Err("far color dimensions must be no larger than source dimensions".to_owned());
        }

        self.stored_tile_size()?;
        self.near_tile_count()?;

        let water = &self.water;
        validate_nonzero(water.source_width, "water_source_width")?;
        validate_nonzero(water.source_height, "water_source_height")?;
        validate_nonzero(water.tile_size_x, "water_tile_size_x")?;
        validate_nonzero(water.tile_size_y, "water_tile_size_y")?;

        if water.mesh_format != WATER_MESH_FORMAT_WMESH1 {
            return Err(format!(
                "`water_mesh_format` must be `{WATER_MESH_FORMAT_WMESH1}`, got `{}`",
                water.mesh_format
            ));
        }
        if water.flow_format != WATER_FLOW_FORMAT_RG8 {
            return Err(format!(
                "`water_flow_format` must be `{WATER_FLOW_FORMAT_RG8}`, got `{}`",
                water.flow_format
            ));
        }
        if water.mesh_path.is_empty() || water.flow_path.is_empty() {
            return Err("water manifest paths must not be empty".to_owned());
        }
        if water.ocean_raw_height > u16::MAX as u32 {
            return Err("`water_ocean_raw_height` must fit in u16".to_owned());
        }
        if water.ocean_height < 0.0 || !water.ocean_height.is_finite() {
            return Err("`water_ocean_height` must be finite and non-negative".to_owned());
        }
        if water.source_width % self.tile_count_x != 0
            || water.source_height % self.tile_count_y != 0
        {
            return Err(
                "water dimensions must be evenly divisible by terrain tile counts".to_owned(),
            );
        }
        if water.source_width / self.tile_count_x != water.tile_size_x
            || water.source_height / self.tile_count_y != water.tile_size_y
        {
            return Err(
                "water tile sizes must match water dimensions divided by terrain tile counts"
                    .to_owned(),
            );
        }

        Ok(self)
    }

    pub fn stored_tile_size(&self) -> Result<u32, String> {
        self.tile_padding
            .checked_mul(2)
            .and_then(|padding| self.tile_size.checked_add(padding))
            .ok_or_else(|| "`tile_size + tile_padding * 2` overflows u32".to_owned())
    }

    pub fn near_tile_count(&self) -> Result<u32, String> {
        self.tile_count_x
            .checked_mul(self.tile_count_y)
            .ok_or_else(|| "`tile_count_x * tile_count_y` overflows u32".to_owned())
    }

    pub fn terrain_size(&self) -> [f32; 2] {
        [
            self.source_width as f32 * self.horizontal_scale,
            self.source_height as f32 * self.horizontal_scale,
        ]
    }

    pub fn tile_world_size(&self) -> f32 {
        self.tile_size as f32 * self.horizontal_scale
    }

    pub fn height_near_dir(&self, worldmap_dir: impl AsRef<Path>) -> PathBuf {
        worldmap_dir.as_ref().join(&self.height_near_path)
    }

    pub fn color_near_dir(&self, worldmap_dir: impl AsRef<Path>) -> PathBuf {
        worldmap_dir.as_ref().join(&self.color_near_path)
    }

    pub fn height_far_path(&self, worldmap_dir: impl AsRef<Path>) -> PathBuf {
        worldmap_dir.as_ref().join(&self.height_far_path)
    }

    pub fn color_far_path(&self, worldmap_dir: impl AsRef<Path>) -> PathBuf {
        worldmap_dir.as_ref().join(&self.color_far_path)
    }

    pub fn height_tile_path(
        &self,
        worldmap_dir: impl AsRef<Path>,
        tile_x: u32,
        tile_y: u32,
    ) -> PathBuf {
        self.height_near_dir(worldmap_dir)
            .join(height_tile_file_name(tile_x, tile_y))
    }

    pub fn color_tile_path(
        &self,
        worldmap_dir: impl AsRef<Path>,
        tile_x: u32,
        tile_y: u32,
    ) -> PathBuf {
        self.color_near_dir(worldmap_dir)
            .join(color_tile_file_name(tile_x, tile_y))
    }

    pub fn water_mesh_dir(&self, worldmap_dir: impl AsRef<Path>) -> PathBuf {
        worldmap_dir.as_ref().join(&self.water.mesh_path)
    }

    pub fn water_flow_dir(&self, worldmap_dir: impl AsRef<Path>) -> PathBuf {
        worldmap_dir.as_ref().join(&self.water.flow_path)
    }

    pub fn water_mesh_tile_path(
        &self,
        worldmap_dir: impl AsRef<Path>,
        tile_x: u32,
        tile_y: u32,
    ) -> PathBuf {
        self.water_mesh_dir(worldmap_dir)
            .join(water_mesh_tile_file_name(tile_x, tile_y))
    }

    pub fn water_flow_tile_path(
        &self,
        worldmap_dir: impl AsRef<Path>,
        tile_x: u32,
        tile_y: u32,
    ) -> PathBuf {
        self.water_flow_dir(worldmap_dir)
            .join(water_flow_tile_file_name(tile_x, tile_y))
    }

    pub fn props_catalog_dir(&self, worldmap_dir: impl AsRef<Path>) -> PathBuf {
        worldmap_dir.as_ref().join(&self.props_catalog_path)
    }

    pub fn props_tiles_dir(&self, worldmap_dir: impl AsRef<Path>) -> PathBuf {
        worldmap_dir.as_ref().join(&self.props_tiles_path)
    }

    pub fn props_tile_path(
        &self,
        worldmap_dir: impl AsRef<Path>,
        tile_x: u32,
        tile_y: u32,
    ) -> PathBuf {
        self.props_tiles_dir(worldmap_dir)
            .join(props_tile_file_name(tile_x, tile_y))
    }
}

pub fn manifest_dir(manifest_path: impl AsRef<Path>) -> Result<PathBuf, Box<dyn Error>> {
    let manifest_path = manifest_path.as_ref();
    manifest_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no parent directory", manifest_path.display()).into())
}

pub fn height_tile_file_name(tile_x: u32, tile_y: u32) -> String {
    format!("tile_{tile_x:04}_{tile_y:04}.r16")
}

pub fn color_tile_file_name(tile_x: u32, tile_y: u32) -> String {
    format!("tile_{tile_x:04}_{tile_y:04}.rgba")
}

pub fn water_mesh_tile_file_name(tile_x: u32, tile_y: u32) -> String {
    format!("tile_{tile_x:04}_{tile_y:04}.wmesh")
}

pub fn water_flow_tile_file_name(tile_x: u32, tile_y: u32) -> String {
    format!("tile_{tile_x:04}_{tile_y:04}.rg8")
}

pub fn props_tile_file_name(tile_x: u32, tile_y: u32) -> String {
    format!("tile_{tile_x:04}_{tile_y:04}.toml")
}

fn validate_nonzero(value: u32, key: &'static str) -> Result<(), String> {
    if value == 0 {
        Err(format!("`{key}` must be greater than zero"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> WorldmapManifest {
        WorldmapManifest {
            name: "continent".to_owned(),
            source_width: 4096,
            source_height: 4096,
            horizontal_scale: 0.5,
            height_scale: 535.5,
            tile_size: 1024,
            tile_padding: 2,
            tile_count_x: 4,
            tile_count_y: 4,
            height_format: HEIGHT_FORMAT_R16LE.to_owned(),
            height_near_path: HEIGHT_NEAR_DIR.to_owned(),
            height_far_path: "height/far/max_2048.r16".to_owned(),
            height_far_width: 2048,
            height_far_height: 2048,
            color_format: COLOR_FORMAT_RGBA8.to_owned(),
            color_near_path: COLOR_NEAR_DIR.to_owned(),
            color_far_path: "color/far/overview_4096.rgba".to_owned(),
            color_far_width: 4096,
            color_far_height: 4096,
            props_catalog_path: PROPS_CATALOG_DIR.to_owned(),
            props_tiles_path: PROPS_TILES_DIR.to_owned(),
            water: WaterManifest {
                source_width: 2048,
                source_height: 2048,
                tile_size_x: 512,
                tile_size_y: 512,
                mesh_format: WATER_MESH_FORMAT_WMESH1.to_owned(),
                mesh_path: WATER_MESH_DIR.to_owned(),
                flow_format: WATER_FLOW_FORMAT_RG8.to_owned(),
                flow_path: WATER_FLOW_DIR.to_owned(),
                ocean_raw_height: 24965,
                ocean_height: 203.939,
            },
        }
    }

    #[test]
    fn round_trips_manifest_toml() {
        let manifest = sample_manifest();
        let parsed = WorldmapManifest::parse(&manifest.to_toml()).unwrap();

        assert_eq!(parsed, manifest);
        assert_eq!(parsed.stored_tile_size().unwrap(), 1028);
        assert_eq!(parsed.near_tile_count().unwrap(), 16);
        assert_eq!(parsed.terrain_size(), [2048.0, 2048.0]);
    }

    #[test]
    fn rejects_mismatched_tile_count() {
        let mut manifest = sample_manifest();
        manifest.tile_count_x = 3;

        let error = manifest.validate().unwrap_err();
        assert!(error.contains("source_width"));
    }

    #[test]
    fn rejects_unknown_key() {
        let error = WorldmapManifest::parse("name = \"x\"\nwat = 1\n").unwrap_err();
        assert!(error.contains("wat"));
        assert!(error.contains("unknown field"));
    }

    #[test]
    fn round_trips_water_manifest_toml() {
        let manifest = sample_manifest();
        let parsed = WorldmapManifest::parse(&manifest.to_toml()).unwrap();

        assert_eq!(parsed, manifest);
        assert_eq!(
            parsed.water_mesh_tile_path("world", 1, 2),
            PathBuf::from("world/water/mesh/tile_0001_0002.wmesh")
        );
        assert_eq!(
            parsed.water_flow_tile_path("world", 1, 2),
            PathBuf::from("world/water/flow/tile_0001_0002.rg8")
        );
        assert_eq!(
            parsed.props_tile_path("world", 1, 2),
            PathBuf::from("world/props/tiles/tile_0001_0002.toml")
        );
    }

    #[test]
    fn rejects_missing_water_manifest_key() {
        let toml = sample_manifest()
            .to_toml()
            .lines()
            .filter(|line| !line.starts_with("water_source_width"))
            .collect::<Vec<_>>()
            .join("\n");

        let error = WorldmapManifest::parse(&toml).unwrap_err();

        assert!(error.contains("water_source_width"));
    }

    #[test]
    fn rejects_non_finite_manifest_values() {
        let toml = sample_manifest()
            .to_toml()
            .replace("horizontal_scale = 0.5", "horizontal_scale = nan");

        let error = WorldmapManifest::parse(&toml).unwrap_err();

        assert!(error.contains("horizontal_scale"));
        assert!(error.contains("finite"));
    }
}
