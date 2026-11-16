// One pipeline draws everything: flat rects, images, discs, rings and
// image-textured discs. Vertices arrive in clip space; `mode` picks the shape.
//
//   mode 0  flat colour
//   mode 1  image (uv is the texture coordinate)
//   mode 2  disc (uv is -1..1 across the disc)
//   mode 3  ring (as a disc; `param` is the inner radius as a fraction of the outer)
//   mode 4  disc textured with the image (texture coordinate = uv * 0.5 + 0.5)

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) extra: vec2<f32>, // x = mode, y = param
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) extra: vec2<f32>,
};

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = vec4<f32>(v.pos, 0.0, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    o.extra = v.extra;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    // Derivatives and sampling stay in uniform control flow, so do them first.
    let d = length(i.uv);
    let aa = max(fwidth(d), 0.0001);
    let outer = clamp(0.5 + (1.0 - d) / aa, 0.0, 1.0);
    let inner = clamp(0.5 + (d - i.extra.y) / aa, 0.0, 1.0);
    let uv = select(i.uv, i.uv * 0.5 + vec2<f32>(0.5, 0.5), i.extra.x > 3.5);
    let t = textureSample(tex, samp, uv);

    let mode = i.extra.x;
    if (mode < 0.5) {
        return i.color;
    } else if (mode < 1.5) {
        return t * i.color;
    } else if (mode < 2.5) {
        return vec4<f32>(i.color.rgb, i.color.a * outer);
    } else if (mode < 3.5) {
        return vec4<f32>(i.color.rgb, i.color.a * outer * inner);
    }
    return vec4<f32>(t.rgb * i.color.rgb, t.a * i.color.a * outer);
}
