// Settings' Scheduled tasks page: the host's tasks listed with the next run and the last result, added through the dialog, switched
// off, run at once and removed, each as the host's `schedules/*` methods are called, and its words in both languages.
import 'dart:io';

import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/l10n/app_text.dart';
import 'package:canoa/settings/schedules_settings.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/store_fixtures.dart' show PlatformTolerant;

/// A host that keeps its tasks in memory and records what it was asked.
final class _Host {
  final asked = <(String, Map<String, Object?>)>[];
  final tables = <String, Map<String, Object?>>{
    'nightly': {'entry': 'fixture', 'folder': '/work', 'prompt': 'run the tests', 'repeat': 'daily', 'at': '03:00', 'enabled': true, 'timeout_minutes': 120},
  };
  ControlFailed? refuse;

  Future<Object?> call(String method, Map<String, Object?> params) async {
    asked.add((method, params));
    if (refuse != null) throw refuse!;
    switch (method) {
      case 'schedules/list':
        return {
          'tasks': [
            for (final MapEntry(:key, :value) in tables.entries)
              {
                'id': key,
                ...value,
                'weekdays': [
                  if (value['weekdays'] is List) for (final day in value['weekdays']! as List) weekdayKeys.indexOf(day! as String),
                ],
                'next': DateTime(2030, 1, 2, 3).millisecondsSinceEpoch,
                'running': false,
                'last': key == 'nightly' ? {'at': DateTime(2029, 12, 31, 3).millisecondsSinceEpoch, 'result': 'completed', 'note': 'All 12 tests pass', 'session': 's1'} : null,
              },
          ],
          'entries': ['fixture', 'other'],
          'now': 0,
        };
      case 'schedules/set':
        tables[params['id']! as String] = (params['task']! as Map).cast<String, Object?>();
        return {'id': params['id']};
      case 'schedules/remove':
        tables.remove(params['id']);
        return {'id': params['id']};
      default:
        return {'started': true};
    }
  }
}

Future<(_Host, ScheduledTasks)> _page(WidgetTester tester, {String language = 'en'}) async {
  await tester.binding.setSurfaceSize(const Size(900, 1000));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final host = _Host();
  final tasks = ScheduledTasks(host.call);
  addTearDown(tasks.dispose);
  await tester.pumpWidget(
    MaterialApp(
      builder: (context, child) => AppTextHost(language: language, child: child!),
      home: Scaffold(body: SingleChildScrollView(child: SchedulesSection(tasks: tasks, folders: const ['/work']))),
    ),
  );
  await tester.pump();
  await tester.pump();
  return (host, tasks);
}

