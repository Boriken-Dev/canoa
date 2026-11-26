/// Settings › Network › Proxy: the routes of the host (how its traffic leaves this computer), which one is the default, where each is
/// used, and making, trying and removing them. A route is a provider of the host's `network_route` kind and its settings; the form of
/// a new one is drawn from the provider's own schema, and a setting that names a host variable asks for the name of the variable.
library;

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show hostWords;
import '../forms/schema_fields.dart';
import '../l10n/app_text.dart';
import '../shell/confirm_remove.dart';
import 'settings_row.dart';

/// What the app asks the host with: the `routes/*` methods.
typedef RoutesCall = Future<Object?> Function(String method, Map<String, Object?> params);

Map<String, Object?> _map(Object? value) => value is Map ? value.cast<String, Object?>() : const {};
List<String> _names(Object? value) => value is List ? [for (final each in value) '$each'] : const [];

/// One route as `routes/list` says it.
final class RouteRow {
  const RouteRow({
    required this.name,
    required this.provider,
    required this.title,
    required this.isDefault,
    required this.builtin,
    required this.settings,
    required this.accounts,
    required this.profiles,
    required this.leases,
    required this.state,
    required this.problems,
  });

  static RouteRow? read(Object? raw) {
    final map = _map(raw);
    final name = map['name'];
    if (name is! String) return null;
    final uses = _map(map['uses']);
    return RouteRow(
      name: name,
      provider: '${map['provider'] ?? name}',
      title: map['title'] is String ? map['title'] as String : '${map['provider'] ?? name}',
      isDefault: map['default'] == true,
      builtin: map['builtin'] == true,
      settings: _map(map['settings']),
      accounts: _names(uses['accounts']),
      profiles: _names(uses['profiles']),
      leases: map['leases'] is int ? map['leases'] as int : 0,
      state: map['state'] is String ? map['state'] as String : null,
      problems: _names(map['problems']),
    );
  }

  final String name;
  final String provider;
  final String title;
  final bool isDefault;
  final bool builtin;
  final Map<String, Object?> settings;
  final List<String> accounts;
  final List<String> profiles;
  final int leases;
  final String? state;
  final List<String> problems;
}

/// One provider a route can be made of, as `routes/offers` says it.
final class RouteOffer {
  const RouteOffer({required this.provider, required this.title, required this.description, required this.secrets, required this.schema, required this.form});

  static RouteOffer? read(Object? raw) {
    final map = _map(raw);
    final provider = map['provider'];
    if (provider is! String) return null;
    return RouteOffer(
      provider: provider,
      title: map['title'] is String ? map['title'] as String : provider,
      description: map['description'] is String ? map['description'] as String : null,
      secrets: _names(map['secrets']),
      schema: _map(map['schema']),
      form: _map(map['form']),
    );
  }

  final String provider;
  final String title;
  final String? description;

  /// The settings whose value is the name of a host variable.
  final List<String> secrets;
  final Map<String, Object?> schema;
  final Map<String, Object?> form;
}

/// The host's routes, read and changed over the control channel.
class NetworkRoutes extends ChangeNotifier {
  NetworkRoutes(this._call);

  final RoutesCall _call;
  List<RouteRow> routes = const [];
  List<RouteOffer> offers = const [];
  bool loaded = false;

  /// Why the last read failed, in the host's words.
  String? problem;

  Future<void> refresh() async {
    try {
      final list = _map(await _call('routes/list', const {}));
      final offered = _map(await _call('routes/offers', const {}));
      routes = [for (final each in list['routes'] as List<Object?>? ?? const []) ?RouteRow.read(each)];
      offers = [for (final each in offered['offers'] as List<Object?>? ?? const []) ?RouteOffer.read(each)];
      problem = null;
    } on Object catch (error) {
      problem = hostWords(error);
    }
    loaded = true;
    notifyListeners();
  }

  /// Makes or changes a route; the host refuses one its provider finds wrong, saying why.
  Future<void> save(String name, String provider, Map<String, Object?> settings) async {
    await _call('routes/set', {'name': name, 'provider': provider, 'settings': settings});
    await refresh();
  }

  Future<void> remove(String name) async {
    await _call('routes/remove', {'name': name});
    await refresh();
  }

  Future<void> makeDefault(String name) async {
    await _call('routes/default', {'name': name});
    await refresh();
  }

  /// How the route reaches [url]: the host's answer.
  Future<Map<String, Object?>> test(String name, String? url) async =>
      _map(await _call('routes/test', {'name': name, if (url != null && url.isNotEmpty) 'url': url}));
}

/// The Network section: the Proxy page.
class NetworkSection extends StatefulWidget {
  const NetworkSection({super.key, required this.routes});

  final NetworkRoutes routes;

  @override
  State<NetworkSection> createState() => _NetworkSectionState();
}

