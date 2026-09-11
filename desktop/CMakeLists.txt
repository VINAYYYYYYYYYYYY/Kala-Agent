cmake_minimum_required(VERSION 3.24)
project(KalaDesktop VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} -Wall -Wextra -Wpedantic -Wconversion -Wsign-conversion")
elseif(MSVC)
    set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} /W4 /permissive-")
endif()

# Qt6
find_package(Qt6 REQUIRED COMPONENTS Core Widgets Gui)

# Auto-generate moc, uic, rcc
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTOUIC ON)
set(CMAKE_AUTORCC ON)

# Output directories
set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)
set(CMAKE_ARCHIVE_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/lib)

# Sources
set(SOURCES
    src/main.cpp
    src/MainWindow.cpp
    src/RunWorker.cpp
    src/ZoneTitleBlock.cpp
    src/ZoneSheet.cpp
    src/ZoneMargin.cpp
    src/ZoneFeed.cpp
    src/ZoneComposer.cpp
    src/ZoneStatusRule.cpp
)

set(HEADERS
    include/MainWindow.hpp
    include/RunWorker.hpp
    include/ZoneTitleBlock.hpp
    include/ZoneSheet.hpp
    include/ZoneMargin.hpp
    include/ZoneFeed.hpp
    include/ZoneComposer.hpp
    include/ZoneStatusRule.hpp
    include/Types.hpp
)

add_executable(kala-desktop ${SOURCES} ${HEADERS})

target_include_directories(kala-desktop PRIVATE include)

target_link_libraries(kala-desktop PRIVATE Qt6::Core Qt6::Widgets Qt6::Gui)

# Resources (QSS, icons, etc.)
qt_add_resources(kala-desktop "resources/resources.qrc")

# Install
install(TARGETS kala-desktop RUNTIME DESTINATION bin)