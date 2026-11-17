// The one line the conversation shows of a profile: each thing not applied names the overlay it came from, once.
import 'package:canoa/profile/report.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('two overlays that miss the same thing are two lines, and one that says it twice is one', () {
    const missed = ItemState.unsupported;
    final report = ProfileReport('codex', const [
      ProfileItem('env.PATH', 'overlay:alpha', ItemState.applied, 'the agent process environment'),
      ProfileItem('setup: setup.sh', 'overlay:alpha', missed, 'Canoa does not apply it'),
      ProfileItem('setup: setup.sh', 'overlay:alpha', missed, 'Canoa does not apply it'),
      ProfileItem('setup: setup.sh', 'overlay:beta', missed, 'Canoa does not apply it'),
      ProfileItem('model', 'user', missed, 'the session sets it'),
    ]);
    expect(
      report.summary,
      'Profile for Codex: 1 applied; not applied: alpha: setup: setup.sh (not supported), '
      'beta: setup: setup.sh (not supported), model (not supported)',
    );
  });
}
