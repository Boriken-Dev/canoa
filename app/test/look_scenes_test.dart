// Settings, Loadouts, Accounts and Hosts drawn in each look and brightness, at desktop width and, in two of the four, phone width,
// so the management pages can be seen as one app with the chat. Goldens are drawn in the test font; with CANOA_SAMPLES_DIR set the
// same scenes are written with real fonts for `ui/samples/canoa/`.
@Timeout(Duration(minutes: 3))
library;

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/connection.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/served/host_files.dart';
import 'package:canoa/shell/hosts/host_status.dart';
import 'package:canoa/shell/hosts/hosts_controller.dart';
import 'package:canoa/shell/hosts/hosts_section.dart';
import 'package:canoa/shell/layout_state.dart';
import 'package:canoa/store/store_section.dart';
import 'package:canoa/surfaces/files_surface.dart';
import 'package:canoa/theme/look.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/catalog_standin.dart';
import 'fixtures/vendor_looks_fixture.dart';
import 'fixtures/fake_accounts.dart';
import 'fixtures/settings_fixture.dart';
import 'fixtures/store_fixtures.dart';
import 'loadout_test.dart' show registryAnswer, world, withCatalog;

final _deskId = 'sha256:${'a' * 64}';
final _agentId = 'sha256:${'b' * 64}';
final _desk = Connection(name: 'Desk', kind: ConnectionKind.canoa, address: 'https://desk.example:7800/', peer: _deskId, method: 'noise');
final _agent = Connection(name: 'One agent', kind: ConnectionKind.acp, address: 'wss://agent.example/acp', peer: _agentId, method: 'webcrypto');

const _accountsConfig = CanoaConfig(host: {
  'agents': {
    'Claude': {'harness': 'claude', 'accounts': ['work', 'metered', 'old']},
  },
});

/// A host's workspace held in memory: two folders and two files.
final class _Workspace implements WorkspaceFiles {
  static const root = '/work/canoa';
  static const _main = "import 'dart:io';\n\nvoid main() {\n  stdout.writeln('hello');\n}\n";

  @override
  Future<HostFolderListing> listFolders({String? path, bool hidden = false, bool files = false}) async => HostFolderListing(
    path: root,
    parent: '/work',
    more: false,
    entries: const [
      HostFolderEntry(name: 'docs', path: '$root/docs', isFolder: true),
      HostFolderEntry(name: 'src', path: '$root/src', isFolder: true),
      HostFolderEntry(name: 'main.dart', path: '$root/main.dart', isFolder: false),
      HostFolderEntry(name: 'README.md', path: '$root/README.md', isFolder: false),
    ],
  );

  @override
  Future<HostFileRead> readFile(String path, {int? maxBytes}) async =>
      HostFileRead(path: path, name: 'main.dart', size: _main.length, kind: 'text', truncated: false, bytes: Uint8List.fromList(utf8.encode(_main)));
}

