// depthcopy.wgsl - copy the merged depth buffer into HiZ mip 0.
//
// Port of depthcopy.comp. The C++ source is `FinalZBuffer.view`, which is an
// R32_SFLOAT *color* image (not the depth attachment - see `createRasterizeBuffer`),
// so both sides are plain r32float storage textures and no depth-format trick is
// needed here.

@group(0) @binding(0)
var iImage: texture_storage_2d<r32float, read>;
@group(0) @binding(1)
var oImage: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
  let index = vec2<i32>(i32(gid.x), i32(gid.y));
  let outputSize = vec2<i32>(textureDimensions(oImage));
  if (index.x >= outputSize.x || index.y >= outputSize.y) {
    return;
  }
  textureStore(oImage, index, textureLoad(iImage, index));
}
