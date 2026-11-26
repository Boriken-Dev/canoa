import 'dart:async';

import 'package:flutter/foundation.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../desktop/host_control.dart';

/// One event of the host's catalog and the notifiers it goes to now (`notify/events`).
final class NotificationEvent {
  const NotificationEvent({required this.id, required this.title, required this.notifiers, required this.set});

  /// The key `[notifications]` names it by.
  final String id;

  /// What the host calls it, which the app uses only for an event it has no words for.
  final String title;
  final List<String> notifiers;

  /// Whether canoa.cfg names the event, and not the catalog's default.
  final bool set;

  static NotificationEvent? parse(Object? json) {
    if (json is! Map) return null;
    final id = json['id'], title = json['title'], names = json['notifiers'];
    if (id is! String || id.isEmpty || title is! String) return null;
    return NotificationEvent(
      id: id,
      title: title,
      notifiers: [if (names is List) for (final each in names) if (each is String) each],
      set: json['set'] == true,
    );
  }
}

/// One notifier by name (`notify/notifiers`): `desktop`, or an entry of `[notifications.notifiers]`.
final class NotifierRow {
  const NotifierRow({
    required this.name,
    required this.provider,
    required this.title,
    this.help,
    this.replies = false,
    required this.state,
    this.origin,
    this.removable = false,
    this.settings = const {},
    this.secretsSet = const [],
    this.schema,
    this.form,
  });

  final String name, provider, title;
  final String? help;
  final bool replies;

  /// `ready`, or one line saying why a notice cannot be sent now.
  final String state;

  /// The layer of canoa.cfg that defines the entry, when it is not the person's own.
  final String? origin;
  final bool removable;
  final Map<String, Object?> settings;

  /// The sensitive members the host keeps, which it never sends back.
  final List<String> secretsSet;
  final Map<String, Object?>? schema, form;

  bool get ready => state == 'ready';

  static NotifierRow? parse(Object? json) {
    if (json is! Map) return null;
    final name = json['name'], provider = json['provider'], state = json['state'];
    if (name is! String || provider is! String || state is! String) return null;
    final title = json['title'];
    final settings = json['settings'], secrets = json['secrets_set'];
    return NotifierRow(
      name: name,
      provider: provider,
      title: title is String ? title : provider,
      help: json['help'] is String ? json['help']! as String : null,
      replies: json['replies'] == true,
      state: state,
      origin: json['origin'] is String ? json['origin']! as String : null,
      removable: json['removable'] == true,
      settings: settings is Map ? settings.cast<String, Object?>() : const {},
      secretsSet: [if (secrets is List) for (final each in secrets) if (each is String) each],
      schema: json['schema'] is Map ? (json['schema']! as Map).cast<String, Object?>() : null,
      form: json['form'] is Map ? (json['form']! as Map).cast<String, Object?>() : null,
    );
  }
}

/// A provider an entry may choose (`notify/notifiers` `providers`).
final class NotifierProvider {
  const NotifierProvider({required this.id, required this.title, this.help, this.schema, this.form});

  final String id, title;
  final String? help;
  final Map<String, Object?>? schema, form;

  static NotifierProvider? parse(Object? json) {
    if (json is! Map) return null;
    final id = json['id'], title = json['title'];
    if (id is! String || id.isEmpty || title is! String) return null;
    return NotifierProvider(
      id: id,
      title: title,
      help: json['help'] is String ? json['help']! as String : null,
      schema: json['schema'] is Map ? (json['schema']! as Map).cast<String, Object?>() : null,
      form: json['form'] is Map ? (json['form']! as Map).cast<String, Object?>() : null,
    );
  }
}

/// What the host says of its events and notifiers.
final class NotificationsSnapshot {
  const NotificationsSnapshot({this.events = const [], this.notifiers = const [], this.providers = const []});

  final List<NotificationEvent> events;
  final List<NotifierRow> notifiers;
  final List<NotifierProvider> providers;
}

/// What `notify/test` answered.
final class NotifierTest {
  const NotifierTest({required this.ok, this.why});

  final bool ok;
  final String? why;
}

/// Where Settings' notifications page reads and changes what the host tells the person.
abstract interface class NotificationsHost {
  Future<NotificationsSnapshot> list();

  /// Makes `[notifications] EVENT` the list [names].
  Future<void> route(String event, List<String> names);

  /// Makes the entry [name] of [provider] with [settings] and keeps [secrets] in the host's secrets.
  Future<void> save(String name, String provider, Map<String, Object?> settings, Map<String, Object?> secrets);

  /// Takes the entry [name] out, and the name out of every list of [routes] (event to the names it goes to now).
  Future<void> remove(String name, Map<String, List<String>> routes);
  Future<NotifierTest> test(String name);
}

