# Bakes a model with the standalone asset-pipeline tool and re-reads the result in a *separate*
# process. Parameters (-D):
#   BAKE_EXE   the bake tool (nanite_bake)
#   ASSETS_DIR assets directory
#   MODEL      model name under assets/models
# The second run is `--verify`, which does not build and does not open Vulkan: it only reads
# <model>_naniteCache/nanite.blob back from disk and checks magic, versions, struct sizes,
# the section table and the strict source hash. Without it a self-consistent but broken blob
# would go unnoticed.

foreach(var BAKE_EXE ASSETS_DIR MODEL)
    if(NOT DEFINED ${var})
        message(FATAL_ERROR "RunBakeTest: ${var} is not set")
    endif()
endforeach()

set(cache_dir "${ASSETS_DIR}/models/${MODEL}_naniteCache")
file(REMOVE_RECURSE "${cache_dir}")

execute_process(
    COMMAND "${BAKE_EXE}" --rebuild "${ASSETS_DIR}/models/${MODEL}.gltf"
    RESULT_VARIABLE bake_result
    TIMEOUT 900)
if(NOT bake_result EQUAL 0)
    message(FATAL_ERROR "Bake failed (exit code/result: ${bake_result})")
endif()
if(NOT EXISTS "${cache_dir}/nanite.blob")
    message(FATAL_ERROR "Bake succeeded but wrote no ${cache_dir}/nanite.blob")
endif()

execute_process(
    COMMAND "${BAKE_EXE}" --verify "${ASSETS_DIR}/models/${MODEL}.gltf"
    RESULT_VARIABLE verify_result
    TIMEOUT 300)
if(NOT verify_result EQUAL 0)
    message(FATAL_ERROR "Verifying the baked blob failed (exit code/result: ${verify_result})")
endif()
