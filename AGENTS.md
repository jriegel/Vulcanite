# AGENTS.md

Working notes for AI agents (and humans) editing this repository. Everything here has been
verified against the tree, not guessed. When you find something wrong, update this file.

## What this project is

Vulkan C++17 renderer implementing a Nanite-style cluster/LOD pipeline. The example base is
derived from Sascha Willems' Vulkan examples (`base/`), the Nanite pipeline lives in `mesh/`,
the viewer is `examples/pbrtexture/pbrtexture.cpp` (~4000 lines, the only example that uses the
mesh library), `tools/nanite_bake/` is the headless asset-pipeline front end of the builder, and
`rust/` is a parallel Rust implementation of that asset pipeline (`nanite-bake-rs`) writing the
same `.fcc` format. The C++ builder stays in place; see *Rust asset pipeline*.

Pipeline: glTF -> `vkglTF::Model` -> OpenMesh `MyMesh` per LOD -> clusters (56 tris) ->
cluster groups (15) -> QEM edge-collapse (boundary halfedges locked) -> re-cluster, 6 LODs ->
per-LOD BVH -> runtime LOD selection via `parentError > threshold && clusterError <= threshold`
-> mixed software/hardware rasterization into a 64-bit visibility buffer
(`depth:32 | clusterId:15 | objectId:11 | triangleId:6`).

`LOG.md` is the chronological design log (mixed Chinese/English) — useful for *why* a
structure looks the way it does, not for current API.

## Build

Windows only in practice. Generator `Visual Studio 17 2022`, build dir `build/`
(`build/Vulcanite.sln`), dependencies via vcpkg (`vcpkg.json`: openmesh, assimp, vulkan,
glslang) and METIS via `cmake/FetchMetis.cmake`. `Setup.bat` configures the build dir.

```
cmake --build build --config Release
```

Executables land in `build/bin/Release/` (`pbrtexture.exe`, `nanite_bake.exe`, `imagecompare.exe`)
and `build/bin/Debug/`.

The Rust workspace is built by cargo, not CMake, and is not part of `ALL_BUILD`:

```
cargo build --release -p nanite-bake-rs    # from rust/
```

It lands in `rust/target/release/nanite-bake-rs.exe`; test 14 uses it and is `DISABLED` when that
file is missing. Toolchain as of writing: `cargo 1.97.1`.

**Do not use the CMake Tools extension tools here**: `ListBuildTargets` and `ListTests`
return empty for this project even though the solution builds fine. Use the terminal.

## Test

```
ctest --test-dir build -C Release --output-on-failure
```

Fifteen tests, all green as of writing (`render_pbrtexture_fresh`, `render_pbrtexture_cached`,
`render_pbrtexture_blob`, `render_pbrtexture_reload`, `render_pbrtexture_reload_swap`,
`render_pbrtexture_baked_by_tool`, `nanite_bake_verify`, `render_pbrtexture_blob_only`,
`render_pbrtexture_vis1..3`, `render_pbrtexture_vis_modes_differ`,
`render_pbrtexture_blob_reload_twice`, `render_pbrtexture_rust_blob`, `imagecompare_selftest`). The
render tests are `RUN_SERIAL` with a fixture chain: *fresh* builds the caches, everything else
reuses them.

- `tests/RunRenderTest.cmake` parameters: `RENDER_EXE`, `COMPARE_EXE`, `MODEL`, `OUT`,
  `REFERENCE` (required), `ASSETS_DIR`, `FRAMES` (default 10), `CLEAN_CACHE`, `BAKE_EXE`,
  `RENDER_ARGS`, `COMPARE_ARGS`. `MODEL` is the extension-less model name (`bunny`) or the baked
  blob itself (`bunny.fcc`, rendered without loading any glTF); the cache/blob paths are derived
  from the extension-less part. With `BAKE_EXE` set it first deletes the cache *and* the `.fcc`
  and builds them with that tool before rendering. It runs
  `RENDER_EXE --capture OUT --captureframes FRAMES --model MODEL RENDER_ARGS`
  then `COMPARE_EXE OUT REFERENCE COMPARE_ARGS`.
- `tests/RunBakeTest.cmake` (`nanite_bake_verify`) takes only `BAKE_EXE`, `ASSETS_DIR`, `MODEL`.
  It runs `nanite_bake --rebuild` and then `nanite_bake --verify` **twice as a separate process**,
  once addressed by the `.gltf` and once by the `.fcc` path itself, which is what makes the test
  mean anything: `--verify` opens a blob the builder did not write in the same process, so it
  really re-reads magic, versions, struct sizes, the section table and the strict source hash from
  disk — and the second run proves a blob can be validated without its source mesh being named.
- `imagecompare` flags: `--mean-tol <f>`, `--pixel-tol <f>`, `--max-bad <f>`, `--diff <png>`,
  `--update`. **Defaults are `mean-tol 2.0`, `pixel-tol 24`, `max-bad 0.02`**, and tests 1/2/6 pass
  no `COMPARE_ARGS` at all, so they run with those. Those tolerances are slack, not needed: since
  the camera stopped moving (see *Camera and "View all"*), every bunny capture measured on this
  machine is **bit-identical** to its golden (`mean abs diff 0.0000`, 0% bad pixels) — fresh,
  cached, blob, reload, baked-by-tool and blob-only alike. The tolerances stay because a different
  GPU may still pick a slightly different LOD set; do not tighten them to "prove" anything.
  Only tests 3, 4, 5 and 13 demand bit equality, and each of them compares against an *earlier
  capture*, not the golden image.
