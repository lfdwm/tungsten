#version 450

layout(set = 2, binding = 0) uniform sampler2D color_map;
layout(set = 2, binding = 1) uniform sampler2D height_near_map;
layout(set = 2, binding = 2) uniform sampler2D height_far_map;

layout(set = 3, binding = 0) uniform Params {
    vec4 camera;
    vec4 render;
    vec4 terrain;
    vec4 height_maps;
    vec4 lod_distances;
    vec4 raymarch;
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
const float NEAR_DDA_DISTANCE = 512.0;
const int NEAR_DDA_MAX_STEPS = 1024;
const float NEAR_DDA_AXIS_EPSILON = 0.00001;
const float NEAR_DDA_T_EPSILON = 0.00001;

float height_cell(sampler2D height_map, vec2 world_pos, vec2 map_size) {
    vec2 terrain_uv = world_pos / terrain.xy;
    ivec2 size = ivec2(map_size);
    ivec2 cell = clamp(ivec2(floor(terrain_uv * map_size)), ivec2(0), size - ivec2(1));
    return texelFetch(height_map, cell, 0).r * render.w;
}

float height_near_cell(ivec2 cell) {
    ivec2 size = ivec2(height_maps.xy);
    ivec2 clamped_cell = clamp(cell, ivec2(0), size - ivec2(1));
    return texelFetch(height_near_map, clamped_cell, 0).r * render.w;
}

float height_lod_blend(float horizontal_dist) {
    return smoothstep(lod_distances.x, lod_distances.y, horizontal_dist);
}

float height_at(vec2 world_pos, float lod_blend) {
    if (lod_blend <= 0.0) {
        return height_cell(height_near_map, world_pos, height_maps.xy);
    }
    if (lod_blend >= 1.0) {
        return height_cell(height_far_map, world_pos, height_maps.zw);
    }

    float near_height = height_cell(height_near_map, world_pos, height_maps.xy);
    float far_height = height_cell(height_far_map, world_pos, height_maps.zw);
    return mix(near_height, far_height, lod_blend);
}

float height_sample_radius(float lod_blend) {
    float near_cell_size = terrain.x / height_maps.x;
    float far_cell_size = terrain.x / height_maps.z;
    return mix(near_cell_size, far_cell_size, lod_blend);
}

vec3 color_at(vec2 world_pos) {
    return textureLod(color_map, world_pos / terrain.xy, 0.0).rgb;
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
    float sample_radius = height_sample_radius(lod_blend);
    float h_left = height_at(world_pos - vec2(sample_radius, 0.0), lod_blend);
    float h_right = height_at(world_pos + vec2(sample_radius, 0.0), lod_blend);
    float h_back = height_at(world_pos - vec2(0.0, sample_radius), lod_blend);
    float h_front = height_at(world_pos + vec2(0.0, sample_radius), lod_blend);

    return normalize(vec3(h_left - h_right, sample_radius * 2.0, h_back - h_front));
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

float raymarch_step_size(float horizontal_dist, float lod_blend) {
    float near_cell_size = terrain.x / height_maps.x;
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
    if (horizontal_dist >= lod_distances.w) {
        return HIT_REFINE_FAR_STEPS;
    }
    if (horizontal_dist >= lod_distances.y) {
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
    ivec2 size = ivec2(height_maps.xy);
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
    float dda_end_t = min(max_t, NEAR_DDA_DISTANCE / ray_horizontal);
    exit_t = start_t;

    if (start_t >= dda_end_t) {
        return false;
    }

    vec2 cell_size = terrain.xy / height_maps.xy;
    vec2 start_pos = origin.xz + ray.xz * (start_t + NEAR_DDA_T_EPSILON);
    ivec2 cell = ivec2(floor(start_pos / cell_size));

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
    float current_height = height_near_cell(cell);

    for (int i = 0; i < NEAR_DDA_MAX_STEPS; i++) {
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
        float next_height = height_near_cell(next_cell);
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

bool raymarch_terrain(vec3 origin, vec3 ray, out vec3 hit_pos, out float hit_dist, out float hit_horizontal_dist) {
    float bounds_enter_t;
    float bounds_exit_t;
    int iteration_count = clamp(int(raymarch.w + 0.5), 1, MAX_RAY_ITERATIONS);

    if (!terrain_bounds_interval(origin, ray, bounds_enter_t, bounds_exit_t)) {
        return false;
    }

    float previous_t = max(max(raymarch.y, 0.05), bounds_enter_t);
    float ray_horizontal = max(length(ray.xz), 0.001);
    float max_t = min(raymarch.z / ray_horizontal, bounds_exit_t);

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
                return true;
            }
        }

        vec3 point = origin + ray * t;
        float lod_blend = height_lod_blend(horizontal);
        float delta = terrain_delta(point, lod_blend);

        if (delta <= 0.0) {
            return refine_terrain_hit(origin, ray, ray_horizontal, previous_t, t, hit_pos, hit_dist, hit_horizontal_dist);
        }

        if (reached_max_t) {
            break;
        }

        previous_t = t;
        previous_delta = delta;
        previous_horizontal = horizontal;
        previous_lod_blend = lod_blend;
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

    if (raymarch_terrain(origin, ray, hit_pos, hit_dist, hit_horizontal_dist)) {
        float fog = smoothstep(raymarch.z * 0.62, raymarch.z, hit_horizontal_dist);
        vec3 color = mix(terrain_color(hit_pos, hit_horizontal_dist), sky, fog * 0.86);
        out_color = vec4(color, 1.0);
    } else {
        out_color = vec4(sky, 1.0);
    }
}
