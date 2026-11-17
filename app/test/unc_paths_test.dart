// A workspace on a Windows share (`\\server\share\folder`) is a place like any other: chosen, grouped and named the same however
// the host wrote its path.
import 'package:canoa/rail/session_groups.dart';
import 'package:canoa/served/folder_dialog.dart' show breadcrumbs;
import 'package:canoa/session_store.dart';
import 'package:canoa/workspace/remote_host.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const share = r'\\localhost\C$\work\repo';

  test('a share is a Windows path, written with either slash', () {
    expect(pathFitsSystem('windows', share), isTrue);
    expect(pathFitsSystem('windows', '//localhost/C\$/work/repo'), isTrue);
    expect(pathFitsSystem('linux', share), isFalse);
    expect(pathFitsSystem('linux', '//usr/lib'), isTrue);
  });

  test('breadcrumbs of a share start at the server and the share', () {
    expect(
      [for (final each in breadcrumbs(share)) each.path],
      [r'\\localhost', r'\\localhost\C$', r'\\localhost\C$\work', share],
    );
  });

  test('every way of writing a share names the same workspace', () {
    final same = {
      workspaceIdentity(share),
      workspaceIdentity(r'\\?\UNC\localhost\C$\work\repo'),
      workspaceIdentity('//localhost/C\$/work/repo'),
      workspaceIdentity(r'\\LOCALHOST\c$\work\repo\'),
    };
    expect(same, hasLength(1));
    expect(workspaceIdentity(r'\\?\C:\work'), workspaceIdentity(r'c:\work\'));
    expect(workspaceIdentity('/srv/App'), '/srv/App', reason: 'a path elsewhere is as it was');
  });

  test('recent workspaces keep one entry for a share however it was written, the first way first', () {
    expect(
      distinctWorkspaces([
        share,
        r'\\?\UNC\localhost\C$\work\repo',
        '//localhost/C\$/work/repo/',
        r'C:\work\other',
        r'c:\WORK\other',
        '/srv/app',
        '/srv/App',
      ]),
      [share, r'C:\work\other', '/srv/app', '/srv/App'],
    );
  });

  test('the list grouped by workspace shows one group for a share, named by its folder', () {
    final store = SessionStore();
    final shared = store.createDraft(cwd: share);
    store.createDraft(cwd: r'\\?\UNC\localhost\C$\work\repo');
    store.createDraft(cwd: r'C:\work\other');
    final groups = groupByWorkspace(store.sessions);
    expect([for (final group in groups) group.label], containsAll(['repo', 'other']));
    expect(groups.where((group) => group.label == 'repo').single.sessions, hasLength(2));
    expect(locationOf(shared), 'repo');
  });
}
