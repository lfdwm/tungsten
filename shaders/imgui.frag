#version 450

layout(set = 2, binding = 0) uniform sampler2D font_texture;

layout(location = 0) in vec2 frag_uv;
layout(location = 1) in vec4 frag_color;
layout(location = 0) out vec4 out_color;

void main() {
    out_color = frag_color * texture(font_texture, frag_uv);
}
