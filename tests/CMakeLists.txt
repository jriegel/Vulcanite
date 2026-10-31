add_executable(imagecompare imagecompare.cpp)
set_target_properties(imagecompare PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin/")

set(TEST_OUTPUT_DIR "${CMAKE_BINARY_DIR}/test_output")
set(RENDER_TEST_MODEL bunny)
set(RENDER_TEST_GOLDEN "${CMAKE_CURRENT_SOURCE_DIR}/golden/pbrtexture_${RENDER_TEST_MODEL}.png")

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

# Self-test of the comparison tool: an image compares equal to itself and the tool reports missing files.
add_test(NAME imagecompare_selftest
    COMMAND imagecompare ${RENDER_TEST_GOLDEN} ${RENDER_TEST_GOLDEN} --mean-tol 0 --max-bad 0)
set_tests_properties(imagecompare_selftest PROPERTIES REQUIRED_FILES ${RENDER_TEST_GOLDEN})
