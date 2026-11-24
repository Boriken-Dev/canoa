import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../l10n/app_text.dart';
import 'off_switch.dart';
import 'settings_row.dart';

/// What the app asks the host with: `providers/registered`, `providers/switch` and `accounts/list`.
typedef ProviderCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// One provider a library registers: a kind (`account_provider`, `acp_harness`, `store_extension`, ...) and an id, with whether the
/// host's `[host.disabled]` leaves it off.
final class RegisteredProvider {
  const RegisteredProvider({required this.kind, required this.id, required this.title, required this.library, required this.origin, required this.off, this.accountKinds = const []});

  factory RegisteredProvider.read(Map<Object?, Object?> raw) => RegisteredProvider(
    kind: '${raw['kind']}',
    id: '${raw['id']}',
    title: (raw['title'] ?? raw['id']).toString(),
    library: raw['library'] is String ? raw['library'] as String : '',
    origin: raw['origin'] is String ? raw['origin'] as String : '',
    off: raw['off'] == true,
    accountKinds: [for (final kind in raw['kinds'] as List<Object?>? ?? const []) '$kind'],
  );

  final String kind, id, title, library, origin;
  final bool off;

  /// For an account provider, the kinds of account it makes.
  final List<String> accountKinds;
}

/// The providers the host's libraries register, each with a switch: turned off, a provider is neither offered nor run, and the accounts
/// of an account provider stay in canoa.cfg, shown as off. The host reads `[host.disabled]` when it starts, so [restartPending] is
/// true once a switch was turned.
class ProviderSwitches extends ChangeNotifier {
  ProviderSwitches(this._call, {this.restart});

  final ProviderCall _call;

  /// Has the host start again, which applies the change; null where this page cannot.
  final Future<bool> Function()? restart;

  bool _disposed = false;
  List<RegisteredProvider> _providers = const [];
  List<String> _unknown = const [];
  Map<String, int> _accountsOff = const {};
  String? _problem;
  bool _loading = false, _loaded = false, _restartPending = false;
  final _turning = <String>{};

  List<RegisteredProvider> get providers => _providers;

  /// What `[host.disabled]` names that no loaded library offers, in the host's words.
  List<String> get unknown => _unknown;
  String? get problem => _problem;
  bool get loading => _loading;
  bool get loaded => _loaded;
  bool get restartPending => _restartPending;

  /// How many accounts canoa.cfg holds that the provider [id] makes while it is off: they are not offered, and are shown as off.
  int accountsOff(String id) => _accountsOff[id] ?? 0;

  /// Whether the provider is being turned now.
  bool turning(String kind, String id) => _turning.contains('$kind/$id');

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// The kinds in the order Settings lists them, each with its providers by title.
  List<(String, List<RegisteredProvider>)> get byKind {
    final kinds = <String, List<RegisteredProvider>>{};
    for (final provider in _providers) {
      kinds.putIfAbsent(provider.kind, () => []).add(provider);
    }
    final names = kinds.keys.toList()..sort();
    return [
      for (final name in names) (name, kinds[name]!..sort((a, b) => a.title.toLowerCase().compareTo(b.title.toLowerCase()))),
    ];
  }

  /// Reads the providers and, to say what a switch would put out of reach, the accounts that are off.
  Future<void> load() async {
    _loading = true;
    notifyListeners();
    try {
      final answer = await _call('providers/registered', const {});
      final map = answer is Map<Object?, Object?> ? answer : const <Object?, Object?>{};
      _providers = [
        for (final each in map['providers'] as List<Object?>? ?? const [])
          if (each is Map<Object?, Object?>) RegisteredProvider.read(each),
      ];
      _unknown = [for (final line in map['unknown'] as List<Object?>? ?? const []) '$line'];
      _problem = null;
      _loaded = true;
      try {
        final accounts = await _call('accounts/list', const {});
        final counts = <String, int>{};
        for (final row in (accounts is Map<Object?, Object?> ? accounts['accounts'] as List<Object?>? : null) ?? const []) {
          if (row is Map<Object?, Object?> && row['disabled'] is String) counts.update(row['disabled'] as String, (n) => n + 1, ifAbsent: () => 1);
        }
        _accountsOff = counts;
      } on Object {
        // The count is a note on a row; the list stands without it.
      }
    } on ControlFailed catch (error) {
      _problem = error.code == -32001 ? "This key may not change the host's own settings." : error.message;
    } on Object catch (error) {
      _problem = 'The host could not be reached: $error';
    } finally {
      _loading = false;
      notifyListeners();
    }
  }

