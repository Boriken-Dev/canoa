// The Canoa on this computer, as the extension's panel sees it: whether the browser can reach it, whether it runs, whether this
// browser is paired with it, and what it serves. The service worker owns the pairing and the proof
// (bindings/browser/chromium/acp-local.js); this is the panel's view of it and the person's requests to change it.
import 'dart:convert';

import 'local_host_stub.dart' if (dart.library.js_interop) 'local_host_web.dart' as platform;

/// What the worker found when it asked the native messaging host.
enum LocalHostState {
  /// The native host is not registered, or the extension may not start it.
  notInstalled,

  /// Registered, but no Canoa holds its home.
  stopped,

  /// A Canoa answers.
  running,

  /// Something else: no answer in time, Canoa running with its door shut, or an answer that was no host's.
  unavailable,
}

/// The state of the local Canoa at one moment.
final class LocalHostView {
  const LocalHostView({
    required this.state,
    this.why,
    this.forbidden = false,
    this.name = 'Canoa',
    this.paired = false,
    this.changed = false,
    this.canPin = true,
    this.entries = const [],
    this.notice,
  });

  final LocalHostState state;

  /// Why it is unavailable, when it is.
  final String? why;

  /// Registered, but this extension is not one the native host allows.
  final bool forbidden;

  /// What the host calls itself: a claim, shown and nothing more.
  final String name;

  /// Whether this browser's key is paired with the Canoa that answers, which the proof then checks.
  final bool paired;

  /// Whether the Canoa that answers is not the one this browser paired with.
  final bool changed;

  /// Whether the host's method is one a PIN pairs (Noise); otherwise the key waits for `canoa accept`.
  final bool canPin;

  /// The labels of the entries it serves, as it says them.
  final List<String> entries;

  /// Something the person should read after the last request, such as the host still listing a key that was unpaired.
  final String? notice;

  factory LocalHostView.fromJson(String text) {
    final map = jsonDecode(text) as Map<String, Object?>;
    final host = map['host'] as Map<String, Object?>?;
    return LocalHostView(
      state: switch (map['state']) {
        'running' => LocalHostState.running,
        'stopped' => LocalHostState.stopped,
        'not-installed' => LocalHostState.notInstalled,
        _ => LocalHostState.unavailable,
      },
      why: map['why'] is String ? map['why'] as String : null,
      forbidden: map['forbidden'] == true,
      name: host?['name'] is String ? host!['name'] as String : 'Canoa',
      paired: map['paired'] == true,
      changed: map['changed'] == true,
      canPin: map['canPin'] != false,
      notice: map['notice'] is String ? map['notice'] as String : null,
      entries: [for (final entry in (host?['entries'] as List? ?? const [])) '${(entry as Map)['label'] ?? entry['name']}'],
    );
  }
}

/// The calls the panel makes of the worker about the local Canoa. A refusal is an exception whose message says why.
abstract interface class LocalHostBridge {
  Future<LocalHostView> status();

  /// Starts it through the native host, and answers once it answers or the wait is over. Only a click calls this.
  Future<LocalHostView> start();

  /// Pairs this browser with it by the PIN the person read on their own computer.
  Future<LocalHostView> pair(String pin);

  /// Forgets the pairing.
  Future<LocalHostView> unpair();

  /// The result of a control method of the host (`ledger/list`), run by the worker over a link it proves; a refusal is an
  /// exception. Only what the host grants a browser key is answered.
  Future<Object?> control(String method, Map<String, Object?> params);
}

/// The extension's own bridge, or null where this app is not running as the extension.
LocalHostBridge? platformLocalHostBridge() => platform.platformLocalHostBridge();
