add_executable(imagecompare imagecompare.cpp)
set_target_properties(imagecompare PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin/")

set(TEST_OUTPUT_DIR "${CMAKE_BINARY_DIR}/test_output")
set(RENDER_TEST_MODEL bunny)
set(RENDER_TEST_GOLDEN "${CMAKE_CURRENT_SOURCE_DIR}/golden/pbrtexture_${RENDER_TEST_MODEL}.png")
# The "Visualize Clusters" modes (0..3) each have their own golden image, named after the mode.
set(VIS_MODES 1 2 3)

# Common parameters for every render test
set(RENDER_TEST_COMMON
    -DRENDER_EXE=$<TARGET_FILE:pbrtexture>
    -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
    -DMODEL=${RENDER_TEST_MODEL}
    -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
    -DFRAMES=10)

# 1. Whole pipeline from scratch: load glTF -> build clusters/LODs/BVH (METIS, OpenMesh) -> write cache
#    -> upload -> render -> compare with the golden image.
add_test(NAME render_pbrtexture_fresh
    COMMAND ${CMAKE_COMMAND} ${RENDER_TEST_COMMON}
        -DOUT=${TEST_OUTPUT_DIR}/fresh.png
        -DREFERENCE=${RENDER_TEST_GOLDEN}
        -DCLEAN_CACHE=1
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)

# 2. Same, but the nanite data is loaded from the JSON cache written by test 1: the result must match the golden image too.
add_test(NAME render_pbrtexture_cached
    COMMAND ${CMAKE_COMMAND} ${RENDER_TEST_COMMON}
        -DOUT=${TEST_OUTPUT_DIR}/cached.png
        -DREFERENCE=${RENDER_TEST_GOLDEN}
        -DRENDER_ARGS=--nanite-json
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)

# 3. Viewer path: the nanite data comes from the binary blob baked by test 1, so no OpenMesh/METIS
#    work and no JSON parsing happens. The payload is byte identical to the builder's, so the image
#    must match the JSON-cache render exactly.
add_test(NAME render_pbrtexture_blob
    COMMAND ${CMAKE_COMMAND} ${RENDER_TEST_COMMON}
        -DOUT=${TEST_OUTPUT_DIR}/blob.png
        -DREFERENCE=${TEST_OUTPUT_DIR}/cached.png
        -DRENDER_ARGS=--nanite-blob
        -DCOMPARE_ARGS=--mean-tol\;0\;--pixel-tol\;0\;--max-bad\;0
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)
set_tests_properties(render_pbrtexture_fresh PROPERTIES FIXTURES_SETUP nanite_cache)
set_tests_properties(render_pbrtexture_cached render_pbrtexture_blob PROPERTIES FIXTURES_REQUIRED nanite_cache)
set_tests_properties(render_pbrtexture_fresh render_pbrtexture_cached render_pbrtexture_blob PROPERTIES TIMEOUT 1200 RUN_SERIAL TRUE)

# 4. Runtime reload: the model is swapped at frame 5 through the very same entry point the file
#    dialog feeds (`--reload` exists only because the dialog is modal and cannot be driven from a
#    test). Reloading the same model has to yield a pixel identical image, which proves that the
#    buffers, descriptor sets and command buffers are all rebuilt consistently.
add_test(NAME render_pbrtexture_reload
    COMMAND ${CMAKE_COMMAND} ${RENDER_TEST_COMMON}
        -DOUT=${TEST_OUTPUT_DIR}/reload.png
        -DREFERENCE=${TEST_OUTPUT_DIR}/blob.png
        -DRENDER_ARGS=--nanite-blob\;--reload\;${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}.gltf\;--reloadframe\;5
        -DCOMPARE_ARGS=--mean-tol\;0\;--pixel-tol\;0\;--max-bad\;0
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)
set_tests_properties(render_pbrtexture_reload PROPERTIES FIXTURES_REQUIRED nanite_cache TIMEOUT 1200 RUN_SERIAL TRUE)

# 5. Reload a *different* model: bunny is loaded at startup, venus is swapped in at frame 5, and the
#    result is compared against venus loaded the normal way. Catches a reload that leaves the old
#    scene on screen. Not bit exact on purpose - the runtime LOD/culling state is temporal, so the
#    frame after a reload can never equal a frame of a model that was loaded at startup.
add_test(NAME render_pbrtexture_reload_swap
    COMMAND ${CMAKE_COMMAND}
        -DRENDER_EXE=$<TARGET_FILE:pbrtexture>
        -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
        -DMODEL=${RENDER_TEST_MODEL}
        -DRELOAD_MODEL=venus
        -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
        -DOUT=${TEST_OUTPUT_DIR}/reload_swap.png
        -DFRAMES=40
        -DRELOAD_FRAME=5
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunReloadSwapTest.cmake)
set_tests_properties(render_pbrtexture_reload_swap PROPERTIES FIXTURES_REQUIRED nanite_cache TIMEOUT 1200 RUN_SERIAL TRUE)

# 6. The two-stage split: `nanite_bake` builds the caches in its own process (no window, no
#    swapchain) and the viewer then renders them with --nanite-blob. The image has to match the
#    golden image just like the in-viewer build does, which proves the blob - not the builder -
#    is what the viewer depends on. Compared with imagecompare's default tolerances, exactly like
#    tests 1 and 2.
add_test(NAME render_pbrtexture_baked_by_tool
    COMMAND ${CMAKE_COMMAND} ${RENDER_TEST_COMMON}
        -DOUT=${TEST_OUTPUT_DIR}/baked_by_tool.png
        -DREFERENCE=${RENDER_TEST_GOLDEN}
        -DBAKE_EXE=$<TARGET_FILE:nanite_bake>
        -DRENDER_ARGS=--nanite-blob
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)
set_tests_properties(render_pbrtexture_baked_by_tool PROPERTIES FIXTURES_REQUIRED nanite_cache TIMEOUT 1200 RUN_SERIAL TRUE)

# 7. The bake tool on its own: rebuild a model nothing else caches (venus) and re-read the result.
#    `--verify` never opens a blob the builder itself wrote in the same process, so this checks the
#    file on disk: magic, versions, struct sizes, section table and the strict source hash.
add_test(NAME nanite_bake_verify
    COMMAND ${CMAKE_COMMAND}
        -DBAKE_EXE=$<TARGET_FILE:nanite_bake>
        -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
        -DMODEL=venus
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunBakeTest.cmake)
set_tests_properties(nanite_bake_verify PROPERTIES TIMEOUT 900 RUN_SERIAL TRUE)

# 8. Opening a baked blob *instead of* a mesh: `--model bunny.fcc` loads the standalone blob the
#    tool baked in test 6, so no glTF is parsed and OpenMesh/METIS/JSON are never touched. The
#    image has to be bit identical to the blob-backed glTF render of test 3, which proves the
#    blob really is a self contained asset - the .fcc alone is enough to draw the model.
add_test(NAME render_pbrtexture_blob_only
    COMMAND ${CMAKE_COMMAND}
        -DRENDER_EXE=$<TARGET_FILE:pbrtexture>
        -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
        -DMODEL=${RENDER_TEST_MODEL}.fcc
        -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
        -DFRAMES=10
        -DOUT=${TEST_OUTPUT_DIR}/blob_only.png
        -DREFERENCE=${TEST_OUTPUT_DIR}/blob.png
        -DCOMPARE_ARGS=--mean-tol\;0\;--pixel-tol\;0\;--max-bad\;0
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)
set_tests_properties(render_pbrtexture_blob_only PROPERTIES
    FIXTURES_REQUIRED nanite_cache TIMEOUT 1200 RUN_SERIAL TRUE
    REQUIRED_FILES ${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}.fcc)

# 9-11. The other cluster visualization modes. `--visualize N` is the non-interactive equivalent of
#    the "Visualize Clusters" slider (the overlay is switched off during --capture), so each mode is
#    rendered and compared with its own golden image: 1 = LOD level, 2 = cluster / cluster group id,
#    3 = raw visibility buffer id. Together with the shaded golden of test 1 that pins all four modes.
#    Regenerate them with VULCANITE_UPDATE_GOLDEN=1 before ctest.
foreach(vis_mode ${VIS_MODES})
    add_test(NAME render_pbrtexture_vis${vis_mode}
        COMMAND ${CMAKE_COMMAND} ${RENDER_TEST_COMMON}
            -DOUT=${TEST_OUTPUT_DIR}/vis${vis_mode}.png
            -DREFERENCE=${CMAKE_CURRENT_SOURCE_DIR}/golden/pbrtexture_${RENDER_TEST_MODEL}_vis${vis_mode}.png
            -DRENDER_ARGS=--nanite-blob\;--visualize\;${vis_mode}
            -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)
    set_tests_properties(render_pbrtexture_vis${vis_mode} PROPERTIES
        FIXTURES_REQUIRED nanite_cache TIMEOUT 1200 RUN_SERIAL TRUE
        REQUIRED_FILES ${CMAKE_CURRENT_SOURCE_DIR}/golden/pbrtexture_${RENDER_TEST_MODEL}_vis${vis_mode}.png)
endforeach()

# 12. The visualization modes must actually differ from each other and from the shaded image. Without
#     this, a viewer that ignored --visualize would render the shaded image four times, every golden
#     would silently be a copy of the shaded one, and tests 1-11 would all stay green.
add_test(NAME render_pbrtexture_vis_modes_differ
    COMMAND ${CMAKE_COMMAND}
        -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
        -DIMAGES=${TEST_OUTPUT_DIR}/blob.png\;${TEST_OUTPUT_DIR}/vis1.png\;${TEST_OUTPUT_DIR}/vis2.png\;${TEST_OUTPUT_DIR}/vis3.png
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunImagesDifferTest.cmake)
set_tests_properties(render_pbrtexture_vis_modes_differ PROPERTIES
    FIXTURES_REQUIRED nanite_cache TIMEOUT 300 RUN_SERIAL TRUE
    REQUIRED_FILES ${TEST_OUTPUT_DIR}/vis3.png)

# 13. Loading a blob *twice at runtime*: `bunny.fcc` is opened at startup and then re-loaded two
#     more times through the reload path - the regression test for the crash when a `.fcc` was picked
#     a second time in the file dialog. On the blob path `models.object` is never loaded, so every
#     reload tears down an unloaded `vkglTF::Model`, and an uninitialized `Model::device` turned that
#     into a wild read at `vkDestroyDescriptorPool`. The image must stay bit identical to the single
#     blob-only render of test 8, so a reload that silently drops the scene fails too.
add_test(NAME render_pbrtexture_blob_reload_twice
    COMMAND ${CMAKE_COMMAND}
        -DRENDER_EXE=$<TARGET_FILE:pbrtexture>
        -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
        -DMODEL=${RENDER_TEST_MODEL}.fcc
        -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
        -DFRAMES=10
        -DOUT=${TEST_OUTPUT_DIR}/blob_reload_twice.png
        -DREFERENCE=${TEST_OUTPUT_DIR}/blob_only.png
        -DRENDER_ARGS=--reload\;${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}.fcc\;--reloadframe\;5\;--reload2\;${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}.fcc\;--reload2frame\;8
        -DCOMPARE_ARGS=--mean-tol\;0\;--pixel-tol\;0\;--max-bad\;0
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)
set_tests_properties(render_pbrtexture_blob_reload_twice PROPERTIES
    FIXTURES_REQUIRED nanite_cache TIMEOUT 1200 RUN_SERIAL TRUE
    REQUIRED_FILES ${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}.fcc)

# 14. The **Rust** asset pipeline: `rust/nanite-bake-rs` bakes `bunny_rs.fcc` from the same glTF the
#     C++ builder reads, and the C++ viewer renders that blob. Nothing but the `.fcc` format is
#     shared, so this is the smoke test for the port: a Rust-written blob has to survive
#     `NaniteMesh::loadBlobFile`'s magic/version/struct-size/config-hash/strict-source-hash checks and
#     draw the model. Built by cargo rather than CMake, so its path is a cache variable and a missing
#     executable skips the test instead of failing it - build it with
#     `cargo build --release -p nanite-bake-rs`.
set(NANITE_BAKE_RS_EXE "${CMAKE_SOURCE_DIR}/rust/target/release/nanite-bake-rs.exe"
    CACHE FILEPATH "nanite-bake-rs executable (cargo build --release -p nanite-bake-rs)")
add_test(NAME render_pbrtexture_rust_blob
    COMMAND ${CMAKE_COMMAND}
        -DRENDER_EXE=$<TARGET_FILE:pbrtexture>
        -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
        -DBAKE_RS_EXE=${NANITE_BAKE_RS_EXE}
        -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
        -DMODEL=${RENDER_TEST_MODEL}
        -DBLOB_RS=${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}_rs.fcc
        -DFRAMES=10
        -DOUT=${TEST_OUTPUT_DIR}/rust_blob.png
        -DREFERENCE=${RENDER_TEST_GOLDEN}
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRustBlobTest.cmake)
set_tests_properties(render_pbrtexture_rust_blob PROPERTIES
    FIXTURES_REQUIRED nanite_cache TIMEOUT 1200 RUN_SERIAL TRUE
    REQUIRED_FILES "${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}.fcc;${NANITE_BAKE_RS_EXE}")
if(NOT EXISTS "${NANITE_BAKE_RS_EXE}")
    set_tests_properties(render_pbrtexture_rust_blob PROPERTIES DISABLED TRUE)
endif()

# 15-20. The **Rust** viewer (`rust/nanite-viewer-rs`): a wgpu/WGSL/egui re-implementation of
#      `pbrtexture` that shares nothing with the C++ renderer but the `.fcc` format and the WGSL
#      ports of the GLSL shaders. It is built by cargo, not CMake, so its path is a cache variable
#      and a missing executable *disables* the tests instead of failing them - build it with
#      `cargo build -p nanite-viewer-rs`. Its CLI mirrors pbrtexture's flag for flag, which is why
#      `RunRenderTest.cmake` drives it unchanged: only `RENDER_EXE` differs.
#      Measured against the C++ goldens at 1920x1080: shaded 0.0077 / vis1 0.0221 / vis2 0.0087 /
#      vis3 0.0055 mean abs diff, ~0.013% bad pixels - far inside imagecompare's defaults, and that
#      slack is kept on purpose because a different GPU may pick a different LOD set.
set(NANITE_VIEWER_RS_EXE "${CMAKE_SOURCE_DIR}/rust/target/debug/nanite-viewer-rs.exe"
    CACHE FILEPATH "nanite-viewer-rs executable (cargo build -p nanite-viewer-rs)")
set(RUST_VIEWER_COMMON
    -DRENDER_EXE=${NANITE_VIEWER_RS_EXE}
    -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
    -DMODEL=${RENDER_TEST_MODEL}
    -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
    -DFRAMES=10)

# 15. The shaded image through the Rust viewer. `--model bunny` resolves to `bunny.gltf` and `Auto`
#     prefers the sibling `bunny.fcc` baked by the C++ builder, so this exercises the Rust blob
#     reader, the whole WGSL port (BVH traversal, cull, both rasterizers, the merge, PBR + IBL) and
#     the Rust `viewAll()` framing against the same golden the C++ viewer is pinned to.
add_test(NAME render_rust_viewer_bunny
    COMMAND ${CMAKE_COMMAND} ${RUST_VIEWER_COMMON}
        -DOUT=${TEST_OUTPUT_DIR}/rust_viewer_bunny.png
        -DREFERENCE=${RENDER_TEST_GOLDEN}
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)

# 16-18. The other visualization modes, pinned to their own goldens exactly like tests 9-11.
foreach(vis_mode ${VIS_MODES})
    add_test(NAME render_rust_viewer_vis${vis_mode}
        COMMAND ${CMAKE_COMMAND} ${RUST_VIEWER_COMMON}
            -DOUT=${TEST_OUTPUT_DIR}/rust_viewer_vis${vis_mode}.png
            -DREFERENCE=${CMAKE_CURRENT_SOURCE_DIR}/golden/pbrtexture_${RENDER_TEST_MODEL}_vis${vis_mode}.png
            -DRENDER_ARGS=--nanite-blob\;--visualize\;${vis_mode}
            -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)
endforeach()

# 19. Opening the blob *instead of* the mesh: bit identical to test 15, so the `.fcc` alone is enough
#     for the Rust viewer to draw the model - no glTF and no asset pipeline involved. The same claim
#     as C++ test 8, and it holds bit-exactly here too because both runs read the same payload.
add_test(NAME render_rust_viewer_blob_only
    COMMAND ${CMAKE_COMMAND}
        -DRENDER_EXE=${NANITE_VIEWER_RS_EXE}
        -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
        -DMODEL=${RENDER_TEST_MODEL}.fcc
        -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
        -DFRAMES=10
        -DOUT=${TEST_OUTPUT_DIR}/rust_viewer_blob_only.png
        -DREFERENCE=${TEST_OUTPUT_DIR}/rust_viewer_bunny.png
        -DCOMPARE_ARGS=--mean-tol\;0\;--pixel-tol\;0\;--max-bad\;0
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)

# 20. Reloading a blob **twice** in the Rust viewer - the port of the C++ test 13 regression (picking
#     a `.fcc` a second time in the file dialog used to fault on an uninitialized
#     `vkglTF::Model::device`). Bit exact against test 19, so a reload that silently drops the scene
#     or leaves the previous one behind fails too.
add_test(NAME render_rust_viewer_blob_reload_twice
    COMMAND ${CMAKE_COMMAND}
        -DRENDER_EXE=${NANITE_VIEWER_RS_EXE}
        -DCOMPARE_EXE=$<TARGET_FILE:imagecompare>
        -DMODEL=${RENDER_TEST_MODEL}.fcc
        -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
        -DFRAMES=10
        -DOUT=${TEST_OUTPUT_DIR}/rust_viewer_blob_reload_twice.png
        -DREFERENCE=${TEST_OUTPUT_DIR}/rust_viewer_blob_only.png
        -DRENDER_ARGS=--reload\;${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}.fcc\;--reloadframe\;5\;--reload2\;${CMAKE_SOURCE_DIR}/assets/models/${RENDER_TEST_MODEL}.fcc\;--reload2frame\;8
        -DCOMPARE_ARGS=--mean-tol\;0\;--pixel-tol\;0\;--max-bad\;0
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)

# 21. Every dataset, in the Rust viewer. Deliberately *not* image-compared: `nanite_test` crashes the
#     **C++ builder** (`nanite_bake --rebuild` dies with 0xC0000005 right after "LOD 3 generated", the
#     same pre-existing small-mesh bug family as teapot/sphere/voyager), so for that model no C++
#     reference image exists at all - while the Rust builder handles it fine. What is asserted instead
#     is that each source loads with a non-empty scene and that a frame is written. Under `Auto` that
#     means the `.fcc` reader for `bunny`/`bunny.fcc`/`bunny_rs.fcc`/`venus`/`nanite_test_plane` (each
#     has a sibling blob) and in-process `nanite-bake-rs` for `dragon` and `nanite_test`, which have
#     no `.fcc` - that is the point of those two entries.
add_test(NAME render_rust_viewer_datasets
    COMMAND ${CMAKE_COMMAND}
        -DVIEWER_RS_EXE=${NANITE_VIEWER_RS_EXE}
        -DASSETS_DIR=${CMAKE_SOURCE_DIR}/assets
        -DOUT_DIR=${TEST_OUTPUT_DIR}
        -DMODELS=${RENDER_TEST_MODEL}\;${RENDER_TEST_MODEL}.fcc\;${RENDER_TEST_MODEL}_rs.fcc\;venus\;dragon\;nanite_test\;nanite_test_plane
        -DFRAMES=10
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRustViewerSmokeTest.cmake)

foreach(rust_test render_rust_viewer_bunny render_rust_viewer_vis1 render_rust_viewer_vis2
                  render_rust_viewer_vis3 render_rust_viewer_blob_only
                  render_rust_viewer_blob_reload_twice render_rust_viewer_datasets)
    set_tests_properties(${rust_test} PROPERTIES
        FIXTURES_REQUIRED nanite_cache TIMEOUT 1200 RUN_SERIAL TRUE)
endforeach()
if(NOT EXISTS "${NANITE_VIEWER_RS_EXE}")
    set_tests_properties(render_rust_viewer_bunny render_rust_viewer_vis1 render_rust_viewer_vis2
        render_rust_viewer_vis3 render_rust_viewer_blob_only render_rust_viewer_blob_reload_twice
        render_rust_viewer_datasets PROPERTIES DISABLED TRUE)
endif()

# Self-test of the comparison tool: an image compares equal to itself and the tool reports missing files.
add_test(NAME imagecompare_selftest
    COMMAND imagecompare ${RENDER_TEST_GOLDEN} ${RENDER_TEST_GOLDEN} --mean-tol 0 --max-bad 0)
set_tests_properties(imagecompare_selftest PROPERTIES REQUIRED_FILES ${RENDER_TEST_GOLDEN})
