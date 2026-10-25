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

# 2. Same, but the nanite data is loaded from the cache written by test 1: the result must match the golden image too.
add_test(NAME render_pbrtexture_cached
    COMMAND ${CMAKE_COMMAND} ${RENDER_TEST_COMMON}
        -DOUT=${TEST_OUTPUT_DIR}/cached.png
        -DREFERENCE=${RENDER_TEST_GOLDEN}
        -P ${CMAKE_CURRENT_SOURCE_DIR}/RunRenderTest.cmake)
set_tests_properties(render_pbrtexture_fresh PROPERTIES FIXTURES_SETUP nanite_cache)
set_tests_properties(render_pbrtexture_cached PROPERTIES FIXTURES_REQUIRED nanite_cache)
set_tests_properties(render_pbrtexture_fresh render_pbrtexture_cached PROPERTIES TIMEOUT 1200 RUN_SERIAL TRUE)

# Self-test of the comparison tool: an image compares equal to itself and the tool reports missing files.
add_test(NAME imagecompare_selftest
    COMMAND imagecompare ${RENDER_TEST_GOLDEN} ${RENDER_TEST_GOLDEN} --mean-tol 0 --max-bad 0)
set_tests_properties(imagecompare_selftest PROPERTIES REQUIRED_FILES ${RENDER_TEST_GOLDEN})
