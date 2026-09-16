// 体素渲染着色器：固定方向光 + 方块边缘压暗 + 选中方块描边。
struct Uniforms {
    mvp: mat4x4<f32>,
    // xyz = 选中方块的最小角，w > 0.5 表示有选中目标。
    selection: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) world_position: vec3<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) uv: vec2<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.mvp * vec4<f32>(position, 1.0);
    out.normal = normal;
    out.color = color;
    out.uv = uv;
    out.world_position = position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // 固定方向光，让不同朝向的面产生明暗差（顶面最亮）。
    let light = normalize(vec3<f32>(0.42, 0.86, 0.30));
    let diffuse = max(dot(normalize(in.normal), light), 0.0);
    let shade = 0.62 + 0.38 * diffuse;

    // 面内边缘压暗，让每个方块之间有一条缝。
    let distance_to_edge = min(
        min(in.uv.x, 1.0 - in.uv.x),
        min(in.uv.y, 1.0 - in.uv.y),
    );
    let edge = smoothstep(0.0, 0.06, distance_to_edge);

    var color = in.color * shade * mix(0.70, 1.0, edge);

    // 选中方块：在它可见的几个面上画一圈黑边。
    if (uniforms.selection.w > 0.5) {
        let lo = uniforms.selection.xyz;
        let hi = lo + vec3<f32>(1.0);
        if (all(in.world_position >= lo) && all(in.world_position <= hi)) {
            if (distance_to_edge < 0.06) {
                color *= 0.25;
            } else {
                color *= 0.90;
            }
        }
    }

    return vec4<f32>(color, 1.0);
}
