#version 450

layout(set = 2, binding = 0) uniform sampler2D color_map;
layout(set = 2, binding = 1) uniform sampler2D height_map;

layout(set = 3, binding = 0) uniform Params {
    vec4 camera;
    vec4 render;
    vec4 maps;
    vec4 tuning;
};

layout(location = 0) in vec2 frag_uv;
layout(location = 0) out vec4 out_color;

ivec2 wrap_height_cell(ivec2 cell) {
    ivec2 map_size = ivec2(int(maps.z), int(maps.w));
    ivec2 wrapped = cell % map_size;
    return (wrapped + map_size) % map_size;
}

float height_cell(ivec2 cell) {
    return texelFetch(height_map, wrap_height_cell(cell), 0).r * render.w;
}

float height_at(vec2 world_pos) {
    return height_cell(ivec2(floor(world_pos)));
}

vec3 color_at(vec2 world_pos) {
    return textureLod(color_map, world_pos / maps.xy, 0.0).rgb;
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
    float yaw = camera.w;
    float pitch = tuning.x;
    float sin_yaw = sin(yaw);
    float cos_yaw = cos(yaw);
    float sin_pitch = sin(pitch);
    float cos_pitch = cos(pitch);

    vec3 forward_flat = vec3(sin_yaw, 0.0, -cos_yaw);
    vec3 right = vec3(cos_yaw, 0.0, sin_yaw);
    vec3 world_up = vec3(0.0, 1.0, 0.0);
    vec3 forward = normalize(forward_flat * cos_pitch + world_up * sin_pitch);
    vec3 up = normalize(world_up * cos_pitch - forward_flat * sin_pitch);

    float aspect = render.x / max(render.y, 1.0);
    float tan_half_fov = tan(render.z * 0.5);
    vec2 ndc = vec2(screen_uv.x * 2.0 - 1.0, 1.0 - screen_uv.y * 2.0);

    return normalize(
        forward
            + right * ndc.x * aspect * tan_half_fov
            + up * ndc.y * tan_half_fov
    );
}

float terrain_delta(vec3 point) {
    return point.y - height_at(point.xz);
}

vec3 terrain_normal(vec2 world_pos) {
    float h_left = height_at(world_pos - vec2(1.0, 0.0));
    float h_right = height_at(world_pos + vec2(1.0, 0.0));
    float h_back = height_at(world_pos - vec2(0.0, 1.0));
    float h_front = height_at(world_pos + vec2(0.0, 1.0));

    return normalize(vec3(h_left - h_right, 2.0, h_back - h_front));
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

bool raymarch_terrain(vec3 origin, vec3 ray, out vec3 hit_pos, out float hit_dist) {
    float t = max(tuning.y, 0.05);
    float max_dist = tuning.z;
    float previous_t = t;

    if (terrain_delta(origin + ray * previous_t) <= 0.0) {
        hit_dist = previous_t;
        hit_pos = origin + ray * hit_dist;
        return true;
    }

    for (int i = 0; i < 360; i++) {
        float step_size = max(0.35, 0.55 + t * 0.0055);
        t += step_size;

        if (t > max_dist) {
            break;
        }

        vec3 point = origin + ray * t;
        float delta = terrain_delta(point);

        if (delta <= 0.0) {
            float low = previous_t;
            float high = t;

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

        previous_t = t;
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
        float fog = smoothstep(tuning.z * 0.34, tuning.z, hit_dist);
        vec3 color = mix(terrain_color(hit_pos), sky, fog * 0.86);
        out_color = vec4(color, 1.0);
    } else {
        out_color = vec4(sky, 1.0);
    }
}
