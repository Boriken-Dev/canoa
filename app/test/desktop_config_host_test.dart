// The desktop app's configuration read and written through the host's own store, with no host serving: `canoa control` over canoa.cfg's
// files, as the app runs it before its host is up and when none starts. The app opens no canoa.cfg itself. CANOA_TEST_HOST names a built
// canoa; without it the test is skipped.
@Timeout(Duration(minutes: 2))
library;

import 'dart:io';

import 'package:canoa/config/canoa_config_io.dart';
import 'package:canoa/config/config_layers.dart';
import 'package:canoa/desktop/host_tool_io.dart' show ProcessHostTool;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ChatDensity;
import 'package:flutter_test/flutter_test.dart';

final String? _host = switch (Platform.environment['CANOA_TEST_HOST']) {
  final String path when path.isNotEmpty => File(path).absolute.path,
  _ => null,
};

void main() {
  test('layers are read, and a change writes only the tables that changed to the own layer', () async {
    final dir = await Directory.systemTemp.createTemp('canoa-desktop-config-');
    addTearDown(() => dir.delete(recursive: true));
    final low = File('${dir.path}${Platform.pathSeparator}system.cfg')
      ..writeAsStringSync('[sessions]\nkeep = 500\n\n[desktop]\nstart_hidden = true\n');
    final own = File('${dir.path}${Platform.pathSeparator}own.cfg')..writeAsStringSync('');
    final separator = Platform.isWindows ? ';' : ':';
    // The first file is lower, and the second, which is empty, is the person's own.
    final files = ConfigFiles.resolve(
      env: {'CANOA_CONFIG': '${low.path}$separator${own.path}'},
      windows: Platform.isWindows,
      programDir: dir.path,
      exists: (path) => File(path).existsSync(),
    );
    final tool = ProcessHostTool(_host!);

    final config = (await loadDesktopConfig(tool: tool, files: files))!;
    expect(config.sessions.keep, 500, reason: 'a lower layer is read');
    expect(config.lowerLayerOf(['sessions', 'keep']), isNotNull, reason: 'and said to be one');

    await saveDesktopConfig(config.copyWith(brightness: 'light'), tool: tool, files: files);
    final written = own.readAsStringSync();
    expect(written, contains('brightness = "light"'));
    expect(written, isNot(contains('keep')), reason: 'a value a lower layer gives is never copied into the own file');
    expect(low.readAsStringSync(), contains('keep = 500'), reason: 'a lower layer is never written');

    // The next change sends its own table only: the first one is already there.
    final again = (await loadDesktopConfig(tool: tool, files: files))!;
    expect(again.brightness, 'light');
    await saveDesktopConfig(again.copyWith(density: ChatDensity.verbose), tool: tool, files: files);
    expect(own.readAsStringSync(), allOf(contains('brightness = "light"'), contains('verbose')));
  }, skip: _host == null ? 'CANOA_TEST_HOST names a built canoa' : null);
}
