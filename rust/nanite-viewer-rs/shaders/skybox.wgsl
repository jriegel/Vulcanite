// skybox.wgsl - the environment cube background.
//
// Port of skybox.vert + skybox.frag. Bindings are the C++ "objectDraw" set 4
// (0 uniform skybox, 1 uniform params, 2 samplerCube) with the combined image
// sampler split into a texture + sampler pair, so the cube map is binding 2 and
// its sampler binding 3.
//
// The UBO is the full 144-byte `UboMatrices` (`uniformBuffers.skybox` is created
// with `sizeof(uboMatrices1)`); the GLSL struct only declares the first two
// matrices, which is exactly what is used here.

@group(0) @binding(0)
var<uniform> ubo: UboMatrices;
@group(0) @binding(1)
var<uniform> uboParams: UboParams;
@group(0) @binding(2)
var samplerEnv: texture_cube<f32>;
@group(0) @binding(3)
var envSampler: sampler;

struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uvw: vec3<f32>,
};

// skybox.vert: "outUVW = inPos; gl_Position = ubo.projection * ubo.model * vec4(inPos, 1);"
@vertex
fn vs_main(@location(0) inPos: vec3<f32>) -> VsOut {
  var out: VsOut;
  out.uvw = inPos;
  out.pos = ubo.projection * ubo.model * vec4<f32>(inPos, 1.0);
  return out;
}

@fragment
fn fs_main(@location(0) inUVW: vec3<f32>) -> @location(0) vec4<f32> {
  let color = textureSample(samplerEnv, envSampler, inUVW).rgb;
  return vec4<f32>(tonemapAndGamma(color, uboParams.exposure, uboParams.gamma), 1.0);
}
