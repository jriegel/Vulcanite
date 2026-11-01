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

# Self-test of the comparison tool: an image compares equal to itself and the tool reports missing files.
add_test(NAME imagecompare_selftest
    COMMAND imagecompare ${RENDER_TEST_GOLDEN} ${RENDER_TEST_GOLDEN} --mean-tol 0 --max-bad 0)
set_tests_properties(imagecompare_selftest PROPERTIES REQUIRED_FILES ${RENDER_TEST_GOLDEN})
