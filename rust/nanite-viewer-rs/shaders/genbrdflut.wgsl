// genbrdflut.wgsl - the split-sum BRDF integration LUT.
//
// Port of genbrdflut.vert + genbrdflut.frag, run as a render pass into an Rg16Float attachment
// (wgpu's format table gives `Rg16Float` RENDER_ATTACHMENT but no STORAGE_BINDING, so a compute
// write is not possible - which matches the C++ original anyway).
//
// `layout(constant_id = 0) const uint NUM_SAMPLES = 1024u;` has no WGSL equivalent, so it is a
// plain const with the same value.
//
// NOTE the local `G_SchlicksmithGGX` here uses `k = roughness*roughness/2`, *not* the
// `base/pbr.glsl` version in `common.wgsl` (`k = (roughness+1)^2/8`). Hence the distinct name.

const NUM_SAMPLES: u32 = 1024u;

// Geometric shadowing function - the *BRDF LUT* variant, k = roughness^2 / 2. It is
// deliberately NOT `common.wgsl`'s `G_SchlicksmithGGX`, which uses
// `k = (roughness + 1)^2 / 8`.
fn G_SchlicksmithGGX_LUT(dotNL: f32, dotNV: f32, roughness: f32) -> f32 {
  let k = (roughness * roughness) / 2.0;
  let GL = dotNL / (dotNL * (1.0 - k) + k);
  let GV = dotNV / (dotNV * (1.0 - k) + k);
  return GL * GV;
}

fn brdf(NoV: f32, roughness: f32) -> vec2<f32> {
  // Normal always points along the z-axis for the 2D lookup.
  let N = vec3<f32>(0.0, 0.0, 1.0);
  let V = vec3<f32>(sqrt(1.0 - NoV * NoV), 0.0, NoV);

  var LUT = vec2<f32>(0.0);
  for (var i: u32 = 0u; i < NUM_SAMPLES; i = i + 1u) {
    let Xi = hammersley2d(i, NUM_SAMPLES);
    let H = importanceSample_GGX(Xi, roughness, N);
    let L = 2.0 * dot(V, H) * H - V;

    let dotNL = max(dot(N, L), 0.0);
    let dotNV = max(dot(N, V), 0.0);
    let dotVH = max(dot(V, H), 0.0);
    let dotNH = max(dot(H, N), 0.0);

    if (dotNL > 0.0) {
      let G = G_SchlicksmithGGX_LUT(dotNL, dotNV, roughness);
      let G_Vis = (G * dotVH) / (dotNH * dotNV);
      let Fc = pow(1.0 - dotVH, 5.0);
      LUT = LUT + vec2<f32>((1.0 - Fc) * G_Vis, Fc * G_Vis);
    }
  }
  return LUT / f32(NUM_SAMPLES);
}

// genbrdflut.vert: the same 3-vertex fullscreen triangle as `shading.vert`.
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertexIndex: u32) -> VsOut {
  var out: VsOut;
  let uv = vec2<f32>(f32((vertexIndex << 1u) & 2u), f32(vertexIndex & 2u));
  out.uv = uv;
  out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
  return out;
}

@fragment
fn fs_main(@location(0) inUV: vec2<f32>) -> @location(0) vec4<f32> {
  // `inUV.y` runs *downwards* here: wgpu installs a Y-flipped viewport
  // (`wgpu-hal/src/vulkan/command.rs:1028`), so framebuffer row 0 is NDC y = +1 and `inUV.y` is 1
  // there, while the C++ pass (positive viewport) has `inUV.y` = 0 at row 0. Both sample the LUT at
  // `v = roughness`, which addresses row `roughness * height` in either API, so the *content* has
  // to be written mirrored to end up addressed the same way. Same reason as
  // `env::cube_projection`.
  return vec4<f32>(brdf(inUV.x, 1.0 - inUV.y), 0.0, 1.0);
}
