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

float height_sample(vec2 uv) {
    return textureLod(height_map, uv, 0.0).r * render.w;
}

float height_at(vec2 world_pos) {
    vec2 uv = world_pos / maps.zw;
    vec2 texel = 1.0 / maps.zw;
    float h = height_sample(uv);

    h = max(h, height_sample(uv + vec2(texel.x, 0.0)));
    h = max(h, height_sample(uv - vec2(texel.x, 0.0)));
    h = max(h, height_sample(uv + vec2(0.0, texel.y)));
    h = max(h, height_sample(uv - vec2(0.0, texel.y)));

    return h;
}

vec3 color_at(vec2 world_pos) {
    return textureLod(color_map, world_pos / maps.zw, 0.0).rgb;
}

vec3 sky_color(float y) {
    vec3 zenith = vec3(0.36, 0.58, 0.78);
    vec3 haze = vec3(0.74, 0.80, 0.82);
    return mix(haze, zenith, clamp(y * 1.35, 0.0, 1.0));
}

float terrain_projected_y(vec2 ray, float dist, float ray_depth, float horizon) {
    vec2 world_pos = camera.xy + ray * dist;
    float terrain_height = height_at(world_pos);
    float depth = dist * ray_depth;
    return horizon + (camera.w - terrain_height) / depth * render.z;
}

void main() {
    vec2 screen_uv = vec2(frag_uv.x, 1.0 - frag_uv.y);
    vec2 pixel = screen_uv * render.xy;
    float horizon = render.y * tuning.x + tuning.y;
    vec3 sky = sky_color(1.0 - screen_uv.y);

    float sin_phi = sin(camera.z);
    float cos_phi = cos(camera.z);
    vec2 forward = vec2(sin_phi, -cos_phi);
    vec2 right = vec2(cos_phi, sin_phi);

    float aspect = render.x / max(render.y, 1.0);
    float screen_x = (screen_uv.x * 2.0 - 1.0) * aspect;
    vec2 ray = normalize(forward + right * screen_x * tuning.z);
    float ray_depth = max(dot(ray, forward), 0.001);

    vec3 color = sky;
    float prev_dist = 2.0;
    float prev_projected_y = terrain_projected_y(ray, prev_dist, ray_depth, horizon);
    float step_size = 1.0;

    if (pixel.y >= prev_projected_y) {
        vec3 terrain_color = color_at(camera.xy + ray * prev_dist);
        color = mix(terrain_color, sky, 0.02);
        out_color = vec4(color, 1.0);
        return;
    }

    for (int i = 0; i < 260; i++) {
        float dist = prev_dist + step_size;
        if (dist > tuning.w) {
            break;
        }

        float projected_y = terrain_projected_y(ray, dist, ray_depth, horizon);

        if (pixel.y >= projected_y) {
            float low = prev_dist;
            float high = dist;

            for (int j = 0; j < 5; j++) {
                float mid = (low + high) * 0.5;
                float mid_projected_y = terrain_projected_y(ray, mid, ray_depth, horizon);

                if (pixel.y >= mid_projected_y) {
                    high = mid;
                } else {
                    low = mid;
                }
            }

            float hit_dist = high;
            vec3 terrain_color = color_at(camera.xy + ray * hit_dist);
            float fog = smoothstep(tuning.w * 0.42, tuning.w, hit_dist);
            color = mix(terrain_color, sky, fog * 0.82);
            break;
        }

        prev_dist = dist;
        step_size += 0.02;
    }

    out_color = vec4(color, 1.0);
}
