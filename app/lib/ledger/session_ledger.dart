import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../worktree/worktree.dart';

/// Where a session's agent runs, as the app reached it.
enum SessionPlace {
  local('on this computer'),
  host('through this app\'s canoa-host'),
  peer('on a paired peer'),
  browser('through the browser\'s provider');

  const SessionPlace(this.label);
  final String label;

  static SessionPlace? named(Object? name) => values.where((place) => place.name == name).firstOrNull;
}

/// The harness each `agentInfo` name stands for. Three adapters report their own package name rather than the harness's, and
/// Grok reports none: [grokName] stands for an agent whose `_meta.grokShell` is set.
const Map<String, String> builtInHarnesses = {
  '@agentclientprotocol/claude-agent-acp': 'Claude Code',
  '@agentclientprotocol/codex-acp': 'Codex',
  'gemini-cli': 'Gemini',
  'pi-acp': 'Pi',
  'OpenCode': 'OpenCode',
  grokName: 'Grok',
};

/// The name an agent with `_meta.grokShell` and no `agentInfo` is looked up by.
const String grokName = 'grok';

/// Harness labels: [builtInHarnesses], extended and overridden by Canoa.cfg's `[harnesses]`.
final class HarnessNames {
  HarnessNames([Map<String, String> extra = const {}]) : _names = {...builtInHarnesses, ...extra};
  final Map<String, String> _names;

  /// The harness for what an agent reported, else the agent entry or preset it was started from.
  String label({String? reportedName, bool grokShell = false, required String entry}) =>
      _names[reportedName ?? (grokShell ? grokName : '')] ?? entry;
}

/// A message's role and text, kept only when the person turned that on.
final class KeptMessage {
  const KeptMessage(this.role, this.text, this.at);
  final String role;
  final String text;
  final DateTime at;

  Map<String, Object?> toJson() => {'role': role, 'text': text, 'at': at.toUtc().toIso8601String()};

  static KeptMessage? fromJson(Object? json) {
    if (json is! Map || json['role'] is! String || json['text'] is! String) return null;
    return KeptMessage(json['role'] as String, json['text'] as String,
        DateTime.tryParse('${json['at']}') ?? DateTime.fromMillisecondsSinceEpoch(0, isUtc: true));
  }
}

/// One session the app opened or resumed. The agent keeps the session itself; this is Canoa's note of it.
final class LedgerEntry {
  LedgerEntry({
    required this.peerId,
    required this.sessionId,
    required this.entry,
    required this.place,
    required this.created,
    required this.lastUsed,
    this.harness = '',
    this.reportedName,
    this.grokShell = false,
    this.version,
    this.cwd,
    this.title,
    this.endpoint,
    this.fallback,
    this.method,
    this.reopenBy,
    this.worktree,
    this.messages = const [],
  });

  /// The proven peer, or `local:<entry>` for an agent this app runs.
  final String peerId;

  /// The agent's own id for the session.
  final String sessionId;

  /// The agent entry or preset it was started from, or what the person named a peer.
  final String entry;
  final SessionPlace place;
  final DateTime created;
  DateTime lastUsed;

  /// The label [HarnessNames] gives it.
  String harness;
  String? reportedName;
  bool grokShell;

  /// The version the agent reported: an adapter's, for an adapter.
  String? version;
  String? cwd;
  String? title;

  /// Where a peer was reached, and the proof its key makes, to reach it again.
  final String? endpoint, fallback, method;

  /// `session/load` or `session/resume`, whichever the agent offered when last opened, or null for neither.
  String? reopenBy;
  List<KeptMessage> messages;

  /// The git worktree the session works in, on its own branch, when it was started in one.
  SessionWorktree? worktree;

  String get key => keyOf(peerId, sessionId);
  static String keyOf(String peerId, String sessionId) => '$peerId\n$sessionId';

  bool get canReopen => reopenBy != null && place != SessionPlace.browser;

  /// The version, named by what reported it when that is an adapter rather than the harness, such as `claude-agent-acp 0.99.1`.
  String? get versionLabel {
    final reporter = reportedName?.replaceFirst(RegExp(r'^@[^/]+/'), '');
    if (version == null) return null;
    return reporter == null || reporter == harness ? version : '$reporter $version';
  }

  /// Why this cannot be opened again from the list, in a person's words, or null.
  String? get whyNotReopened => switch (this) {
    LedgerEntry(reopenBy: null) =>
      'Cannot be reopened: $harness offers neither session/load nor session/resume.',
    LedgerEntry(place: SessionPlace.browser) =>
      'Reopen it from Earlier conversations once the browser grants this agent again.',
    _ => null,
  };

