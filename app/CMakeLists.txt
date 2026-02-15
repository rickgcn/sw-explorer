find_package(QT NAMES Qt6 Qt5 REQUIRED COMPONENTS Widgets Core Concurrent)
find_package(Qt${QT_VERSION_MAJOR} REQUIRED COMPONENTS Widgets Core Concurrent)

set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

set(APP_SOURCES
    main.cpp
    mainwindow.h
    mainwindow.cpp
    file_table_model.h
    file_table_model.cpp
    resources.qrc
)

if (WIN32)
    configure_file(
        ${CMAKE_CURRENT_SOURCE_DIR}/version.rc.in
        ${CMAKE_CURRENT_BINARY_DIR}/version.rc
        @ONLY
    )
    list(APPEND APP_SOURCES ${CMAKE_CURRENT_BINARY_DIR}/version.rc)
endif()

add_executable(sw-explorer
    ${APP_SOURCES}
)

if (WIN32)
    set_target_properties(sw-explorer PROPERTIES WIN32_EXECUTABLE TRUE)
endif()

target_compile_definitions(sw-explorer
    PRIVATE
        SW_EXPLORER_VERSION="${PROJECT_VERSION}"
)

target_link_libraries(sw-explorer
    PRIVATE
        Qt${QT_VERSION_MAJOR}::Widgets
        Qt${QT_VERSION_MAJOR}::Core
        Qt${QT_VERSION_MAJOR}::Concurrent
        swcore
)
