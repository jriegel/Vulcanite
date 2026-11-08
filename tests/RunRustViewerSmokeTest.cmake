# Loads and renders every dataset in the **Rust** viewer and asserts each one produced a PNG.
# Invoked through `cmake -P` by CTest. Parameters (-D):
#   VIEWER_RS_EXE  nanite-viewer-rs executable
#   ASSETS_DIR     assets directory (models are resolved under <ASSETS_DIR>/models)
#   OUT_DIR        directory the captures are written to
#   MODELS         list of `--model` arguments to try
#   FRAMES         frames to render before capturing (default 10)
#
# Why this is not an `imagecompare` test: the datasets are compared against the C++ viewer
# individually (see the `render_rust_viewer_*` tests in CMakeLists.txt), and several of them cannot
# be. `nanite_test` crashes the **C++ builder** (`nanite_bake --rebuild` dies with 0xC0000005 right
# after "LOD 3 generated", a pre-existing bug in the same family as teapot/sphere/voyager), so there
# is no C++ reference image to compare against at all - the Rust builder handles it fine. What this
# script can assert is the thing that actually regresses: that each source loads through either the
# `.fcc` reader or the Rust asset pipeline, that the scene is non-empty, and that a frame is written.

foreach(var VIEWER_RS_EXE ASSETS_DIR OUT_DIR MODELS)
    if(NOT DEFINED ${var})
        message(FATAL_ERROR "RunRustViewerSmokeTest: ${var} is not set")
    endif()
endforeach()
if(NOT DEFINED FRAMES)
    set(FRAMES 10)
endif()

file(MAKE_DIRECTORY "${OUT_DIR}")

foreach(model ${MODELS})
    set(out "${OUT_DIR}/rust_smoke_${model}.png")
    file(REMOVE "${out}")
    message(STATUS "[rust viewer] ${model}")
    execute_process(
        COMMAND "${VIEWER_RS_EXE}" --capture "${out}" --captureframes ${FRAMES} --model ${model}
        RESULT_VARIABLE render_result
        TIMEOUT 900
        OUTPUT_VARIABLE render_out
        ERROR_VARIABLE render_out
        OUTPUT_STRIP_TRAILING_WHITESPACE)
    if(NOT render_result EQUAL 0)
        message(FATAL_ERROR "[rust viewer] ${model} failed (exit code/result: ${render_result}):\n${render_out}")
    endif()
    if(NOT EXISTS "${out}")
        message(FATAL_ERROR "[rust viewer] ${model} exited 0 but wrote no ${out}")
    endif()
    # The viewer logs `[scene] blob|baked <path>: <n> triangles, ...`; a scene that loaded nothing
    # would still write a skybox-only PNG, so pin the counters instead of trusting the exit code.
    if(NOT render_out MATCHES "\\[scene\\] (blob|baked) [^ ]*: ([0-9]+) triangles, ([0-9]+) clusters")
        message(FATAL_ERROR "[rust viewer] ${model}: no [scene] summary in the output:\n${render_out}")
    endif()
    if(CMAKE_MATCH_2 LESS 1000)
        message(FATAL_ERROR "[rust viewer] ${model}: only ${CMAKE_MATCH_2} triangles loaded")
    endif()
    message(STATUS "[rust viewer] ${model}: ${CMAKE_MATCH_2} triangles, ${CMAKE_MATCH_3} clusters")
endforeach()
