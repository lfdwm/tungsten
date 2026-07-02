#version 450

layout(set = 2, binding = 0) uniform sampler2D color_near_map;
layout(set = 2, binding = 1) uniform sampler2D height_near_atlas;
layout(set = 2, binding = 2) uniform sampler2D height_far_map;
layout(set = 2, binding = 3) uniform sampler2D color_far_map;

layout(set = 3, binding = 0) uniform Params {
    vec4 camera;
    vec4 render;
    vec4 terrain;
    vec4 height_maps;
    vec4 source_maps;
    vec4 tile_info;
    vec4 tile_window;
    vec4 lod_distances;
    vec4 raymarch;
    vec4 near_dda;
    vec4 debug;
    vec4 ray_forward;
    vec4 ray_right;
    vec4 ray_up;
};

layout(location = 0) in vec2 frag_uv;
layout(location = 0) out vec4 out_color;

const float FAR_TERRAIN_LIGHT = 0.84;
const int MAX_RAY_ITERATIONS = 4096;
const int HIT_REFINE_NEAR_STEPS = 6;
const int HIT_REFINE_MID_STEPS = 5;
const int HIT_REFINE_FAR_STEPS = 4;
const float LARGE_STEP_PROBE_MIN_STEP = 2.0;
const float LARGE_STEP_PROBE_CELL_FACTOR = 0.75;
const float CLOSE_TERRAIN_STEP_CELL_FACTOR = 0.45;
const float CLOSE_TERRAIN_STEP_BLEND_START = 2.0;
const float CLOSE_TERRAIN_STEP_BLEND_END = 12.0;
const int MAX_NEAR_DDA_STEPS = 4096;
const float NEAR_DDA_AXIS_EPSILON = 0.00001;
const float NEAR_DDA_T_EPSILON = 0.00001;
const int BACKDROP_MAX_STEPS = 256;
const float BACKDROP_MIN_HORIZONTAL_STEP = 0.5;
const float BACKDROP_MAX_HORIZONTAL_STEP = 8.0;
const float BACKDROP_HIT_BIAS = 2.0;
const float BACKDROP_START_BIAS = 2.0;
const float BACKDROP_HEIGHT_OFFSET_FRACTION = 0.005;
const int DEBUG_NONE = 0;
const int DEBUG_HEIGHT_SOURCES = 1;
const int DEBUG_HIT_METHODS = 2;
const int DEBUG_NORMAL_LIGHTING = 3;
const float DEBUG_COLOR_BLEND = 0.5;
const int HIT_METHOD_NONE = 0;
const int HIT_METHOD_NEAR_DDA = 1;
const int HIT_METHOD_RAYMARCH = 2;
const int HIT_METHOD_LARGE_STEP_PROBE = 3;
const int HIT_METHOD_BACKDROP = 4;

float height_cell(sampler2D height_map, vec2 world_pos, vec2 map_size) {
    vec2 terrain_uv = clamp(world_pos / terrain.xy, vec2(0.0), vec2(1.0));
    ivec2 size = ivec2(map_size);
    ivec2 cell = clamp(ivec2(floor(terrain_uv * map_size)), ivec2(0), size - ivec2(1));
    return texelFetch(height_map, cell, 0).r * render.w;
}

ivec2 source_size() {
    return ivec2(source_maps.xy);
}

int tile_size() {
    return int(tile_info.x + 0.5);
}

int tile_cache_width() {
    return int(tile_info.y + 0.5);
}

ivec2 tile_slot_origin() {
    return ivec2(tile_info.zw + vec2(0.5));
}

ivec2 source_cell_for_world(vec2 world_pos) {
    vec2 terrain_uv = clamp(world_pos / terrain.xy, vec2(0.0), vec2(1.0));
    ivec2 size = source_size();
    return clamp(ivec2(floor(terrain_uv * vec2(size))), ivec2(0), size - ivec2(1));
}

ivec2 window_min_tile() {
    return ivec2(tile_window.xy + vec2(0.5));
}

ivec2 window_max_tile() {
    return ivec2(tile_window.zw + vec2(0.5));
}

bool source_cell_is_resident(ivec2 cell) {
    int size = tile_size();
    ivec2 min_cell = window_min_tile() * size;
    ivec2 max_cell = (window_max_tile() + ivec2(1)) * size - ivec2(1);
    return cell.x >= min_cell.x
        && cell.y >= min_cell.y
        && cell.x <= max_cell.x
        && cell.y <= max_cell.y;
}

