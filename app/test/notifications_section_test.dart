// Settings > Notifications, drawn from what the host lists: each event with the notifiers it goes to, the notifiers with their state, and
// the forms that add, test and remove one. Every word comes from the app's text, in English and in Spanish.
import 'package:canoa/attention/notifications_host.dart';
import 'package:canoa/l10n/app_text.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

import 'fixtures/settings_fixture.dart';

const _events = ['permission', 'question', 'finished', 'failed', 'usage_warn', 'usage_stop', 'update_failed', 'host_restarted', 'host_updated'];

final class _Host implements NotificationsHost {
  _Host() {
    for (final id in _events) {
      routes[id] = id.startsWith('host_') ? [] : ['desktop'];
    }
  }

  final routes = <String, List<String>>{};
  final entries = <String, Map<String, Object?>>{};
  final saved = <({String name, String provider, Map<String, Object?> settings, Map<String, Object?> secrets})>[];
  final removed = <({String name, Map<String, List<String>> routes})>[];
  final tested = <String>[];
  bool testOk = true;

  static const providers = [
    NotifierProvider(
      id: 'ntfy',
      title: 'Phone push',
      help: 'A notice through a ntfy server.',
      schema: {
        'type': 'object',
        'required': ['topic'],
        'properties': {
          'topic': {'type': 'string', 'title': 'Topic'},
          'token': {'type': 'string', 'title': 'Token', 'writeOnly': true},
        },
      },
      form: {},
    ),
  ];

  @override
  Future<NotificationsSnapshot> list() async => NotificationsSnapshot(
    events: [
      for (final id in _events) NotificationEvent(id: id, title: 'host $id', notifiers: [...routes[id]!], set: false),
    ],
    notifiers: [
      const NotifierRow(name: 'desktop', provider: 'desktop', title: 'Desktop', state: 'no window is open'),
      for (final MapEntry(:key, :value) in entries.entries)
        NotifierRow(
          name: key,
          provider: 'ntfy',
          title: 'Phone push',
          state: 'ready',
          origin: value['layer'] as String?,
          removable: value['layer'] == null,
        ),
    ],
    providers: providers,
  );

  @override
  Future<void> route(String event, List<String> names) async => routes[event] = names;

  @override
  Future<void> save(String name, String provider, Map<String, Object?> settings, Map<String, Object?> secrets) async {
    saved.add((name: name, provider: provider, settings: settings, secrets: secrets));
    entries[name] = {};
  }

  @override
  Future<void> remove(String name, Map<String, List<String>> changed) async {
    removed.add((name: name, routes: changed));
    entries.remove(name);
    routes.addAll(changed);
  }

  @override
  Future<NotifierTest> test(String name) async {
    tested.add(name);
    return NotifierTest(ok: testOk, why: testOk ? null : 'the server said no');
  }
}

Future<(_Host, NotificationsController)> _open(WidgetTester tester, {_Host? host}) async {
  final held = host ?? _Host();
  final controller = NotificationsController(host: held);
  addTearDown(controller.dispose);
  await openSettings(tester, section: 'notifications', notifications: controller);
  await tester.pumpAndSettle();
  return (held, controller);
}

