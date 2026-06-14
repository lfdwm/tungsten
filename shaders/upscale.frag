#version 450

layout(set = 2, binding = 0) uniform sampler2D source_texture;

layout(set = 3, binding = 0) uniform UpscaleParams {
    vec4 overlay;
};

layout(location = 0) in vec2 frag_uv;
layout(location = 0) out vec4 out_color;

int glyph_row_bits(int glyph, int row) {
    if (glyph == 0) {
        int rows[5] = int[](7, 5, 5, 5, 7);
        return rows[row];
    }
    if (glyph == 1) {
        int rows[5] = int[](2, 6, 2, 2, 7);
        return rows[row];
    }
    if (glyph == 2) {
        int rows[5] = int[](7, 1, 7, 4, 7);
        return rows[row];
    }
    if (glyph == 3) {
        int rows[5] = int[](7, 1, 7, 1, 7);
        return rows[row];
    }
    if (glyph == 4) {
        int rows[5] = int[](5, 5, 7, 1, 1);
        return rows[row];
    }
    if (glyph == 5) {
        int rows[5] = int[](7, 4, 7, 1, 7);
        return rows[row];
    }
    if (glyph == 6) {
        int rows[5] = int[](7, 4, 7, 5, 7);
        return rows[row];
    }
    if (glyph == 7) {
        int rows[5] = int[](7, 1, 1, 2, 2);
        return rows[row];
    }
    if (glyph == 8) {
        int rows[5] = int[](7, 5, 7, 5, 7);
        return rows[row];
    }
    if (glyph == 9) {
        int rows[5] = int[](7, 5, 7, 1, 7);
        return rows[row];
    }
    if (glyph == 10) {
        int rows[5] = int[](7, 4, 6, 4, 4);
        return rows[row];
    }
    if (glyph == 11) {
        int rows[5] = int[](6, 5, 6, 4, 4);
        return rows[row];
    }
    if (glyph == 12) {
        int rows[5] = int[](7, 4, 7, 1, 7);
        return rows[row];
    }
    if (glyph == 13) {
        int rows[5] = int[](6, 5, 5, 5, 6);
        return rows[row];
    }
    if (glyph == 14) {
        int rows[5] = int[](7, 2, 2, 2, 2);
        return rows[row];
    }
    if (glyph == 15) {
        int rows[5] = int[](5, 1, 2, 4, 5);
        return rows[row];
    }

    return 0;
}

int metric_digit(int value, int digit_index, int digit_count) {
    int divisor = 1;
    if (digit_count == 4 && digit_index == 0) {
        divisor = 1000;
    } else if ((digit_count == 4 && digit_index == 1) ||
               (digit_count == 3 && digit_index == 0)) {
        divisor = 100;
    } else if ((digit_count == 4 && digit_index == 2) ||
               (digit_count == 3 && digit_index == 1)) {
        divisor = 10;
    }

    if (digit_index < digit_count - 1 && value < divisor) {
        return -1;
    }

    return (value / divisor) % 10;
}

int fps_glyph(int char_index, int fps) {
    if (char_index == 0) {
        return 10;
    }
    if (char_index == 1) {
        return 11;
    }
    if (char_index == 2) {
        return 12;
    }
    if (char_index >= 4 && char_index <= 7) {
        return metric_digit(fps, char_index - 4, 4);
    }

    return -1;
}

int dt_glyph(int char_index, int render_percent) {
    if (char_index == 0) {
        return 13;
    }
    if (char_index == 1) {
        return 14;
    }
    if (char_index >= 3 && char_index <= 5) {
        return metric_digit(render_percent, char_index - 3, 3);
    }
    if (char_index == 6) {
        return 15;
    }

    return -1;
}

float glyph_alpha(vec2 screen_px, vec2 origin, float scale, int value, int mode) {
    vec2 local = screen_px - origin;
    if (local.x < 0.0 || local.y < 0.0) {
        return 0.0;
    }

    float char_stride = scale * 4.0;
    int char_index = int(floor(local.x / char_stride));
    int char_count = mode == 0 ? 8 : 7;
    if (char_index < 0 || char_index >= char_count || local.y >= scale * 5.0) {
        return 0.0;
    }

    vec2 glyph_px = local - vec2(float(char_index) * char_stride, 0.0);
    ivec2 cell = ivec2(floor(glyph_px / scale));
    if (cell.x < 0 || cell.x >= 3 || cell.y < 0 || cell.y >= 5) {
        return 0.0;
    }

    int glyph = -1;
    if (mode == 0) {
        glyph = fps_glyph(char_index, value);
    } else {
        glyph = dt_glyph(char_index, value);
    }

    if (glyph < 0) {
        return 0.0;
    }

    int row_bits = glyph_row_bits(glyph, cell.y);
    int bit = (row_bits >> (2 - cell.x)) & 1;
    return float(bit);
}

vec3 overlay_fps(vec3 color, vec2 screen_px) {
    vec2 origin = vec2(10.0, 10.0);
    float scale = 3.0;
    float line_stride = scale * 7.0;
    vec2 bounds = vec2(scale * 31.0, scale * 14.0);
    int fps = clamp(int(overlay.x + 0.5), 0, 9999);
    int render_percent = clamp(int(overlay.w + 0.5), 0, 999);

    if (screen_px.x >= origin.x - 4.0 && screen_px.x <= origin.x + bounds.x &&
        screen_px.y >= origin.y - 4.0 && screen_px.y <= origin.y + bounds.y) {
        color = mix(color, vec3(0.02, 0.025, 0.025), 0.55);
    }

    float shadow = max(
        glyph_alpha(screen_px, origin + vec2(1.0, 1.0), scale, fps, 0),
        glyph_alpha(screen_px, origin + vec2(1.0, line_stride + 1.0), scale, render_percent, 1)
    );
    color = mix(color, vec3(0.0), shadow * 0.85);

    float text = max(
        glyph_alpha(screen_px, origin, scale, fps, 0),
        glyph_alpha(screen_px, origin + vec2(0.0, line_stride), scale, render_percent, 1)
    );
    return mix(color, vec3(0.90, 1.0, 0.82), text);
}

void main() {
    vec3 color = textureLod(source_texture, vec2(frag_uv.x, 1.0 - frag_uv.y), 0.0).rgb;
    vec2 screen_px = vec2(frag_uv.x * overlay.y, (1.0 - frag_uv.y) * overlay.z);
    out_color = vec4(overlay_fps(color, screen_px), 1.0);
}