  /// Turns the provider [id] of [kind] off ([off] true) or on. Returns what is wrong in the host's words, or null once canoa.cfg
  /// holds it.
  Future<String?> turn(String kind, String id, {required bool off}) async {
    final key = '$kind/$id';
    if (!_turning.add(key)) return null;
    notifyListeners();
    try {
      await _call('providers/switch', {'kind': kind, 'id': id, 'off': off});
      _restartPending = true;
      _providers = [
        for (final each in _providers)
          if (each.kind == kind && each.id == id)
            RegisteredProvider(kind: each.kind, id: each.id, title: each.title, library: each.library, origin: each.origin, off: off, accountKinds: each.accountKinds)
          else
            each,
      ];
      return null;
    } on ControlFailed catch (error) {
      return error.message;
    } on Object catch (error) {
      return '$error';
    } finally {
      _turning.remove(key);
      notifyListeners();
    }
  }

  /// Has the host start again, so what was switched takes hold.
  Future<void> restartHost() async {
    if (restart == null || !await restart!()) return;
    _restartPending = false;
    notifyListeners();
    unawaited(load());
  }
}

/// Settings, Providers: every provider the libraries register, by kind, each with a switch.
class ProvidersSection extends StatefulWidget {
  const ProvidersSection({super.key, required this.switches});

  final ProviderSwitches switches;

  @override
  State<ProvidersSection> createState() => _ProvidersSectionState();
}

class _ProvidersSectionState extends State<ProvidersSection> {
  @override
  void initState() {
    super.initState();
    if (!widget.switches.loaded && !widget.switches.loading) unawaited(widget.switches.load());
  }

  Future<void> _turn(BuildContext context, RegisteredProvider provider, bool on) async {
    final problem = await widget.switches.turn(provider.kind, provider.id, off: !on);
    if (problem != null && context.mounted) ScaffoldMessenger.maybeOf(context)?.showSnackBar(SnackBar(content: Text(problem)));
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.switches,
    builder: (context, _) {
      final s = widget.switches;
      final scheme = Theme.of(context).colorScheme;
      final text = Theme.of(context).textTheme;
      return Column(key: const Key('providers-section'), crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(context.t('providers.intro'), style: text.bodyMedium?.copyWith(color: scheme.onSurfaceVariant)),
        const SizedBox(height: 12),
        if (s.problem case final problem?)
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: Text(problem, key: const Key('providers-problem'), style: TextStyle(color: scheme.error)),
          ),
        if (s.restartPending)
          Card(
            key: const Key('providers-restart-note'),
            child: Padding(
              padding: const EdgeInsets.all(12),
              child: Row(children: [
                Expanded(child: Text(context.t('providers.restart_note'))),
                if (s.restart != null)
                  FilledButton(key: const Key('providers-restart'), onPressed: () => unawaited(s.restartHost()), child: Text(context.t('providers.restart'))),
              ]),
            ),
          ),
        if (s.loading && !s.loaded) const Padding(padding: EdgeInsets.all(12), child: LinearProgressIndicator()),
        if (s.loaded && s.providers.isEmpty) Text(context.t('providers.none'), key: const Key('providers-none')),
        for (final (kind, providers) in s.byKind)
          SettingsGroup(
            key: Key('providers-kind-$kind'),
            title: kind,
            children: [
              for (final provider in providers)
                SettingsRow(
                  key: Key('provider-row-$kind-${provider.id}'),
                  title: provider.title == provider.id ? provider.id : '${provider.title} (${provider.id})',
                  description: _describe(context, s, provider),
                  control: Row(mainAxisSize: MainAxisSize.min, children: [
                    if (provider.off) ...[OffBadge(reason: context.t('providers.off_reason')), const SizedBox(width: 8)],
                    OffSwitch(
                      key: Key('provider-switch-$kind-${provider.id}'),
                      on: !provider.off,
                      label: context.t('switch.turn', {'name': provider.title}),
                      onChanged: s.turning(kind, provider.id) ? null : (on) => unawaited(_turn(context, provider, on)),
                    ),
                  ]),
                ),
            ],
          ),
        for (final line in s.unknown)
          Padding(padding: const EdgeInsets.only(top: 4), child: Text(line, key: Key('providers-unknown-${line.hashCode}'), style: TextStyle(color: scheme.error))),
      ]);
    },
  );

  String _describe(BuildContext context, ProviderSwitches switches, RegisteredProvider provider) {
    final off = switches.accountsOff(provider.id);
    return [
      if (provider.library.isNotEmpty) context.t('providers.library', {'library': provider.library, 'origin': provider.origin}),
      if (provider.accountKinds.isNotEmpty) context.t('providers.makes', {'kinds': provider.accountKinds.join(', ')}),
      if (provider.off && off > 0) context.t('providers.accounts_off', {'count': off}),
    ].join('\n');
  }
}
