# Runs one render test: renders a frame with the example in capture mode and compares it to a reference image.
# Invoked through `cmake -P` by CTest. Parameters (-D):
#   RENDER_EXE   example executable (e.g. pbrtexture)
#   COMPARE_EXE  imagecompare executable
#   MODEL        model name under assets/models, without extension (`bunny`) or the baked blob
#                itself (`bunny.fcc`) - the latter is rendered without loading its glTF
#   ASSETS_DIR   assets directory (used to clean the nanite cache)
#   OUT          where the captured PNG is written
#   REFERENCE    image to compare against (golden image, or an earlier capture)
#   FRAMES       frames to render before capturing (default 10)
#   CLEAN_CACHE  if set, delete the model's nanite cache and blob first so the whole pipeline
#                is rebuilt
#   BAKE_EXE     if set, delete the cache and build it with this tool (nanite_bake) instead of
#                letting the example build it itself - this is what proves the two-stage split
#   RENDER_ARGS  extra arguments for the example executable (list), e.g. --nanite-blob
#   COMPARE_ARGS extra imagecompare arguments (list)
# Set the environment variable VULCANITE_UPDATE_GOLDEN=1 to overwrite REFERENCE instead of comparing.

foreach(var RENDER_EXE COMPARE_EXE MODEL OUT REFERENCE)
    if(NOT DEFINED ${var})
        message(FATAL_ERROR "RunRenderTest: ${var} is not set")
    endif()
endforeach()
if(NOT DEFINED FRAMES)
    set(FRAMES 10)
endif()

# `--model bunny.fcc` renders the baked blob, but the cache and the source mesh are still named
# after the model, so every path below is derived from the extension-less name.
string(REGEX REPLACE "\\.[^.]+$" "" MODEL_BASE "${MODEL}")
set(cache_dir "${ASSETS_DIR}/models/${MODEL_BASE}_naniteCache")
set(blob "${ASSETS_DIR}/models/${MODEL_BASE}.fcc")

if(CLEAN_CACHE)
    file(REMOVE_RECURSE "${cache_dir}")
    file(REMOVE "${blob}")
endif()

# The asset-pipeline stage: a separate process builds the caches, so the render below must
# produce the same image without running OpenMesh/METIS itself.
if(DEFINED BAKE_EXE)
    file(REMOVE_RECURSE "${cache_dir}")
    file(REMOVE "${blob}")
    execute_process(
        COMMAND "${BAKE_EXE}" --rebuild "${ASSETS_DIR}/models/${MODEL_BASE}.gltf"
        RESULT_VARIABLE bake_result
        TIMEOUT 900)
    if(NOT bake_result EQUAL 0)
        message(FATAL_ERROR "Bake failed (exit code/result: ${bake_result})")
    endif()
    if(NOT EXISTS "${blob}")
        message(FATAL_ERROR "Bake succeeded but wrote no ${blob}")
    endif()
endif()

file(REMOVE "${OUT}")
get_filename_component(OUT_DIR "${OUT}" DIRECTORY)
file(MAKE_DIRECTORY "${OUT_DIR}")

execute_process(
    COMMAND "${RENDER_EXE}" --capture "${OUT}" --captureframes ${FRAMES} --model ${MODEL} ${RENDER_ARGS}
    RESULT_VARIABLE render_result
    TIMEOUT 900)
if(NOT render_result EQUAL 0)
    message(FATAL_ERROR "Render failed (exit code/result: ${render_result})")
endif()
if(NOT EXISTS "${OUT}")
    message(FATAL_ERROR "Render finished but did not write ${OUT}")
endif()

set(update_args "")
if("$ENV{VULCANITE_UPDATE_GOLDEN}" STREQUAL "1")
    set(update_args --update)
endif()

execute_process(
    COMMAND "${COMPARE_EXE}" "${OUT}" "${REFERENCE}" --diff "${OUT}.diff.png" ${COMPARE_ARGS} ${update_args}
    RESULT_VARIABLE compare_result)
if(NOT compare_result EQUAL 0)
    message(FATAL_ERROR "Image comparison failed (result: ${compare_result}), see ${OUT} and ${OUT}.diff.png")
endif()