ivec2 ring_atlas_cell_for_source_cell(ivec2 cell) {
    int size = tile_size();
    int atlas_size = tile_cache_width() * size;
    ivec2 atlas_cell = cell - window_min_tile() * size + tile_slot_origin() * size;
    if (atlas_cell.x >= atlas_size) {
        atlas_cell.x -= atlas_size;
    }
    if (atlas_cell.y >= atlas_size) {
        atlas_cell.y -= atlas_size;
    }
    return atlas_cell;
}

bool near_height_cell(ivec2 cell, out float height) {
    ivec2 clamped_cell = clamp(cell, ivec2(0), source_size() - ivec2(1));
    if (!source_cell_is_resident(clamped_cell)) {
        height = 0.0;
        return false;
    }

    ivec2 atlas_cell = ring_atlas_cell_for_source_cell(clamped_cell);
    height = texelFetch(height_near_atlas, atlas_cell, 0).r * render.w;
    return true;
}

bool near_height_at(vec2 world_pos, out float height) {
    return near_height_cell(source_cell_for_world(world_pos), height);
}

vec3 far_color_at(vec2 world_pos) {
    return textureLod(color_far_map, clamp(world_pos / terrain.xy, vec2(0.0), vec2(1.0)), 0.0).rgb;
}

bool near_color_at(vec2 world_pos, out vec3 color) {
    ivec2 source_cell = source_cell_for_world(world_pos);
    if (!source_cell_is_resident(source_cell)) {
        color = vec3(0.0);
        return false;
    }

    color = texelFetch(color_near_map, ring_atlas_cell_for_source_cell(source_cell), 0).rgb;
    return true;
}

float height_lod_blend(float horizontal_dist) {
    return smoothstep(lod_distances.x, lod_distances.y, horizontal_dist);
}

float height_at(vec2 world_pos, float lod_blend) {
    if (lod_blend >= 1.0) {
        return height_cell(height_far_map, world_pos, height_maps.zw);
    }

    float near_height;
    bool has_near_height = near_height_at(world_pos, near_height);
    if (lod_blend <= 0.0 && has_near_height) {
        return near_height;
    }

    float far_height = height_cell(height_far_map, world_pos, height_maps.zw);
    if (!has_near_height) {
        return far_height;
    }

    return mix(near_height, far_height, lod_blend);
}

vec2 height_sample_radius2(float lod_blend) {
    vec2 near_cell_size = terrain.xy / source_maps.xy;
    vec2 far_cell_size = terrain.xy / height_maps.zw;
    return mix(near_cell_size, far_cell_size, lod_blend);
}

float height_sample_radius(float lod_blend) {
    vec2 sample_radius = height_sample_radius2(lod_blend);
    return min(sample_radius.x, sample_radius.y);
}

vec3 color_at(vec2 world_pos) {
    vec3 near_color;
    if (near_color_at(world_pos, near_color)) {
        return near_color;
    }

    return far_color_at(world_pos);
}

vec3 sky_color(float ray_y) {
    vec3 zenith = vec3(0.36, 0.58, 0.78);
    vec3 haze = vec3(0.74, 0.80, 0.82);
    return mix(haze, zenith, clamp(ray_y * 1.25 + 0.22, 0.0, 1.0));
}

vec3 camera_origin() {
    return vec3(camera.x, camera.z, camera.y);
}

vec3 ray_direction(vec2 screen_uv) {
    vec2 ndc = vec2(screen_uv.x * 2.0 - 1.0, 1.0 - screen_uv.y * 2.0);

    return normalize(ray_forward.xyz + ray_right.xyz * ndc.x + ray_up.xyz * ndc.y);
}

float terrain_delta(vec3 point, float lod_blend) {
    return point.y - height_at(point.xz, lod_blend);
}

bool clip_ray_axis(
    float origin_axis,
    float ray_axis,
    float bounds_min,
    float bounds_max,
    inout float enter_t,
    inout float exit_t
) {
    if (abs(ray_axis) < 0.0001) {
        return origin_axis >= bounds_min && origin_axis <= bounds_max;
    }

    float t0 = (bounds_min - origin_axis) / ray_axis;
    float t1 = (bounds_max - origin_axis) / ray_axis;

    if (t0 > t1) {
        float swap_t = t0;
        t0 = t1;
        t1 = swap_t;
    }

    enter_t = max(enter_t, t0);
    exit_t = min(exit_t, t1);
    return enter_t <= exit_t;
}

