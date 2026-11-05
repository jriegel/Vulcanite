# Bakes a model with the **Rust** asset pipeline (`rust/nanite-bake-rs`) and renders the result with
# the C++ viewer. This is the smoke test for the Rust port: the blob it writes has to be accepted by
# `NaniteMesh::loadBlobFile` unchanged, and the picture has to be the same model. Parameters (-D):
#   RENDER_EXE     pbrtexture
#   COMPARE_EXE    imagecompare
#   BAKE_RS_EXE    nanite-bake-rs
#   ASSETS_DIR     assets directory
#   MODEL          model name under assets/models (`bunny`)
#   OUT            where the captured PNG is written
#   REFERENCE      golden image for the default (LOD-selected) render
#   FRAMES         frames to render before capturing (default 10)
#   BLOB_RS        the blob the Rust tool writes - deliberately *not* `<model>.fcc`, which is the
#                  C++ builder's output and the reference of tests 3/8/13. Overwriting it would make
#                  the C++ tests compare against the Rust blob.
#
# Two comparisons, because the Rust builder produces a *single* LOD (no OpenMesh decimation, no METIS
# collapse order to reproduce), so it always draws LOD 0 at full detail while the C++ blob draws a
# mix of LOD levels:
#   1. default render vs the golden image, at imagecompare's default tolerances - proves the model,
#      camera, culling and visibility buffer are all right.
#   2. `--threshold 0` on both blobs, which pins the C++ blob to LOD 0 as well, so the two are
#      drawing the same triangles. That is the comparison which would catch a wrong cluster, a wrong
#      index range or a wrong AABB, instead of only "roughly the same silhouette".

foreach(var RENDER_EXE COMPARE_EXE BAKE_RS_EXE ASSETS_DIR MODEL OUT REFERENCE)
    if(NOT DEFINED ${var})
        message(FATAL_ERROR "RunRustBlobTest: ${var} is not set")
    endif()
endforeach()
if(NOT DEFINED FRAMES)
    set(FRAMES 10)
endif()
if(NOT DEFINED BLOB_RS)
    set(BLOB_RS "${ASSETS_DIR}/models/${MODEL}_rs.fcc")
endif()

get_filename_component(OUT_DIR "${OUT}" DIRECTORY)
file(MAKE_DIRECTORY "${OUT_DIR}")

# 1. Bake with Rust. `--out` keeps the C++ blob untouched; `--rebuild` because the blob is a build
#    output (git-ignored) and the test has to exercise the builder, not a stale file.
file(REMOVE "${BLOB_RS}")
execute_process(
    COMMAND "${BAKE_RS_EXE}" --rebuild --out "${BLOB_RS}" "${ASSETS_DIR}/models/${MODEL}.gltf"
    RESULT_VARIABLE bake_result
    TIMEOUT 900)
if(NOT bake_result EQUAL 0)
    message(FATAL_ERROR "Rust bake failed (exit code/result: ${bake_result})")
endif()
if(NOT EXISTS "${BLOB_RS}")
    message(FATAL_ERROR "Rust bake succeeded but wrote no ${BLOB_RS}")
endif()

# 2. Re-read it in a separate process: magic, versions, the struct sizes the C++ ABI requires, the
#    section table and the strict source hash. Same contract `nanite_bake_verify` checks for C++.
execute_process(
    COMMAND "${BAKE_RS_EXE}" --verify "${BLOB_RS}"
    RESULT_VARIABLE verify_result
    TIMEOUT 300)
if(NOT verify_result EQUAL 0)
    message(FATAL_ERROR "Verifying ${BLOB_RS} failed (exit code/result: ${verify_result})")
endif()

# 3. Render it. `--model <name>.fcc` opens the blob without parsing any glTF, so the only thing in
#    common between this and the C++ pipeline is the file format.
set(blob_rs_name "${MODEL}_rs.fcc")
file(REMOVE "${OUT}")
execute_process(
    COMMAND "${RENDER_EXE}" --capture "${OUT}" --captureframes ${FRAMES} --model ${blob_rs_name} --nanite-blob
    RESULT_VARIABLE render_result
    TIMEOUT 900)
if(NOT render_result EQUAL 0)
    message(FATAL_ERROR "Render of the Rust blob failed (exit code/result: ${render_result})")
endif()
if(NOT EXISTS "${OUT}")
    message(FATAL_ERROR "Render finished but did not write ${OUT}")
endif()

set(update_args "")
if("$ENV{VULCANITE_UPDATE_GOLDEN}" STREQUAL "1")
    set(update_args --update)
endif()
execute_process(
    COMMAND "${COMPARE_EXE}" "${OUT}" "${REFERENCE}" --diff "${OUT}.diff.png" ${update_args}
    RESULT_VARIABLE compare_result)
if(NOT compare_result EQUAL 0)
    message(FATAL_ERROR "Rust blob render differs from ${REFERENCE} (result: ${compare_result}), see ${OUT} and ${OUT}.diff.png")
endif()

# 4. The LOD-pinned comparison. `--threshold 0` makes the selector keep every LOD 0 cluster of the
#    C++ blob too, so both images are the full-detail surface of the same mesh.
set(cpp_lod0 "${OUT}.cpp_lod0.png")
set(rs_lod0 "${OUT}.rs_lod0.png")
file(REMOVE "${cpp_lod0}" "${rs_lod0}")
execute_process(
    COMMAND "${RENDER_EXE}" --capture "${cpp_lod0}" --captureframes ${FRAMES} --model ${MODEL}.fcc --nanite-blob --threshold 0
    RESULT_VARIABLE cpp_lod_result
    TIMEOUT 900)
if(NOT cpp_lod_result EQUAL 0)
    message(FATAL_ERROR "LOD-0 render of ${MODEL}.fcc failed (exit code/result: ${cpp_lod_result})")
endif()
execute_process(
    COMMAND "${RENDER_EXE}" --capture "${rs_lod0}" --captureframes ${FRAMES} --model ${blob_rs_name} --nanite-blob --threshold 0
    RESULT_VARIABLE rs_lod_result
    TIMEOUT 900)
if(NOT rs_lod_result EQUAL 0)
    message(FATAL_ERROR "LOD-0 render of ${blob_rs_name} failed (exit code/result: ${rs_lod_result})")
endif()
foreach(image "${cpp_lod0}" "${rs_lod0}")
    if(NOT EXISTS "${image}")
        message(FATAL_ERROR "LOD-0 render finished but did not write ${image}")
    endif()
endforeach()

execute_process(
    COMMAND "${COMPARE_EXE}" "${rs_lod0}" "${cpp_lod0}" --diff "${OUT}.lod0.diff.png"
    RESULT_VARIABLE lod_compare_result)
if(NOT lod_compare_result EQUAL 0)
    message(FATAL_ERROR "Rust LOD-0 render differs from the C++ LOD-0 render (result: ${lod_compare_result}), see ${rs_lod0}, ${cpp_lod0} and ${OUT}.lod0.diff.png")
endif()