const _looks = [
  (LookId.canoa, Brightness.dark),
  (LookId.canoa, Brightness.light),
  (LookId.terminal, Brightness.dark),
  (LookId.terminal, Brightness.light),
  (claudeLook, Brightness.dark),
  (claudeLook, Brightness.light),
  (chatgptLook, Brightness.dark),
  (chatgptLook, Brightness.light),
];

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/look_scenes_test.dart')));

  for (final (id, brightness) in _looks) {
    final name = '${id.name}_${brightness.name}';
    for (final (label, size) in [('desktop', const Size(1280, 800)), ('phone', const Size(390, 844))]) {
      // The phone is drawn in dark canoa and light terminal; the desktop in every family and brightness.
      if (label == 'phone' && !((id == LookId.canoa && brightness == Brightness.dark) || (id == LookId.terminal && brightness == Brightness.light))) continue;
      group('$name at $label width', () {
        final theme = themeFor(id, brightness);

        Future<void> golden(WidgetTester tester, String surface) async {
          expect(tester.takeException(), isNull);
          await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/look_${surface}_${name}_$label.png'));
        }

        testWidgets('Settings, Appearance', (tester) async {
          await openSettings(
            tester,
            config: CanoaConfig.parse('[appearance]\nlook = "${id.name}"\nbrightness = "${brightness.name}"\n'),
            size: size,
            section: 'appearance',
            theme: theme,
            ledger: SessionLedger(MemoryLedgerBackend()),
            version: '0.0.1',
          );
          await golden(tester, 'settings');
        });

        testWidgets('Loadouts', (tester) async {
          final overlays = world();
          await tester.binding.setSurfaceSize(size);
          addTearDown(() => tester.binding.setSurfaceSize(null));
          await tester.pumpWidget(MaterialApp(
            debugShowCheckedModeBanner: false,
            theme: theme,
            home: StoreSection(
              config: CanoaConfig.parse(withCatalog),
              home: MemoryHome(overlays),
              index: standInIndex(overlays, () => CanoaConfig.parse(withCatalog)),
              agents: const ['Claude', 'Codex'],
              onSave: (changed) async => changed,
              readCatalog: standInCatalogRead(overlays, () => CanoaConfig.parse(withCatalog), registry: (uri) async => registryAnswer),
              findProgram: (program) async => '/usr/bin/$program',
            ),
          ));
          await tester.pumpAndSettle();
          await golden(tester, 'loadouts');
        });

        testWidgets('Accounts', (tester) async {
          final host = FakeAccountsHost(accounts: [
            account('work', kind: 'claude-login', id: 'id-work'),
            account('metered'),
            account('old', kind: 'claude-login', state: 'needs sign-in'),
          ]);
          final controller = AccountsController(host: host, config: () => _accountsConfig, change: (_) async {}, usage: (_) => null);
          await controller.refresh();
          await openSettings(tester, config: _accountsConfig, size: size, section: 'accounts', accounts: controller, theme: theme, version: '0.0.1');
          await golden(tester, 'accounts');
        });

        testWidgets('Files, a folder and a file previewed', (tester) async {
          await tester.binding.setSurfaceSize(Size(size.width < 500 ? size.width : 480, size.height));
          addTearDown(() => tester.binding.setSurfaceSize(null));
          await tester.pumpWidget(MaterialApp(
            debugShowCheckedModeBanner: false,
            theme: theme,
            home: Scaffold(body: FilesPanel(files: _Workspace(), root: _Workspace.root)),
          ));
          await tester.pumpAndSettle();
          await tester.tap(find.byKey(const ValueKey('files entry main.dart')));
          await tester.pumpAndSettle();
          await golden(tester, 'files');
        });

        testWidgets('Hosts', (tester) async {
          await tester.binding.setSurfaceSize(size);
          addTearDown(() => tester.binding.setSurfaceSize(null));
          final controller = HostsController(
            layout: ShellLayout(backend: MemoryLayoutBackend()),
            save: (_) async {},
            sessions: () => const [],
            probe: (connection) async => connection.name == 'Desk'
                ? HostStatus(HostState.online, peer: connection.peer, method: connection.method, agents: const [HostAgent('Claude', 'wss://desk.example:7801/claude')],
                    capabilities: const ['Management (sessions, folders, files)'])
                : const HostStatus(HostState.offline),
            local: () => const HostSummary(state: 'Running', agents: ['Claude', 'Codex'], capabilities: ['Session list', 'Folder browsing']),
            mayWrite: () => true,
          )..adopt(const CanoaConfig().withConnection(_agent).withConnection(_desk));
          addTearDown(controller.dispose);
          await tester.pumpWidget(MaterialApp(
            debugShowCheckedModeBanner: false,
            theme: theme,
            home: Scaffold(
              body: label == 'phone'
                  ? HostsOverview(controller: controller, localDetail: (_) => const Text('Agents this computer hosts'))
                  : Row(children: [
                      SizedBox(width: 300, child: HostsIndex(controller: controller)),
                      Expanded(child: HostsOverview(controller: controller, localDetail: (_) => const Text('Agents this computer hosts'))),
                    ]),
            ),
          ));
          await tester.pumpAndSettle();
          if (label == 'desktop') {
            await tester.tap(find.byKey(const ValueKey('host-row connection:Desk')));
            await tester.pumpAndSettle();
          }
          await golden(tester, 'hosts');
        });
      });
    }
  }
}
