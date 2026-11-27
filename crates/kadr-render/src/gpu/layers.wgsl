// Draws one layer (a picture) onto the canvas.
//
// The layer is a quad whose four corners are computed on the CPU (fit, scale,
// rotation, position) and given in clip space, with the matching texture
// coordinates (crop). The fragment shader reads the picture with bilinear
// filtering and returns straight-alpha RGBA; blending over the canvas is done by
// the pipeline's blend state.

struct LayerUniforms {
    // One per corner: top-left, top-right, bottom-left, bottom-right.
    // xy: clip-space position; zw: texture coordinates (0 to 1).
    corners: array<vec4<f32>, 4>,
    // Rows of the YUV → RGB conversion: weights of (Y, U, V) in xyz, offset in w.
    // Values are the stored 8-bit values divided by 255.
    yuv_to_red: vec4<f32>,
    yuv_to_green: vec4<f32>,
    yuv_to_blue: vec4<f32>,
    // x: opacity of the layer, from 0 to 1.
    opacity: vec4<f32>,
    // Colour adjustments, applied in order: exposure multiplier (2^stops),
    // contrast, saturation, temperature. Same formula as color_adjustment.rs.
    adjustments: array<vec4<f32>, 4>,
    // x: how many adjustments are used.
    adjustment_count: vec4<f32>,
    // The colour LUT (color_lookup_table.rs): x: 0 none, 1 one curve per channel,
    // 2 a cube; y: N; z: strength; w: width of a row of a 1D table's texture.
    lut_settings: vec4<f32>,
    // xyz: the domain's minimum.
    lut_domain_minimum: vec4<f32>,
    // xyz: 1 / (maximum − minimum) of the domain.
    lut_domain_scale: vec4<f32>,
};

@group(0) @binding(0) var<uniform> layer: LayerUniforms;
@group(0) @binding(1) var picture_sampler: sampler;
// The RGBA picture, or the Y plane of a YUV picture.
@group(0) @binding(2) var first_plane: texture_2d<f32>;
// The U and V planes of a YUV picture (placeholders for RGBA pictures).
@group(0) @binding(3) var second_plane: texture_2d<f32>;
@group(0) @binding(4) var third_plane: texture_2d<f32>;
// The colour LUT: an N × N × N cube (red along x), or a 1D table wrapped onto
// rows. RGBA 32-bit floats, read with textureLoad (not filterable without an
// extra GPU feature), interpolated here exactly like the CPU does. A 1 × 1 × 1
// placeholder when the layer has no LUT.
@group(0) @binding(5) var color_lut: texture_3d<f32>;

// Contribution of red, green and blue to brightness (BT.709).
const BRIGHTNESS_WEIGHTS = vec3<f32>(0.2126, 0.7152, 0.0722);
// Red added and blue removed at temperature +1.
const TEMPERATURE_STRENGTH = 0.1;

fn adjust_color(color: vec3<f32>) -> vec3<f32> {
    var rgb = color;
    let adjustment_count = u32(layer.adjustment_count.x);
    for (var index = 0u; index < adjustment_count; index += 1u) {
        let adjustment = layer.adjustments[index];
        rgb = rgb * adjustment.x;
        rgb = (rgb - vec3<f32>(0.5)) * adjustment.y + vec3<f32>(0.5);
        let brightness = dot(rgb, BRIGHTNESS_WEIGHTS);
        rgb = vec3<f32>(brightness) + (rgb - vec3<f32>(brightness)) * adjustment.z;
        let shift = TEMPERATURE_STRENGTH * adjustment.w;
        rgb = clamp(rgb + vec3<f32>(shift, 0.0, -shift), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return rgb;
}

fn cube_entry(base: vec3<i32>, offset: vec3<i32>) -> vec3<f32> {
    return textureLoad(color_lut, base + offset, 0).rgb;
}

fn curve_entry(index: i32) -> vec3<f32> {
    let row_width = i32(layer.lut_settings.w);
    return textureLoad(color_lut, vec3<i32>(index % row_width, index / row_width, 0), 0).rgb;
}

// Same arithmetic as ColorLookupTable::look_up and LayerColorLut::apply.
fn look_up_color(color: vec3<f32>) -> vec3<f32> {
    let kind = layer.lut_settings.x;
    if (kind == 0.0) {
        return color;
    }
    let size = layer.lut_settings.y;
    let position = clamp(
        (color - layer.lut_domain_minimum.xyz) * layer.lut_domain_scale.xyz,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    ) * (size - 1.0);
    let lower = min(floor(position), vec3<f32>(size - 2.0));
    let fraction = position - lower;
    var looked_up: vec3<f32>;
    if (kind == 2.0) {
        let base = vec3<i32>(lower);
        // Along red, then green, then blue.
        let red_00 = mix(cube_entry(base, vec3<i32>(0, 0, 0)), cube_entry(base, vec3<i32>(1, 0, 0)), fraction.x);
        let red_10 = mix(cube_entry(base, vec3<i32>(0, 1, 0)), cube_entry(base, vec3<i32>(1, 1, 0)), fraction.x);
        let red_01 = mix(cube_entry(base, vec3<i32>(0, 0, 1)), cube_entry(base, vec3<i32>(1, 0, 1)), fraction.x);
        let red_11 = mix(cube_entry(base, vec3<i32>(0, 1, 1)), cube_entry(base, vec3<i32>(1, 1, 1)), fraction.x);
        let green_0 = mix(red_00, red_10, fraction.y);
        let green_1 = mix(red_01, red_11, fraction.y);
        looked_up = mix(green_0, green_1, fraction.z);
    } else {
        let index = vec3<i32>(lower);
        looked_up = vec3<f32>(
            mix(curve_entry(index.x).r, curve_entry(index.x + 1).r, fraction.x),
            mix(curve_entry(index.y).g, curve_entry(index.y + 1).g, fraction.y),
            mix(curve_entry(index.z).b, curve_entry(index.z + 1).b, fraction.z),
        );
    }
    return clamp(mix(color, looked_up, layer.lut_settings.z), vec3<f32>(0.0), vec3<f32>(1.0));
}

struct LayerVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) texture_coordinates: vec2<f32>,
};

// Drawn as a 4-vertex triangle strip, one vertex per corner.
@vertex
fn layer_vertex(@builtin(vertex_index) corner_index: u32) -> LayerVertexOutput {
    let corner = layer.corners[corner_index];
    var output: LayerVertexOutput;
    output.position = vec4<f32>(corner.xy, 0.0, 1.0);
    output.texture_coordinates = corner.zw;
    return output;
}

@fragment
fn rgba_layer_fragment(input: LayerVertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(first_plane, picture_sampler, input.texture_coordinates);
    return vec4<f32>(look_up_color(adjust_color(color.rgb)), color.a * layer.opacity.x);
}

@fragment
fn yuv_layer_fragment(input: LayerVertexOutput) -> @location(0) vec4<f32> {
    let yuv = vec3<f32>(
        textureSample(first_plane, picture_sampler, input.texture_coordinates).r,
        textureSample(second_plane, picture_sampler, input.texture_coordinates).r,
        textureSample(third_plane, picture_sampler, input.texture_coordinates).r,
    );
    let rgb = vec3<f32>(
        dot(layer.yuv_to_red.xyz, yuv) + layer.yuv_to_red.w,
        dot(layer.yuv_to_green.xyz, yuv) + layer.yuv_to_green.w,
        dot(layer.yuv_to_blue.xyz, yuv) + layer.yuv_to_blue.w,
    );
    let adjusted = adjust_color(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
    return vec4<f32>(look_up_color(adjusted), layer.opacity.x);
}
