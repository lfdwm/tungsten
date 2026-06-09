#version 450

layout(set = 2, binding = 0) uniform sampler2D source_texture;

layout(location = 0) in vec2 frag_uv;
layout(location = 0) out vec4 out_color;

void main() {
    out_color = textureLod(source_texture, vec2(frag_uv.x, 1.0 - frag_uv.y), 0.0);
}
