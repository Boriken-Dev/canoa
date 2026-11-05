# Written for macOS and not yet run there. Builds crates/canoa-ffi as a static library linked whole into this pod's framework,
# whose load-time entry (crates/canoa-ffi/src/platform.rs) makes the application the host's own command when it starts itself.
Pod::Spec.new do |s|
  s.name             = 'canoa_native'
  s.version          = '0.0.1'
  s.summary          = "The Canoa host's library, built with the desktop app and carried inside it."
  s.homepage         = 'https://github.com/Boriken-Dev/canoa'
  s.license          = { :type => 'MPL-2.0' }
  s.author           = { 'Boriken Dev' => 'contact@boriken.dev' }
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'FlutterMacOS'
  s.platform         = :osx, '10.15'
  s.script_phase = {
    :name => 'Build canoa-ffi',
    :execution_position => :before_compile,
    :script => <<~'SCRIPT',
      set -e
      export PATH="$HOME/.cargo/bin:$PATH"
      workspace="$(cd "$PODS_TARGET_SRCROOT/../../.." && pwd)"
      profile=release; flag=--release
      if [ "$CONFIGURATION" = "Debug" ]; then profile=debug; flag=; fi
      features=
      if [ -n "${CANOA_WEB_DIST:-}" ]; then features=--features=web; fi
      target="${CARGO_TARGET_DIR:-$workspace/target}"
      cargo rustc --manifest-path "$workspace/Cargo.toml" --target-dir "$target" -p canoa-ffi --lib $features $flag --crate-type staticlib
      cp "$target/$profile/libcanoa_ffi.a" "$BUILT_PRODUCTS_DIR/libcanoa_ffi.a"
    SCRIPT
  }
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/libcanoa_ffi.a',
  }
  s.swift_version = '5.0'
end
