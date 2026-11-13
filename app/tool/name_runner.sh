#!/bin/sh
# Names the desktop executable CanoaUI, and what people see of it Canoa, in the runners `flutter create` generated in app/.
# The command-line program is `canoa`, which can sit beside the executable: on Windows and macOS names ignore case, so the
# executable cannot be `Canoa`. Run it after `flutter create` and before the first build, in app/ or with its directory
# as the argument; it is safe to run again. It also lets the desktop runners open a pop-out window (a second engine).
set -eu
cd "${1:-$(dirname "$0")/..}"

# edit FILE SED-SCRIPT PATTERN: applies the script when FILE exists, then fails unless PATTERN is in it.
edit() {
  [ -f "$1" ] || return 0
  sed -i.bak "$2" "$1"
  rm "$1.bak"
  grep -q "$3" "$1" || { echo "name_runner: $1 has no $3 after the edit" >&2; exit 1; }
}

edit windows/CMakeLists.txt 's/set(BINARY_NAME "canoa")/set(BINARY_NAME "CanoaUI")/' 'BINARY_NAME "CanoaUI"'
edit linux/CMakeLists.txt 's/set(BINARY_NAME "canoa")/set(BINARY_NAME "CanoaUI")/' 'BINARY_NAME "CanoaUI"'
edit windows/runner/Runner.rc \
  's/"FileDescription", "canoa"/"FileDescription", "Canoa"/; s/"InternalName", "canoa"/"InternalName", "CanoaUI"/; s/"OriginalFilename", "canoa.exe"/"OriginalFilename", "CanoaUI.exe"/; s/"ProductName", "canoa"/"ProductName", "Canoa"/' \
  'OriginalFilename", "CanoaUI.exe"'
edit windows/runner/main.cpp 's/window.Create(L"canoa"/window.Create(L"Canoa"/' 'Create(L"Canoa"'
edit linux/runner/my_application.cc 's/_title(header_bar, "canoa")/_title(header_bar, "Canoa")/; s/_title(window, "canoa")/_title(window, "Canoa")/' '_title(window, "Canoa")'
# The bundle is Canoa.app; its executable CanoaUI keeps clear of the `canoa` host in Contents/MacOS.
if [ -f macos/Runner/Configs/AppInfo.xcconfig ]; then
  sed -i.bak 's/^PRODUCT_NAME = .*/PRODUCT_NAME = Canoa/' macos/Runner/Configs/AppInfo.xcconfig
  rm macos/Runner/Configs/AppInfo.xcconfig.bak
  grep -q '^EXECUTABLE_NAME = CanoaUI' macos/Runner/Configs/AppInfo.xcconfig ||
    echo 'EXECUTABLE_NAME = CanoaUI' >> macos/Runner/Configs/AppInfo.xcconfig
fi

# A window popped out of a conversation is a second Flutter engine of the same program (desktop_multi_window), which needs the
# plugins registered for it too. `once FILE SED-SCRIPT PATTERN` is `edit` for a change that must not be made twice.
once() {
  [ -f "$1" ] || return 0
  grep -q "$3" "$1" || edit "$1" "$2" "$3"
}
once windows/runner/flutter_window.cpp \
  's|#include "flutter/generated_plugin_registrant.h"|#include "flutter/generated_plugin_registrant.h"\
#include "desktop_multi_window/desktop_multi_window_plugin.h"|' 'desktop_multi_window_plugin.h'
once windows/runner/flutter_window.cpp \
  '/RegisterPlugins(flutter_controller_->engine());/a\
  DesktopMultiWindowSetWindowCreatedCallback([](void *controller) {\
    auto *window = reinterpret_cast<flutter::FlutterViewController *>(controller);\
    RegisterPlugins(window->engine());\
  });' 'DesktopMultiWindowSetWindowCreatedCallback'
once linux/runner/my_application.cc \
  's|#include "flutter/generated_plugin_registrant.h"|#include "flutter/generated_plugin_registrant.h"\
#include "desktop_multi_window/desktop_multi_window_plugin.h"|' 'desktop_multi_window_plugin.h'
once linux/runner/my_application.cc \
  '/fl_register_plugins(FL_PLUGIN_REGISTRY(view));/a\
  desktop_multi_window_plugin_set_window_created_callback([](FlPluginRegistry* registry) {\
    fl_register_plugins(registry);\
  });' 'desktop_multi_window_plugin_set_window_created_callback'
once macos/Runner/MainFlutterWindow.swift \
  's|^import FlutterMacOS$|import FlutterMacOS\
import desktop_multi_window|' 'import desktop_multi_window'
once macos/Runner/MainFlutterWindow.swift \
  '/RegisterGeneratedPlugins(registry: flutterViewController)/a\
    FlutterMultiWindowPlugin.setOnWindowCreatedCallback { controller in\
      RegisterGeneratedPlugins(registry: controller)\
    }' 'FlutterMultiWindowPlugin.setOnWindowCreatedCallback'
