cmake_minimum_required(VERSION 3.25)
project(werp-kde VERSION 0.1.0 LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
find_package(ECM 6.0 REQUIRED NO_MODULE)
list(APPEND CMAKE_MODULE_PATH ${ECM_MODULE_PATH})
include(KDEInstallDirs6)
find_package(Qt6 6.6 REQUIRED COMPONENTS Widgets DBus Test)
find_package(KF6CoreAddons 6.0 REQUIRED)
find_package(KF6I18n 6.0 REQUIRED)
find_program(CARGO_EXECUTABLE cargo REQUIRED)
get_filename_component(WERP_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
add_custom_target(werp-helper ALL
    COMMAND ${CARGO_EXECUTABLE} build --locked -p werp-backend
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${WERP_ROOT}/target/debug/werp-backend" "${CMAKE_CURRENT_BINARY_DIR}/werp-backend"
    WORKING_DIRECTORY "${WERP_ROOT}"
    VERBATIM)
add_executable(werp-kde main.cpp backend.cpp backend.h protocol.h guisettings.cpp guisettings.h mpris.cpp mpris.h window.cpp window.h conversionwindow.cpp conversionwindow.h selectedvideopanel.h)
add_dependencies(werp-kde werp-helper)
target_link_libraries(werp-kde PRIVATE Qt6::Widgets Qt6::DBus KF6::CoreAddons KF6::I18n)
target_compile_options(werp-kde PRIVATE -Wall -Wextra -Wpedantic)
set(WERP_ICON_SIZES 16 22 24 32 48 64 96 128 256 512)
set(WERP_ICON_FILES)
foreach(size IN LISTS WERP_ICON_SIZES)
    set(icon "icons/${size}x${size}/apps/nl.hnsr.Werp.png")
    list(APPEND WERP_ICON_FILES "${icon}")
    install(FILES "${icon}" DESTINATION "${KDE_INSTALL_ICONDIR}/hicolor/${size}x${size}/apps")
endforeach()
install(FILES icons/scalable/apps/nl.hnsr.Werp.svg
    DESTINATION "${KDE_INSTALL_ICONDIR}/hicolor/scalable/apps")
qt_add_resources(werp-kde app-icons PREFIX "/icons" BASE "icons" FILES ${WERP_ICON_FILES})
install(TARGETS werp-kde DESTINATION ${KDE_INSTALL_BINDIR})
install(PROGRAMS "${CMAKE_CURRENT_BINARY_DIR}/werp-backend" DESTINATION ${KDE_INSTALL_LIBEXECDIR}/werp)
file(RELATIVE_PATH WERP_HELPER_RELATIVE "${KDE_INSTALL_FULL_BINDIR}" "${KDE_INSTALL_FULL_LIBEXECDIR}/werp/werp-backend")
target_compile_definitions(werp-kde PRIVATE WERP_HELPER_RELATIVE="${WERP_HELPER_RELATIVE}")
install(FILES nl.hnsr.Werp.desktop nl.hnsr.Werp.ConvertOnly.desktop DESTINATION ${KDE_INSTALL_APPDIR})
enable_testing()
add_executable(werp-window-test tests/window-test.cpp backend.cpp backend.h protocol.h guisettings.cpp guisettings.h mpris.cpp mpris.h window.cpp window.h conversionwindow.cpp conversionwindow.h selectedvideopanel.h)
add_dependencies(werp-window-test werp-helper)
target_link_libraries(werp-window-test PRIVATE Qt6::Widgets Qt6::DBus Qt6::Test KF6::I18n)
target_compile_definitions(werp-window-test PRIVATE
    WERP_TEST_BACKEND="${CMAKE_CURRENT_SOURCE_DIR}/tests/backend.py"
    WERP_TEST_METADATA="${WERP_ROOT}/crates/werp-core/tests/fixtures/h264.json"
    WERP_REAL_HELPER="${CMAKE_CURRENT_BINARY_DIR}/werp-backend")
target_compile_options(werp-window-test PRIVATE -Wall -Wextra -Wpedantic)
add_test(NAME werp-window COMMAND werp-window-test)
set_tests_properties(werp-window PROPERTIES ENVIRONMENT "QT_QPA_PLATFORM=offscreen;QT_QPA_PLATFORMTHEME=;QT_STYLE_OVERRIDE=Fusion")
