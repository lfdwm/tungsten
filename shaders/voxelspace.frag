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

ivec2 wrap_cell(ivec2 cell, ivec2 map_size) {
    ivec2 wrapped = cell % map_size;
    return (wrapped + map_size) % map_size;
}

float height_cell(sampler2D height_map, vec2 world_pos, vec2 map_size) {
    vec2 terrain_uv = world_pos / terrain.xy;
    ivec2 cell = ivec2(floor(terrain_uv * map_size));
    return texelFetch(height_map, wrap_cell(cell, ivec2(map_size)), 0).r * render.w;
}

float height_at(vec2 world_pos) {
    float dist = distance(world_pos, camera.xy);
    float blend = smoothstep(lod_distances.x, lod_distances.y, dist);

    if (blend <= 0.0) {
        return height_cell(height_near_map, world_pos, height_maps.xy);
    }
    if (blend >= 1.0) {
        return height_cell(height_far_map, world_pos, height_maps.zw);
    }

    float near_height = height_cell(height_near_map, world_pos, height_maps.xy);
    float far_height = height_cell(height_far_map, world_pos, height_maps.zw);
    return mix(near_height, far_height, blend);
}

float height_sample_radius(vec2 world_pos) {
    float dist = distance(world_pos, camera.xy);
    float blend = smoothstep(lod_distances.x, lod_distances.y, dist);
    float near_cell_size = terrain.x / height_maps.x;
    float far_cell_size = terrain.x / height_maps.z;
    return mix(near_cell_size, far_cell_size, blend);
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

float terrain_delta(vec3 point) {
    return point.y - height_at(point.xz);
}

vec3 terrain_normal(vec2 world_pos) {
    float sample_radius = height_sample_radius(world_pos);
    float h_left = height_at(world_pos - vec2(sample_radius, 0.0));
    float h_right = height_at(world_pos + vec2(sample_radius, 0.0));
    float h_back = height_at(world_pos - vec2(0.0, sample_radius));
    float h_front = height_at(world_pos + vec2(0.0, sample_radius));

    return normalize(vec3(h_left - h_right, sample_radius * 2.0, h_back - h_front));
}

vec3 terrain_color(vec3 hit_pos) {
    vec2 world_pos = hit_pos.xz;
    vec3 base = color_at(world_pos);
    vec3 normal = terrain_normal(world_pos);
    vec3 sun_dir = normalize(vec3(-0.45, 0.78, -0.34));
    float diffuse = clamp(dot(normal, sun_dir), 0.0, 1.0);
    float sky_fill = clamp(normal.y, 0.0, 1.0);

    return base * (0.48 + diffuse * 0.44 + sky_fill * 0.12);
}

float raymarch_step_size(float horizontal_dist) {
    float lod_blend = smoothstep(lod_distances.x, lod_distances.y, horizontal_dist);
    float near_step = 0.55 + horizontal_dist * 0.0055;
    float far_step = 1.0 + horizontal_dist * 0.0095;
    return clamp(mix(near_step, far_step, lod_blend), 0.45, 4.0);
}

bool refine_terrain_hit(vec3 origin, vec3 ray, float low, float high, out vec3 hit_pos, out float hit_dist) {
    for (int j = 0; j < 8; j++) {
        float mid = (low + high) * 0.5;
        float mid_delta = terrain_delta(origin + ray * mid);

        if (mid_delta <= 0.0) {
            high = mid;
        } else {
            low = mid;
        }
    }

    hit_dist = high;
    hit_pos = origin + ray * hit_dist;
    return true;
}

bool probe_large_step(vec3 origin, vec3 ray, float low, float high, out vec3 hit_pos, out float hit_dist) {
    float first_t = mix(low, high, 0.333333);
    float first_delta = terrain_delta(origin + ray * first_t);

    if (first_delta <= 0.0) {
        return refine_terrain_hit(origin, ray, low, first_t, hit_pos, hit_dist);
    }

    float second_t = mix(low, high, 0.666667);
    float second_delta = terrain_delta(origin + ray * second_t);

    if (second_delta <= 0.0) {
        return refine_terrain_hit(origin, ray, first_t, second_t, hit_pos, hit_dist);
    }

    return false;
}

bool raymarch_terrain(vec3 origin, vec3 ray, out vec3 hit_pos, out float hit_dist) {
    float previous_t = max(raymarch.y, 0.05);
    float ray_horizontal = max(length(ray.xz), 0.001);
    float max_t = raymarch.z / ray_horizontal;
    float previous_delta = terrain_delta(origin + ray * previous_t);
    float previous_horizontal = previous_t * ray_horizontal;

    if (previous_delta <= 0.0) {
        hit_dist = previous_t;
        hit_pos = origin + ray * hit_dist;
        return true;
    }

    for (int i = 0; i < 700; i++) {
        float step_size = raymarch_step_size(previous_horizontal);
        float t = previous_t + step_size / ray_horizontal;

        if (t > max_t) {
            break;
        }

        if (step_size > 10.0 && previous_delta < render.w * 0.55) {
            if (probe_large_step(origin, ray, previous_t, t, hit_pos, hit_dist)) {
                return true;
            }
        }

        vec3 point = origin + ray * t;
        float delta = terrain_delta(point);

        if (delta <= 0.0) {
            return refine_terrain_hit(origin, ray, previous_t, t, hit_pos, hit_dist);
        }

        previous_t = t;
        previous_delta = delta;
        previous_horizontal = t * ray_horizontal;
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

    if (raymarch_terrain(origin, ray, hit_pos, hit_dist)) {
        float hit_horizontal_dist = distance(hit_pos.xz, camera.xy);
        float fog = smoothstep(raymarch.z * 0.62, raymarch.z, hit_horizontal_dist);
        vec3 color = mix(terrain_color(hit_pos), sky, fog * 0.86);
        out_color = vec4(color, 1.0);
    } else {
        out_color = vec4(sky, 1.0);
    }
}
