# Build-environment shim, kept OUTSIDE the source worktree so the tree stays
# pristine (BUILD_GIT_DIRTY 0). Included via -DCMAKE_PROJECT_INCLUDE right
# after project(), before CMakeLists.txt calls find_package(ZeroMQ).
#
# Why: cmake/module/FindZeroMQ.cmake on macOS only knows Homebrew's layout
# (pkg-config libzmq.pc + a static libsodium.a beside libzmq.a). This Mac has
# no Homebrew; libzmq.a comes from the depends tree, built with
# WITH_LIBSODIUM=OFF and ENABLE_CURVE=OFF, and depends deletes lib/pkgconfig.
# So the `zeromq` target the finder would create is created here from the
# same static archive, and the finder's `if(NOT TARGET zeromq)` block is
# skipped. Nothing about the source changes; the shipped btxd links the
# depends libzmq.a statically, which is what upstream's
# scripts/release/verify_release_btxd.py requires on macOS.
# Route find_package(Libevent MODULE) to the depends-aware override in
# modules/ (see modules/FindLibevent.cmake). Prepended, so it wins over the
# tree's cmake/module, which CMakeLists.txt appends after project().
list(PREPEND CMAKE_MODULE_PATH "${CMAKE_CURRENT_LIST_DIR}/modules")
# CMAKE_PROJECT_INCLUDE runs after EVERY project() call, sub-projects
# included (secp256k1, libbitcoinpqc), so guard the one-time work.
if(TARGET zeromq)
  return()
endif()
set(_dep "${CMAKE_SOURCE_DIR}/depends/arm64-apple-darwin")
if(NOT EXISTS "${_dep}/lib/libzmq.a")
  message(FATAL_ERROR "zmq-depends.cmake: ${_dep}/lib/libzmq.a not found")
endif()
add_library(zeromq STATIC IMPORTED GLOBAL)
set_target_properties(zeromq PROPERTIES
  IMPORTED_LOCATION "${_dep}/lib/libzmq.a"
  INTERFACE_INCLUDE_DIRECTORIES "${_dep}/include"
  INTERFACE_LINK_LIBRARIES "pthread"
)
set(ZMQ_LINKAGE "static:${_dep}/lib/libzmq.a" CACHE INTERNAL "How ZeroMQ was resolved (shared vs static path)")
set(ZeroMQ_FOUND TRUE)
set(ZeroMQ_VERSION "4.3.5")
message(STATUS "ZeroMQ: depends static ${_dep}/lib/libzmq.a (project-include shim)")
