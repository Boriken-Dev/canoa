// The protected list: its syntax (the one coqui-acp-pi's `protected` setting has), what a remembered rule does with it, and the
// canoa.cfg key and Settings field that hold it.
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/config/settings_dialog.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/permissions/protected_paths.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

const _home = ['h'];

List<String> _at(String rest) => ['h', ...rest.split('/')];

ProtectedPaths _list(List<String> lines) => ProtectedPaths(lines);

bool _protects(ProtectedPaths list, String rest, {List<String>? home = _home}) => list.protects(_at(rest), home: home);

/// A path rule over `/work/calc`, asked about [file].
String? _problem(String file, {ProtectedPaths? list, String? home, String scope = '/work/calc'}) => pathProblem(
  scope: scope,
  file: file,
  folder: '/work/calc',
  guards: PermissionGuards(home: home, protected: list),
  real: (path) => path,
);

void main() {
  group('the list syntax is the one of the Pi translator', () {
    test('a pattern covers what is under it and a later line wins', () {
      final rules = _list(['~/.ssh', '!~/.ssh/known_hosts', '~/.ssh/known_hosts2']);
      expect(_protects(rules, '.ssh/id_rsa'), isTrue);
      expect(_protects(rules, '.ssh'), isTrue);
      expect(_protects(rules, '.ssh/known_hosts'), isFalse);
      expect(_protects(rules, '.ssh/known_hosts2'), isTrue);
      expect(_protects(rules, '.sshx/id'), isFalse);
      expect(_protects(rules, 'other/.ssh'), isFalse, reason: 'anchored at the home folder');
    });

    test('a pattern without an anchor matches anywhere and wildcards work', () {
      final rules = _list(['# secrets', '', '.env', 'keys/*.pem', '**/vault/**/token']);
      expect(_protects(rules, 'a/b/.env'), isTrue);
      expect(_protects(rules, 'a/keys/x.pem'), isTrue);
      expect(_protects(rules, 'a/keys/x.txt'), isFalse);
      expect(_protects(rules, 'p/vault/q/r/token'), isTrue);
      expect(_protects(rules, 'p/vault/q/r/tokens'), isFalse);
    });

    test('what is not in the list is not protected, and a home entry needs a home', () {
      expect(_protects(_list([]), '.ssh/id_rsa'), isFalse);
      expect(_protects(_list(['~/x']), 'x', home: null), isFalse);
    });

    test('a drive or a slash starts at the file system, and names compare whatever their case', () {
      final rules = _list(['C:\\Secrets', '/etc/shadow']);
      expect(rules.protects(['c:', 'secrets', 'a'], home: null), isTrue);
      expect(rules.protects(['d:', 'secrets', 'a'], home: null), isFalse);
      expect(rules.protects(['etc', 'shadow'], home: null), isTrue);
      expect(rules.protects(['x', 'etc', 'shadow'], home: null), isFalse);
      expect(_protects(_list(['.ENV']), 'a/.env'), isTrue);
    });

    test('the recommended list is a list a person can cut', () {
      expect(ProtectedPaths.recommended.isRecommended, isTrue);
      final all = ProtectedPaths.recommended;
      expect(_protects(all, 'p/.git/config'), isTrue);
      expect(_protects(all, 'p/package.json'), isTrue);
      expect(_protects(all, 'p/a.config.js'), isTrue);
      expect(_protects(all, 'p/src/main.dart'), isFalse);
      final cut = _list([...recommendedProtected.where((line) => line != 'package.json'), '!.github']);
      expect(cut.isRecommended, isFalse);
      expect(_protects(cut, 'p/package.json'), isFalse);
      expect(_protects(cut, 'p/.github/ci.yml'), isFalse);
      expect(_protects(cut, 'p/.git/config'), isTrue);
    });

    test('a canoa.cfg value is a string or a list of strings', () {
      expect(protectedLinesOf('a\r\nb\n!c'), ['a', 'b', '!c']);
      expect(protectedLinesOf(['a', '#b']), ['a', '#b']);
      expect(protectedLinesOf(3), isNull);
      expect(protectedLinesOf(['a', 3]), isNull);
    });

    test('a name no list can judge is refused, not normalised', () {
      for (final raw in ['con', 'NUL.txt', 'GIT~1', '...', ' ', 'caf\u00e9', 'a\u200bb']) {
        expect(readName(raw), isNull, reason: raw);
      }
      expect(readName('Package.JSON. '), 'package.json');
    });
  });

  group('a remembered rule and the list', () {
    test('a path matching a pattern gets no rule offer', () {
      final list = _list(['secrets', 'out/*.log']);
      expect(_problem('/work/calc/src/a.js', list: list), isNull);
      expect(_problem('/work/calc/secrets/key', list: list, scope: '/work/calc/secrets'), isNotNull);
      expect(_problem('/work/calc/src/secrets', list: list, scope: '/work/calc/src'), isNotNull);
      expect(_problem('/work/calc/out/a.log', list: list, scope: '/work/calc/out'), isNotNull);
      expect(_problem('/work/calc/out/a.txt', list: list, scope: '/work/calc/out'), isNull);
    });

    test('a "!" line allows one back', () {
      final list = _list([...recommendedProtected, '!.github']);
      expect(_problem('/work/calc/.github/ci.yml', list: ProtectedPaths.recommended, scope: '/work/calc/.github'), isNotNull);
      expect(_problem('/work/calc/.github/ci.yml', list: list, scope: '/work/calc/.github'), isNull);
      expect(_problem('/work/calc/.git/config', list: list, scope: '/work/calc/.git'), isNotNull);
    });

    test('removing a recommended entry takes effect, and the default list is the recommended one', () {
      expect(_problem('/work/calc/package.json'), isNotNull);
      final without = _list(recommendedProtected.where((line) => line != 'package.json').toList());
      expect(_problem('/work/calc/package.json', list: without), isNull);
      expect(_problem('/work/calc/Makefile', list: without), isNotNull);
    });

    test('the mechanism stays when the list is empty: names that cannot be placed, and a link into a listed place', () {
      final none = _list([]);
      expect(_problem('/work/calc/CON', list: none), isNotNull);
      expect(_problem('/work/calc/GIT~1/config', list: none), isNotNull);
      expect(_problem('/work/calc/caf\u00e9', list: none), isNotNull);
      expect(_problem('/work/calc/.git/config', list: none, scope: '/work/calc/.git'), isNull, reason: 'nothing listed, nothing protected');
      expect(
        pathProblem(scope: '/work/calc/src', file: '/work/calc/src/pkg', folder: '/work/calc', guards: PermissionGuards(protected: _list(['package.json'])), real: (path) => path == '/work/calc/src/pkg' ? '/work/calc/package.json' : path),
        isNotNull,
        reason: 'a link is judged by where it leads',
      );
    });

    test('a dot above the session folder is not the folder\'s, and a home line protects a folder inside it', () {
      final list = ProtectedPaths.recommended;
      expect(
        pathProblem(scope: '/work/.hidden/calc', file: '/work/.hidden/calc/a.js', folder: '/work/.hidden/calc', guards: PermissionGuards(protected: list), real: (path) => path),
        isNull,
      );
      final home = _list(['~/vault']);
      expect(
        pathProblem(scope: '/home/u/vault/p', file: '/home/u/vault/p/a.js', folder: '/home/u/vault/p', guards: PermissionGuards(protected: home), real: (path) => path),
        isNull,
        reason: 'with no home folder a "~/" line is inert',
      );
      expect(
        pathProblem(scope: '/home/u/vault/p', file: '/home/u/vault/p/a.js', folder: '/home/u/vault/p', guards: PermissionGuards(home: '/home/u', protected: home), real: (path) => path),
        isNotNull,
        reason: 'the folder is inside a protected place',
      );
    });

    test('PermissionRules uses a list at once, for rules already made', () {
      final rules = PermissionRules(MemoryLedgerBackend(), real: (path) => path);
      final subject = subjectOf(kind: 'edit', rawInput: {'file_path': '/work/calc/package.json', 'old_string': 'a', 'new_string': 'b'})!;
      expect(rules.checkScope(RuleType.path, '/work/calc', subject, '/work/calc'), isNotNull);
      rules.useProtected(_list([]));
      expect(rules.checkScope(RuleType.path, '/work/calc', subject, '/work/calc'), isNull);
      rules.useProtected(null);
      expect(rules.checkScope(RuleType.path, '/work/calc', subject, '/work/calc'), isNotNull);
    });
  });

  group('canoa.cfg and Settings', () {
    test('`[permissions] protected` is read as text or a list, checked and written back', () {
      expect(const CanoaConfig().protectedPaths, isNull);
      final text = CanoaConfig.parse('[permissions]\nprotected = """\n# mine\nsecrets\n!secrets/ok\n"""\n');
      expect(text.protectedPaths, ['# mine', 'secrets', '!secrets/ok', '']);
      final list = CanoaConfig.parse('[permissions]\nprotected = ["secrets", "# mine", "!secrets/ok"]\n');
      expect(list.protectedPaths, ['secrets', '# mine', '!secrets/ok']);
      expect(CanoaConfig.parse(list.encode()).protectedPaths, list.protectedPaths);
      expect(CanoaConfig.parse(const CanoaConfig().encode()).protectedPaths, isNull);
      expect(const CanoaConfig().encode(), isNot(contains('protected')));
      expect(CanoaConfig.parse('[permissions]\nprotected = []\n').protectedPaths, isEmpty);
      expect(() => CanoaConfig.parse('[permissions]\nprotected = 3\n'), throwsFormatException);
      expect(() => CanoaConfig.parse('[permissions]\nprotected = ["a", 3]\n'), throwsFormatException);
    });

    Future<void> open(WidgetTester tester, PermissionRules rules, void Function(CanoaConfig?) saved, {CanoaConfig config = const CanoaConfig()}) async {
      await tester.binding.setSurfaceSize(const Size(1100, 1800));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await tester.pumpWidget(MaterialApp(
        home: Builder(
          builder: (context) => TextButton(
            onPressed: () async => saved(await showDialog<CanoaConfig>(context: context, builder: (_) => SettingsDialog(config: config, rules: rules))),
            child: const Text('open'),
          ),
        ),
      ));
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('settings-permissions')));
      await tester.pumpAndSettle();
    }

    testWidgets('the field shows the recommended list, an edit replaces it, Save puts it to use', (tester) async {
      final rules = PermissionRules(MemoryLedgerBackend(), real: (path) => path);
      CanoaConfig? saved;
      await open(tester, rules, (config) => saved = config);
      final field = find.byKey(const Key('settings-protected-paths'));
      expect(tester.widget<TextField>(field).controller!.text, recommendedProtected.join('\n'));
      expect(find.text('Every recommended line is kept.'), findsOneWidget);
      await tester.enterText(field, recommendedProtected.where((line) => line != 'package.json').join('\n'));
      await tester.pump();
      expect(find.text('1 recommended line is removed.'), findsOneWidget);
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(saved!.protectedPaths, isNot(contains('package.json')));
      expect(saved!.protectedPaths, contains('makefile'));
      final subject = subjectOf(kind: 'edit', rawInput: {'file_path': '/work/calc/package.json', 'old_string': 'a', 'new_string': 'b'})!;
      expect(rules.checkScope(RuleType.path, '/work/calc', subject, '/work/calc'), isNull, reason: 'the removed entry no longer protects');
    });

    testWidgets('Restore recommended brings the list back and writes none', (tester) async {
      final rules = PermissionRules(MemoryLedgerBackend(), real: (path) => path);
      CanoaConfig? saved;
      await open(tester, rules, (config) => saved = config, config: const CanoaConfig(protectedPaths: ['secrets']));
      final field = find.byKey(const Key('settings-protected-paths'));
      expect(tester.widget<TextField>(field).controller!.text, 'secrets');
      await tester.tap(find.byKey(const Key('restore-recommended-paths')));
      await tester.pump();
      expect(tester.widget<TextField>(field).controller!.text, recommendedProtected.join('\n'));
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(saved!.protectedPaths, isNull);
      expect(saved!.encode(), isNot(contains('protected')));
    });
  });
}
