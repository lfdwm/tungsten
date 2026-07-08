#version 450

layout(set = 2, binding = 0) uniform sampler2D terrain_depth_texture;

layout(set = 3, binding = 0) uniform WaterParams {
    vec4 camera;
    vec4 render;
    vec4 water;
    vec4 ray_forward;
    vec4 ray_right;
    vec4 ray_up;
};

layout(location = 0) in vec3 world_pos;
layout(location = 1) in vec3 world_normal;
layout(location = 2) in vec2 world_uv;

layout(location = 0) out vec4 out_color;
layout(location = 1) out float out_scene_depth;

const float DEPTH_EPSILON_WORLD = 0.1;

vec3 camera_origin() {
    return vec3(camera.x, camera.z, camera.y);
}

float normalized_linear_view_depth(vec3 point) {
    float view_depth = dot(point - camera_origin(), ray_forward.xyz);
    float near_depth = render.z;
    float far_depth = max(render.w, near_depth + 0.0001);

    return clamp((view_depth - near_depth) / (far_depth - near_depth), 0.0, 1.0);
}

float normalized_depth_epsilon() {
    float near_depth = render.z;
    float far_depth = max(render.w, near_depth + 0.0001);

    return DEPTH_EPSILON_WORLD / (far_depth - near_depth);
}

vec3 water_color() {
    vec3 normal = normalize(world_normal);
    vec3 light_dir = normalize(vec3(-0.35, 0.82, -0.28));
    float diffuse = max(dot(normal, light_dir), 0.0);
    float shimmer = sin(world_uv.x * 0.035 + world_uv.y * 0.021) * 0.025;

    return water.rgb * (0.72 + diffuse * 0.22 + shimmer);
}

void main() {
    float water_depth = normalized_linear_view_depth(world_pos);
    ivec2 depth_size = textureSize(terrain_depth_texture, 0);
    ivec2 depth_pixel = clamp(ivec2(gl_FragCoord.xy), ivec2(0), depth_size - ivec2(1));
    float terrain_depth = clamp(texelFetch(terrain_depth_texture, depth_pixel, 0).r, 0.0, 1.0);
    if (water_depth > terrain_depth + normalized_depth_epsilon()) {
        discard;
    }

    out_color = vec4(water_color(), 1.0);
    out_scene_depth = water_depth;
}