class _NetworkSectionState extends State<NetworkSection> {
  NetworkRoutes get _routes => widget.routes;
  final Map<String, String> _tried = {};

  @override
  void initState() {
    super.initState();
    _routes.addListener(_changed);
    unawaited(_routes.refresh());
  }

  @override
  void dispose() {
    _routes.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  void _say(String words) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(words, key: const Key('route-problem'))));
  }

  Future<void> _edit([RouteRow? route]) async {
    await showDialog<void>(context: context, builder: (_) => RouteDialog(routes: _routes, route: route));
  }

  Future<void> _remove(RouteRow route) async {
    final text = AppText.of(context);
    final uses = [
      if (route.accounts.isNotEmpty) text('network.used_by_accounts', {'names': route.accounts.join(', ')}),
      if (route.profiles.isNotEmpty) text('network.used_by_profiles', {'names': route.profiles.join(', ')}),
    ];
    final confirmed = await confirmRemoval(
      context,
      title: text('network.remove_title', {'name': route.name}),
      body: [text('network.remove_body'), ...uses].join(' '),
      confirmLabel: text('common.remove'),
      cancelLabel: text('common.keep'),
      dialogKey: const Key('remove-route'),
      cancelKey: null,
      confirmKey: const Key('remove-route-confirm'),
    );
    if (!confirmed) return;
    try {
      await _routes.remove(route.name);
    } on Object catch (error) {
      _say(hostWords(error));
    }
  }

  Future<void> _default(RouteRow route) async {
    try {
      await _routes.makeDefault(route.name);
    } on Object catch (error) {
      _say(hostWords(error));
    }
  }

  Future<void> _try(RouteRow route) async {
    final text = AppText.of(context);
    setState(() => _tried[route.name] = text('network.testing'));
    String words;
    try {
      final answer = await _routes.test(route.name, null);
      if (answer['ok'] == true) {
        final via = answer['via'] is List && (answer['via'] as List).isNotEmpty ? (answer['via'] as List).join(', ') : text('network.direct_word');
        words = text('network.test_ok', {'url': '${answer['url']}', 'via': via, 'ms': '${answer['millis']}'});
      } else {
        words = text('network.test_failed', {'why': '${answer['why']}'});
      }
    } on Object catch (error) {
      words = text('network.test_failed', {'why': hostWords(error)});
    }
    if (mounted) setState(() => _tried[route.name] = words);
  }

  @override
  Widget build(BuildContext context) {
    final text = AppText.of(context);
    final scheme = Theme.of(context).colorScheme;
    return Column(
      key: const Key('settings-network'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsGroup(
          key: const Key('network-routes'),
          title: text('network.title'),
          leading: const Icon(Icons.alt_route, size: 20),
          trailing: TextButton.icon(
            key: const Key('add-route'),
            onPressed: _routes.offers.isEmpty ? null : () => unawaited(_edit()),
            icon: const Icon(Icons.add, size: 18),
            label: Text(text('network.add')),
          ),
          children: [
            SettingsRow(title: text('network.intro_title'), description: text('network.intro')),
            if (_routes.problem != null) Padding(padding: const EdgeInsets.all(8), child: Text(_routes.problem!, key: const Key('routes-problem'), style: TextStyle(color: scheme.error))),
            if (!_routes.loaded) const Padding(padding: EdgeInsets.all(16), child: LinearProgressIndicator()),
            for (final route in _routes.routes) _row(context, route),
          ],
        ),
      ],
    );
  }

  Widget _row(BuildContext context, RouteRow route) {
    final text = AppText.of(context);
    final scheme = Theme.of(context).colorScheme;
    final used = [
      if (route.isDefault) text('network.used_by_host'),
      if (route.accounts.isNotEmpty) text('network.used_by_accounts', {'names': route.accounts.join(', ')}),
      if (route.profiles.isNotEmpty) text('network.used_by_profiles', {'names': route.profiles.join(', ')}),
    ];
    final words = [
      route.title,
      if (used.isEmpty) text('network.unused') else ...used,
      if (route.leases > 0) text('network.leases', {'count': '${route.leases}'}),
      if (route.state != null) route.state!,
    ].join(' · ');
    final tried = _tried[route.name];
    return SettingsRow(
      key: Key('route ${route.name}'),
      title: route.name,
      leading: Icon(route.isDefault ? Icons.alt_route : Icons.call_split, size: 24),
      description: words,
      below: (route.problems.isEmpty && tried == null)
          ? null
          : Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              for (final problem in route.problems) Text(problem, key: Key('route-issue ${route.name}'), style: TextStyle(color: scheme.error)),
              if (tried != null) Text(tried, key: Key('route-test ${route.name}'), style: Theme.of(context).textTheme.bodySmall),
            ]),
      control: Wrap(spacing: 4, crossAxisAlignment: WrapCrossAlignment.center, children: [
        if (route.isDefault)
          Chip(key: Key('route-default ${route.name}'), label: Text(text('network.default_badge')))
        else
          TextButton(key: Key('route-make-default ${route.name}'), onPressed: () => unawaited(_default(route)), child: Text(text('network.make_default'))),
        TextButton(key: Key('route-test-run ${route.name}'), onPressed: () => unawaited(_try(route)), child: Text(text('network.test'))),
        if (!route.builtin) ...[
          IconButton(key: Key('route-edit ${route.name}'), tooltip: text('common.edit'), icon: const Icon(Icons.edit_outlined, size: 20), onPressed: () => unawaited(_edit(route))),
          IconButton(
            key: Key('route-remove ${route.name}'),
            tooltip: text('common.remove'),
            icon: Icon(Icons.delete_outline, size: 20, color: scheme.error),
            onPressed: () => unawaited(_remove(route)),
          ),
        ],
      ]),
    );
  }
}

