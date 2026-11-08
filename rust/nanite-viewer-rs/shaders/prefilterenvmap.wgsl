// prefilterenvmap.wgsl - roughness-prefiltered environment cube.
//
// Port of filtercube.vert + prefilterenvmap.frag, run once per cube face and mip level of the
// prefiltered cube (512 px, 10 mips) into an offscreen Rgba16Float target that is copied into the
// cubemap layer, exactly as C++ `generatePrefilteredCube` does.
//
// The C++ push block is `PushBlock { mat4 mvp; float roughness; uint32_t numSamples = 32u; }`, so
// the scalars are a float at 64 and a u32 at 68.
//
// `random` / `hammersley2d` / `importanceSample_GGX` / `D_GGX` come from `common.wgsl` - the GLSL
// originals duplicate them, and the copies are byte-identical.

@group(0) @binding(0)
var samplerEnv: texture_cube<f32>;
@group(0) @binding(1)
var envSampler: sampler;

struct PrefilterPush {
  mvp: mat4x4<f32>,
  roughness: f32,
  numSamples: u32,
};
// No @group/@binding - see the note in `irradiancecube.wgsl`: immediates must be unbound.
var<immediate> consts: PrefilterPush;

// See `irradiancecube.wgsl`: `uvw` must keep the **default perspective interpolation** - the GLSL
// original has no `flat` qualifier, and interpolating the position vector is what yields the exact
// per-pixel face direction.
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uvw: vec3<f32>,
};

// filtercube.vert, shared with `irradiancecube.wgsl`.
@vertex
fn vs_main(@location(0) inPos: vec3<f32>) -> VsOut {
  var out: VsOut;
  out.uvw = inPos;
  out.pos = consts.mvp * vec4<f32>(inPos.xyz, 1.0);
  return out;
}

fn prefilterEnvMap(R: vec3<f32>, roughness: f32) -> vec3<f32> {
  let N = R;
  let V = R;
  var color = vec3<f32>(0.0);
  var totalWeight: f32 = 0.0;
  let envMapDim = f32(textureDimensions(samplerEnv).x);

  for (var i: u32 = 0u; i < consts.numSamples; i = i + 1u) {
    let Xi = hammersley2d(i, consts.numSamples);
    let H = importanceSample_GGX(Xi, roughness, N);
    let L = 2.0 * dot(V, H) * H - V;

    let dotNL = clamp(dot(N, L), 0.0, 1.0);
    if (dotNL > 0.0) {
      let dotNH = clamp(dot(N, H), 0.0, 1.0);
      let dotVH = clamp(dot(V, H), 0.0, 1.0);

      // PDF based on importance sampling
      let pdf = D_GGX(dotNH, roughness) * dotNH / (4.0 * dotVH) + 0.0001;
      // Solid angle of this sample
      let omegaS = 1.0 / (f32(consts.numSamples) * pdf);
      // Solid angle of texel
      let omegaP = 4.0 * PI / (6.0 * envMapDim * envMapDim);
      // Derived mip level
      let mipLevel = select(max(0.5 * log2(omegaS / omegaP) + 1.0, 0.0), 0.0, roughness == 0.0);

      color = color + textureSampleLevel(samplerEnv, envSampler, L, mipLevel).rgb * dotNL;
      totalWeight = totalWeight + dotNL;
    }
  }
  return color / totalWeight;
}

@fragment
fn fs_main(@location(0) inPos: vec3<f32>) -> @location(0) vec4<f32> {
  let N = normalize(inPos);
  return vec4<f32>(prefilterEnvMap(N, consts.roughness), 1.0);
}
