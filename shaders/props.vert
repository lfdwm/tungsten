#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;
layout(location = 2) in vec2 in_texcoord;
layout(location = 3) in vec4 in_tangent;
layout(location = 4) in vec4 in_model;
layout(location = 5) in vec4 in_rotation;

layout(set = 1, binding = 0) uniform PropsParams {
    vec4 camera;
    vec4 render;
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

vec3 rotate_quat(vec3 value, vec4 rotation) {
    vec3 t = 2.0 * cross(rotation.xyz, value);
    return value + rotation.w * t + cross(rotation.xyz, t);
}

vec3 transform_position(vec3 local_position) {
    vec3 scaled = local_position * in_model.w;
    vec3 rotated = rotate_quat(scaled, in_rotation);
    vec3 model_center = vec3(in_model.x, in_model.z, in_model.y);

    return model_center + rotated;
}

vec3 transform_direction(vec3 local_direction) {
    return normalize(rotate_quat(local_direction, in_rotation));
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