- `tests/RunReloadSwapTest.cmake` is the reload harness: it renders `MODEL` at startup, reloads
  `RELOAD_MODEL` at `RELOAD_FRAME` via `--reload`, then renders `RELOAD_MODEL` normally and
  compares the two. It has **no** `REFERENCE` parameter — the reference is generated.
- **`render_pbrtexture_blob_reload_twice` is the `.fcc` crash regression test.** It opens
  `bunny.fcc` at startup and reloads it **twice** (`--reload … --reloadframe 5 --reload2 …
  --reload2frame 8`), which is the only way a test can reach the file dialog's "pick a second blob"
  path — the old harness could express exactly one reload. Every reload tears down `models.object`,
  which on the blob path was never loaded, so this is what catches uninitialized `vkglTF::Model`
  members (see *Runtime model reload*). Compared bit-exact against `blob_only.png` from test 8, so a
  reload that silently drops the scene fails too. Measured: bit-identical (`0.0000`, 0% bad).
- **Cluster visualization is regression tested too.** The ImGui *Visualize Clusters* slider
  (`renderingPushConstants.vis_clusters`, 0..3) is unreachable from a test because `--capture`
  disables the overlay (`vulkanexamplebase.cpp`: `settings.overlay = ... && !capture.active`), so
  the viewer also takes `--visualize <0..3>`. Tests 9–11 render `bunny` with modes 1/2/3 and
  compare against `tests/golden/pbrtexture_bunny_vis<N>.png`; mode 0 is the shaded golden of test
  1, so all four modes are pinned. What each mode shows (`shading.frag`, `pbrtexture.frag`,
  `merger.comp`): 1 = LOD level as flat colours, 2 = per-triangle hash of the visibility-buffer id
  (or, depending on `uboParams.gamma`, cluster vs. cluster-group colour — the shader branches on
  `gamma < 2.15`, so **never change gamma in a visualization test**), 3 = grayscale visibility
  buffer id, which also swaps the merge shader to an occlusion-only result.
  Measured: a repeat capture of each mode is **bit-identical** to its golden (`mean abs diff
  0.0000`, 0% bad pixels) — unlike the shaded image, these modes are flat colours with no PBR
  noise. They still run with imagecompare's default tolerances, exactly like tests 1/2/6, so a
  different GPU that picks a slightly different LOD set does not fail the suite.
- `tests/RunImagesDifferTest.cmake` (`render_pbrtexture_vis_modes_differ`) asserts that the four
  captures are pairwise *different* (imagecompare exiting non-zero). Without it, a viewer that
  ignored `--visualize` would render the shaded image four times, every vis golden would be a copy
  of the shaded one, and tests 1–11 would all stay green. Verified: feeding it two identical images
  fails with "Images are identical but must differ".
- **`tests/RunRustBlobTest.cmake` (`render_pbrtexture_rust_blob`) is the Rust pipeline smoke test.**
  It runs `nanite-bake-rs --rebuild --out assets/models/bunny_rs.fcc assets/models/bunny.gltf`,
  re-reads that blob with `--verify` in a *separate* process, then renders `--model bunny_rs.fcc
  --nanite-blob` in the C++ viewer. `--out` is mandatory: baking over `bunny.fcc` would silently
  replace the C++ builder's output that tests 3/8/13 compare against. Two comparisons, because the
  Rust builder emits **one LOD** (no OpenMesh decimation, no METIS collapse order to reproduce) and
  therefore always draws LOD 0 at full detail while the C++ blob draws a mix of levels:
  1. default render vs the golden image at imagecompare's defaults — measured `0.93` mean abs diff /
     `1.91%` bad, i.e. inside the tolerance but nowhere near bit-exact, and that is expected.
  2. `--threshold 0` on **both** blobs, which pins the C++ blob to LOD 0 too, so both draw the same
     triangles — measured `0.26` / `0.52%` bad. This is the comparison that catches a wrong cluster,
     a wrong index range or a wrong AABB instead of only "roughly the same silhouette".
  `NANITE_BAKE_RS_EXE` is a cache variable pointing at the cargo output; the test is marked `DISABLED`
  when the file does not exist, so a C++-only checkout still passes 15/15.
- Re-baseline the golden image with `$env:VULCANITE_UPDATE_GOLDEN = "1"` before ctest. That env var
  only helps once a golden exists: the vis tests declare `REQUIRED_FILES` on their golden, so a
  missing one is reported as `***Not Run`, not as a failure to record. Bootstrap a new golden by
  calling the script directly with absolute paths, e.g.
  `cmake -DRENDER_EXE=... -DCOMPARE_EXE=... -DMODEL=bunny -DASSETS_DIR=... -DFRAMES=10 -DOUT=... -DREFERENCE=tests/golden/<name>.png "-DRENDER_ARGS=--nanite-blob;--visualize;1" -P tests/RunRenderTest.cmake`
  with `VULCANITE_UPDATE_GOLDEN=1`.

Gotchas that cost real debugging time:

- **Never set `PASS_REGULAR_EXPRESSION` on a render test.** It replaces CTest's exit-code
  check, and it reported a hard-crashing test (`0xc0000409`, `Assertion failed`) as *Passed*.
- `add_test` arguments containing `;` must be escaped as `\;`, otherwise CMake splits them
  into separate argv entries (`-DCOMPARE_ARGS=--mean-tol\;0\;--pixel-tol\;0`).
