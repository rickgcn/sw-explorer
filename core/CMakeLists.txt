find_package(QT NAMES Qt6 Qt5 REQUIRED COMPONENTS Core)
find_package(Qt${QT_VERSION_MAJOR} REQUIRED COMPONENTS Core)

add_library(swcore STATIC
    src/idb_parser.cpp
    src/extractor.cpp
)

target_include_directories(swcore
    PUBLIC
        ${CMAKE_CURRENT_SOURCE_DIR}/include
)

target_link_libraries(swcore PUBLIC Qt${QT_VERSION_MAJOR}::Core)