bool terrain_bounds_interval(vec3 origin, vec3 ray, out float enter_t, out float exit_t) {
    enter_t = 0.0;
    exit_t = 1.0e30;

    bool inside_x = clip_ray_axis(origin.x, ray.x, 0.0, terrain.x, enter_t, exit_t);
    bool inside_y = clip_ray_axis(origin.y, ray.y, 0.0, render.w, enter_t, exit_t);
    bool inside_z = clip_ray_axis(origin.z, ray.z, 0.0, terrain.y, enter_t, exit_t);

    return inside_x && inside_y && inside_z && exit_t >= max(enter_t, 0.0);
}

vec3 terrain_normal(vec2 world_pos, float lod_blend) {
    vec2 sample_radius = height_sample_radius2(lod_blend);
    float h_left = height_at(world_pos - vec2(sample_radius.x, 0.0), lod_blend);
    float h_right = height_at(world_pos + vec2(sample_radius.x, 0.0), lod_blend);
    float h_back = height_at(world_pos - vec2(0.0, sample_radius.y), lod_blend);
    float h_front = height_at(world_pos + vec2(0.0, sample_radius.y), lod_blend);

    return normalize(vec3(
        (h_left - h_right) / max(sample_radius.x * 2.0, 0.0001),
        1.0,
        (h_back - h_front) / max(sample_radius.y * 2.0, 0.0001)
    ));
}

float terrain_light(vec3 normal) {
    vec3 sun_dir = normalize(vec3(-0.45, 0.78, -0.34));
    float diffuse = clamp(dot(normal, sun_dir), 0.0, 1.0);
    float sky_fill = clamp(normal.y, 0.0, 1.0);

    return 0.48 + diffuse * 0.44 + sky_fill * 0.12;
}

vec3 terrain_color(vec3 hit_pos, float horizontal_dist) {
    vec2 world_pos = hit_pos.xz;
    float lod_blend = height_lod_blend(horizontal_dist);
    float normal_blend = smoothstep(lod_distances.z, lod_distances.w, horizontal_dist);
    vec3 base = color_at(world_pos);
    float light = FAR_TERRAIN_LIGHT;

    if (normal_blend < 1.0) {
        float detailed_light = terrain_light(terrain_normal(world_pos, lod_blend));
        light = mix(detailed_light, FAR_TERRAIN_LIGHT, normal_blend);
    }

    return base * light;
}

vec3 backdrop_terrain_color(vec3 hit_pos, float horizontal_dist) {
    return terrain_color(hit_pos, horizontal_dist);
}

int debug_mode() {
    return int(debug.x + 0.5);
}

vec3 debug_height_source_color(vec2 world_pos, float horizontal_dist, bool backdrop_hit) {
    if (backdrop_hit) {
        return vec3(1.0, 0.20, 0.05);
    }

    float near_height;
    bool has_near_height = near_height_at(world_pos, near_height);
    float lod_blend = height_lod_blend(horizontal_dist);
    if (has_near_height && lod_blend <= 0.001) {
        return vec3(0.05, 0.35, 1.0);
    }
    if (!has_near_height || lod_blend >= 0.999) {
        return vec3(1.0, 0.70, 0.05);
    }

    return vec3(0.75, 0.15, 1.0);
}

vec3 debug_hit_method_color(int hit_method) {
    if (hit_method == HIT_METHOD_NEAR_DDA) {
        return vec3(0.05, 1.0, 0.25);
    }
    if (hit_method == HIT_METHOD_RAYMARCH) {
        return vec3(0.05, 0.75, 1.0);
    }
    if (hit_method == HIT_METHOD_LARGE_STEP_PROBE) {
        return vec3(1.0, 0.90, 0.05);
    }
    if (hit_method == HIT_METHOD_BACKDROP) {
        return vec3(1.0, 0.15, 0.85);
    }

    return vec3(0.1);
}

vec3 debug_normal_lighting_color(float horizontal_dist) {
    float normal_blend = smoothstep(lod_distances.z, lod_distances.w, horizontal_dist);
    if (normal_blend <= 0.001) {
        return vec3(0.05, 1.0, 0.25);
    }
    if (normal_blend >= 0.999) {
        return vec3(0.95, 0.20, 0.05);
    }

    return vec3(1.0, 0.85, 0.05);
}

