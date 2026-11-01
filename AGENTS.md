# AGENTS.md

Working notes for AI agents (and humans) editing this repository. Everything here has been
verified against the tree, not guessed. When you find something wrong, update this file.

## What this project is

Vulkan C++17 renderer implementing a Nanite-style cluster/LOD pipeline. The example base is
derived from Sascha Willems' Vulkan examples (`base/`), the Nanite pipeline lives in `mesh/`,
the viewer is `examples/pbrtexture/pbrtexture.cpp` (~4000 lines, the only example that uses the
mesh library), and `tools/nanite_bake/` is the headless asset-pipeline front end of the builder.

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

**Do not use the CMake Tools extension tools here**: `ListBuildTargets` and `ListTests`
return empty for this project even though the solution builds fine. Use the terminal.

## Test

```
ctest --test-dir build -C Release --output-on-failure
```

Nine tests, all green as of writing (`render_pbrtexture_fresh`, `render_pbrtexture_cached`,
`render_pbrtexture_blob`, `render_pbrtexture_reload`, `render_pbrtexture_reload_swap`,
`render_pbrtexture_baked_by_tool`, `nanite_bake_verify`, `render_pbrtexture_blob_only`,
`imagecompare_selftest`). The render tests are `RUN_SERIAL` with a fixture chain: *fresh* builds
the caches, everything else reuses them.

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
  no `COMPARE_ARGS` at all, so they run with those. Do not "verify" a render by hand with
  `--mean-tol 0 --pixel-tol 0 --max-bad 0` and conclude something is broken: the bunny render is
  *not* bit-identical to the golden image (mean abs diff 0.0076, 2.05% pixels differ by >0) and
  never was — that is GPU/timing noise, invisible in the `--diff` PNG. Only tests 3, 4 and 5
  actually demand bit equality, and each of them compares against an *earlier capture*, not the
  golden image.
- `tests/RunReloadSwapTest.cmake` is the reload harness: it renders `MODEL` at startup, reloads
  `RELOAD_MODEL` at `RELOAD_FRAME` via `--reload`, then renders `RELOAD_MODEL` normally and
  compares the two. It has **no** `REFERENCE` parameter — the reference is generated.
- Re-baseline the golden image with `$env:VULCANITE_UPDATE_GOLDEN = "1"` before ctest.

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
- **The render is frame-count sensitive.** `--captureframes 40` and `--captureframes 41` of the
  *same* model differ (mean abs diff ~1.7, ~3.5% bad pixels) because the LOD/culling state is
  temporal (`lastView/lastProj` occlusion culling, HiZ). Only compare captures with an identical
  frame count, and never expect two runs that differ in *when* a model appeared to be bit
  identical — that is why `render_pbrtexture_reload_swap` uses `--mean-tol 5 --pixel-tol 16`.
- `cmake -P script.cmake -DVAR=x` does **not** set the variables — every `-D` has to come
  *before* `-P`, i.e. `cmake -DVAR=x -P script.cmake`.
- Known pre-existing builder bug: `--model teapot` / `--model sphere` dies with an access
  violation (`0xC0000005`) at the end of `NaniteMesh::generateNaniteInfo` — small meshes reach
  `clusterGroupNum == 1`, so the last LODs are identical copies. Reproduced at HEAD with a
  `git stash`, so it is not a regression. Nothing is cached in that case (`serialize` never runs),
  which is why `--nanite-blob` then asserts "no blob cache exists". Safe models to bake: `bunny`,
  `venus`, `dragon`, `nanite_test*`.

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
(`KEY_O`) opens a native file dialog, and `--reload <file> [--reloadframe N]` feeds the same entry
point non-interactively (the dialog is modal, so a test cannot drive it).

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
  model still in use (`models.skybox`, `models.cube`).
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
  CMake files but never `set()`, so system libraries must be listed explicitly — `comdlg32` was
  added to `base/CMakeLists.txt` for this.
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
  `OpenMeshCore.dll`, `OpenMeshTools.dll`, `COMDLG32.dll`.
- Shaders live in `shaders/glsl/<example>/` and are compiled to `.spv` by a custom command
  using `glslangValidator`; the example depends on those outputs.
- Formatting follows the file: tabs in `pbrtexture.cpp` and `NaniteMesh.cpp`, 4 spaces in
  `NaniteScene.cpp`. Don't reformat surrounding code.
- `base/CommandLineParser.hpp` has `add/isSet/getValueAsString/getValueAsInt` — there is **no**
  `getValueAsBool`; flags are `hasValue=false` + `isSet`.
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

- Viewer / render loop: `examples/pbrtexture/pbrtexture.cpp` — `loadNaniteModel()` (1174),
  `loadAssets()` (1223), `reloadModel()` (1345), `setupDescriptorLayouts()` (1411),
  `writeDescriptors()` (1547), `prepare()` (4140), `handlePendingModelDialog()` (4195),
  `render()` (4221), `keyPressed()` (4235), `OnUpdateUIOverlay()` (4247).
- Builder: `mesh/NaniteMesh.cpp` (`generateNaniteInfo`, `serialize`/`deserialize`,
  `bakeBlob`/`loadBlob`/`loadBlobFile`/`initNaniteInfoFromBlob`, `initNaniteInfo`,
  `cachePathFor`/`blobPathFor`/`isBlobPath`), `mesh/Mesh.cpp` (clustering, BVH),
  `tools/nanite_bake/nanite_bake.cpp` (headless front end of the same builder).
- Scene assembly: `mesh/NaniteScene.cpp` (`createNaniteSceneInfo` and its blob/legacy branches).
- Base framework: `base/vulkanexamplebase.{h,cpp}`, `base/VulkanUIOverlay.{h,cpp}`.
