# Bakes a model with the standalone asset-pipeline tool and re-reads the result in a *separate*
# process. Parameters (-D):
#   BAKE_EXE   the bake tool (nanite_bake)
#   ASSETS_DIR assets directory
#   MODEL      model name under assets/models
# The second run is `--verify`, which does not build and does not open Vulkan: it only reads
# <model>.fcc back from disk and checks magic, versions, struct sizes, the section table and
# the strict source hash. Without it a self-consistent but broken blob would go unnoticed.
# The blob is verified twice: once addressed through the glTF and once by its own path, which
# is how a viewer opens a shipped blob without ever knowing the mesh it came from.

foreach(var BAKE_EXE ASSETS_DIR MODEL)
    if(NOT DEFINED ${var})
        message(FATAL_ERROR "RunBakeTest: ${var} is not set")
    endif()
endforeach()

set(cache_dir "${ASSETS_DIR}/models/${MODEL}_naniteCache")
set(blob "${ASSETS_DIR}/models/${MODEL}.fcc")
file(REMOVE_RECURSE "${cache_dir}")
file(REMOVE "${blob}")

execute_process(
    COMMAND "${BAKE_EXE}" --rebuild "${ASSETS_DIR}/models/${MODEL}.gltf"
    RESULT_VARIABLE bake_result
    TIMEOUT 900)
if(NOT bake_result EQUAL 0)
    message(FATAL_ERROR "Bake failed (exit code/result: ${bake_result})")
endif()
if(NOT EXISTS "${blob}")
    message(FATAL_ERROR "Bake succeeded but wrote no ${blob}")
endif()

execute_process(
    COMMAND "${BAKE_EXE}" --verify "${ASSETS_DIR}/models/${MODEL}.gltf"
    RESULT_VARIABLE verify_result
    TIMEOUT 300)
if(NOT verify_result EQUAL 0)
    message(FATAL_ERROR "Verifying the baked blob failed (exit code/result: ${verify_result})")
endif()

execute_process(
    COMMAND "${BAKE_EXE}" --verify "${blob}"
    RESULT_VARIABLE blob_verify_result
    TIMEOUT 300)
if(NOT blob_verify_result EQUAL 0)
    message(FATAL_ERROR "Verifying ${blob} by path failed (exit code/result: ${blob_verify_result})")
endif()
