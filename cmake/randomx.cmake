# Copyright (c) 2019, The Monero Project
#
# All rights reserved.
#
# Redistribution and use in source and binary forms, with or without modification, are
# permitted provided that the following conditions are met:
#
# 1. Redistributions of source code must retain the above copyright notice, this list of
#    conditions and the following disclaimer.
#
# 2. Redistributions in binary form must reproduce the above copyright notice, this list
#    of conditions and the following disclaimer in the documentation and/or other
#    materials provided with the distribution.
#
# 3. Neither the name of the copyright holder nor the names of its contributors may be
#    used to endorse or promote products derived from this software without specific
#    prior written permission.
#
# THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY
# EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF
# MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL
# THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
# SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
# PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
# INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT,
# STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF
# THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

# Optional reference tools. Cargo builds these sources separately in randomx-cpp.
if(NOT CMAKE_SYSTEM_NAME STREQUAL "Linux" OR
   NOT CMAKE_SYSTEM_PROCESSOR MATCHES "^(x86_64|amd64|AMD64)$")
  message(FATAL_ERROR "The optional CMake reference build supports Linux x86-64")
endif()

set(randomx_sources
  aes_hash.cpp
  allocator.cpp
  argon2_ref.c
  argon2_core.c
  argon2_ssse3.c
  argon2_avx2.c
  blake2/blake2b.c
  blake2_generator.cpp
  bytecode_machine.cpp
  cpu.cpp
  dataset.cpp
  instruction.cpp
  instructions_portable.cpp
  jit_compiler_x86.cpp
  jit_compiler_x86_static.S
  randomx.cpp
  reciprocal.c
  soft_aes.cpp
  superscalar.cpp
  virtual_machine.cpp
  virtual_memory.c
  vm_compiled.cpp
  vm_compiled_light.cpp
  vm_interpreted.cpp
  vm_interpreted_light.cpp
)
list(TRANSFORM randomx_sources PREPEND "${PROJECT_SOURCE_DIR}/crates/randomx-cpp/native/")
add_library(randomx ${randomx_sources})
set(rx_source "${PROJECT_SOURCE_DIR}/crates/randomx-cpp/native")
set_source_files_properties("${rx_source}/jit_compiler_x86_static.S" PROPERTIES LANGUAGE C)
set_source_files_properties("${rx_source}/argon2_ssse3.c" PROPERTIES COMPILE_OPTIONS "-mssse3")
set_source_files_properties("${rx_source}/argon2_avx2.c" PROPERTIES COMPILE_OPTIONS "-mavx2")
set_target_properties(randomx PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_include_directories(randomx PUBLIC "${rx_source}")
target_compile_definitions(randomx PRIVATE NDEBUG HAVE_POSIX_MEMALIGN)
target_compile_options(randomx PRIVATE -O3 -maes -fno-semantic-interposition)

set(ARCH "default" CACHE STRING "Native architecture tuning: default or native")
if(ARCH STREQUAL "native")
  target_compile_options(randomx PRIVATE -march=native)
elseif(NOT ARCH STREQUAL "default")
  message(FATAL_ERROR "ARCH must be default or native")
endif()

option(RXMINER_VAES "Enable experimental VAES 256-bit AES fill" OFF)
if(RXMINER_VAES)
  target_sources(randomx PRIVATE "${rx_source}/aes_hash_vaes.cpp")
  target_compile_definitions(randomx PRIVATE RANDOMX_HAVE_VAES=1)
  set_source_files_properties("${rx_source}/aes_hash_vaes.cpp" PROPERTIES COMPILE_OPTIONS "-mvaes;-mavx2")
endif()
