// Open in on a page a host serves, by the host's web mode: local, the host starts the program; remote, VS Code Remote's link and
// the Files panel. Starts nothing real: the host and the browser's link are stand-ins.
import 'package:canoa/acp/served_host.dart';
import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/ledger/session_ledger.dart' show SessionPlace;
import 'package:canoa/served/host_files.dart' show HostOpeners, HostOpening;
import 'package:canoa/served/web_mode.dart';
import 'package:canoa/workspace/open_in.dart';
import 'package:canoa/workspace/open_in_host.dart';
import 'package:flutter_test/flutter_test.dart';

final class _Host implements HostOpening {
  _Host({this.ids = const ['vscode', 'file_manager', 'reveal'], this.os = 'linux', this.refuse});

  final List<String> ids;
  final String os;
  final ControlFailed? refuse;
  final started = <(String, String)>[];

  @override
  Future<HostOpeners> workspaceOpeners() async {
    if (refuse case final failure?) throw failure;
    return HostOpeners(os: os, ids: ids);
  }

  @override
  Future<void> workspaceOpen(String path, String opener) async {
    if (refuse case final failure?) throw failure;
    started.add((path, opener));
  }
}

const _peer = 'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

Future<WorkspaceOpener> _page(WebMode mode, _Host host, {Uri? page, List<Uri>? links, bool linkWorks = true}) async {
  final hosted = HostedOpening(
    mode: mode,
    opening: host,
    hostPeer: _peer,
    page: page ?? Uri.parse('http://127.0.0.1:7800/'),
    link: (uri) async {
      links?.add(uri);
      return linkWorks;
    },
  );
  await hosted.load();
  return WorkspaceOpener.hosted(hosted);
}

List<String> _ids(WorkspaceOpener opener, WorkspaceContext context, WorkspaceTarget target) => [for (final a in opener.actionsFor(context, target)) a.id];

