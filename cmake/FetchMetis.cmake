# Upstream METIS + GKlib built from source.
# The vcpkg "metis" port crashes in METIS_PartGraphKway (k >= 4) on Windows, so we build the
# reference implementation ourselves. Provides the static target `metis`.
include(FetchContent)

set(GKLIB_BUILD_APPS OFF CACHE BOOL "" FORCE)
set(SHARED OFF CACHE BOOL "" FORCE)

FetchContent_Declare(gklib_src
    EXCLUDE_FROM_ALL
    GIT_REPOSITORY https://github.com/KarypisLab/GKlib.git
    GIT_TAG        3b7d61b9f885063c89901f3901fb4426f9cfb58f)
# SOURCE_SUBDIR points at a directory without CMakeLists.txt: we only want the sources, METIS's own build is bypassed
FetchContent_Declare(metis_src
    GIT_REPOSITORY https://github.com/KarypisLab/METIS.git
    GIT_TAG        272d4a91c5f66c92327493339a476c553ebf1f5d
    SOURCE_SUBDIR  no_cmake_build)

FetchContent_MakeAvailable(gklib_src metis_src)

file(GLOB METIS_SOURCES ${metis_src_SOURCE_DIR}/libmetis/*.c)
add_library(metis STATIC ${METIS_SOURCES})
target_include_directories(metis
    PUBLIC  ${metis_src_SOURCE_DIR}/include
    PRIVATE ${metis_src_SOURCE_DIR}/libmetis)
target_compile_definitions(metis PUBLIC IDXTYPEWIDTH=32 REALTYPEWIDTH=32)
# PRIVATE: GKlib's usage requirements (__thread define, -O3) must not leak into the C++ code
target_link_libraries(metis PRIVATE GKlib)
set_target_properties(metis PROPERTIES POSITION_INDEPENDENT_CODE ON)
