//! `nanite-bake-rs` as a library.
//!
//! The binary (`src/main.rs`) is the CLI front end; the viewer (`nanite-viewer-rs`) reuses the
//! exact same pipeline in-process so that "load a glTF" means the same thing in both tools:
//! glTF -> clusters -> cluster groups -> BVH -> `NaniteBlob::Asset`. Keeping one implementation is
//! what makes a glTF-loaded model and a `.fcc` of that model render identically.

pub mod bake;
pub mod bvh;
pub mod cluster;
pub mod emit;
pub mod gltf_load;
pub mod mesh;

pub use bake::{bake_asset, BakeOptions, BakeReport};