vec3 debug_terrain_color(vec2 world_pos, float horizontal_dist, int hit_method, bool backdrop_hit) {
    int mode = debug_mode();
    if (mode == DEBUG_HEIGHT_SOURCES) {
        return debug_height_source_color(world_pos, horizontal_dist, backdrop_hit);
    }
    if (mode == DEBUG_HIT_METHODS) {
        return debug_hit_method_color(hit_method);
    }
    if (mode == DEBUG_NORMAL_LIGHTING) {
        return debug_normal_lighting_color(horizontal_dist);
    }

    return vec3(0.0);
}

float raymarch_step_size(float horizontal_dist, float lod_blend) {
    vec2 near_cell_size2 = terrain.xy / source_maps.xy;
    float near_cell_size = min(near_cell_size2.x, near_cell_size2.y);
    float close_step = near_cell_size * CLOSE_TERRAIN_STEP_CELL_FACTOR;
    float near_step = 0.55 + horizontal_dist * 0.0055;
    float close_step_blend = smoothstep(
        CLOSE_TERRAIN_STEP_BLEND_START,
        CLOSE_TERRAIN_STEP_BLEND_END,
        horizontal_dist
    );
    float far_step = 1.0 + horizontal_dist * 0.0095;
    near_step = mix(close_step, near_step, close_step_blend);

    return clamp(mix(near_step, far_step, lod_blend), min(close_step, 0.45), 4.0);
}

int hit_refine_steps(float horizontal_dist) {
    if (horizontal_dist >= lod_distances.y) {
        return HIT_REFINE_FAR_STEPS;
    }
    if (horizontal_dist >= lod_distances.x) {
        return HIT_REFINE_MID_STEPS;
    }
    return HIT_REFINE_NEAR_STEPS;
}

bool refine_terrain_hit(
    vec3 origin,
    vec3 ray,
    float ray_horizontal,
    float low,
    float high,
    out vec3 hit_pos,
    out float hit_dist,
    out float hit_horizontal_dist
) {
    int refine_steps = hit_refine_steps(high * ray_horizontal);
    for (int j = 0; j < HIT_REFINE_NEAR_STEPS; j++) {
        if (j >= refine_steps) {
            break;
        }

        float mid = (low + high) * 0.5;
        float mid_horizontal = mid * ray_horizontal;
        float mid_delta = terrain_delta(origin + ray * mid, height_lod_blend(mid_horizontal));

        if (mid_delta <= 0.0) {
            high = mid;
        } else {
            low = mid;
        }
    }

    hit_dist = high;
    hit_pos = origin + ray * hit_dist;
    hit_horizontal_dist = hit_dist * ray_horizontal;
    return true;
}

bool probe_large_step(
    vec3 origin,
    vec3 ray,
    float ray_horizontal,
    float low,
    float high,
    out vec3 hit_pos,
    out float hit_dist,
    out float hit_horizontal_dist
) {
    float first_t = mix(low, high, 0.333333);
    float first_horizontal = first_t * ray_horizontal;
    float first_delta = terrain_delta(origin + ray * first_t, height_lod_blend(first_horizontal));

    if (first_delta <= 0.0) {
        return refine_terrain_hit(origin, ray, ray_horizontal, low, first_t, hit_pos, hit_dist, hit_horizontal_dist);
    }

    float second_t = mix(low, high, 0.666667);
    float second_horizontal = second_t * ray_horizontal;
    float second_delta = terrain_delta(origin + ray * second_t, height_lod_blend(second_horizontal));

    if (second_delta <= 0.0) {
        return refine_terrain_hit(origin, ray, ray_horizontal, first_t, second_t, hit_pos, hit_dist, hit_horizontal_dist);
    }

    return false;
}

bool should_probe_large_step(float horizontal_step, float lod_blend, float previous_delta) {
    float probe_threshold = max(
        LARGE_STEP_PROBE_MIN_STEP,
        height_sample_radius(lod_blend) * LARGE_STEP_PROBE_CELL_FACTOR
    );

    return horizontal_step > probe_threshold && previous_delta < render.w * 0.55;
}

bool near_cell_in_bounds(ivec2 cell) {
    ivec2 size = source_size();
    return cell.x >= 0 && cell.y >= 0 && cell.x < size.x && cell.y < size.y;
}

