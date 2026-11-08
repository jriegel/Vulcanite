// Port of `shaders/glsl/pbrtexture/swrasterize.comp`: the software rasterizer.
// One invocation per *triangle* (the indirect dispatch is `indexCount / 3`),
// scanning its bounding box and `imageAtomicMax`-ing the packed 64-bit
// visibility value into `SWRBuffer`.
//
// Requires `SHADER_INT64` + `SHADER_INT64_ATOMIC_MIN_MAX` +
// `TEXTURE_INT64_ATOMIC`, the mirror of the GLSL file's three `#extension` lines.

@group(0) @binding(0) var<storage, read> inVertices: array<Vertex>;
@group(0) @binding(1) var<storage, read> inTriangles: array<u32>;
@group(0) @binding(2) var<storage, read> inModelMats: array<mat4x4<f32>>;
@group(0) @binding(3) var<storage, read> inID: array<vec3<u32>>;
@group(0) @binding(4) var<uniform> drawInfo: DrawInfo;
@group(0) @binding(5) var swrImage: texture_storage_2d<r64uint, atomic>;
@group(0) @binding(6) var<uniform> ubo: DrawMatrices;

fn insideOnePixel(v0: vec2<f32>, v1: vec2<f32>, v2: vec2<f32>) -> bool {
  return floor(v0.x) == floor(v1.x) && floor(v1.x) == floor(v2.x)
    && floor(v0.y) == floor(v1.y) && floor(v1.y) == floor(v2.y);
}

fn updatePixel(screenSize: vec2<i32>, pos: vec2<i32>, val: u64) {
  if (pos.x < 0 || pos.x >= screenSize.x || pos.y < 0 || pos.y >= screenSize.y) {
    return;
  }
  textureAtomicMax(swrImage, vec2<i32>(pos), val);
}

fn rasterize(model: mat4x4<f32>, pos0: vec3<f32>, pos1: vec3<f32>, pos2: vec3<f32>, clusterID: u32, triangleID: u32, objectID: u32) {
  let mvp = ubo.proj * ubo.view * model;
  var pos0H = mvp * vec4<f32>(pos0, 1.0);
  var pos1H = mvp * vec4<f32>(pos1, 1.0);
  var pos2H = mvp * vec4<f32>(pos2, 1.0);
  pos0H = pos0H / pos0H.w;
  pos1H = pos1H / pos1H.w;
  pos2H = pos2H / pos2H.w;
  // [`windowFromNdc`], not the GLSL `*0.5+0.5`: wgpu's viewport is Y-flipped, and the projection
  // already carries the compensating negation, so this lands the triangle on the *same* rows the
  // hardware rasterizer writes - which is what `merger.comp` then compares. Writing the GLSL
  // mapping here mirrors this pass against the hardware one, and the merge shows up as holes plus
  // fragments outside the silhouette.
  pos0H = windowFromNdc(pos0H);
  pos1H = windowFromNdc(pos1H);
  pos2H = windowFromNdc(pos2H);
  let z0 = pos0H.z;
  let z1 = pos1H.z;
  let z2 = pos2H.z;

  let size = vec2<f32>(textureDimensions(swrImage));
  let screenSize = vec2<i32>(textureDimensions(swrImage));
  let v0 = pos0H.xy * size;
  let v1 = pos1H.xy * size;
  let v2 = pos2H.xy * size;
  if (insideOnePixel(v0, v1, v2)) {
    updatePixel(screenSize, vec2<i32>(i32(v0.x), i32(v0.y)), packVis(z0, clusterID, objectID, triangleID));
  }

  let maxPixel = max(max(ceil(v0), ceil(v1)), max(ceil(v0), ceil(v2)));
  let minPixel = min(min(floor(v0), floor(v1)), min(floor(v0), floor(v2)));

  let edge01 = vec2<f32>(v1.y - v0.y, v0.x - v1.x);
  let edge12 = vec2<f32>(v2.y - v1.y, v1.x - v2.x);
  let edge20 = vec2<f32>(v0.y - v2.y, v2.x - v0.x);

  let denom = (v1.x - v0.x) * (v2.y - v0.y) - (v2.x - v0.x) * (v1.y - v0.y);
  let dzdx = ((z1 - z0) * (v2.y - v0.y) - (z2 - z0) * (v1.y - v0.y)) / denom;
  let dzdy = ((z2 - z0) * (v1.x - v0.x) - (z1 - z0) * (v2.x - v0.x)) / denom;
  let gradZ = vec2<f32>(dzdx, dzdy);

  let startPixel = minPixel + vec2<f32>(0.5);
  var CY0 = (startPixel.x - v0.x) * edge01.x + (startPixel.y - v0.y) * edge01.y;
  var CY1 = (startPixel.x - v1.x) * edge12.x + (startPixel.y - v1.y) * edge12.y;
  var CY2 = (startPixel.x - v2.x) * edge20.x + (startPixel.y - v2.y) * edge20.y;
  var ZY = z0 + (startPixel.x - v0.x) * gradZ.x + (startPixel.y - v0.y) * gradZ.y;

  for (var y = startPixel.y; y < maxPixel.y; y = y + 1.0) {
    var CX0 = CY0;
    var CX1 = CY1;
    var CX2 = CY2;
    var ZX = ZY;
    for (var x = startPixel.x; x < maxPixel.x; x = x + 1.0) {
      if (CX0 >= 0.0 && CX1 >= 0.0 && CX2 >= 0.0) {
        updatePixel(
          screenSize,
          vec2<i32>(i32(x), i32(y)),
          packVis(ZX, clusterID, objectID, triangleID),
        );
      }
      CX0 = CX0 + edge01.x;
      CX1 = CX1 + edge12.x;
      CX2 = CX2 + edge20.x;
      ZX = ZX + gradZ.x;
    }
    CY0 = CY0 + edge01.y;
    CY1 = CY1 + edge12.y;
    CY2 = CY2 + edge20.y;
    ZY = ZY + gradZ.y;
  }
}

@compute @workgroup_size(32)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let index = gid.x;
  if (index * 3u >= drawInfo.num_indices) {
    return;
  }
  let v0i = inTriangles[index * 3u];
  let v1i = inTriangles[index * 3u + 1u];
  let v2i = inTriangles[index * 3u + 2u];
  let pos0 = inVertices[v0i].pos;
  let pos1 = inVertices[v1i].pos;
  let pos2 = inVertices[v2i].pos;

  let currID = inID[index];
  rasterize(inModelMats[currID.x], pos0, pos1, pos2, currID.y, currID.z, currID.x);
}
