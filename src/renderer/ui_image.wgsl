// Textured quad projected to clip space: a world-space sprite or a HUD image.

@group(0) @binding(0)
var image: texture_2d<f32>;
@group(0) @binding(1)
var image_sampler: sampler;

struct VertexInput {
    @location(0) top: vec4<f32>,
    @location(1) bottom: vec4<f32>,
    @location(2) uv_bounds: vec4<f32>,
    @location(3) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput, @builtin(vertex_index) index: u32) -> VertexOutput {
    // BL, BR, TL, TR: keep the same TL-to-BR diagonal as the old triangle list.
    let right = (index & 1u) != 0u;
    let top = index >= 2u;
    let edge = select(in.bottom.zwxy, in.top, top);
    let position = select(edge.xy, edge.zw, right);
    let uv = vec2<f32>(
        select(in.uv_bounds.x, in.uv_bounds.z, right),
        select(in.uv_bounds.w, in.uv_bounds.y, top),
    );
    var out: VertexOutput;
    out.position = vec4<f32>(position, 0.0, 1.0);
    out.uv = uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let sampled = textureSample(image, image_sampler, in.uv);
    return sampled * in.color;
}
