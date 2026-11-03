// An app that starts canoa-host as Canoa's desktop app does, prints its pid, and dies without stopping it, as a crash
// would. Plain Dart: `dart run test/fixtures/dying_parent.dart HOST CONFIG PEER_HOME`.
import 'dart:io';

import 'package:canoa/desktop/host_tool_io.dart';

Future<void> main(List<String> args) async {
  final tool = ProcessHostTool(args[0], environment: {'COQUI_AGENTIC_PEER_HOME': args[2]});
  // The app's browser proof is WebCrypto only, and a new home would prove by Noise.
  final child = await tool.start(['serve', '--method', 'webcrypto', '--config', args[1], '--until-stdin-closes']);
  child.errors.listen((_) {});
  await child.lines.first;
  stdout.writeln(child.pid);
  await stdout.flush();
  exit(0);
}