  Map<String, Object?> toJson() => {
    'harness': harness,
    'reportedName': ?reportedName,
    if (grokShell) 'grokShell': true,
    'version': ?version,
    'entry': entry,
    'where': place.name,
    'peerId': peerId,
    'endpoint': ?endpoint,
    'fallback': ?fallback,
    'method': ?method,
    'cwd': ?cwd,
    'sessionId': sessionId,
    'title': ?title,
    'created': created.toUtc().toIso8601String(),
    'lastUsed': lastUsed.toUtc().toIso8601String(),
    'reopenBy': ?reopenBy,
    'worktree': ?worktree?.toJson(),
    if (messages.isNotEmpty) 'messages': [for (final message in messages) message.toJson()],
  };

  static LedgerEntry? fromJson(Object? json) {
    if (json is! Map) return null;
    String? text(String key) => json[key] is String && (json[key] as String).isNotEmpty ? json[key] as String : null;
    final peerId = text('peerId'), sessionId = text('sessionId'), entry = text('entry');
    final place = SessionPlace.named(json['where']);
    final created = DateTime.tryParse('${json['created']}');
    final lastUsed = DateTime.tryParse('${json['lastUsed']}');
    if (peerId == null || sessionId == null || entry == null || place == null || created == null || lastUsed == null) return null;
    final reopenBy = text('reopenBy');
    return LedgerEntry(
      peerId: peerId,
      sessionId: sessionId,
      entry: entry,
      place: place,
      created: created,
      lastUsed: lastUsed,
      harness: text('harness') ?? entry,
      reportedName: text('reportedName'),
      grokShell: json['grokShell'] == true,
      version: text('version'),
      cwd: text('cwd'),
      title: text('title'),
      endpoint: text('endpoint'),
      fallback: text('fallback'),
      method: text('method'),
      reopenBy: reopenBy == 'session/load' || reopenBy == 'session/resume' ? reopenBy : null,
      worktree: SessionWorktree.fromJson(json['worktree']),
      messages: [
        if (json['messages'] case final List<dynamic> kept)
          for (final message in kept) ?KeptMessage.fromJson(message),
      ],
    );
  }
}

/// How much the ledger keeps, from Canoa.cfg's `[sessions]`.
final class LedgerSettings {
  const LedgerSettings({
    this.keep = defaultKeep,
    this.keepText = false,
    this.textMessages = defaultTextMessages,
    this.harnesses = const {},
  });

  static const defaultKeep = 200;
  static const defaultTextMessages = 200;

  /// How many sessions are kept, the least recently used going first; 0 keeps none.
  final int keep;

  /// Whether what was said is kept with each session. Off by default.
  final bool keepText;

  /// How many of a session's latest messages are kept when [keepText] is on.
  final int textMessages;

  /// Canoa.cfg's `[harnesses]`: reported name to harness label.
  final Map<String, String> harnesses;
}

/// Where the ledger is kept: one JSON document, read whole and written whole.
abstract interface class LedgerBackend {
  /// The document, or null when none was written yet.
  Future<String?> read();
  Future<void> write(String document);

  /// Where it is, for a person to find.
  String get location;
}

/// Kept only while the app runs.
final class MemoryLedgerBackend implements LedgerBackend {
  String? document;

  @override
  Future<String?> read() async => document;

  @override
  Future<void> write(String document) async => this.document = document;

  @override
  String get location => 'memory only';
}

/// Every session the app opened or resumed, by harness, bounded by [LedgerSettings.keep]. It never deletes an agent's own
/// session, and it holds what was said only when [LedgerSettings.keepText] is on.
class SessionLedger extends ChangeNotifier {
  SessionLedger(this.backend, {LedgerSettings settings = const LedgerSettings(), DateTime Function()? clock})
    : _settings = settings,
      _names = HarnessNames(settings.harnesses),
      _clock = clock ?? DateTime.now;

  /// The most of one message kept, since an agent can send any amount.
  static const maxKeptMessage = 64 * 1024;

  /// How long changes gather before they are written.
  static const saveDelay = Duration(milliseconds: 300);

  final LedgerBackend backend;
  final DateTime Function() _clock;
  LedgerSettings _settings;
  HarnessNames _names;
  final List<LedgerEntry> _entries = [];
  Timer? _saveTimer;
  Future<void> _writing = Future.value();

  /// Why the stored ledger could not be read, if it could not.
  String? problem;

  LedgerSettings get settings => _settings;
  HarnessNames get names => _names;

  /// Most recently used first.
  List<LedgerEntry> get entries => List.unmodifiable(_entries);

  /// The harnesses present, most recently used first.
  List<String> get harnesses => {for (final entry in _entries) entry.harness}.toList();

  LedgerEntry? find(String peerId, String sessionId) {
    final key = LedgerEntry.keyOf(peerId, sessionId);
    return _entries.where((entry) => entry.key == key).firstOrNull;
  }