/// Makes a route or changes one: its name (fixed once made), the provider and the form that provider's schema describes. A setting
/// that names a host variable is the name of the variable, never a value. A refusal is shown in the host's own words.
class RouteDialog extends StatefulWidget {
  const RouteDialog({super.key, required this.routes, this.route});

  final NetworkRoutes routes;

  /// The route being changed; null makes a new one.
  final RouteRow? route;

  @override
  State<RouteDialog> createState() => _RouteDialogState();
}

class _RouteDialogState extends State<RouteDialog> {
  late final TextEditingController _name = TextEditingController(text: widget.route?.name ?? '');
  late String? _provider = _initial()?.provider;
  late Map<String, Object?> _values = {...?widget.route?.settings};
  int _generation = 0;
  bool _saving = false;
  String? _problem;

  RouteOffer? get _offer => widget.routes.offers.where((each) => each.provider == _provider).firstOrNull;

  RouteOffer? _initial() {
    final offers = widget.routes.offers;
    final held = widget.route?.provider;
    return offers.where((each) => each.provider == held).firstOrNull ?? offers.firstOrNull;
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    final text = AppText.of(context);
    final offer = _offer;
    final name = _name.text.trim();
    if (offer == null) return;
    if (name.isEmpty) {
      setState(() => _problem = text('network.name_needed'));
      return;
    }
    final problem = schemaProblem(offer.schema, offer.form, _values);
    if (problem != null) {
      setState(() => _problem = problem);
      return;
    }
    setState(() {
      _saving = true;
      _problem = null;
    });
    try {
      await widget.routes.save(name, offer.provider, collectSchemaValues(offer.schema, offer.form, _values));
      if (mounted) Navigator.pop(context);
    } on Object catch (error) {
      if (mounted) {
        setState(() {
          _saving = false;
          _problem = hostWords(error);
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final text = AppText.of(context);
    final offer = _offer;
    final scheme = Theme.of(context).colorScheme;
    return AlertDialog(
      key: const Key('route-dialog'),
      title: Text(widget.route == null ? text('network.add_title') : text('network.edit_title', {'name': widget.route!.name})),
      content: SizedBox(
        width: 520,
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            TextField(
              key: const Key('route-name'),
              controller: _name,
              enabled: widget.route == null,
              decoration: InputDecoration(labelText: text('network.name')),
            ),
            const SizedBox(height: 12),
            DropdownButtonFormField<String>(
              key: const Key('route-provider'),
              initialValue: offer?.provider,
              decoration: InputDecoration(labelText: text('network.provider')),
              items: [for (final each in widget.routes.offers) DropdownMenuItem(value: each.provider, child: Text(each.title))],
              onChanged: widget.route != null
                  ? null
                  : (picked) => setState(() {
                      _provider = picked;
                      _values = {};
                      _generation++;
                    }),
            ),
            if (offer?.description != null) Padding(padding: const EdgeInsets.only(top: 6), child: Text(offer!.description!, style: Theme.of(context).textTheme.bodySmall)),
            if (offer != null && offer.secrets.isNotEmpty)
              Padding(padding: const EdgeInsets.only(top: 6), child: Text(text('network.secret_note'), style: Theme.of(context).textTheme.bodySmall)),
            if (offer != null)
              SchemaFields(
                key: ValueKey('route-form-${offer.provider}-$_generation'),
                schema: offer.schema,
                form: offer.form,
                values: _values,
                idPrefix: 'route-form',
                onValues: (held) => _values = held,
              ),
            if (_problem != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(_problem!, key: const Key('route-dialog-problem'), style: TextStyle(color: scheme.error))),
          ]),
        ),
      ),
      actions: [
        TextButton(key: const Key('route-cancel'), onPressed: () => Navigator.pop(context), child: Text(text('common.cancel'))),
        FilledButton(key: const Key('route-save'), onPressed: _saving ? null : () => unawaited(_save()), child: Text(text('common.save'))),
      ],
    );
  }
}
