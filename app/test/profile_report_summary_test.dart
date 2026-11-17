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

  test('the line names the loadout the host started the session with, and says nothing of one when none applied', () {
    const applied = ItemState.applied;
    final items = [const ProfileItem('env.PATH', 'overlay:alpha', applied, 'the agent process environment')];
    expect(ProfileReport('claude-code', items, loadout: 'mail').summary, 'Profile for Claude Code (loadout mail): 1 applied');
    expect(ProfileReport('claude-code', items).summary, 'Profile for Claude Code: 1 applied');
    final report = hostProfileOf({'harness': 'claude-code', 'loadout': 'mail', 'items': const []}, fallback: 'a')!.report;
    expect(report.loadout, 'mail');
    expect(hostProfileOf({'harness': 'claude-code', 'loadout': null, 'items': const []}, fallback: 'a')!.report.loadout, isNull);
    expect(hostProfileOf({'harness': 'claude-code', 'loadout': 3, 'items': const []}, fallback: 'a')!.report.loadout, isNull);
  });
}