void main() {
  setUpAll(() => goldenFileComparator = PlatformTolerant(Uri.file('${Directory.current.path}/test/schedules_settings_test.dart')));

  testWidgets('lists each task with when it runs next and what became of its last run', (tester) async {
    await _page(tester);
    expect(find.byKey(const Key('schedule-nightly')), findsOneWidget);
    expect(find.text('Every day at 03:00'), findsOneWidget);
    expect(find.text('On fixture, in /work'), findsOneWidget);
    expect(find.textContaining('Next run: '), findsOneWidget);
    expect(find.textContaining('Last run, finished, '), findsOneWidget);
    expect(find.textContaining(': All 12 tests pass'), findsOneWidget);
  });

  testWidgets('draws at desktop width', (tester) async {
    final (host, _) = await _page(tester);
    host.tables['weekly-review'] = {
      'entry': 'other',
      'folder': '/work/site',
      'prompt': 'Summarise what changed this week and list what is left',
      'title': 'Weekly review',
      'repeat': 'weekly',
      'at': '08:15',
      'weekdays': ['mon', 'fri'],
      'enabled': false,
      'timeout_minutes': 120,
    };
    await tester.tap(find.byKey(const Key('schedule-run-nightly')));
    await tester.pumpAndSettle();
    await expectLater(find.byType(MaterialApp), matchesGoldenFile('goldens/schedules_settings_desktop.png'));
  });

  testWidgets('says so when nothing is scheduled', (tester) async {
    final (host, _) = await _page(tester);
    host.tables.clear();
    await tester.tap(find.byKey(const Key('schedule-delete-nightly')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('schedule-remove-confirm')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('schedules-empty')), findsOneWidget);
  });

  testWidgets('adds a weekly task through the dialog, read in the host\'s own time zone', (tester) async {
    final (host, _) = await _page(tester);
    await tester.tap(find.byKey(const Key('schedule-add')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('schedule-name')), 'Weekly review');
    await tester.enterText(find.byKey(const Key('schedule-prompt')), 'Summarise the week');
    await tester.tap(find.byKey(const Key('schedule-repeat')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Some days of the week').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('schedule-day-5')));
    await tester.enterText(find.byKey(const Key('schedule-time')), '8:15');
    await tester.tap(find.byKey(const Key('schedule-save')));
    await tester.pumpAndSettle();
    final (method, params) = host.asked.lastWhere((each) => each.$1 == 'schedules/set');
    expect(method, 'schedules/set');
    expect(params['id'], 'weekly-review');
    expect(params['task'], {
      'entry': 'fixture',
      'folder': '/work',
      'prompt': 'Summarise the week',
      'title': 'Weekly review',
      'repeat': 'weekly',
      'at': '08:15',
      'weekdays': ['mon', 'fri'],
    });
    expect(find.byKey(const Key('schedule-weekly-review')), findsOneWidget);
  });

  testWidgets('does not save a task that lacks what the host needs, and says what', (tester) async {
    final (host, _) = await _page(tester);
    await tester.tap(find.byKey(const Key('schedule-add')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('schedule-save')));
    await tester.pumpAndSettle();
    expect(find.text('A folder and a prompt are needed'), findsOneWidget, reason: 'the folder is offered, so the prompt is what is missing');
    expect(find.byKey(const Key('schedule-wrong')), findsOneWidget);
    expect(host.asked.where((each) => each.$1 == 'schedules/set'), isEmpty);
  });

  testWidgets('switches a task off, runs it now and removes it, each by the host\'s own methods', (tester) async {
    final (host, _) = await _page(tester);
    await tester.tap(find.byKey(const Key('schedule-enabled-nightly')));
    await tester.pumpAndSettle();
    expect(host.tables['nightly']!['enabled'], false);
    expect(host.tables['nightly']!['prompt'], 'run the tests', reason: 'what else it said is kept');
    await tester.tap(find.byKey(const Key('schedule-run-nightly')));
    await tester.pumpAndSettle();
    expect(host.asked.any((each) => each.$1 == 'schedules/run' && each.$2['id'] == 'nightly'), isTrue);
    await tester.tap(find.byKey(const Key('schedule-delete-nightly')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('schedule-remove-confirm')));
    await tester.pumpAndSettle();
    expect(host.tables, isEmpty);
    expect(find.byKey(const Key('schedule-nightly')), findsNothing);
  });

  testWidgets('a key that is not the host\'s owner is told only the owner sets tasks', (tester) async {
    final host = _Host()..refuse = const ControlFailed(-32001, 'denied');
    await tester.binding.setSurfaceSize(const Size(900, 1000));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    final tasks = ScheduledTasks(host.call);
    addTearDown(tasks.dispose);
    await tester.pumpWidget(MaterialApp(home: Scaffold(body: SchedulesSection(tasks: tasks))));
    await tester.pump();
    await tester.pump();
    expect(find.byKey(const Key('schedules-problem')), findsOneWidget);
    expect(find.textContaining('Only the person running this host'), findsOneWidget);
  });

  testWidgets('is in Spanish when the app is', (tester) async {
    await _page(tester, language: 'es');
    expect(find.text('Todos los días a las 03:00'), findsOneWidget);
    expect(find.textContaining('Próxima ejecución: '), findsOneWidget);
    expect(find.text('Añadir una tarea'), findsOneWidget);
  });
}
