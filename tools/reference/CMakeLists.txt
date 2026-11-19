# Builds the reference TADS 3 toolchain used as the conformance oracle:
# the stock compiler and interpreter (t3make, t3run) from tads-runner, plus
# the original TADS 3 test suite programs (test_exec, test_tok, test_prs_top,
# t3pre, test_obj), which tads-runner does not build itself.
#
# Nothing here is part of Hexicon proper. tads-runner is used in place and
# is not modified.

cmake_minimum_required(VERSION 3.19)
project(HexiconReference C CXX)

set(TADS_RUNNER_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../../_tads/tads-runner"
    CACHE PATH "Checkout of github.com/captain-mayhem/tads-runner")
get_filename_component(TADS_RUNNER_DIR "${TADS_RUNNER_DIR}" ABSOLUTE)
if (NOT EXISTS "${TADS_RUNNER_DIR}/tads3/CMakeLists.txt")
    message(FATAL_ERROR "tads-runner not found at ${TADS_RUNNER_DIR}; set TADS_RUNNER_DIR")
endif()

if (NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()
set(CMAKE_CXX_STANDARD 11)
set(CMAKE_C_STANDARD 11)

# Put every executable in one directory so the test scripts can find them.
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin")

# tads-runner's build file calls this IDE-only helper, which assumes
# tads-runner is the top-level project. It has no effect on the build.
function(tads_source_group)
endfunction()

add_subdirectory("${TADS_RUNNER_DIR}/tads3" tads3)

set(T3 "${TADS_RUNNER_DIR}/tads3")
set(T2 "${TADS_RUNNER_DIR}/tads2")

# The interpreter looks for its time zone database next to the executable.
add_custom_command(
    OUTPUT "${CMAKE_RUNTIME_OUTPUT_DIRECTORY}/timezones.t3tz"
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${T3}/tz/timezones.t3tz" "${CMAKE_RUNTIME_OUTPUT_DIRECTORY}/timezones.t3tz"
    DEPENDS "${T3}/tz/timezones.t3tz")
add_custom_target(timezone_db DEPENDS "${CMAKE_RUNTIME_OUTPUT_DIRECTORY}/timezones.t3tz")

# ---------------------------------------------------------------------------
# test_exec: runs an image through the VM with a plain stdio console.
# This is t3run's source set with os_stdio in place of the terminal layer.

add_executable(test_exec
    ${T3}/test/test_exec.cpp
    ${T3}/vmconsol.cpp
    ${T3}/vmconmor.cpp
    ${T3}/vmconhmp.cpp
    ${T3}/vmbiftio.cpp
    ${T3}/askf_tx3.cpp
    ${T3}/indlg_tx3.cpp
    ${T3}/vmhosttx.cpp
    ${T3}/vmhostsi.cpp
    ${T3}/vmmcreg.cpp
    ${T3}/vmbifregn.cpp
    ${T3}/os_stdio.cpp
    ${T2}/osifc.c
    ${T2}/osnoui.c
    ${T2}/osstzprs.c
    ${T2}/osrestad.c
)
target_include_directories(test_exec PRIVATE ${T3}/test)
target_link_libraries(test_exec PRIVATE t3core tads_settings)
if (WIN32)
    target_sources(test_exec PRIVATE
        ${T2}/msdos/osdos.c
        ${T2}/msdos/osdosnui.c
        ${T2}/msdos/osdosex.c
        ${T2}/msdos/osnogen.c
        ${T2}/msdos/oswinmem.c
        ${T2}/msdos/ostzw32.c
    )
    target_link_libraries(test_exec PRIVATE Ws2_32.lib Wininet.lib)
endif()

# ---------------------------------------------------------------------------
# Compiler-side test programs. Each is the compiler's source set with the
# test program's main() in place of the compiler's command-line driver.

get_target_property(t3make_sources t3make SOURCES)
set(compiler_sources)
foreach (src IN LISTS t3make_sources)
    if (NOT src MATCHES "tcmakecl\\.cpp$")
        get_filename_component(abs "${src}" ABSOLUTE BASE_DIR "${T3}")
        list(APPEND compiler_sources "${abs}")
    endif()
endforeach()

function(add_compiler_test name main_source)
    add_executable(${name} ${T3}/test/${main_source} ${compiler_sources})
    target_include_directories(${name} PRIVATE ${T3}/test)
    target_link_libraries(${name} PRIVATE tads_settings)
    if (WIN32 AND VM_WITH_NETWORK)
        target_link_libraries(${name} PRIVATE Ws2_32.lib Wininet.lib)
    endif()
endfunction()

add_compiler_test(t3pre test_pre.cpp)

# ---------------------------------------------------------------------------
# Test programs that supply their own stubs for parts of the compiler or VM,
# so they must link an exact subset of sources. The lists follow the original
# win32/makefile.vc5 rules.

set(os_sources_min
    ${T2}/osnoui.c
    ${T2}/msdos/osdos.c
    ${T2}/msdos/osdosnui.c
    ${T2}/msdos/osdosex.c
    ${T2}/msdos/osnogen.c
    ${T2}/msdos/oswinmem.c
)

function(add_listed_test name main_source)
    set(sources ${T3}/test/${main_source})
    foreach (stem IN LISTS ARGN)
        if (IS_ABSOLUTE "${stem}")
            list(APPEND sources "${stem}")
        else()
            list(APPEND sources "${T3}/${stem}.cpp")
        endif()
    endforeach()
    add_executable(${name} ${sources})
    target_include_directories(${name} PRIVATE ${T3}/test)
    target_link_libraries(${name} PRIVATE tads_settings)
    if (WIN32 AND VM_WITH_NETWORK)
        target_link_libraries(${name} PRIVATE Ws2_32.lib Wininet.lib)
    endif()
endfunction()

add_listed_test(test_tok test_tok.cpp
    std std_dbg derived/vmuni_cs tcglob vmerr vmerrmsg utf8 charmap resload
    resnoexe tcmain tcerr tcerrmsg tchostsi tcsrc tctok tcprs tcprsnl tcprsnf
    tcgen tct3 vmop tct3unas tct3nl tct3_d vmhash vmtypedh vmbignumlib
    ${os_sources_min}
)

# A driver of Hexicon's own, not from the suite: lists the tokens of a
# source file. It links what test_tok links.
add_listed_test(tok_dump test_tok.cpp
    std std_dbg derived/vmuni_cs tcglob vmerr vmerrmsg utf8 charmap resload
    resnoexe tcmain tcerr tcerrmsg tchostsi tcsrc tctok tcprs tcprsnl tcprsnf
    tcgen tct3 vmop tct3unas tct3nl tct3_d vmhash vmtypedh vmbignumlib
    ${os_sources_min}
)
get_target_property(tok_dump_sources tok_dump SOURCES)
list(REMOVE_ITEM tok_dump_sources ${T3}/test/test_tok.cpp)
list(APPEND tok_dump_sources ${CMAKE_CURRENT_LIST_DIR}/tok_dump.cpp)
set_target_properties(tok_dump PROPERTIES SOURCES "${tok_dump_sources}")

add_listed_test(test_prs_top test_prs_top.cpp
    tcglob std std_dbg vmerr vmerrmsg utf8 derived/vmuni_cs charmap resload
    resnoexe tcmain tcerr tcerrmsg tchostsi tcsrc tctok tcprs tcprsfil
    tcprsstm tcprsprg tcprsimg tcgen tcgenfil vmop vmbignumlib vmhash
    vmwrtimg vmtypedh vmfile vmglob
    tct3 tct3stm tct3unas tct3img tct3prg
    ${os_sources_min}
)

# test_prs_top links the VM globals without the VM itself, so it cannot use
# the "individual global variables" mode, whose globals are VM objects with
# constructors. Use the static-structure mode for this one program.
if (MSVC)
    target_compile_options(test_prs_top PRIVATE /UVMGLOB_VARS)
else()
    target_compile_options(test_prs_top PRIVATE -UVMGLOB_VARS)
endif()
target_compile_definitions(test_prs_top PRIVATE VMGLOB_STRUCT)

# The network sources are defined inside tads-runner's build file; recover
# them from the VM library's source list.
get_target_property(t3core_sources t3core SOURCES)
set(net_sources)
foreach (src IN LISTS t3core_sources)
    if (src MATCHES "(vmhttp|vmbifnet|vmnet|osifcnet|vmrefcnt|osnet)[^/]*[.]cpp$")
        get_filename_component(abs "${src}" ABSOLUTE BASE_DIR "${T3}")
        list(APPEND net_sources "${abs}")
    endif()
endforeach()

add_listed_test(test_obj test_obj.cpp
    std std_dbg charmap resload resnoexe vmfile vmundo vmobj vmtobj vmpat
    vmstrcmp vmstr vmcoll vmiter vmfref vmlst vmsort vmsortv vmbignum
    vmbignumlib vmsave vmcrc vmvec vmintcls vmanonfn vmlookup vmstrbuf
    vmdynfunc vmbytarr vmdate vmtzobj vmtz vmcset vmfilobj vmfilnam vmtmpfil
    vmpack vmstack vmdict vmgram sha2 md5 vmerr vmerrmsg vmpool vmpoolim
    vmtype vmtypedh utf8 vmglob vmrun vmop vmsrcf vmfunc vmmeta vmbif vmsa
    vmbifl vmbiftad vmisaac vmbiftio askf_tx3 indlg_tx3 vmbift3 vmbt3_nd
    vminit vmini_nd vmconsol vmconmor vmconhmp os_stdio vminitim vmcfgmem
    vmregex vmhosttx vmhostsi vmhash vmlog vmmcreg vmbifregn derived/vmuni_cs
    tcprs tcprsnl tcprsfil tcprsstm tcprsprg tcgen tcgenfil tcglob tcerr
    tcerrmsg tctok tcmain tcsrc tchostsi tclibprs tccmdutl vmrunsym tct3prg
    tct3 tct3stm tct3unas tct3nl tct3_d
    ${net_sources}
    ${os_sources_min}
    ${T2}/osifc.c
    ${T2}/osstzprs.c
    ${T2}/osrestad.c
    ${T2}/msdos/ostzw32.c
)

add_custom_target(reference ALL DEPENDS
    t3make t3run t3res test_exec test_tok tok_dump test_prs_top t3pre test_obj
    timezone_db)
