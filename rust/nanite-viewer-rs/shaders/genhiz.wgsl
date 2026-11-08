// genhiz.wgsl - one HiZ mip reduction step.
//
// Port of genhiz.comp. Dispatched once per mip level (mipLevels - 1 times), each time with its
// own bind group: input = HiZ mip j, output = HiZ mip j+1. The dispatch stays at the *full*
// resolution grid; the `index >= outputSize` guard is what limits each level.

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
  var depth = 0.0;
  let inputCoord = index * 2;
  depth = max(depth, textureLoad(iImage, inputCoord).x);
  depth = max(depth, textureLoad(iImage, inputCoord + vec2<i32>(0, 1)).x);
  depth = max(depth, textureLoad(iImage, inputCoord + vec2<i32>(1, 0)).x);
  depth = max(depth, textureLoad(iImage, inputCoord + vec2<i32>(1, 1)).x);
  textureStore(oImage, index, vec4<f32>(depth, vec3<f32>(0.0)));
}
