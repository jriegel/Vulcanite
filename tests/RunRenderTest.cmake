# Runs one render test: renders a frame with the example in capture mode and compares it to a reference image.
# Invoked through `cmake -P` by CTest. Parameters (-D):
#   RENDER_EXE   example executable (e.g. pbrtexture)
#   COMPARE_EXE  imagecompare executable
#   MODEL        model name under assets/models
#   ASSETS_DIR   assets directory (used to clean the nanite cache)
#   OUT          where the captured PNG is written
#   REFERENCE    image to compare against (golden image, or an earlier capture)
#   FRAMES       frames to render before capturing (default 10)
#   CLEAN_CACHE  if set, delete the model's nanite cache first so the whole pipeline is rebuilt
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

if(CLEAN_CACHE)
    file(REMOVE_RECURSE "${ASSETS_DIR}/models/${MODEL}_naniteCache")
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