- Editing `tests/CMakeLists.txt` does nothing until a rebuild regenerates
  `CTestTestfile.cmake`.
- When running an exe by hand, always pass **absolute** `--capture` paths; relative paths
  resolve against the current shell directory, not `build/bin/Release`.
- Do not pipe an interactive exe through `Select-Object -First N` — it truncates and hides
  the output you were looking for, and it **kills the process** once the count is reached, so a
  `--capture` never gets written and a cache never gets baked. Use `Select-String` on the stream
  instead.
- `get_errors` (the language server) can report "No errors found" while `static_assert`s
  fail. Trust the compiler.
- **The old "frame-count sensitive" gotcha is gone — because the camera no longer moves.** With
  `viewAll()` the pose is fixed by the model, so `--captureframes 10/11/20/21/30/40/41` of the same
  model are now bit-identical (measured, both with and without `--nanite-blob`). Earlier captures
  were taken while the camera was still being rotated, which is why 40 vs 41 frames used to differ
  by mean abs diff ~1.7. If you reintroduce any camera animation (or a `viewChanged()` during
  capture), temporal LOD/occlusion state makes the frame count matter again — and
  `render_pbrtexture_reload_swap` already needs `--mean-tol 5 --pixel-tol 16` because there the
  model *appears* at a different time in the two runs (measured 0.57 / 1.27% bad).
- `cmake -P script.cmake -DVAR=x` does **not** set the variables — every `-D` has to come
  *before* `-P`, i.e. `cmake -DVAR=x -P script.cmake`.
- In a `-P` script there is **no** auto-generated `<LIST>_COUNT` for a `-D` list argument —
  `-DIMAGES=a;b;c` gives `IMAGES` and `IMAGES_COUNT` is empty. Use `list(LENGTH IMAGES n)`.
- In PowerShell, `"$PWD/x"` inside a *double-quoted cmake argument* is not expanded the way you
  expect once the argument itself is quoted — a `cmake -DOUT=$PWD/...` bootstrap run silently
  created a literal `./$PWD/` directory in the repo root. Pass absolute paths.
- **The raster output buffers are a correctness limit, not a memory hint.** `culling.comp` reserves
  a slot with an unbounded `atomicAdd` and then writes — there is **no bounds check anywhere** — so
  `HWRIndicesBuffer`/`SWRIndicesBuffer` (and the two `…IDBuffer`s) must be able to hold everything the
  cull shaders can emit. They used to be `sceneIndicesCount / 8`, which silently assumes LOD selection
  discards seven eighths of the scene. A single-LOD blob has nothing to discard: `bunny` emitted
  288,092 triangles into 108,034 slots (**2.7× overflow**) and rendered as scattered scraps — no
  validation layer complained, because the writes land inside the next allocation. They are now
  `sceneIndicesCount` (and `sceneIndicesCount / 3` `uvec3`s), which is the *exact* upper bound:
  cluster triangle ranges are disjoint inside the scene index buffer and every cluster is emitted at
  most once, but the hw/sw split means each buffer alone needs full capacity. Measured after the fix:
  Rust blob `286,664 hw + 410 sw = 287,074` tris of 288,092 possible; C++ blob `54,416`.
  `--raster-stats` prints those counters (`[raster] frame N …`) during `--capture`, when the overlay
  that normally shows them is off — that is how the overflow was found.
- **`--threshold 0` used to be impossible.** `base/CommandLineParser.hpp::getValueAsInt` returned the
  default whenever the parsed value was `<= 0` (`return (intVal > 0) ? intVal : defaultValue;`). It now
  only falls back when nothing parsed at all (`numConvPtr != value.c_str()`), so `0` and negatives
  work. Checked all 13 call sites — the base ones (`--width`, `--height`, `--captureframes`, …) never
  receive `0` from a test. `--threshold <0..1000>` pins the LOD selector; `0` keeps every LOD 0 cluster,
  which is how a single-LOD render is compared against a multi-LOD one.
- Known pre-existing builder bug: `--model teapot` / `--model sphere` dies with an access
  violation (`0xC0000005`) at the end of `NaniteMesh::generateNaniteInfo` — small meshes reach
  `clusterGroupNum == 1`, so the last LODs are identical copies. Reproduced at HEAD with a
  `git stash`, so it is not a regression. Nothing is cached in that case (`serialize` never runs),
  which is why `--nanite-blob` then asserts "no blob cache exists". `--model voyager` crashes the
  same way (exit 1 right after "LOD 0 generated"), also pre-existing and unrelated to the viewer.
  Safe models to bake: `bunny`, `venus`, `dragon`, `nanite_test*`.

## The two-stage asset pipeline

The builder (OpenMesh + METIS + JSON) and the viewer (raw POD payloads) are deliberately
separated, and the binary blob is the contract between them. The blob is a **standalone asset
file**, `<model>.fcc`, not a file inside the cache directory — that is what lets it be copied,
shipped, and opened by a viewer that never ran the builder.