void set_terrain_hit(
    vec3 origin,
    vec3 ray,
    float ray_horizontal,
    float t,
    out vec3 hit_pos,
    out float hit_dist,
    out float hit_horizontal_dist
) {
    hit_dist = t;
    hit_pos = origin + ray * hit_dist;
    hit_horizontal_dist = hit_dist * ray_horizontal;
}

bool raycast_near_height_cells(
    vec3 origin,
    vec3 ray,
    float ray_horizontal,
    float start_t,
    float max_t,
    out float exit_t,
    out vec3 hit_pos,
    out float hit_dist,
    out float hit_horizontal_dist
) {
    float dda_distance = max(near_dda.x, 0.0);
    int dda_max_steps = clamp(int(near_dda.y + 0.5), 1, MAX_NEAR_DDA_STEPS);
    float dda_end_t = min(max_t, dda_distance / ray_horizontal);
    exit_t = start_t;

    if (start_t >= dda_end_t) {
        return false;
    }

    vec2 cell_size = terrain.xy / source_maps.xy;
    vec2 start_pos = origin.xz + ray.xz * (start_t + NEAR_DDA_T_EPSILON);
    ivec2 cell = source_cell_for_world(start_pos);

    if (!near_cell_in_bounds(cell)) {
        return false;
    }

    ivec2 step_cell = ivec2(
        ray.x > 0.0 ? 1 : (ray.x < 0.0 ? -1 : 0),
        ray.z > 0.0 ? 1 : (ray.z < 0.0 ? -1 : 0)
    );
    float next_boundary_x = (float(cell.x) + (step_cell.x > 0 ? 1.0 : 0.0)) * cell_size.x;
    float next_boundary_z = (float(cell.y) + (step_cell.y > 0 ? 1.0 : 0.0)) * cell_size.y;
    float next_x_t = step_cell.x == 0 ? 1.0e30 : (next_boundary_x - origin.x) / ray.x;
    float next_z_t = step_cell.y == 0 ? 1.0e30 : (next_boundary_z - origin.z) / ray.z;
    float delta_x_t = step_cell.x == 0 ? 1.0e30 : cell_size.x / abs(ray.x);
    float delta_z_t = step_cell.y == 0 ? 1.0e30 : cell_size.y / abs(ray.z);
    float current_t = start_t;
    float current_height;
    if (!near_height_cell(cell, current_height)) {
        return false;
    }

    for (int i = 0; i < MAX_NEAR_DDA_STEPS; i++) {
        if (i >= dda_max_steps) {
            break;
        }

        float current_y = origin.y + ray.y * current_t;
        if (current_y <= current_height) {
            set_terrain_hit(origin, ray, ray_horizontal, current_t, hit_pos, hit_dist, hit_horizontal_dist);
            return true;
        }

        float next_t = min(min(next_x_t, next_z_t), dda_end_t);

        if (ray.y < -NEAR_DDA_AXIS_EPSILON) {
            float top_t = (current_height - origin.y) / ray.y;
            if (top_t >= current_t - NEAR_DDA_T_EPSILON && top_t <= next_t + NEAR_DDA_T_EPSILON) {
                set_terrain_hit(origin, ray, ray_horizontal, max(top_t, current_t), hit_pos, hit_dist, hit_horizontal_dist);
                return true;
            }
        }

        if (next_t >= dda_end_t) {
            exit_t = dda_end_t;
            return false;
        }

        bool cross_x = next_x_t <= next_t + NEAR_DDA_T_EPSILON;
        bool cross_z = next_z_t <= next_t + NEAR_DDA_T_EPSILON;
        ivec2 next_cell = cell;
        if (cross_x) {
            next_cell.x += step_cell.x;
            next_x_t += delta_x_t;
        }
        if (cross_z) {
            next_cell.y += step_cell.y;
            next_z_t += delta_z_t;
        }

        if (!near_cell_in_bounds(next_cell)) {
            exit_t = next_t;
            return false;
        }

        float boundary_y = origin.y + ray.y * next_t;
        float next_height;
        if (!near_height_cell(next_cell, next_height)) {
            exit_t = next_t;
            return false;
        }
        float side_height = max(current_height, next_height);
        if (boundary_y <= side_height) {
            set_terrain_hit(origin, ray, ray_horizontal, next_t, hit_pos, hit_dist, hit_horizontal_dist);
            return true;
        }

        cell = next_cell;
        current_height = next_height;
        current_t = next_t;
    }

    exit_t = current_t;
    return false;
}

