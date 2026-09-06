# Build-environment override of cmake/module/FindLibevent.cmake, kept OUTSIDE
# the source worktree (BUILD_GIT_DIRTY stays 0). Reached through
# CMAKE_MODULE_PATH, which zmq-depends.cmake prepends.
#
# Why: the tree's finder, on macOS, searches ONLY Homebrew's directories for
# the static archives and is fatal otherwise. This Mac has no Homebrew; the
# same three static archives come from the depends tree. Everything below is
# the tree's own macOS branch with the search dirs replaced by the depends
# prefix: the same target names, the same static linkage, the same
# evhttp_connection_get_peer probe, the same LIBEVENT_LINKAGE marker.

function(check_evhttp_connection_get_peer target)
  include(CMakePushCheckState)
  cmake_push_check_state(RESET)
  set(CMAKE_REQUIRED_LIBRARIES ${target})
  include(CheckCXXSourceCompiles)
  check_cxx_source_compiles("
    #include <cstdint>
    #include <event2/http.h>

    int main()
    {
        evhttp_connection* conn = (evhttp_connection*)1;
        const char* host;
        uint16_t port;
        evhttp_connection_get_peer(conn, &host, &port);
    }
    " HAVE_EVHTTP_CONNECTION_GET_PEER_CONST_CHAR
  )
  cmake_pop_check_state()
  target_compile_definitions(${target} INTERFACE
    $<$<BOOL:${HAVE_EVHTTP_CONNECTION_GET_PEER_CONST_CHAR}>:HAVE_EVHTTP_CONNECTION_GET_PEER_CONST_CHAR>
  )
endfunction()

function(btx_import_macos_static_libevent component archive include_dirs)
  add_library(libevent::${component} STATIC IMPORTED GLOBAL)
  set_target_properties(libevent::${component} PROPERTIES
    IMPORTED_LOCATION "${archive}"
    INTERFACE_INCLUDE_DIRECTORIES "${include_dirs}"
  )
endfunction()

set(LIBEVENT_LINKAGE "" CACHE INTERNAL "How Libevent was resolved (shared vs static path)")

set(_dep "${CMAKE_SOURCE_DIR}/depends/arm64-apple-darwin")
set(_event_core_a     "${_dep}/lib/libevent_core.a")
set(_event_extra_a    "${_dep}/lib/libevent_extra.a")
set(_event_pthreads_a "${_dep}/lib/libevent_pthreads.a")
set(_event_inc        "${_dep}/include")
foreach(_f IN ITEMS "${_event_core_a}" "${_event_extra_a}" "${_event_pthreads_a}" "${_event_inc}/event2/event.h")
  if(NOT EXISTS "${_f}")
    message(FATAL_ERROR "FindLibevent (depends override): ${_f} not found; build depends first")
  endif()
endforeach()

btx_import_macos_static_libevent(core "${_event_core_a}" "${_event_inc}")
btx_import_macos_static_libevent(extra "${_event_extra_a}" "${_event_inc}")
btx_import_macos_static_libevent(pthreads "${_event_pthreads_a}" "${_event_inc}")
set_target_properties(libevent::extra PROPERTIES INTERFACE_LINK_LIBRARIES "libevent::core")
set_target_properties(libevent::pthreads PROPERTIES INTERFACE_LINK_LIBRARIES "libevent::core")

include(FindPackageHandleStandardArgs)
find_package_handle_standard_args(Libevent
  REQUIRED_VARS _event_core_a _event_extra_a _event_pthreads_a _event_inc
)
check_evhttp_connection_get_peer(libevent::extra)
set(LIBEVENT_LINKAGE "static:${_event_core_a}" CACHE INTERNAL "How Libevent was resolved (shared vs static path)")
message(STATUS "Libevent: depends static ${_event_core_a} (module override)")
unset(_event_core_a)
unset(_event_extra_a)
unset(_event_pthreads_a)
unset(_event_inc)
unset(_dep)
