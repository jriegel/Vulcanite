// irradiancecube.wgsl - irradiance convolution over the environment cube.
//
// Port of filtercube.vert + irradiancecube.frag, run once per cube face and mip level of the
// irradiance cube, into an offscreen Rgba32Float target that is then copied into the cubemap
// layer (see `env.rs`) - exactly as the C++ `generateIrradianceCube` does.
//
// The C++ push block is `PushBlock { mat4 mvp; float deltaPhi = 2pi/180; float deltaTheta =
// 0.5pi/64; }`, i.e. mat4 at 0 and the two scalars at 64 and 68.

@group(0) @binding(0)
var samplerEnv: texture_cube<f32>;
@group(0) @binding(1)
var envSampler: sampler;

struct IrradiancePush {
  mvp: mat4x4<f32>,
  deltaPhi: f32,
  deltaTheta: f32,
};
// No @group/@binding: naga's global-variable check is `is_resource != var.binding.is_some()`, and
// an immediate is *not* a resource, so a binding decoration here is a hard validation error.
// Immediates are supplied out-of-band via `RenderPass::set_immediates`.
var<immediate> consts: IrradiancePush;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uvw: vec3<f32>,
};

// filtercube.vert: "outUVW = inPos; gl_Position = pushConsts.mvp * vec4(inPos.xyz, 1.0);"
//
// The interpolation is deliberately the **default (perspective) one**: `filtercube.vert` and
// `irradiancecube.frag` carry no `flat`/`noperspective` qualifier anywhere in `shaders/glsl/`, and
// perspective-correct interpolation of a *position* vector is exactly what produces the per-pixel
// view direction for the face (the same trick `skybox.vert` uses). With `@interpolate(flat)` every
// fragment reads a single corner vertex instead, which collapses the whole convolution to 12
// constant directions - visibly wrong reflections, and a cubemap that is warm in one spot and
// blue in another purely by accident of which corner won.
@vertex
fn vs_main(@location(0) inPos: vec3<f32>) -> VsOut {
  var out: VsOut;
  out.uvw = inPos;
  out.pos = consts.mvp * vec4<f32>(inPos.xyz, 1.0);
  return out;
}

@fragment
fn fs_main(@location(0) inPos: vec3<f32>) -> @location(0) vec4<f32> {
  let N = normalize(inPos);
  let up0 = vec3<f32>(0.0, 1.0, 0.0);
  let right = normalize(cross(up0, N));
  let up = cross(N, right);

  let TWO_PI = PI * 2.0;
  let HALF_PI = PI * 0.5;

  var color = vec3<f32>(0.0);
  var sampleCount: u32 = 0u;
  // The GLSL original loops with a float counter; the equivalent integer trip count keeps the
  // sample sequence identical (phi = k * deltaPhi for k = 0..floor((TWO_PI-eps)/deltaPhi)).
  var phi = 0.0;
  while (phi < TWO_PI) {
    var theta = 0.0;
    while (theta < HALF_PI) {
      let tempVec = cos(phi) * right + sin(phi) * up;
      let sampleVector = cos(theta) * N + sin(theta) * tempVec;
      color = color + textureSample(samplerEnv, envSampler, sampleVector).rgb * cos(theta) * sin(theta);
      sampleCount = sampleCount + 1u;
      theta = theta + consts.deltaTheta;
    }
    phi = phi + consts.deltaPhi;
  }
  return vec4<f32>(PI * color / f32(sampleCount), 1.0);
}