bool raymarch_terrain(
    vec3 origin,
    vec3 ray,
    out vec3 hit_pos,
    out float hit_dist,
    out float hit_horizontal_dist,
    out bool backdrop_available,
    out float backdrop_start_horizontal_dist,
    out float backdrop_end_horizontal_dist,
    out int hit_method
) {
    float bounds_enter_t;
    float bounds_exit_t;
    int iteration_count = clamp(int(raymarch.w + 0.5), 1, MAX_RAY_ITERATIONS);
    hit_method = HIT_METHOD_NONE;
    backdrop_available = false;
    backdrop_start_horizontal_dist = 0.0;
    backdrop_end_horizontal_dist = 0.0;

    if (!terrain_bounds_interval(origin, ray, bounds_enter_t, bounds_exit_t)) {
        return false;
    }

    float previous_t = max(max(raymarch.y, 0.05), bounds_enter_t);
    float ray_horizontal = max(length(ray.xz), 0.001);
    float max_t = min(raymarch.z / ray_horizontal, bounds_exit_t);
    backdrop_end_horizontal_dist = max_t * ray_horizontal;

    if (previous_t > max_t) {
        return false;
    }

    float near_cell_exit_t;
    if (raycast_near_height_cells(
        origin,
        ray,
        ray_horizontal,
        previous_t,
        max_t,
        near_cell_exit_t,
        hit_pos,
        hit_dist,
        hit_horizontal_dist
    )) {
        hit_method = HIT_METHOD_NEAR_DDA;
        return true;
    }

    if (near_cell_exit_t >= max_t) {
        return false;
    }

    previous_t = max(previous_t, near_cell_exit_t);

    float previous_horizontal = previous_t * ray_horizontal;
    float previous_lod_blend = height_lod_blend(previous_horizontal);
    float previous_delta = terrain_delta(origin + ray * previous_t, previous_lod_blend);

    if (previous_delta <= 0.0) {
        hit_dist = previous_t;
        hit_pos = origin + ray * hit_dist;
        hit_horizontal_dist = previous_horizontal;
        hit_method = HIT_METHOD_RAYMARCH;
        return true;
    }

    for (int i = 0; i < iteration_count; i++) {
        float step_size = raymarch_step_size(previous_horizontal, previous_lod_blend);
        float target_t = previous_t + step_size / ray_horizontal;
        bool reached_max_t = target_t >= max_t;
        float t = min(target_t, max_t);

        if (t <= previous_t) {
            break;
        }

        float horizontal = t * ray_horizontal;
        float horizontal_step = horizontal - previous_horizontal;

        if (should_probe_large_step(horizontal_step, previous_lod_blend, previous_delta)) {
            if (probe_large_step(origin, ray, ray_horizontal, previous_t, t, hit_pos, hit_dist, hit_horizontal_dist)) {
                hit_method = HIT_METHOD_LARGE_STEP_PROBE;
                return true;
            }
        }

        vec3 point = origin + ray * t;
        float lod_blend = height_lod_blend(horizontal);
        float delta = terrain_delta(point, lod_blend);

        if (delta <= 0.0) {
            hit_method = HIT_METHOD_RAYMARCH;
            return refine_terrain_hit(origin, ray, ray_horizontal, previous_t, t, hit_pos, hit_dist, hit_horizontal_dist);
        }

        if (reached_max_t) {
            return false;
        }

        previous_t = t;
        previous_delta = delta;
        previous_horizontal = horizontal;
        previous_lod_blend = lod_blend;
    }

    backdrop_start_horizontal_dist = previous_horizontal;
    backdrop_available = backdrop_end_horizontal_dist > backdrop_start_horizontal_dist + BACKDROP_MIN_HORIZONTAL_STEP;
    return false;
}

float backdrop_height_margin(vec3 origin, vec3 ray, float ray_horizontal, float horizontal_dist) {
    float t = horizontal_dist / ray_horizontal;
    vec2 world_pos = origin.xz + ray.xz * t;
    float ray_height = origin.y + ray.y * t;
    float far_height = height_cell(height_far_map, world_pos, height_maps.zw);
    float terrain_height = max(far_height - render.w * BACKDROP_HEIGHT_OFFSET_FRACTION, 0.0);

    return terrain_height - ray_height;
}