- `tools/nanite_bake/nanite_bake.cpp` is the builder as a standalone console tool:
  `nanite_bake [--rebuild|--json-only] <model.gltf>...`, and `nanite_bake --verify <model.gltf>`
  (or `--verify <model>.fcc`) re-reads a blob without Vulkan or a build. It creates a headless
  `VkInstance` (no WSI extension) and a logical device with `useSwapChain = false` only because
  `vkglTF::Model::loadFromFile` uploads the buffers it parses.
  **Both stages load the mesh through `NaniteMesh::kBuilderLoadingFlags`** (`PreTransformVertices |
  PreMultiplyVertexColors | FlipY`, the tool adds `DontLoadImages`): `loadFromFile` rewrites
  `vertexBuffer` in place — `FlipY` negates `pos.y`/`normal.y`, `PreTransformVertices` applies the
  node matrix — and the builder consumes exactly those vertices. Baking with a different flag set
  yields a mirrored mesh that passes *every* hash check (the source hash only covers the file
  bytes); it showed up as mean abs diff 34.8 / 63% bad pixels. Do not re-inline the flag
  expression anywhere.
  Measured: the tool's `<model>.fcc` is byte-identical (SHA1) to one baked by the viewer, and the
  render is byte-identical to `fresh.png`/`cached.png`/`blob.png`. bunny bake ~8 s, venus ~2.5 s.
- `mesh/NaniteBlob.{h,cpp}` — format: header + section table + 256-byte-aligned POD payloads.
  Magic `NANITE_BLOB_MAGIC 0x4244414E` ('NADB'), `NANITE_BLOB_EXTENSION ".fcc"`,
  `NANITE_BLOB_VERSION 2`, `NANITE_BUILDER_VERSION 1`. Sections: `VERTICES, INDICES, CLUSTER_INFO,
  ERROR_INFO, BVH_NODES, SORTED_CLUSTER_INDICES, LOD_CLUSTER_NUMS, BVH_ROOTS, SOURCE_PATH`.
  Version 2 added `SECTION_SOURCE_PATH`: the blob records where it came from, so a lone `.fcc` is
  self-describing and can still be re-hashed. It is *informational* — a blob stays usable when
  that mesh is gone. `bakeBlob` stores it **absolute and canonical** (`weakly_canonical`): a
  relative path baked by one tool would be resolved against the viewer's cwd and silently hash to
  0, and it is what keeps a blob byte-identical no matter how the model was addressed — verified:
  `nanite_bake assets\models\bunny.gltf`, `nanite_bake C:/Workspaces/.../bunny.gltf` and a viewer
  `--nanite-rebuild` all produce the same SHA256.
- Invalidation: FNV-1a 64-bit `sourceHash` over the source mesh plus a `configHash` over the
  build configuration. A mismatch makes `read()` reject the blob. The strict source check runs at
  the **very end** of `read()` because `asset.sourcePath` may only become known through
  `SECTION_SOURCE_PATH`; `fnv1aFile()` returns 0 for a missing file and 0 skips the check, which
  is deliberate — a shipped blob whose mesh was deleted still loads.
- Cache location: `<model>_naniteCache/` **next to the source mesh** (`NaniteMesh::cachePathFor`),
  holding `LOD_*.obj` and `nanite_info.json`. The trailing separator comes from
  `std::filesystem::path::preferred_separator`, so it is not Windows-only.
- Blob location: `<model>.fcc`, a **sibling** of the mesh (`NaniteMesh::blobPathFor`). That
  function is idempotent for a `.fcc` input, which is what lets the viewer hand a picked `.fcc`
  straight to it; `NaniteMesh::isBlobPath()` recognises one (case-insensitive extension).
  Generated `.fcc` files are git-ignored (`assets/models/*.fcc`) — they are output, not input.
- Writing the cache is **best effort**: `NaniteMesh::serialize` returns `bool` and never asserts.
  If the directory or the JSON file cannot be written (read-only folder, network share, disk full)
  it logs `[cache] cannot create ... - continuing without a cache` and `initNaniteInfo` skips
  `bakeBlob`. A model picked with the file dialog from such a folder still renders; it is simply
  rebuilt on the next run. Keep it that way — a viewer must never abort over a cache.
- `NaniteMesh::BlobMode { Auto, JsonOnly, Require, Rebuild }` ← viewer flags
  `--nanite-blob` (Require), `--nanite-json` (JsonOnly), `--nanite-rebuild` (Rebuild).
  `Auto` prefers the blob, falls back to JSON, then builds, and always re-bakes after a build.
- Blob path skips OpenMesh/METIS/JSON entirely: `NaniteScene::linkBlobBVHNodeInfos()` links
  the baked BVHs arithmetically instead of rebuilding and re-flattening a `shared_ptr` tree.
- If you change anything that affects the baked payloads (GPU struct layout, cluster size,
  LOD count, index/vertex packing), **bump `NANITE_BLOB_VERSION` or `NANITE_BUILDER_VERSION`**
  so old blobs are rejected instead of rendering garbage.
- A mesh's BVH node 0 is a `VIRTUAL_NODE` that the BFS never emits — its children are the
  per-LOD roots stored in `SECTION_BVH_ROOTS`. Any "every baked node must be reachable"
  check has to skip it.
- Measured on `bunny`: fresh build ~8 s, JSON cache ~8 s, blob ~1.7 s. Blob render is
  bit-identical to the JSON-cache render (`mean abs diff 0.0000` at tol 0).
- **The viewer can open a `.fcc` instead of a glTF.** `--model bunny.fcc` (or the file dialog, which
  offers a `*.fcc` filter) goes through `NaniteMesh::initNaniteInfoFromBlob()` →
  `loadBlobFile()`: no glTF is parsed, `models.object` is only *cleared*, `NaniteMesh::meshes` stays
  empty, and every consumer branches on `blobLoaded`. `render_pbrtexture_blob_only` asserts that
  image is bit-identical to the blob-backed glTF render, so the `.fcc` really is self contained.
  In `pbrtexture.cpp`, `loadNaniteModel()` is the single place that loads a model (glTF *or* blob)
  and assembles the scene — `loadAssets()` and `reloadModel()` both call it, so the two paths
  cannot drift apart. A blob that fails to load throws `std::runtime_error`; the reload call sites
  catch it, `loadAssets()` does not.

