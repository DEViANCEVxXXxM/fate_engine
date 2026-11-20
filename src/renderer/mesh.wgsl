struct Uniforms {
    view_projection: mat4x4<f32>,
    light: vec4<f32>,
    fog_rgb_start: vec4<f32>,
    fog_end: f32,
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
    @location(2) world: vec3<f32>,
    @location(3) normal: vec3<f32>,
};

fn max_component(v: vec3<f32>) -> f32 {
    return max(max(abs(v.x), abs(v.y)), abs(v.z));
}

fn transform_normal(normal: vec3<f32>, x: vec3<f32>, y: vec3<f32>, z: vec3<f32>) -> vec3<f32> {
    // The cofactor matrix is the inverse transpose times the determinant.
    // Keep its sign for reflections, but skip division by its magnitude:
    // the fragment shader normalizes after interpolation.
    let scale = max(max(max_component(x), max_component(y)), max_component(z));
    if scale > 0.0 && scale <= 3.402823e38 {
        let a = x / scale;
        let b = y / scale;
        let c = z / scale;
        let bc = cross(b, c);
        let determinant = dot(a, bc);
        if abs(determinant) > 0.0 {
            return sign(determinant) * (bc * normal.x + cross(c, a) * normal.y + cross(a, b) * normal.z);
        }
    }
    return vec3<f32>(0.0);
}

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
    @location(8) normal: vec3<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    let model = mat4x4<f32>(model_x, model_y, model_z, model_w);
    out.position = uniforms.view_projection * model * vec4<f32>(position, 1.0);
    out.world = (model * vec4<f32>(position, 1.0)).xyz;
    out.normal = vec3<f32>(0.0);
    if uniforms.light.w != 1.0 && any(normal != vec3<f32>(0.0)) {
        out.normal = transform_normal(normal, model_x.xyz, model_y.xyz, model_z.xyz);
    }
    out.color = color * tint.rgb;
    out.uv = uv;
    return out;
}

fn brightness(world: vec3<f32>, vertex_normal: vec3<f32>) -> f32 {
    let ambient = uniforms.light.w;
    if ambient == 1.0 {
        return 1.0;
    }
    // Framebuffer Y increases down, so this cross product points toward the
    // visible side regardless of mesh winding or mirrored model transforms.
    let face_normal = cross(dpdy(world), dpdx(world));
    let normal = select(face_normal, vertex_normal, any(vertex_normal != vec3<f32>(0.0)));
    let scale = max_component(normal);
    if scale > 0.0 && scale <= 3.402823e38 {
        let diffuse = clamp(dot(normalize(normal / scale), uniforms.light.xyz), 0.0, 1.0);
        return ambient + (1.0 - ambient) * diffuse;
    }
    return ambient;
}

fn apply_fog(color: vec3<f32>, reciprocal_clip_w: f32) -> vec3<f32> {
    if uniforms.fog_end == 0.0 {
        return color;
    }
    // Camera3d's perspective projection sets clip W to forward view depth.
    // Fragment position.w is its reciprocal at the sample.
    let depth = 1.0 / reciprocal_clip_w;
    let start = uniforms.fog_rgb_start.w;
    if depth <= start {
        return color;
    }
    let fog_color = uniforms.fog_rgb_start.xyz;
    if depth >= uniforms.fog_end {
        return fog_color;
    }
    return mix(color, fog_color, (depth - start) / (uniforms.fog_end - start));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(apply_fog(in.color * brightness(in.world, in.normal), in.position.w), 1.0);
}

@fragment
fn fs_textured(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(color_texture, color_sampler, in.uv);
    return vec4<f32>(apply_fog(in.color * texel.rgb * brightness(in.world, in.normal), in.position.w), 1.0);
}
