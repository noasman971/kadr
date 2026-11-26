// Converts the finished RGBA canvas to YUV 4:2:0 planes for the encoder.
//
// Two passes, each drawing one triangle covering the whole target:
// - `luma_fragment` writes the Y plane, one value per canvas pixel;
// - `chroma_fragment` writes the U and V planes (half size) at once, each value
//   from the average colour of its 2×2 block, exactly like the CPU conversion.

struct OutputConversion {
    // Rows of the RGB → YUV conversion: weights of (R, G, B) in xyz, offset in w.
    // Results are the stored 8-bit values divided by 255.
    rgb_to_y: vec4<f32>,
    rgb_to_u: vec4<f32>,
    rgb_to_v: vec4<f32>,
};

@group(0) @binding(0) var<uniform> conversion: OutputConversion;
@group(0) @binding(1) var finished_canvas: texture_2d<f32>;

// One triangle larger than the screen: vertices (-1, -1), (3, -1), (-1, 3).
@vertex
fn full_target_vertex(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vertex_index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn convert(row: vec4<f32>, rgb: vec3<f32>) -> f32 {
    return clamp(dot(row.xyz, rgb) + row.w, 0.0, 1.0);
}

@fragment
fn luma_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let rgb = textureLoad(finished_canvas, vec2<i32>(position.xy), 0).rgb;
    return vec4<f32>(convert(conversion.rgb_to_y, rgb), 0.0, 0.0, 1.0);
}

struct ChromaOutput {
    @location(0) u: vec4<f32>,
    @location(1) v: vec4<f32>,
};

@fragment
fn chroma_fragment(@builtin(position) position: vec4<f32>) -> ChromaOutput {
    let canvas_size = vec2<i32>(textureDimensions(finished_canvas));
    let block_origin = vec2<i32>(position.xy) * 2;
    var color_total = vec3<f32>(0.0);
    var pixel_count = 0.0;
    for (var row_offset = 0; row_offset < 2; row_offset += 1) {
        for (var column_offset = 0; column_offset < 2; column_offset += 1) {
            let pixel = block_origin + vec2<i32>(column_offset, row_offset);
            if (pixel.x < canvas_size.x && pixel.y < canvas_size.y) {
                color_total += textureLoad(finished_canvas, pixel, 0).rgb;
                pixel_count += 1.0;
            }
        }
    }
    let average_rgb = color_total / max(pixel_count, 1.0);
    var output: ChromaOutput;
    output.u = vec4<f32>(convert(conversion.rgb_to_u, average_rgb), 0.0, 0.0, 1.0);
    output.v = vec4<f32>(convert(conversion.rgb_to_v, average_rgb), 0.0, 0.0, 1.0);
    return output;
}