bool raycast_backdrop(
    vec3 origin,
    vec3 ray,
    float start_horizontal_dist,
    float end_horizontal_dist,
    out vec3 hit_pos,
    out float hit_horizontal_dist
) {
    float ray_horizontal = max(length(ray.xz), 0.001);
    float horizontal = start_horizontal_dist + BACKDROP_START_BIAS;

    if (horizontal >= end_horizontal_dist || ray_horizontal <= 0.001) {
        return false;
    }

    float range_step = max((end_horizontal_dist - horizontal) / float(BACKDROP_MAX_STEPS), BACKDROP_MIN_HORIZONTAL_STEP);
    float previous_margin = backdrop_height_margin(origin, ray, ray_horizontal, horizontal);
    if (previous_margin >= -BACKDROP_HIT_BIAS) {
        hit_horizontal_dist = horizontal;
        hit_pos = origin + ray * (hit_horizontal_dist / ray_horizontal);
        return true;
    }

    for (int i = 0; i < BACKDROP_MAX_STEPS; i++) {
        float step_size = max(
            range_step,
            clamp(horizontal * 0.025, BACKDROP_MIN_HORIZONTAL_STEP, BACKDROP_MAX_HORIZONTAL_STEP)
        );
        float next_horizontal = min(horizontal + step_size, end_horizontal_dist);
        float margin = backdrop_height_margin(origin, ray, ray_horizontal, next_horizontal);

        if (margin >= -BACKDROP_HIT_BIAS) {
            float denominator = margin - previous_margin;
            float blend = abs(denominator) > 0.0001
                ? clamp((-BACKDROP_HIT_BIAS - previous_margin) / denominator, 0.0, 1.0)
                : 1.0;

            hit_horizontal_dist = mix(horizontal, next_horizontal, blend);
            hit_pos = origin + ray * (hit_horizontal_dist / ray_horizontal);
            return true;
        }

        if (next_horizontal >= end_horizontal_dist) {
            break;
        }

        horizontal = next_horizontal;
        previous_margin = margin;
    }

    return false;
}

void main() {
    vec2 screen_uv = vec2(frag_uv.x, 1.0 - frag_uv.y);
    vec3 origin = camera_origin();
    vec3 ray = ray_direction(screen_uv);
    vec3 sky = sky_color(ray.y);

    vec3 hit_pos;
    float hit_dist;
    float hit_horizontal_dist;
    bool backdrop_available;
    float backdrop_start_horizontal_dist;
    float backdrop_end_horizontal_dist;
    int hit_method;

    if (raymarch_terrain(
        origin,
        ray,
        hit_pos,
        hit_dist,
        hit_horizontal_dist,
        backdrop_available,
        backdrop_start_horizontal_dist,
        backdrop_end_horizontal_dist,
        hit_method
    )) {
        int mode = debug_mode();
        float fog = smoothstep(raymarch.z * 0.62, raymarch.z, hit_horizontal_dist);
        vec3 color = mix(terrain_color(hit_pos, hit_horizontal_dist), sky, fog * 0.86);
        if (mode != DEBUG_NONE) {
            vec3 debug_color = debug_terrain_color(hit_pos.xz, hit_horizontal_dist, hit_method, false);
            color = mix(color, debug_color, DEBUG_COLOR_BLEND);
        }
        out_color = vec4(color, 1.0);
    } else if (backdrop_available && raycast_backdrop(
        origin,
        ray,
        backdrop_start_horizontal_dist,
        backdrop_end_horizontal_dist,
        hit_pos,
        hit_horizontal_dist
    )) {
        int mode = debug_mode();
        float fog = smoothstep(raymarch.z * 0.45, raymarch.z, hit_horizontal_dist);
        vec3 color = mix(backdrop_terrain_color(hit_pos, hit_horizontal_dist), sky, fog * 0.9);
        if (mode != DEBUG_NONE) {
            vec3 debug_color = debug_terrain_color(hit_pos.xz, hit_horizontal_dist, HIT_METHOD_BACKDROP, true);
            color = mix(color, debug_color, DEBUG_COLOR_BLEND);
        }
        out_color = vec4(color, 1.0);
    } else {
        out_color = vec4(sky, 1.0);
    }
}
