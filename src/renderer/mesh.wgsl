struct Uniforms {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var color_texture: texture_2d<f32>;
@group(1) @binding(1)
var color_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) model_x: vec4<f32>,
    @location(4) model_y: vec4<f32>,
    @location(5) model_z: vec4<f32>,
    @location(6) model_w: vec4<f32>,
    @location(7) tint: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    let model = mat4x4<f32>(model_x, model_y, model_z, model_w);
    out.position = uniforms.view_projection * model * vec4<f32>(position, 1.0);
    out.color = color * tint.rgb;
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}

@fragment
fn fs_textured(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(color_texture, color_sampler, in.uv);
    return vec4<f32>(in.color * texel.rgb, 1.0);
}