## Rust asset pipeline

`rust/` is a second, independent implementation of the *asset* half of the pipeline: glTF in,
`<model>.fcc` out. No Vulkan, no OpenMesh, no METIS, no CMake. The C++ builder is untouched and is
still what tests 1–13 use; `nanite-bake-rs` is not a replacement for it, it is a parallel producer of
the same contract. Workspace: `nanite-blob` (format) + `nanite-bake-rs` (builder + CLI),
`resolver = "2"`, deps pinned in `[workspace.dependencies]` (anyhow, bytemuck, clap, glam 0.29,
gltf 1.4 with `extras`, indicatif, thiserror 2, tracing, tracing-subscriber).

- `rust/nanite-blob/` is the format crate and the mirror of `mesh/NaniteBlob.{h,cpp}`. Encoding and
  decoding are **hand-rolled little-endian POD**, deliberately *not* `#[repr(C)]` + bytemuck: the C++
  structs have `alignas` padding that a Rust `repr(C)` mirror would lay out differently (the trailing
  `ErrorInfo` pad is the trap). Every offset is pinned by a test — `header_offsets`,
  `blob_bvh_node_offsets`, `cluster_info_offsets`, `error_info_offsets`, `vertex_offsets`,
  `struct_sizes_match_the_c_abi` — plus `config_hash_matches_the_c_builder`
  (`14_829_784_758_405_755_283`) and `fnv1a_matches_the_reference_vectors`. Two tests read the real
  `assets/models/bunny.fcc`: `reads_the_cpp_baked_blob` and `rewriting_the_cpp_blob_is_lossless`
  (re-encoding it must produce byte-identical output). If you change the format, change the C++
  version constant *and* these tests together.
- `rust/nanite-bake-rs/` reproduces the builder invariants the viewer actually depends on:
  `grow_regions` (a single-FIFO wavefront region grower) stands in for METIS k-way, and
  `bounding_sphere` / `build_tree` / `update_error_core` / `flatten` are faithful ports of
  `Mesh::getBoundingSphere`, `Mesh::buildBVH`, `Mesh::updateBVHError` and `NaniteMesh::flattenBVH` —
  including the quirks (the growing sphere never moves its centre; a non-leaf AABB merge walks
  `clusterGroups[start..end]` by *direct* index while the sort happens on a permutation; a non-leaf
  keeps `start = end = -1`).
- **It emits one LOD, not six.** There is no OpenMesh QEM decimation to port, so `lodNums = 1`,
  `normalized_lod_error = 0` and `parentError = LAST_LOD_PARENT_ERROR` (`1e5`). That is legal for the
  format and safe for the viewer **only** because the raster buffers were resized (see the gotcha
  above) — a single-LOD blob draws every triangle of the model. Consequence: `--visualize 1` shows the
  Rust render entirely red (LOD 0) where the golden is blue/yellow, and the triangle counts differ by
  ~5×. Do not "fix" that by leaving `errorWorld` at its C++ defaults (`-1` / `-FLT_MAX`): the selector
  then culls *everything* and the render is empty.
- `check_asset()` in `emit.rs` is the pre-write gate: it re-checks the viewer's hard asserts
  (`triangleIndicesEnd <= indexCount/3`, `start < end`, `<= CLUSTER_MAX_SIZE` triangles per cluster,
  non-inverted AABBs, `sorted_cluster_indices` covering every cluster exactly once, LEAF `start < end`,
  NODE `start == end`) **plus** a world-space LOD gate — every cluster needs
  `errorWorld.x <= RENDER_THRESHOLD < errorWorld.y`, because `culling.comp` keeps a cluster only when
  `errorData.x <= threshold && errorData.y > threshold`. The shader multiplies the world error by
  `getScreenBoundRadiusSq` (a *pixel-count squared* factor, `error.comp:50`), so this is a necessary
  bound, not the exact test; it is what catches the default-`errorWorld` failure mode that passes
  every structural check and then renders as scraps. `--no-check` skips it.
- `canonical()` in `emit.rs` is a **lexical** normalisation, not `std::fs::canonicalize`: the latter
  returns a `\\?\C:\...` prefix on Windows, which would make the stored `sourcePath` differ from the
  C++ `weakly_canonical` result and break byte-identical output.
- CLI: `nanite-bake-rs [--rebuild] [--out <path>] [--verify] [--dump] [--no-check] [--log <level>]
  <model.gltf>…`. `--verify` re-reads a blob in a *separate* process and asserts the five struct
  sizes against the C++ ABI, bailing on any mismatch. **Always pass `--out` when baking a Rust blob
  next to a C++ one** — the default is `blob_path_for(input)`, i.e. it would overwrite `<model>.fcc`,
  which is the reference image for tests 3/8/13. The test suite uses `bunny_rs.fcc`.