  Future<void> load() async {
    final String? document;
    try {
      document = await backend.read();
    } on Object catch (error) {
      problem = 'could not read ${backend.location}: $error';
      notifyListeners();
      return;
    }
    if (document == null) return;
    try {
      final decoded = jsonDecode(document);
      if (decoded is! Map || decoded['version'] != 1 || decoded['sessions'] is! List) {
        throw const FormatException('not a version 1 session list');
      }
      _entries
        ..clear()
        ..addAll([for (final json in decoded['sessions'] as List) ?LedgerEntry.fromJson(json)]);
    } on FormatException catch (error) {
      problem = '${backend.location} is not a session list (${error.message}), so the list starts empty';
    }
    _apply();
    notifyListeners();
  }

  /// Takes new settings: labels are given again from the harness map, and the bound and text choice apply at once.
  void configure(LedgerSettings settings) {
    _settings = settings;
    _names = HarnessNames(settings.harnesses);
    _apply();
    _changed();
  }

  void _apply() {
    for (final entry in _entries) {
      entry.harness = _names.label(reportedName: entry.reportedName, grokShell: entry.grokShell, entry: entry.entry);
      if (!_settings.keepText) {
        entry.messages = const [];
      } else if (entry.messages.length > _settings.textMessages) {
        entry.messages = entry.messages.sublist(entry.messages.length - _settings.textMessages);
      }
    }
    _entries.sort((a, b) => b.lastUsed.compareTo(a.lastUsed));
    if (_entries.length > _settings.keep) _entries.removeRange(_settings.keep < 0 ? 0 : _settings.keep, _entries.length);
  }

  /// Notes a session the app opened or resumed, or updates the note of one it opened before.
  LedgerEntry? record({
    required String peerId,
    required String sessionId,
    required String entry,
    required SessionPlace place,
    String? reportedName,
    bool grokShell = false,
    String? version,
    String? reopenBy,
    String? cwd,
    String? title,
    String? endpoint,
    String? fallback,
    String? method,
    SessionWorktree? worktree,
  }) {
    if (_settings.keep <= 0) return null;
    final now = _clock();
    var found = find(peerId, sessionId);
    if (found == null) {
      found = LedgerEntry(
        peerId: peerId,
        sessionId: sessionId,
        entry: entry,
        place: place,
        created: now,
        lastUsed: now,
        cwd: cwd,
        title: title,
        endpoint: endpoint,
        fallback: fallback,
        method: method,
      );
      _entries.add(found);
    }
    found
      ..lastUsed = now
      ..reportedName = reportedName
      ..grokShell = grokShell
      ..version = version
      ..reopenBy = reopenBy
      ..cwd = cwd ?? found.cwd
      ..title = title ?? found.title
      ..worktree = worktree ?? found.worktree;
    _apply();
    _changed();
    return find(peerId, sessionId);
  }

  /// A session was used again, and maybe renamed.
  void touch(String peerId, String sessionId, {String? title}) {
    final found = find(peerId, sessionId);
    if (found == null) return;
    found.lastUsed = _clock();
    if (title != null) found.title = title;
    _apply();
    _changed();
  }

  /// Keeps the latest of [messages] with the session, only when the person turned that on.
  void keepText(String peerId, String sessionId, List<KeptMessage> messages) {
    final found = find(peerId, sessionId);
    if (found == null || !_settings.keepText) return;
    final latest = messages.length > _settings.textMessages
        ? messages.sublist(messages.length - _settings.textMessages)
        : messages;
    found.messages = [
      for (final message in latest)
        message.text.length > maxKeptMessage
            ? KeptMessage(message.role, message.text.substring(0, maxKeptMessage), message.at)
            : message,
    ];
    _changed();
  }

  /// Removes Canoa's note of a session. The agent's own session stays where it is.
  void remove(LedgerEntry entry) {
    _entries.removeWhere((each) => each.key == entry.key);
    _changed();
  }

  /// Removes every kept message, leaving the sessions listed.
  void clearText() {
    for (final entry in _entries) {
      entry.messages = const [];
    }
    _changed();
  }

  bool get holdsText => _entries.any((entry) => entry.messages.isNotEmpty);

  void _changed() {
    notifyListeners();
    _saveTimer?.cancel();
    _saveTimer = Timer(saveDelay, () => unawaited(flush()));
  }

  String encode() => const JsonEncoder.withIndent('  ').convert({
    'version': 1,
    'sessions': [for (final entry in _entries) entry.toJson()],
  });

  /// Writes what is pending now, after any write already under way.
  Future<void> flush() {
    _saveTimer?.cancel();
    _saveTimer = null;
    final document = encode();
    return _writing = _writing.then((_) => backend.write(document)).catchError((Object error) {
      debugPrint('Canoa sessions: could not write ${backend.location}: $error');
    });
  }

  @override
  void dispose() {
    if (_saveTimer != null) unawaited(flush());
    super.dispose();
  }
}
