# Asserts that a set of captured images are *not* identical to each other.
# Invoked through `cmake -P` by CTest. Parameters (-D):
#   COMPARE_EXE  imagecompare executable
#   IMAGES       list of PNGs; every pair must differ
#   PIXEL_TOL    per-channel difference above which a pixel counts as different (default 8)
#
# This exists to keep the visualization tests honest: if the viewer ignored the visualization
# mode, every capture would be the shaded image, each golden would be a copy of the previous one,
# and all of those tests would pass forever. Comparing them against each other catches that.

foreach(var COMPARE_EXE IMAGES)
    if(NOT DEFINED ${var})
        message(FATAL_ERROR "RunImagesDifferTest: ${var} is not set")
    endif()
endforeach()
if(NOT DEFINED PIXEL_TOL)
    set(PIXEL_TOL 8)
endif()
# `-DIMAGES=a;b;c` arrives as one semicolon separated string, so the count has to be derived here -
# there is no auto-generated IMAGES_COUNT for a -D argument.
list(LENGTH IMAGES image_count)
if(image_count LESS 2)
    message(FATAL_ERROR "RunImagesDifferTest: need at least two images, got ${image_count}")
endif()

# imagecompare exits 0 when the images match within tolerance, so "must differ" means "must exit
# non-zero". --mean-tol 0 --max-bad 0 makes even a single differing pixel a mismatch.
set(pair_count 0)
math(EXPR last "${image_count} - 1")
foreach(i RANGE 0 ${last})
    list(GET IMAGES ${i} a)
    math(EXPR j_start "${i} + 1")
    if(j_start GREATER ${last})
        continue()
    endif()
    foreach(j RANGE ${j_start} ${last})
        list(GET IMAGES ${j} b)
        math(EXPR pair_count "${pair_count} + 1")
        execute_process(
            COMMAND "${COMPARE_EXE}" "${a}" "${b}" --mean-tol 0 --pixel-tol ${PIXEL_TOL} --max-bad 0
            RESULT_VARIABLE same_result
            OUTPUT_QUIET ERROR_QUIET
            TIMEOUT 120)
        if(same_result EQUAL 0)
            message(FATAL_ERROR "Images are identical but must differ: ${a} and ${b}")
        endif()
    endforeach()
endforeach()
if(pair_count EQUAL 0)
    message(FATAL_ERROR "RunImagesDifferTest: no image pair was compared")
endif()
message(STATUS "RunImagesDifferTest: ${pair_count} image pairs differ as required")