- Measured on `bunny`: Rust bake **0.1 s** / 10,119,216 B (1 LOD, 2575 clusters, 260 BVH nodes);
  C++ bake ~8 s / 20,068,144 B (6 LODs, 15450 clusters). Rendered through the C++ viewer at the
  default threshold: mean abs diff `0.71`–`0.93`, `1.3–1.9%` bad pixels against the golden. Pinning
  both blobs to LOD 0 (`--threshold 0`) narrows that to `0.26` / `0.52%`, which is the residual from
  the different cluster partition — bit equality would require porting OpenMesh's decimation order
  and METIS, and is not the goal.

## Camera and "View all"

The viewer frames the model automatically: `viewAll()` is called at the **end of
`loadNaniteModel()`**, which is the single place a model enters the scene, so start-up and runtime
reload frame it identically for free. Interactive entry points: `V` (`KEY_V`) and the ImGui
*View all* / *Reset camera* buttons. `--capture` disables the overlay, so tests rely purely on the
automatic call — there is no `--view-all` flag and none is needed.

- `NaniteScene::worldBounds(pMin, pMax)` (`mesh/NaniteScene.{h,cpp}`) is the bounds source, and it
  is the only one that exists on **both** load paths: a blob-only load (`--model bunny.fcc`)
  clears `models.object` and never builds an OpenMesh mesh, so vkglTF's node dimensions are not
  available there. It walks `NaniteScene::clusterInfo` per instance and applies
  `Instance::rootTransform`.
  Those entries are **mesh-local** (only `triangleIndicesStart/End` get offset by
  `clusterIndexOffsets`), and the per-mesh range end is `clusterIndexOffsets[meshIdx + 1]` —
  `clusterIndexCounts` stores an **end index, not a count**. All eight AABB corners have to be
  transformed, not just two: `createScene2()` rotates instance 0 by −90° about Y.
  Do *not* use `bvhNodeInfos` for bounds — their intervals are per-LOD and deep LODs cover less
  than the model.
- `base/camera.hpp` sign traps, all verified the hard way (the first `viewAll()` rendered only the
  skybox):
  - pbrtexture uses `CameraType::firstperson`, where `matrices.view = rotM * transM` — rotation
    before translation, i.e. the camera orbits its own origin.
  - `Camera::position` is the **negated eye** (`viewPos = position * (-1,1,-1,1)`), so placing the
    eye means `camera.setPosition(-eye)`.
  - `Camera::getFront()` is the **negative** of the actual look direction in this convention. The
    convention-proof way is `-normalize(mat3(inverse(camera.matrices.view))[2])`.
- Near/far are derived from the model (`near = max(0.01, d - 1.1r)`, `far = max(d + 1.6r, 16)`).
  The **floor of 16 on the far plane is load-bearing**: `shaders/glsl/pbrtexture/skybox.vert`
  multiplies by the *camera* projection and `assets/models/cube.gltf` spans ±5 (corner 8.66), so a
  smaller far plane clips the environment away.
- `cameraFov/cameraNear/cameraFar` and `defaultCameraRotation/defaultCameraPosition` are members
  because `viewAll()` overwrites near/far: without them, *Reset camera* would restore the previous
  model's frustum. `viewAllMargin` (1.15) is how much larger than exactly fitting the frustum the
  framing is.
- Measured framing (`[view all] …` on stdout): bunny / `bunny.fcc` center (-0.149, -0.831, 1.557),
  radius 2.392, eye distance 5.501, near 2.870, far 16; venus radius 2.192, distance 5.043;
  dragon radius 3.14, distance 7.22.
- **All four goldens were re-baselined for this framing** (`pbrtexture_bunny.png`,
  `pbrtexture_bunny_vis1/2/3.png`). If you change the framing, the pose, `viewAllMargin`, or the
  instance transforms in `createScene2()`, every golden has to be redone.

## Struct layouts are ABI, not implementation details

`mesh/GpuTypes.h` holds `BVHNodeInfo`, `ClusterInfo` and `ErrorInfo` with `sizeof`/`offsetof`
`static_assert`s. These structs are memcpy'd into storage buffers *and* into the blob file, so
the GLSL structs in `shaders/glsl/pbrtexture/` must mirror them exactly.

Verified in this build (`glm::vec3` is 12 bytes / 4-byte aligned, which trips people up):

| struct | sizeof | notes |
| --- | --- | --- |
| `BVHNodeInfo` | 112 | alignof 16 |
| `ClusterInfo` | 48 | `triangleIndicesStart/End/objectIdx` at **28 / 32 / 36**, not 32/36/40 |
| `ErrorInfo` | 48 | alignof 16 |
| `BlobBVHNode` | 96 | on-disk BVH node, alignof 16 |
| `vkglTF::Vertex` | 112 | `base/VulkanglTFModel.h` |

`NaniteBVHNodeInfo` (`mesh/NaniteBVH.h`) is the *in-memory* builder node (doubles,
`std::vector` children) and is **not** POD — never write it to disk or to a buffer.

## Runtime model reload

The viewer can swap the nanite-ified model while running: the ImGui button *Load model...* or `O`
(`KEY_O`) opens a native file dialog, and `--reload <file> [--reloadframe N]` plus
`--reload2 <file> [--reload2frame N]` feed the same entry point non-interactively (the dialog is
modal, so a test cannot drive it). `handlePendingCommandLineReload()` applies the requests in frame
order, at most one per frame, so two reloads cannot collapse into a single call.

- `pbrtexture.cpp`: `requestModelDialog()` sets a flag that `handlePendingModelDialog()` consumes
  at the **top of `render()`**. Never open the dialog from `OnUpdateUIOverlay` — the ImGui frame is
  open there and `nextFrame()` calls `updateOverlay()` *after* `render()`, so a blocking modal would
  re-enter the renderer while a command buffer is being recorded.
