# Standalone tools for the asset pipeline. Unlike the examples these are console
# applications: they run headless and are meant to be driven from a shell or CI.

find_package(openmesh REQUIRED)

set(NANITE_BAKE_NAME nanite_bake)

add_executable(${NANITE_BAKE_NAME}
    ${CMAKE_CURRENT_SOURCE_DIR}/nanite_bake/nanite_bake.cpp
)
# A console subsystem app, not the WIN32 GUI subsystem the examples use - the builder's
# progress output is the point of running it.
set_target_properties(${NANITE_BAKE_NAME} PROPERTIES
    WIN32_EXECUTABLE OFF
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin/"
)
target_link_libraries(${NANITE_BAKE_NAME}
    base
    mesh
    ${Vulkan_LIBRARY}
    ${WINLIBS}
    OpenMeshCore
    OpenMeshTools
)
