#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec2 in_texcoord;
layout(location = 3) in vec4 in_tangent;

layout(set = 1, binding = 0) uniform RasterParams {
    vec4 camera;
    vec4 render;
    vec4 model;
    vec4 rotation;
    vec4 material_diffuse;
    vec4 material_specular;
    vec4 material_flags;
    vec4 ray_forward;
    vec4 ray_right;
    vec4 ray_up;
};

layout(location = 0) out vec3 world_pos;
layout(location = 1) out vec3 world_normal;
layout(location = 2) out vec2 world_uv;
layout(location = 3) out vec4 world_tangent;

vec3 camera_origin() {
    return vec3(camera.x, camera.z, camera.y);
}

float perspective_clip_depth(float view_depth) {
    float near_depth = render.z;
    float far_depth = max(render.w, near_depth + 0.0001);

    return (far_depth * view_depth - far_depth * near_depth) / (far_depth - near_depth);
}

vec2 rotate_xz(vec2 value) {
    return vec2(
        value.x * rotation.x - value.y * rotation.y,
        value.x * rotation.y + value.y * rotation.x
    );
}

vec3 transform_position(vec3 local_position) {
    vec3 scaled = local_position * model.w;
    vec2 rotated_xz = rotate_xz(scaled.xz);
    vec3 model_center = vec3(model.x, model.z, model.y);

    return model_center + vec3(rotated_xz.x, scaled.y, rotated_xz.y);
}

vec3 transform_direction(vec3 local_direction) {
    vec2 rotated_xz = rotate_xz(local_direction.xz);

    return normalize(vec3(rotated_xz.x, local_direction.y, rotated_xz.y));
}

void main() {
    world_pos = transform_position(in_position);
    world_normal = transform_direction(in_normal);
    world_uv = in_texcoord;
    world_tangent = vec4(transform_direction(in_tangent.xyz), in_tangent.w);

    vec3 delta = world_pos - camera_origin();
    float view_depth = dot(delta, ray_forward.xyz);
    float clip_x = dot(delta, ray_right.xyz) / max(dot(ray_right.xyz, ray_right.xyz), 0.0001);
    float clip_y = dot(delta, ray_up.xyz) / max(dot(ray_up.xyz, ray_up.xyz), 0.0001);

    gl_Position = vec4(clip_x, clip_y, perspective_clip_depth(view_depth), view_depth);
}
