#!/bin/sh
# Names the desktop executable (the brand's `executable`, CanoaUI by default) and what people see of it (the brand's `name`) in
# the runners `flutter create` generated in app/. The brand is app/brand.json, or the file BRAND_FILE names; BRAND_NAME and
# BRAND_EXECUTABLE override what it holds. The command-line program is `canoa`, which can sit beside the executable: on Windows
# and macOS names ignore case, so the executable cannot be `Canoa`. Run it after `flutter create` and before the first build, in
# app/ or with its directory as the argument; it is safe to run again.
set -eu
cd "${1:-$(dirname "$0")/..}"

# brand_field KEY: the string a top-level KEY of the brand file holds (the file is flat JSON, one pair a line).
brand_field() {
  sed -n "s/^ *\"$1\": *\"\([^\"]*\)\".*/\1/p" "${BRAND_FILE:-brand.json}" | head -n 1
}
NAME="${BRAND_NAME:-$(brand_field name)}"
EXECUTABLE="${BRAND_EXECUTABLE:-$(brand_field executable)}"
[ -n "$NAME" ] && [ -n "$EXECUTABLE" ] || { echo "name_runner: the brand file names no name or executable" >&2; exit 1; }

# edit FILE SED-SCRIPT PATTERN: applies the script when FILE exists, then fails unless PATTERN is in it.
edit() {
  [ -f "$1" ] || return 0
  sed -i.bak "$2" "$1"
  rm "$1.bak"
  grep -q "$3" "$1" || { echo "name_runner: $1 has no $3 after the edit" >&2; exit 1; }
}

# The generated runner holds the project's name, `canoa`, or what an earlier run put there: each pattern takes either.
edit windows/CMakeLists.txt "s/set(BINARY_NAME \"[A-Za-z0-9]*\")/set(BINARY_NAME \"$EXECUTABLE\")/" "BINARY_NAME \"$EXECUTABLE\""
edit linux/CMakeLists.txt "s/set(BINARY_NAME \"[A-Za-z0-9]*\")/set(BINARY_NAME \"$EXECUTABLE\")/" "BINARY_NAME \"$EXECUTABLE\""
edit windows/runner/Runner.rc \
  "s/\"FileDescription\", \"[^\"]*\"/\"FileDescription\", \"$NAME\"/; s/\"InternalName\", \"[^\"]*\"/\"InternalName\", \"$EXECUTABLE\"/; s/\"OriginalFilename\", \"[^\"]*\"/\"OriginalFilename\", \"$EXECUTABLE.exe\"/; s/\"ProductName\", \"[^\"]*\"/\"ProductName\", \"$NAME\"/" \
  "OriginalFilename\", \"$EXECUTABLE.exe\""
edit windows/runner/main.cpp "s/window.Create(L\"[^\"]*\"/window.Create(L\"$NAME\"/" "Create(L\"$NAME\""
edit linux/runner/my_application.cc "s/_title(header_bar, \"[^\"]*\")/_title(header_bar, \"$NAME\")/; s/_title(window, \"[^\"]*\")/_title(window, \"$NAME\")/" "_title(window, \"$NAME\")"
# The bundle is NAME.app; its executable keeps clear of the `canoa` host in Contents/MacOS.
if [ -f macos/Runner/Configs/AppInfo.xcconfig ]; then
  sed -i.bak "s/^PRODUCT_NAME = .*/PRODUCT_NAME = $NAME/; s/^EXECUTABLE_NAME = .*/EXECUTABLE_NAME = $EXECUTABLE/" macos/Runner/Configs/AppInfo.xcconfig
  rm macos/Runner/Configs/AppInfo.xcconfig.bak
  grep -q "^EXECUTABLE_NAME = $EXECUTABLE" macos/Runner/Configs/AppInfo.xcconfig ||
    echo "EXECUTABLE_NAME = $EXECUTABLE" >> macos/Runner/Configs/AppInfo.xcconfig
fi