void main() {
  const folder = WorkspaceTarget('/home/ana/work');

  group('the mode', () {
    test('is what the host says, else loopback is local and any other name remote', () {
      expect(WebMode.of(WebMode.remote, Uri.parse('http://127.0.0.1:7800/')), WebMode.remote);
      expect(WebMode.of(WebMode.local, Uri.parse('https://box.example/')), WebMode.local);
      expect(WebMode.of(null, Uri.parse('http://127.0.0.1:7800/')), WebMode.local);
      expect(WebMode.of(null, Uri.parse('http://localhost:7800/')), WebMode.local);
      expect(WebMode.of(null, Uri.parse('http://[::1]:7800/')), WebMode.local);
      expect(WebMode.of(null, Uri.parse('https://box.example/')), WebMode.remote);
      expect(WebMode.of(null, Uri.parse('http://192.0.2.17:7800/')), WebMode.remote);
    });

    test('canoa.json carries it, and an older host or a word it does not know says none', () {
      String index(String extra) => '{"peer":"$_peer","method":"noise","entries":[]$extra}';
      expect(ServedHost.parse(index(',"web":"remote"'))!.web, WebMode.remote);
      expect(ServedHost.parse(index(',"web":"local"'))!.web, WebMode.local);
      expect(ServedHost.parse(index(''))!.web, isNull);
      expect(ServedHost.parse(index(',"web":"elsewhere"'))!.web, isNull);
    });
  });

  group('local: the host starts the program', () {
    test('what the host has is what is offered, and each action asks the host to start it for the folder', () async {
      final host = _Host();
      final opener = await _page(WebMode.local, host);
      expect(opener.canOpen, isTrue);
      expect(_ids(opener, WorkspaceContext.header, folder), ['vscode', 'file_manager']);
      expect(_ids(opener, WorkspaceContext.files, const WorkspaceTarget('/home/ana/work/a.txt', item: true)), ['vscode', 'reveal']);
      for (final action in opener.actionsFor(WorkspaceContext.header, folder)) {
        await action.run(folder);
      }
      await opener.actions.firstWhere((a) => a.id == 'reveal').run(const WorkspaceTarget('/home/ana/work/a.txt', item: true));
      expect(host.started, [
        ('/home/ana/work', 'vscode'),
        ('/home/ana/work', 'file_manager'),
        ('/home/ana/work/a.txt', 'reveal'),
      ]);
      expect(opener.note, isNull);
    });

    test('without VS Code on the host it is not offered, and the file manager is named for the host\'s system', () async {
      final opener = await _page(WebMode.local, _Host(ids: const ['file_manager', 'reveal'], os: 'windows'));
      final offered = opener.actionsFor(WorkspaceContext.header, const WorkspaceTarget(r'C:\work'));
      expect([for (final a in offered) a.id], ['file_manager']);
      expect(offered.single.label, 'Open in Explorer');
    });

    test('a key without the grant sees no action, and a refusal when starting says the host\'s own words', () async {
      final none = await _page(WebMode.local, _Host(refuse: ControlFailed(-32001, 'this key lacks the workspace.open grant')));
      expect(_ids(none, WorkspaceContext.header, folder), isEmpty);

      final failing = _Host(refuse: ControlFailed(-32002, 'VS Code was not found on this computer'));
      final hosted = HostedOpening(mode: WebMode.local, opening: failing, hostPeer: _peer, page: Uri.parse('http://localhost/'));
      final action = WorkspaceOpener.hosted(hosted).actions.firstWhere((a) => a.id == 'vscode');
      await expectLater(
        action.run(folder),
        throwsA(isA<OpenInFailure>().having((e) => e.message, 'message', 'VS Code was not found on this computer')),
      );
    });
  });

  group('remote: VS Code Remote and the Files panel', () {
    final reached = Uri.parse('https://box.example/app/');

    test('VS Code is a vscode-remote link on the host named in the address, the file manager is not offered', () async {
      final links = <Uri>[];
      final host = _Host();
      final opener = await _page(WebMode.remote, host, page: reached, links: links);
      expect(_ids(opener, WorkspaceContext.header, folder), ['vscode']);
      expect(_ids(opener, WorkspaceContext.files, const WorkspaceTarget('/home/ana/work/a.txt', item: true)), ['vscode']);
      await opener.actionsFor(WorkspaceContext.header, folder).single.run(folder);
      expect(links.single.toString(), 'vscode://vscode-remote/ssh-remote+box.example/home/ana/work');
      expect(host.started, isEmpty, reason: 'the host starts nothing for a person who is elsewhere');
      expect(opener.note, contains('Files panel'));
    });

    test('a path with spaces and a Windows path are one link each', () {
      expect(HostedOpening.vsCodeRemote('box', '/home/ana/my work/ñ').toString(), 'vscode://vscode-remote/ssh-remote+box/home/ana/my%20work/%C3%B1');
      expect(HostedOpening.vsCodeRemote('box', r'C:\Work\ana\proj').toString(), 'vscode://vscode-remote/ssh-remote+box/C:/Work/ana/proj');
    });

    test('a browser that opens no link says so', () async {
      final opener = await _page(WebMode.remote, _Host(), page: reached, linkWorks: false);
      await expectLater(opener.actions.first.run(folder), throwsA(isA<OpenInFailure>()));
    });
  });

  group('whose folder', () {
    test('a session on the page\'s own host is a target, another peer\'s or the browser\'s is not, with the reason', () async {
      final opener = await _page(WebMode.local, _Host());
      final here = opener.workspaceOf(place: SessionPlace.host, cwd: '/home/ana/work', peerId: _peer);
      expect(here.target?.path, '/home/ana/work');
      expect(opener.workspaceOf(place: SessionPlace.host, cwd: '/home/ana/work').target?.path, '/home/ana/work');
      final other = opener.workspaceOf(place: SessionPlace.peer, cwd: '/home/ana/work', peerId: 'sha256:${'b' * 64}');
      expect(other.target, isNull);
      expect(other.reason, contains('another host'));
      expect(opener.workspaceOf(place: SessionPlace.browser, cwd: '/home/ana/work').reason, contains('browser'));
      expect(opener.workspaceOf(place: SessionPlace.host, cwd: null), (target: null, reason: null));
    });
  });
}
