#version 450

layout(set = 2, binding = 0) uniform sampler2D terrain_depth_texture;
layout(set = 2, binding = 1) uniform sampler2D base_color_texture;
layout(set = 2, binding = 2) uniform sampler2D normal_texture;

layout(set = 3, binding = 0) uniform PropsParams {
    vec4 camera;
    vec4 render;
    vec4 material_diffuse;
    vec4 material_specular;
    vec4 material_flags;
    vec4 ray_forward;
    vec4 ray_right;
    vec4 ray_up;
};

layout(location = 0) in vec3 world_pos;
layout(location = 1) in vec3 world_normal;
layout(location = 2) in vec2 world_uv;
layout(location = 3) in vec4 world_tangent;

layout(location = 0) out vec4 out_color;
layout(location = 1) out float out_scene_depth;

vec3 camera_origin() {
    return vec3(camera.x, camera.z, camera.y);
}

float normalized_linear_view_depth(vec3 point) {
    float view_depth = dot(point - camera_origin(), ray_forward.xyz);
    float near_depth = render.z;
    float far_depth = max(render.w, near_depth + 0.0001);

    return clamp((view_depth - near_depth) / (far_depth - near_depth), 0.0, 1.0);
}

const float DEPTH_EPSILON_WORLD = 0.1;
float normalized_depth_epsilon() {
    float near_depth = render.z;
    float far_depth = max(render.w, near_depth + 0.0001);

    return DEPTH_EPSILON_WORLD / (far_depth - near_depth);
}

vec3 sampled_normal() {
    vec3 normal = normalize(world_normal);
    if (material_flags.y < 0.5) {
        return normal;
    }

    vec3 tangent = normalize(world_tangent.xyz - normal * dot(normal, world_tangent.xyz));
    vec3 bitangent = normalize(cross(normal, tangent)) * world_tangent.w;
    vec3 tangent_space_normal = texture(normal_texture, world_uv).xyz * 2.0 - vec3(1.0);

    return normalize(mat3(tangent, bitangent, normal) * tangent_space_normal);
}

vec3 material_base_color() {
    vec3 texture_color = material_flags.x > 0.5
        ? texture(base_color_texture, world_uv).rgb
        : vec3(1.0);

    return material_diffuse.rgb * texture_color;
}

vec3 phong_color(vec3 normal) {
    vec3 light_dir = normalize(vec3(-0.42, 0.76, -0.35));
    vec3 view_dir = normalize(camera_origin() - world_pos);
    vec3 reflected = reflect(-light_dir, normal);
    vec3 base_color = material_base_color();
    float diffuse = max(dot(normal, light_dir), 0.0);
    float specular = pow(max(dot(view_dir, reflected), 0.0), material_diffuse.w);
    float sky_fill = max(normal.y, 0.0);

    return base_color * (0.22 + diffuse * 0.62 + sky_fill * 0.12)
        + material_specular.rgb * specular;
}

void main() {
    float prop_depth = normalized_linear_view_depth(world_pos);
    ivec2 depth_size = textureSize(terrain_depth_texture, 0);
    ivec2 depth_pixel = clamp(ivec2(gl_FragCoord.xy), ivec2(0), depth_size - ivec2(1));
    float terrain_depth = clamp(texelFetch(terrain_depth_texture, depth_pixel, 0).r, 0.0, 1.0);
    if (prop_depth > terrain_depth + normalized_depth_epsilon()) {
        discard;
    }

    out_color = vec4(phong_color(sampled_normal()), 1.0);
    out_scene_depth = prop_depth;
}
