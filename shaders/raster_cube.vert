#version 450

layout(location = 0) in vec3 in_position;
layout(location = 1) in vec3 in_normal;

layout(set = 1, binding = 0) uniform RasterCubeParams {
    vec4 camera;
    vec4 render;
    vec4 cube;
    vec4 ray_forward;
    vec4 ray_right;
    vec4 ray_up;
};

layout(location = 0) out vec3 world_pos;
layout(location = 1) out vec3 world_normal;

vec3 camera_origin() {
    return vec3(camera.x, camera.z, camera.y);
}

float perspective_clip_depth(float view_depth) {
    float near_depth = render.z;
    float far_depth = max(render.w, near_depth + 0.0001);

    return (far_depth * view_depth - far_depth * near_depth) / (far_depth - near_depth);
}

void main() {
    vec3 cube_center = vec3(cube.x, cube.z, cube.y);
    world_pos = cube_center + in_position * cube.w;
    world_normal = in_normal;

    vec3 delta = world_pos - camera_origin();
    float view_depth = dot(delta, ray_forward.xyz);
    float clip_x = dot(delta, ray_right.xyz) / max(dot(ray_right.xyz, ray_right.xyz), 0.0001);
    float clip_y = dot(delta, ray_up.xyz) / max(dot(ray_up.xyz, ray_up.xyz), 0.0001);

    gl_Position = vec4(clip_x, clip_y, perspective_clip_depth(view_depth), view_depth);
}
