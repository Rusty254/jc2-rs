#define_import_path jc2::point_lights

// The native point-light term of the surface shaders (JC2-221; research/formats/point-light-model.md
// sections 3, 4 and 10.3). `point_lights.rs` selects the 64 lights of the frame and fills the two
// buffers below; every material that runs the original's light loop declares them at the same
// binding numbers (120, 121).
//
// Native (probe-verified, 163 pixel shaders): for the (up to) four lights a 4 m grid cell holds,
//   a = saturate(1 - |L|^2 / r^2)            L = light - surface point
//   contribution = colour * a^2 * saturate(N . L / |L|)
// summed into the irradiance `E` BEFORE the albedo product and before fog; no inverse-square term.
// Inferred: the grid origin snapping and cell membership are `point_lights.rs` policy.

struct PointLight {
    // xyz: world position, w: 1 / (radius * radius)
    pos_w: vec4<f32>,
    // rgb: colour after the per-hour scale, distance fade and 8.0 cap
    colour: vec4<f32>,
}

struct PointLights {
    // xy: world xz of the grid corner (cell 0, 0); z: cell size; w: lights in use
    origin: vec4<f32>,
    lights: array<PointLight, 64>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(120) var<storage, read> point_lights: PointLights;
// 128 x 128 cells, 4 light ids per u32 (byte k = light index + 1, 0 = none)
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var<storage, read> point_cells: array<u32>;

// The summed light colour (native units, before the exposure) at a world position with normal `n`.
fn point_light_irradiance(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    if point_lights.origin.w < 0.5 {
        return sum;
    }
    let cell = vec2<i32>(floor((p.xz - point_lights.origin.xy) / point_lights.origin.z));
    if cell.x < 0 || cell.y < 0 || cell.x >= 128 || cell.y >= 128 {
        return sum;
    }
    let ids = point_cells[u32(cell.y) * 128u + u32(cell.x)];
    for (var k: u32 = 0u; k < 4u; k = k + 1u) {
        let idx = (ids >> (8u * k)) & 255u;
        if idx == 0u {
            continue;
        }
        let l = point_lights.lights[idx - 1u];
        let d = l.pos_w.xyz - p;
        let d2 = dot(d, d);
        let a = saturate(1.0 - d2 * l.pos_w.w);
        sum = sum + l.colour.rgb * (a * a) * saturate(dot(n, d) * inverseSqrt(max(d2, 1.0e-8)));
    }
    return sum;
}
