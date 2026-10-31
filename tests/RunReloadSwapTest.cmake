# Renders the same model twice: once loaded at startup, once swapped in at runtime through the
# reload path (`--reload`, the non interactive twin of the viewer's file dialog). Both captures must
# show the same model, which proves the reload actually replaces the scene and not just the buffers.
#
# Unlike RunRenderTest.cmake this comparison is *not* bit exact: the runtime LOD/culling state is
# temporal, so a frame rendered after a reload at frame N differs from a frame of a model that was
# loaded at startup. The tolerances below are still tight enough to catch a reload that silently
# failed - the previous model would then still be on screen, which differs by an order of magnitude
# more (see the numbers in the comment at the bottom).
#
# Invoked through `cmake -P` by CTest. Parameters (-D):
#   RENDER_EXE    example executable
#   COMPARE_EXE   imagecompare executable
#   MODEL         model loaded at startup
#   RELOAD_MODEL  model file (under assets/models) swapped in at runtime
#   ASSETS_DIR    assets directory
#   OUT           where the captured PNG is written
#   FRAMES        frames to render (default 40)
#   RELOAD_FRAME  frame at which the reload happens (default 5)
#   RENDER_ARGS   extra arguments for the example executable (list)

foreach(var RENDER_EXE COMPARE_EXE MODEL RELOAD_MODEL OUT)
    if(NOT DEFINED ${var})
        message(FATAL_ERROR "RunReloadSwapTest: ${var} is not set")
    endif()
endforeach()
if(NOT DEFINED FRAMES)
    set(FRAMES 40)
endif()
if(NOT DEFINED RELOAD_FRAME)
    set(RELOAD_FRAME 5)
endif()

set(reload_file "${ASSETS_DIR}/models/${RELOAD_MODEL}.gltf")
if(NOT EXISTS "${reload_file}")
    message(FATAL_ERROR "RunReloadSwapTest: ${reload_file} does not exist")
endif()

file(REMOVE "${OUT}" "${OUT}.direct.png")
get_filename_component(OUT_DIR "${OUT}" DIRECTORY)
file(MAKE_DIRECTORY "${OUT_DIR}")

# 1. MODEL at startup, RELOAD_MODEL swapped in at runtime.
execute_process(
    COMMAND "${RENDER_EXE}" --capture "${OUT}" --captureframes ${FRAMES} --model ${MODEL}
            --reload "${reload_file}" --reloadframe ${RELOAD_FRAME} ${RENDER_ARGS}
    RESULT_VARIABLE reload_result
    TIMEOUT 900)
if(NOT reload_result EQUAL 0)
    message(FATAL_ERROR "Reload render failed (exit code/result: ${reload_result})")
endif()
if(NOT EXISTS "${OUT}")
    message(FATAL_ERROR "Reload render finished but did not write ${OUT}")
endif()

# 2. RELOAD_MODEL loaded the normal way, as the reference.
execute_process(
    COMMAND "${RENDER_EXE}" --capture "${OUT}.direct.png" --captureframes ${FRAMES}
            --model ${RELOAD_MODEL} ${RENDER_ARGS}
    RESULT_VARIABLE direct_result
    TIMEOUT 900)
if(NOT direct_result EQUAL 0)
    message(FATAL_ERROR "Reference render failed (exit code/result: ${direct_result})")
endif()
if(NOT EXISTS "${OUT}.direct.png")
    message(FATAL_ERROR "Reference render finished but did not write ${OUT}.direct.png")
endif()

execute_process(
    COMMAND "${COMPARE_EXE}" "${OUT}" "${OUT}.direct.png" --diff "${OUT}.diff.png"
            --mean-tol 5 --pixel-tol 16 --max-bad 10
    RESULT_VARIABLE compare_result)
if(NOT compare_result EQUAL 0)
    message(FATAL_ERROR "Image comparison failed (result: ${compare_result}), see ${OUT} and ${OUT}.diff.png")
endif()

# Measured on venus: reload vs direct = mean 1.64 / 3.1% bad pixels, while bunny (i.e. a reload that
# did nothing) vs venus = mean 9.1 / 17.0% bad pixels.
