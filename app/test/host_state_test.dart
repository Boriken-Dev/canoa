import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/served/host_state.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('a page whose host cannot be reached keeps its state in the browser: the host\'s state is not offered', () async {
    final control = HostControl.dialing(() async => throw StateError('no link'), wait: const Duration(milliseconds: 50));
    expect(await HostState(control).available().timeout(const Duration(seconds: 20)), isFalse);
    await control.close();
  });
}