- `reloadModel()` is the whole teardown/rebuild: `vkDeviceWaitIdle`, free every nanite buffer, reset
  `scene`/`naniteMesh`/`clusterinfos`/`errorinfos`, `loadNaniteModel()` (glTF or `.fcc`), then
  re-run the *model dependent* steps of `prepare()` and finish with `writeDescriptors()` and
  `buildCommandBuffers()`. One-time work (`preparePipelines`, `setupRenderPass`, `createHiZBuffer`,
  `createRasterizeBuffer`, `createHWRasterizeFramebuffer`, `prepareUniformBuffers`, the env-cube and
  BRDF generators) is deliberately skipped — those buffers are untouched by a reload.
- `setupDescriptorLayouts()` and `writeDescriptors()` are **separate functions** for exactly this
  reason: layouts are created once, bindings are rewritten on every reload. `prepare()` calls both.
- `vkglTF::Model::loadFromFile` **appends**; calling it twice without `Model::clear()` duplicates
  every node, buffer and texture. `clear()` releases the vertex/index buffers, textures, nodes,
  skins, the model's descriptor pool and `emptyTexture`, but *not* the process-wide
  `vkglTF::descriptorSetLayoutUbo/Image` globals — destroying those would invalidate every other
  model still in use (`models.skybox`, `models.cube`). Those globals are now never destroyed
  explicitly; they go away with the device, and `~Model()` is just `clear()`.
- **`vkglTF::Model::device` and `descriptorPool` must stay value-initialized.** On the blob path
  (`--model bunny.fcc`) `models.object` is *never loaded* and every reload still calls
  `models.object.clear()`; the `if (device == nullptr) return;` guard only helps if `device` is
  actually null. Uninitialized it was a wild read — proven from a WER minidump:
  `0xC0000005` reading `0xC4008` at `movq 0x8(%rcx),%rcx` (`device->logicalDevice`, `rcx = 0xC4000`)
  right before `vkDestroyDescriptorPool`, i.e. `pbrtexture.exe+0x7FA8E`. `clear()` also resets
  `device = nullptr` so it is idempotent. Same rule for `NaniteMesh`'s raw pointers
  (`vkglTFModel`, `vkglTFMesh`, `device`, `model`, the tinygltf pair): none of them is ever set on
  the blob path, and a `NaniteMesh` is default-constructed and re-assigned on every reload.
- `vkglTF::Model::Vertices`/`Indices` are zero-initialized so a reload can tell "never created"
  from "created"; keep it that way if you add fields.
- **Never call `vks::Buffer::destroy()` on the pbrtexture storage buffers.** They are created
  through the raw `createBuffer(VkBuffer*, VkDeviceMemory*)` overload, which never fills in
  `Buffer::device`, so `destroy()` would pass an uninitialized `VkDevice` to `vkDestroyBuffer`.
  Use the example's `freeBuffer()`/`freeVkBuffers()` helpers, and reset to `{}` afterwards —
  `destroy()` also leaves the handles dangling.
- `Mesh::createVertexBuffer()` leaks its `VkBuffer` when called again, so the old per-LOD buffers
  have to be destroyed explicitly before re-creating them (`freeNaniteMeshBuffers`).
- `NaniteMesh::setModelPath(const char*)` only stores the pointer — keep the string alive
  (`pbrtexture.cpp` keeps it in the `naniteModelDir` member).
- `base/FileDialog.{h,cpp}` wraps `GetOpenFileNameW` (UTF-8 in/out). `WINLIBS` is referenced in the
  CMake files but never `set()`, so system libraries must be listed explicitly — `comdlg32` and
  `comctl32` are both in `base/CMakeLists.txt`. The dialog is a *common control*: it calls
  `InitCommonControlsEx` once per process (without it the first dialog still opens off the shell
  fallback and a later one faults inside Comctl32) and the viewer passes its own `HWND` as
  `ofn.hwndOwner` so the modal dialog pumps messages through a real owner window.
  Verified: `pbrtexture.exe` imports `COMDLG32.dll` **and** `COMCTL32.dll`.
- **Not our bug, but it looks like one:** a crash in the *second* file dialog on this machine is
  third-party. Its minidump is a DEP execute violation at `comctl32.dll+0x1E9778` — inside `.rdata`, past
  the `fothk` hook section — and the stack is full of `windows.storage.dll` / `comdlg32.dll` frames
  interleaved with `umppc21309.dll` and `CsXumd64_21309.dll`, i.e. the **CrowdStrike Falcon Sensor**
  support modules. Check for injected DLLs in a crash dump before blaming the viewer.
- Reloading a model whose `_naniteCache/` directory cannot be created is fine: `serialize`
  reports `[cache] cannot create ... - continuing without a cache` and the model still renders
  (verified by pointing the cache path at a plain file). It used to hit
  `ASSERT(0, "Error creating directory")`.
  A missing file is handled gracefully (`[reload] no such file: ...`).

## Conventions

- `mesh/CMakeLists.txt` and `base/CMakeLists.txt`: `mesh` uses explicit `headers`/`sources`
  lists, so **new files in `mesh/` must be added there**. `base/` globs `*.cpp`/`*.h`, so new
  files there are picked up automatically (after a CMake re-run).
