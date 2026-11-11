// A preset's program is found on PATH, else in the directory its installer uses.
import 'dart:io';

import 'package:canoa/acp/local_agent_io.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a program off PATH is found in its installer\'s directory', () async {
    final dir = Directory.systemTemp.createTempSync('canoa-install');
    addTearDown(() => dir.deleteSync(recursive: true));
    const name = 'canoa-no-such-tool';
    expect(await findProgram(name), isNull);
    expect(await findProgram(name, searchDirs: [dir.path]), isNull, reason: 'nothing is there yet');
    final file = File('${dir.path}/$name${Platform.isWindows ? '.exe' : ''}')..writeAsBytesSync(const [0]);
    expect(await findProgram(name, searchDirs: ['/no/such/dir', dir.path]), file.path);
  });

  test('~ and %NAME% in a directory are filled in, and a variable that is not set skips it', () async {
    final home = environmentVariable(Platform.isWindows ? 'USERPROFILE' : 'HOME')!;
    final dir = Directory('$home/canoa-install-probe')..createSync();
    addTearDown(() => dir.deleteSync(recursive: true));
    const name = 'canoa-no-such-tool';
    final file = File('${dir.path}/$name${Platform.isWindows ? '.exe' : ''}')..writeAsBytesSync(const [0]);
    expect(await findProgram(name, searchDirs: ['~/canoa-install-probe']), file.path);
    expect(await findProgram(name, searchDirs: ['%CANOA_NOT_SET_VARIABLE%/x']), isNull);
  });
}
