use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

pub const HEIGHT_FORMAT_R16LE: &str = "r16le";
pub const COLOR_FORMAT_RGBA8: &str = "rgba8";
pub const WATER_MESH_FORMAT_WMESH1: &str = "wmesh1";
pub const WATER_FLOW_FORMAT_RG8: &str = "rg8";
pub const MANIFEST_FILE_NAME: &str = "manifest.toml";
pub const HEIGHT_NEAR_DIR: &str = "height/near";
pub const COLOR_NEAR_DIR: &str = "color/near";
pub const WATER_MESH_DIR: &str = "water/mesh";
pub const WATER_FLOW_DIR: &str = "water/flow";

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
    pub water: WaterManifest,
}

impl WorldmapManifest {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Self::parse(&contents).map_err(|error| format!("{}: {error}", path.display()).into())
    }

    pub fn parse(contents: &str) -> Result<Self, String> {
        let mut builder = ManifestBuilder::default();
        let mut seen_keys = Vec::new();

        for (line_index, raw_line) in contents.lines().enumerate() {
            let line_number = line_index + 1;
            let line = raw_line
                .split_once('#')
                .map_or(raw_line, |(before_comment, _)| before_comment)
                .trim();

            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                return Err(format!(
                    "line {line_number}: tables are not supported; use flat `key = value` entries"
                ));
            }

            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {line_number}: expected `key = value`"))?;
            let key = key.trim();
            let value = value.trim();

            if key.is_empty() {
                return Err(format!("line {line_number}: manifest key is empty"));
            }
            if value.is_empty() {
                return Err(format!("line {line_number}: value for `{key}` is empty"));
            }
            if seen_keys.iter().any(|seen_key| seen_key == key) {
                return Err(format!(
                    "line {line_number}: duplicate manifest key `{key}`"
                ));
            }
            seen_keys.push(key.to_owned());

            match key {
                "name" => builder.name = Some(parse_manifest_string(key, value, line_number)?),
                "source_width" => {
                    builder.source_width = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "source_height" => {
                    builder.source_height = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "horizontal_scale" => {
                    builder.horizontal_scale = Some(parse_manifest_f32(key, value, line_number)?)
                }
                "height_scale" => {
                    builder.height_scale = Some(parse_manifest_f32(key, value, line_number)?)
                }
                "tile_size" => {
                    builder.tile_size = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "tile_padding" => {
                    builder.tile_padding = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "tile_count_x" => {
                    builder.tile_count_x = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "tile_count_y" => {
                    builder.tile_count_y = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "height_format" => {
                    builder.height_format = Some(parse_manifest_string(key, value, line_number)?)
                }
                "height_near_path" => {
                    builder.height_near_path = Some(parse_manifest_string(key, value, line_number)?)
                }
                "height_far_path" => {
                    builder.height_far_path = Some(parse_manifest_string(key, value, line_number)?)
                }
                "height_far_width" => {
                    builder.height_far_width = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "height_far_height" => {
                    builder.height_far_height = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "color_format" => {
                    builder.color_format = Some(parse_manifest_string(key, value, line_number)?)
                }
                "color_near_path" => {
                    builder.color_near_path = Some(parse_manifest_string(key, value, line_number)?)
                }
                "color_far_path" => {
                    builder.color_far_path = Some(parse_manifest_string(key, value, line_number)?)
                }
                "color_far_width" => {
                    builder.color_far_width = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "color_far_height" => {
                    builder.color_far_height = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "water_source_width" => {
                    builder.water_source_width = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "water_source_height" => {
                    builder.water_source_height = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "water_tile_size_x" => {
                    builder.water_tile_size_x = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "water_tile_size_y" => {
                    builder.water_tile_size_y = Some(parse_manifest_u32(key, value, line_number)?)
                }
                "water_mesh_format" => {
                    builder.water_mesh_format =
                        Some(parse_manifest_string(key, value, line_number)?)
                }
                "water_mesh_path" => {
                    builder.water_mesh_path = Some(parse_manifest_string(key, value, line_number)?)
                }
                "water_flow_format" => {
                    builder.water_flow_format =
                        Some(parse_manifest_string(key, value, line_number)?)
                }
                "water_flow_path" => {
                    builder.water_flow_path = Some(parse_manifest_string(key, value, line_number)?)
                }
                "water_ocean_raw_height" => {
                    builder.water_ocean_raw_height =
                        Some(parse_manifest_u32(key, value, line_number)?)
                }
                "water_ocean_height" => {
                    builder.water_ocean_height = Some(parse_manifest_f32(key, value, line_number)?)
                }
                _ => return Err(format!("line {line_number}: unknown manifest key `{key}`")),
            }
        }

        builder.build()
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
        let mut toml = format!(
            concat!(
                "name = \"{}\"\n",
                "source_width = {}\n",
                "source_height = {}\n",
                "horizontal_scale = {}\n",
                "height_scale = {}\n",
                "\n",
                "tile_size = {}\n",
                "tile_padding = {}\n",
                "tile_count_x = {}\n",
                "tile_count_y = {}\n",
                "\n",
                "height_format = \"{}\"\n",
                "height_near_path = \"{}\"\n",
                "height_far_path = \"{}\"\n",
                "height_far_width = {}\n",
                "height_far_height = {}\n",
                "\n",
                "color_format = \"{}\"\n",
                "color_near_path = \"{}\"\n",
                "color_far_path = \"{}\"\n",
                "color_far_width = {}\n",
                "color_far_height = {}\n"
            ),
            escape_toml_string(&self.name),
            self.source_width,
            self.source_height,
            format_f32(self.horizontal_scale),
            format_f32(self.height_scale),
            self.tile_size,
            self.tile_padding,
            self.tile_count_x,
            self.tile_count_y,
            escape_toml_string(&self.height_format),
            escape_toml_string(&self.height_near_path),
            escape_toml_string(&self.height_far_path),
            self.height_far_width,
            self.height_far_height,
            escape_toml_string(&self.color_format),
            escape_toml_string(&self.color_near_path),
            escape_toml_string(&self.color_far_path),
            self.color_far_width,
            self.color_far_height,
        );

        let water = &self.water;
        toml.push_str(&format!(
            concat!(
                "\n",
                "water_source_width = {}\n",
                "water_source_height = {}\n",
                "water_tile_size_x = {}\n",
                "water_tile_size_y = {}\n",
                "water_mesh_format = \"{}\"\n",
                "water_mesh_path = \"{}\"\n",
                "water_flow_format = \"{}\"\n",
                "water_flow_path = \"{}\"\n",
                "water_ocean_raw_height = {}\n",
                "water_ocean_height = {}\n"
            ),
            water.source_width,
            water.source_height,
            water.tile_size_x,
            water.tile_size_y,
            escape_toml_string(&water.mesh_format),
            escape_toml_string(&water.mesh_path),
            escape_toml_string(&water.flow_format),
            escape_toml_string(&water.flow_path),
            water.ocean_raw_height,
            format_f32(water.ocean_height),
        ));

        toml
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

        if self.horizontal_scale <= 0.0 {
            return Err("`horizontal_scale` must be greater than 0.0".to_owned());
        }
        if self.height_scale <= 0.0 {
            return Err("`height_scale` must be greater than 0.0".to_owned());
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
}

#[derive(Default)]
struct ManifestBuilder {
    name: Option<String>,
    source_width: Option<u32>,
    source_height: Option<u32>,
    horizontal_scale: Option<f32>,
    height_scale: Option<f32>,
    tile_size: Option<u32>,
    tile_padding: Option<u32>,
    tile_count_x: Option<u32>,
    tile_count_y: Option<u32>,
    height_format: Option<String>,
    height_near_path: Option<String>,
    height_far_path: Option<String>,
    height_far_width: Option<u32>,
    height_far_height: Option<u32>,
    color_format: Option<String>,
    color_near_path: Option<String>,
    color_far_path: Option<String>,
    color_far_width: Option<u32>,
    color_far_height: Option<u32>,
    water_source_width: Option<u32>,
    water_source_height: Option<u32>,
    water_tile_size_x: Option<u32>,
    water_tile_size_y: Option<u32>,
    water_mesh_format: Option<String>,
    water_mesh_path: Option<String>,
    water_flow_format: Option<String>,
    water_flow_path: Option<String>,
    water_ocean_raw_height: Option<u32>,
    water_ocean_height: Option<f32>,
}

impl ManifestBuilder {
    fn build(self) -> Result<WorldmapManifest, String> {
        WorldmapManifest {
            name: required(self.name, "name")?,
            source_width: required(self.source_width, "source_width")?,
            source_height: required(self.source_height, "source_height")?,
            horizontal_scale: required(self.horizontal_scale, "horizontal_scale")?,
            height_scale: required(self.height_scale, "height_scale")?,
            tile_size: required(self.tile_size, "tile_size")?,
            tile_padding: required(self.tile_padding, "tile_padding")?,
            tile_count_x: required(self.tile_count_x, "tile_count_x")?,
            tile_count_y: required(self.tile_count_y, "tile_count_y")?,
            height_format: required(self.height_format, "height_format")?,
            height_near_path: required(self.height_near_path, "height_near_path")?,
            height_far_path: required(self.height_far_path, "height_far_path")?,
            height_far_width: required(self.height_far_width, "height_far_width")?,
            height_far_height: required(self.height_far_height, "height_far_height")?,
            color_format: required(self.color_format, "color_format")?,
            color_near_path: required(self.color_near_path, "color_near_path")?,
            color_far_path: required(self.color_far_path, "color_far_path")?,
            color_far_width: required(self.color_far_width, "color_far_width")?,
            color_far_height: required(self.color_far_height, "color_far_height")?,
            water: WaterManifest {
                source_width: required(self.water_source_width, "water_source_width")?,
                source_height: required(self.water_source_height, "water_source_height")?,
                tile_size_x: required(self.water_tile_size_x, "water_tile_size_x")?,
                tile_size_y: required(self.water_tile_size_y, "water_tile_size_y")?,
                mesh_format: required(self.water_mesh_format, "water_mesh_format")?,
                mesh_path: required(self.water_mesh_path, "water_mesh_path")?,
                flow_format: required(self.water_flow_format, "water_flow_format")?,
                flow_path: required(self.water_flow_path, "water_flow_path")?,
                ocean_raw_height: required(self.water_ocean_raw_height, "water_ocean_raw_height")?,
                ocean_height: required(self.water_ocean_height, "water_ocean_height")?,
            },
        }
        .validate()
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

fn required<T>(value: Option<T>, key: &'static str) -> Result<T, String> {
    value.ok_or_else(|| format!("missing required manifest key `{key}`"))
}

fn validate_nonzero(value: u32, key: &'static str) -> Result<(), String> {
    if value == 0 {
        Err(format!("`{key}` must be greater than zero"))
    } else {
        Ok(())
    }
}

fn parse_manifest_u32(key: &str, value: &str, line_number: usize) -> Result<u32, String> {
    let normalized = value.replace('_', "");
    normalized
        .parse()
        .map_err(|_| format!("line {line_number}: `{key}` must be an unsigned integer"))
}

fn parse_manifest_f32(key: &str, value: &str, line_number: usize) -> Result<f32, String> {
    let normalized = value.replace('_', "");
    let parsed: f32 = normalized
        .parse()
        .map_err(|_| format!("line {line_number}: `{key}` must be a number"))?;

    if !parsed.is_finite() {
        return Err(format!("line {line_number}: `{key}` must be finite"));
    }

    Ok(parsed)
}

fn parse_manifest_string(key: &str, value: &str, line_number: usize) -> Result<String, String> {
    let value = value.trim();
    let unquoted = if value.len() >= 2 {
        let bytes = value.as_bytes();
        if bytes[0] == b'"' && bytes[value.len() - 1] == b'"' {
            &value[1..value.len() - 1]
        } else {
            value
        }
    } else {
        value
    };

    if unquoted.is_empty() {
        return Err(format!("line {line_number}: `{key}` must not be empty"));
    }

    Ok(unquoted.replace("\\\"", "\"").replace("\\\\", "\\"))
}

fn escape_toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn format_f32(value: f32) -> String {
    let mut formatted = value.to_string();
    if !formatted.contains('.') && !formatted.contains('e') && !formatted.contains('E') {
        formatted.push_str(".0");
    }
    formatted
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
        assert!(error.contains("unknown manifest key"));
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

        assert!(error.contains("missing required manifest key `water_source_width`"));
    }
}
