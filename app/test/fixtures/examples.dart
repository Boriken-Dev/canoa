// Where the tests find the programs crates/canoa-host builds for them (`cargo build -p canoa-host --bin canoa --examples`).
import 'dart:io';

/// The built example [name] of canoa-host, or null when none is built. It looks in CANOA_TEST_EXAMPLES, in `examples/` beside the
/// canoa CANOA_TEST_HOST names, then in the workspace's target directory (CARGO_TARGET_DIR, else the one above this app).
String? exampleBinary(String name) {
  final environment = Platform.environment;
  final file = Platform.isWindows ? '$name.exe' : name;
  final app = (environment['CANOA_TEST_APP'] ?? Directory.current.path).replaceAll(r'\', '/');
  final directories = [
    if (environment['CANOA_TEST_EXAMPLES'] case final String dir when dir.isNotEmpty) dir,
    if (environment['CANOA_TEST_HOST'] case final String host when host.isNotEmpty) File(host).parent.uri.resolve('examples/').toFilePath(),
    if (environment['CARGO_TARGET_DIR'] case final String target when target.isNotEmpty) '$target/debug/examples',
    '$app/../target/debug/examples',
  ];
  for (final directory in directories) {
    final path = File('$directory/$file');
    if (path.existsSync()) return path.absolute.path;
  }
  return null;
}

/// The reason a test needing example [name] cannot run here, or null.
String? exampleMissing(String name) =>
    exampleBinary(name) == null ? 'build it: cargo build -p canoa-host --examples ($name)' : null;

/// The arguments of `stdio_agent` as the app's scripted coding agent, keeping its conversations in [state].
List<String> scriptedArgs(String state, [List<String> extra = const []]) => ['--scripted', '--state=$state', ...extra];
