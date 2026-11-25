// The configuration file's name: canoa.cfg. A Canoa.cfg from before the rename is taken over by the host (`canoa control`, tested there).
import 'dart:io';

import 'package:canoa/config/canoa_config_io.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('the default place is canoa.cfg in the Canoa folder', () {
    final path = desktopConfigPath();
    if (path != null && Platform.environment['CANOA_CONFIG'] == null) {
      expect(path, endsWith('${Platform.pathSeparator}Canoa${Platform.pathSeparator}canoa.cfg'));
    }
  });
}