- **METIS must stay out of the shared mesh headers.** `metis` is linked `PRIVATE` on `mesh`, and
  the only file that includes `<metis.h>` is `mesh/MetisGraph.cpp`; `mesh/Graph.h` is metis-free
  and `mesh/IndexTypes.h` provides `metis_idx_t` (int32, matching `IDXTYPEWIDTH=32`). There is no
  `metis.h` under `build/vcpkg_installed`, so nothing else *can* include it — keep it that way.
  OpenMesh, by contrast, still leaks through `Common.h`/`Mesh.h`/`ClusterGroup.h`/`NaniteMesh.h`.
- `examples/CMakeLists.txt`: `buildExample(NAME SPIRV_DEPS USE_MESH_LIB)` and the
  `MESH_EXAMPLES` list decide whether an example links `mesh` + OpenMesh. Only `pbrtexture` is in
  that list; adding a nanite example means adding it there. Verified with
  `dumpbin /DEPENDENTS`: `pbribl.exe` imports only `vulkan-1.dll`, `pbrtexture.exe` also imports
  `OpenMeshCore.dll`, `OpenMeshTools.dll`, `COMDLG32.dll`, `COMCTL32.dll`.
- Shaders live in `shaders/glsl/<example>/` and are compiled to `.spv` by a custom command
  using `glslangValidator`; the example depends on those outputs.
- Formatting follows the file: tabs in `pbrtexture.cpp` and `NaniteMesh.cpp`, 4 spaces in
  `NaniteScene.cpp`. Don't reformat surrounding code.
- `base/CommandLineParser.hpp` has `add/isSet/getValueAsString/getValueAsInt` — there is **no**
  `getValueAsBool`; flags are `hasValue=false` + `isSet`. `getValueAsInt` returns the parsed value
  even when it is `0` or negative (it only falls back to the default if nothing parsed), which is what
  makes `--threshold 0` work — do not reintroduce an `> 0` guard.
- `VulkanExampleBase` has no reload/restart hook. Available virtuals: `render`, `viewChanged`,
  `keyPressed`, `mouseMoved`, `windowResized`, `buildCommandBuffers`, `setupDepthStencil`,
  `setupFrameBuffer`, `setupRenderPass`, `getEnabledFeatures`,
  `getEnabled{Device,Instance}Extensions`, `prepare`, `OnUpdateUIOverlay`, and
  `OnHandleMessage(HWND,UINT,WPARAM,LPARAM)` on Win32. Anything that swaps a model has to
  destroy and recreate the scene buffers itself (see `reloadModel()` in `pbrtexture.cpp`).
- `external/imgui/` contains **core ImGui only** — there is no file-dialog widget, so the
  viewer's open dialog uses the Win32 common dialog (`base/FileDialog.h`).
- `VulkanDescriptorSetManager` is a singleton whose layouts/sets are created once
  (`createLayoutsAndSets` returns early if nothing was added). Reloads must therefore
  *rewrite* existing sets (`writeToSet`) rather than re-adding layouts, and must not destroy
  buffers referenced by a layout without re-writing those bindings.

## Where to look

- Viewer / render loop: `examples/pbrtexture/pbrtexture.cpp` — `createScene2()` (1134),
  `loadNaniteModel()` (1204), `resetCamera()` (1262), `viewAll()` (1283), `loadAssets()` (1325),
  `reloadModel()` (1447), `setupDescriptorLayouts()` (1513), `writeDescriptors()` (1649),
  `prepare()` (4247), `handlePendingCommandLineReload()` (4284),
  `handlePendingModelDialog()` (4306), `render()` (4332), `keyPressed()` (4346),
  `OnUpdateUIOverlay()` (4361).
- Camera framing: `viewAll()` / `resetCamera()` in `pbrtexture.cpp` (see *Camera and "View all"*),
  bounds from `NaniteScene::worldBounds()` in `mesh/NaniteScene.cpp`, key constants in
  `base/keycodes.hpp` (`KEY_V`, all six platform blocks).
- Cluster visualization: `renderingPushConstants.vis_clusters` (0..3) is set by the *Visualize
  Clusters* slider and by `--visualize`; the modes are implemented in `shaders/glsl/pbrtexture/`
  `shading.frag` (main pass), `pbrtexture.frag` (HW-rasterized pass) and `merger.comp` (mode 3
  changes the merge itself). See the *Test* section for how each mode is pinned.
- Builder: `mesh/NaniteMesh.cpp` (`generateNaniteInfo`, `serialize`/`deserialize`,
  `bakeBlob`/`loadBlob`/`loadBlobFile`/`initNaniteInfoFromBlob`, `initNaniteInfo`,
  `cachePathFor`/`blobPathFor`/`isBlobPath`), `mesh/Mesh.cpp` (clustering, BVH),
  `tools/nanite_bake/nanite_bake.cpp` (headless front end of the same builder).
- Scene assembly: `mesh/NaniteScene.cpp` (`createNaniteSceneInfo` and its blob/legacy branches).
- Rust asset pipeline: `rust/nanite-blob/src/lib.rs` (format: header, section table, POD encode/
decode, `fnv1a`, `config_hash`, `read`/`write`) and `rust/nanite-blob/src/tests.rs` (the ABI pins);
`rust/nanite-bake-rs/src/{gltf_load,cluster,bvh,emit,main}.rs` (glTF -> `LodMesh` -> clusters ->
BVH -> `Asset` -> `.fcc`, and the CLI). See *Rust asset pipeline*.
- Base framework: `base/vulkanexamplebase.{h,cpp}`, `base/VulkanUIOverlay.{h,cpp}`.