/// The real control channel of a connected host.
final class ControlNotificationsHost implements NotificationsHost {
  ControlNotificationsHost(this._control);

  final HostControl _control;

  Future<Object?> _call(String method, Map<String, Object?> params) async => (await _control.ready(wait: false)).call(method, params);

  @override
  Future<NotificationsSnapshot> list() async {
    final events = await _call('notify/events', const {});
    final notifiers = await _call('notify/notifiers', const {});
    final rows = events is Map ? events['events'] : null;
    final named = notifiers is Map ? notifiers['notifiers'] : null;
    final offered = notifiers is Map ? notifiers['providers'] : null;
    return NotificationsSnapshot(
      events: [if (rows is List) for (final each in rows) ?NotificationEvent.parse(each)],
      notifiers: [if (named is List) for (final each in named) ?NotifierRow.parse(each)],
      providers: [if (offered is List) for (final each in offered) ?NotifierProvider.parse(each)],
    );
  }

  @override
  Future<void> route(String event, List<String> names) async {
    await _call('config/change', {
      'ops': [
        {'path': ['notifications', event], 'value': names},
      ],
    });
  }

  @override
  Future<void> save(String name, String provider, Map<String, Object?> settings, Map<String, Object?> secrets) async {
    await _call('config/change', {
      'ops': [
        {
          'path': ['notifications', 'notifiers', name],
          'value': {'provider': provider, 'settings': settings},
        },
      ],
    });
    if (secrets.isNotEmpty) await _call('notify/keep', {'name': name, 'values': secrets});
  }

  @override
  Future<void> remove(String name, Map<String, List<String>> routes) async {
    // The lists that name the notifier change in the same write, so the file is never one the host would refuse.
    await _call('config/change', {
      'ops': [
        for (final MapEntry(:key, :value) in routes.entries) {'path': ['notifications', key], 'value': value},
        {'path': ['notifications', 'notifiers', name], 'value': null},
      ],
    });
  }

  @override
  Future<NotifierTest> test(String name) async {
    final answer = await _call('notify/test', {'name': name});
    if (answer is! Map) return const NotifierTest(ok: false, why: 'The host answered something unexpected');
    return NotifierTest(ok: answer['ok'] == true, why: answer['why'] is String ? answer['why']! as String : null);
  }
}

/// What Settings' notifications page shows: the events and the notifiers as the host lists them, and the result of the last test of each.
class NotificationsController extends ChangeNotifier {
  NotificationsController({required this.host});

  final NotificationsHost host;

  NotificationsSnapshot _snapshot = const NotificationsSnapshot();
  final _tests = <String, NotifierTest>{};
  final _testing = <String>{};
  String? _problem;
  bool _loading = false, _disposed = false;

  List<NotificationEvent> get events => _snapshot.events;
  List<NotifierRow> get notifiers => _snapshot.notifiers;
  List<NotifierProvider> get providers => _snapshot.providers;
  String? get problem => _problem;
  bool get loading => _loading;

  NotifierRow? named(String name) => notifiers.where((each) => each.name == name).firstOrNull;
  NotifierTest? testOf(String name) => _tests[name];
  bool isTesting(String name) => _testing.contains(name);

  Future<void> refresh() async {
    _loading = true;
    _notify();
    try {
      _snapshot = await host.list();
      _problem = null;
    } on ControlFailed catch (error) {
      _problem = error.message;
    } on Object catch (error) {
      _problem = '$error';
    } finally {
      _loading = false;
      _notify();
    }
  }

  /// Sends [event] to [names]; the host's refusal is thrown as it words it, and the list is read again either way.
  Future<void> route(String event, List<String> names) async {
    try {
      await host.route(event, names);
    } finally {
      await refresh();
    }
  }

  Future<void> save(String name, String provider, Map<String, Object?> settings, Map<String, Object?> secrets) async {
    try {
      await host.save(name, provider, settings, secrets);
    } finally {
      await refresh();
    }
  }

  Future<void> remove(String name) async {
    final routes = {
      for (final event in events)
        if (event.notifiers.contains(name)) event.id: [for (final each in event.notifiers) if (each != name) each],
    };
    try {
      await host.remove(name, routes);
    } finally {
      _tests.remove(name);
      await refresh();
    }
  }

  Future<NotifierTest> test(String name) async {
    _testing.add(name);
    _tests.remove(name);
    _notify();
    NotifierTest result;
    try {
      result = await host.test(name);
    } on ControlFailed catch (error) {
      result = NotifierTest(ok: false, why: error.message);
    } on Object catch (error) {
      result = NotifierTest(ok: false, why: '$error');
    }
    _testing.remove(name);
    _tests[name] = result;
    _notify();
    return result;
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
