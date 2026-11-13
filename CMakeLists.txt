cmake_minimum_required(VERSION 3.16)
project(rxminer_native LANGUAGES C CXX)

# The miner is built and installed by Cargo. CMake is only for the integrated
# RandomX correctness suite.
set(CMAKE_CXX_STANDARD 11)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_C_STANDARD 11)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release CACHE STRING "" FORCE)
endif()
include(cmake/randomx.cmake)
add_executable(randomx-tests EXCLUDE_FROM_ALL tests/randomx/tests.cpp)
target_link_libraries(randomx-tests PRIVATE randomx)
target_compile_definitions(randomx-tests PRIVATE HAVE_POSIX_MEMALIGN)
target_compile_options(randomx-tests PRIVATE -maes)

# Build a separate reference from the sources also owned by randomx-cpp.
# Build with -DBUILD_SHARED_LIBS=ON for the dynamically loaded Rust differential.
set(oracle_sources program_generation jit_normalization blake_fixed blake_rounds
  aes_pipeline interpreter_dispatch rust_core_oracle rust_vm_oracle
  jit_optimizations jit_rounding jit_swaps jit_memory)
foreach(name IN LISTS oracle_sources)
  add_executable(${name} EXCLUDE_FROM_ALL tests/randomx/${name}.cpp)
  target_link_libraries(${name} PRIVATE randomx pthread)
  target_compile_definitions(${name} PRIVATE HAVE_POSIX_MEMALIGN)
  target_compile_options(${name} PRIVATE -O2 -maes)
endforeach()
target_sources(blake_rounds PRIVATE tests/randomx/blake_compact.c)
target_sources(aes_pipeline PRIVATE tests/randomx/aes_legacy.cpp)
add_custom_target(randomx-oracles DEPENDS randomx-tests ${oracle_sources})