void main() {
  testWidgets('every event of the host is a row with its notifiers as chips, and an event that goes nowhere says Off', (tester) async {
    await _open(tester);
    expect(find.text('A session asks leave to use a tool'), findsOneWidget);
    expect(find.text('The host restarted'), findsOneWidget);
    expect(find.byKey(const Key('notification-off host_restarted')), findsOneWidget);
    expect(find.byKey(const Key('notification-off permission')), findsNothing);
    final chip = tester.widget<FilterChip>(find.byKey(const Key('notification-route permission desktop')));
    expect(chip.selected, isTrue);
    expect(tester.widget<FilterChip>(find.byKey(const Key('notification-route host_restarted desktop'))).selected, isFalse);
  });

  testWidgets('choosing a chip writes the event\'s list through the host and the row follows', (tester) async {
    final (host, _) = await _open(tester);
    await tester.tap(find.byKey(const Key('notification-route finished desktop')));
    await tester.pumpAndSettle();
    expect(host.routes['finished'], isEmpty);
    expect(find.byKey(const Key('notification-off finished')), findsOneWidget);
    await tester.tap(find.byKey(const Key('notification-route host_updated desktop')));
    await tester.pumpAndSettle();
    expect(host.routes['host_updated'], ['desktop']);
  });

  testWidgets('desktop is listed with its state, can be tested, and cannot be removed', (tester) async {
    final (host, _) = await _open(tester);
    expect(find.byKey(const Key('notifier desktop')), findsOneWidget);
    expect(find.textContaining('no window is open'), findsOneWidget);
    expect(find.byKey(const Key('notifier-remove desktop')), findsNothing);
    await tester.tap(find.byKey(const Key('notifier-test desktop')));
    await tester.pumpAndSettle();
    expect(host.tested, ['desktop']);
    expect(find.text('The test notice was sent.'), findsWidgets);
    host.testOk = false;
    await tester.tap(find.byKey(const Key('notifier-test desktop')));
    await tester.pumpAndSettle();
    expect(find.text('Not sent: the server said no'), findsWidgets);
  });

  testWidgets('a notifier is added from the kind the host offers; its secret goes apart from the settings', (tester) async {
    final (host, _) = await _open(tester);
    await tester.tap(find.byKey(const Key('add-notifier')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('add-notifier-submit')));
    await tester.pumpAndSettle();
    expect(find.text('Name the notifier'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('notifier-name')), 'Bad Name');
    await tester.tap(find.byKey(const Key('add-notifier-submit')));
    await tester.pumpAndSettle();
    expect(find.text('A name is 1 to 32 lower-case letters, digits, - and _'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('notifier-name')), 'phone');
    await tester.enterText(find.byKey(const Key('notifier-field-topic')), 'mine');
    await tester.enterText(find.byKey(const Key('notifier-field-token')), 's3cret');
    await tester.tap(find.byKey(const Key('add-notifier-submit')));
    await tester.pumpAndSettle();
    final saved = host.saved.single;
    expect(saved.name, 'phone');
    expect(saved.provider, 'ntfy');
    expect(saved.settings, {'topic': 'mine'});
    expect(saved.secrets, {'token': 's3cret'});
    expect(find.byKey(const Key('notifier phone')), findsOneWidget);
    expect(find.byKey(const Key('add-notifier-dialog')), findsNothing);
  });

  testWidgets('a notifier is removed after asking, and with it the name goes out of every list that held it', (tester) async {
    final host = _Host()
      ..entries['phone'] = {}
      ..routes['permission'] = ['desktop', 'phone']
      ..routes['failed'] = ['phone'];
    await _open(tester, host: host);
    await tester.tap(find.byKey(const Key('notifier-remove phone')));
    await tester.pumpAndSettle();
    expect(find.text('Remove "phone"?'), findsOneWidget);
    await tester.tap(find.byKey(const Key('remove-notifier-confirm')));
    await tester.pumpAndSettle();
    final removed = host.removed.single;
    expect(removed.name, 'phone');
    expect(removed.routes, {'permission': ['desktop'], 'failed': <String>[]});
    expect(find.byKey(const Key('notifier phone')), findsNothing);
  });

  testWidgets('a notifier a lower layer defines says so and has no Remove', (tester) async {
    final host = _Host()..entries['team'] = {'layer': 'the system\'s canoa.cfg'};
    await _open(tester, host: host);
    expect(find.textContaining('Set in the system\'s canoa.cfg'), findsOneWidget);
    expect(find.byKey(const Key('notifier-remove team')), findsNothing);
  });

  testWidgets('a page with no host says so, and the browser button is still there where a browser asks', (tester) async {
    await openSettings(tester, section: 'notifications', onAllowNotifications: () async => true);
    expect(find.byKey(const Key('notifications-no-host')), findsOneWidget);
    await tester.tap(find.byKey(const Key('settings-allow-notifications')));
    await tester.pumpAndSettle();
    expect(find.textContaining('shows notifications from'), findsOneWidget);
  });

  test('every word of the page is in Spanish too, and an event the app has no words for is called what the host calls it', () {
    const english = AppText();
    const spanish = AppText(language: 'es');
    for (final id in _events) {
      final key = 'notifications.event.$id';
      expect(english.maybe(key), isNotNull, reason: key);
      expect(spanish(key), isNot(english(key)), reason: '$key is translated');
    }
    for (final key in ['notifications.intro', 'notifications.off', 'notifications.add', 'notifications.test', 'notifications.remove_body']) {
      expect(spanish(key), isNot(english(key)), reason: key);
    }
    expect(english.maybe('notifications.event.something_new'), isNull);
  });
}
