cmake_minimum_required(VERSION 3.25)
project(yeet-kde VERSION 0.1.0 LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
find_package(ECM 6.0 REQUIRED NO_MODULE)
list(APPEND CMAKE_MODULE_PATH ${ECM_MODULE_PATH})
include(KDEInstallDirs6)
find_package(Qt6 6.6 REQUIRED COMPONENTS Widgets Test)
find_package(KF6CoreAddons 6.0 REQUIRED)
find_package(KF6I18n 6.0 REQUIRED)
find_program(CARGO_EXECUTABLE cargo REQUIRED)
get_filename_component(YEET_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
add_custom_target(yeet-helper ALL
    COMMAND ${CARGO_EXECUTABLE} build --locked -p yeet-backend
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${YEET_ROOT}/target/debug/yeet-backend" "${CMAKE_CURRENT_BINARY_DIR}/yeet-backend"
    WORKING_DIRECTORY "${YEET_ROOT}"
    VERBATIM)
add_executable(yeet-kde main.cpp backend.cpp backend.h window.cpp window.h conversionwindow.cpp conversionwindow.h)
add_dependencies(yeet-kde yeet-helper)
target_link_libraries(yeet-kde PRIVATE Qt6::Widgets KF6::CoreAddons KF6::I18n)
target_compile_options(yeet-kde PRIVATE -Wall -Wextra -Wpedantic)
install(TARGETS yeet-kde DESTINATION ${KDE_INSTALL_BINDIR})
install(PROGRAMS "${CMAKE_CURRENT_BINARY_DIR}/yeet-backend" DESTINATION ${KDE_INSTALL_LIBEXECDIR}/yeet)
file(RELATIVE_PATH YEET_HELPER_RELATIVE "${KDE_INSTALL_FULL_BINDIR}" "${KDE_INSTALL_FULL_LIBEXECDIR}/yeet/yeet-backend")
target_compile_definitions(yeet-kde PRIVATE YEET_HELPER_RELATIVE="${YEET_HELPER_RELATIVE}")
install(FILES org.yeet.Yeet.desktop org.yeet.Yeet.ConvertOnly.desktop DESTINATION ${KDE_INSTALL_APPDIR})
enable_testing()
add_executable(yeet-window-test tests/window-test.cpp backend.cpp backend.h window.cpp window.h conversionwindow.cpp conversionwindow.h)
add_dependencies(yeet-window-test yeet-helper)
target_link_libraries(yeet-window-test PRIVATE Qt6::Widgets Qt6::Test KF6::I18n)
target_compile_definitions(yeet-window-test PRIVATE
    YEET_TEST_BACKEND="${CMAKE_CURRENT_SOURCE_DIR}/tests/backend.py"
    YEET_TEST_METADATA="${YEET_ROOT}/crates/yeet-core/tests/fixtures/h264.json"
    YEET_REAL_HELPER="${CMAKE_CURRENT_BINARY_DIR}/yeet-backend")
target_compile_options(yeet-window-test PRIVATE -Wall -Wextra -Wpedantic)
add_test(NAME yeet-window COMMAND yeet-window-test)
set_tests_properties(yeet-window PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen;QT_QPA_PLATFORMTHEME=;QT_STYLE_OVERRIDE=Fusion")
