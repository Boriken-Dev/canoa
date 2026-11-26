import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../config/canoa_config.dart';
import '../l10n/app_text.dart';
import 'hosting_panel.dart' show SaveHosting;
import 'kb_offers.dart';

/// Where the host's knowledge search gets meaning and ordering, `[host.kb.embedder]` and `[host.kb.ranker]`, and what can do the
/// host's own model work: for each operation the backends offered (an account's API such as `openai:work`, a server of the person's
/// own, an agent on its own login), the one asked first and the ones after it, and the agents the person offers as backends, with the
/// warm sessions they keep. Canoa chooses none of them, fetches no model, and sends the knowledge searched only where the backend
/// chosen sends it, which the page says beside the choice.
///
/// What is offered comes from the host (`kb/offers`); without it the backends and models are typed, as canoa.cfg writes them.
class KnowledgeSearchPanel extends StatefulWidget {
  const KnowledgeSearchPanel({
    super.key,
    required this.config,
    required this.onSave,
    this.host,
  });

  final CanoaConfig config;
  final SaveHosting onSave;

  /// What asks the host for the offers and tries a choice; none leaves the fields to be typed.
  final KbHost? host;

  @override
  State<KnowledgeSearchPanel> createState() => _KnowledgeSearchPanelState();
}

/// The operations the page edits, in the order it shows them.
const _operations = ['embed', 'rerank', 'complete'];

/// One `[assist_harness.NAME]` the page holds.
class _HarnessRow {
  _HarnessRow(this.name, Map<String, String> values)
    : fields = {
        for (final key in CanoaConfig.assistHarnessKeys) key: TextEditingController(text: values[key] ?? ''),
      };

  /// What the table is called; text, so it is edited as one.
  String name;
  final Map<String, TextEditingController> fields;

  Map<String, String> get values => {
    for (final MapEntry(:key, :value) in fields.entries)
      if (value.text.trim().isNotEmpty) key: value.text.trim(),
  };

  void dispose() {
    for (final field in fields.values) {
      field.dispose();
    }
  }
}

class _KnowledgeSearchPanelState extends State<KnowledgeSearchPanel> {
  late CanoaConfig config = widget.config;
  final _fields = <String, TextEditingController>{};
  final _use = <String, TextEditingController>{};
  final _fallback = <String, List<String>>{};
  final _harnesses = <_HarnessRow>[];
  late final TextEditingController _idle;
  late final TextEditingController _max;
  bool busy = false;
  bool saved = false;
  String? message;

  KbOffers? _offers;

  /// The models of the offer a role has chosen, by role, with the offer they were asked for.
  final _models = <String, (String, KbModels?)>{};
  final _tests = <String, String>{};
  final _testing = <String>{};

  @override
  void initState() {
    super.initState();
    final kb = config.hostKb;
    for (final key in CanoaConfig.hostKbKeys) {
      _fields[key] = TextEditingController(text: kb[key] ?? '')..addListener(_edited);
    }
    final held = config.assistOperations;
    for (final operation in _operations) {
      _use[operation] = TextEditingController(text: held[operation]?.use ?? '')..addListener(_edited);
      _fallback[operation] = [...?held[operation]?.fallback];
    }
    for (final MapEntry(:key, :value) in config.assistHarnesses.entries) {
      _harnesses.add(_HarnessRow(key, value)..fields.values.forEach((each) => each.addListener(_edited)));
    }
    final pool = config.assistPool;
    _idle = TextEditingController(text: '${pool.idleSeconds ?? ''}')..addListener(_edited);
    _max = TextEditingController(text: '${pool.maxSessions ?? ''}')..addListener(_edited);
    unawaited(_load());
  }

  Future<void> _load() async {
    final offers = await widget.host?.offers();
    if (mounted && offers != null) setState(() => _offers = offers);
  }

  void _edited() {
    if (mounted) setState(() => message = null);
  }

  @override
  void dispose() {
    for (final field in [..._fields.values, ..._use.values, _idle, _max]) {
      field.dispose();
    }
    for (final row in _harnesses) {
      row.dispose();
    }
    super.dispose();
  }

  Map<String, String> get _values => {
    for (final MapEntry(:key, :value) in _fields.entries) key: value.text.trim(),
  };

  /// What is wrong with the values, or null: a server that is no http or https address would be refused, and a harness needs a
  /// name the host takes and an agent to run.
  String? get _problem {
    final server = _values['server']!;
    if (server.isNotEmpty) {
      final uri = Uri.tryParse(server);
      if (uri == null || !(uri.scheme == 'http' || uri.scheme == 'https') || uri.host.isEmpty) {
        return context.t('kb.problem.server');
      }
    }
    final names = <String>{};
    for (final row in _harnesses) {
      if (!RegExp(r'^[a-z0-9_-]{1,32}$').hasMatch(row.name) || !names.add(row.name)) {
        return context.t('kb.problem.harness_name', {'name': row.name});
      }
      if (row.fields['source']!.text.trim().isEmpty) {
        return context.t('kb.problem.harness_source', {'name': row.name});
      }
    }
    for (final field in [_idle, _max]) {
      final text = field.text.trim();
      if (text.isNotEmpty && (int.tryParse(text) ?? 0) < 1) return context.t('kb.problem.pool');
    }
    return null;
  }

  bool get _kbChanged {
    final held = config.hostKb;
    final values = _values;
    return CanoaConfig.hostKbKeys.any((key) => (held[key] ?? '') != values[key]);
  }

  Map<String, ({String? use, List<String> fallback})> get _operationValues => {
    for (final operation in _operations)
      operation: (
        use: _use[operation]!.text.trim().isEmpty ? null : _use[operation]!.text.trim(),
        fallback: _fallback[operation]!,
      ),
  };

  bool get _assistChanged {
    final held = config.assistOperations;
    for (final operation in _operations) {
      final now = _operationValues[operation]!;
      final was = held[operation];
      if (now.use != was?.use || now.fallback.join('\n') != (was?.fallback ?? const []).join('\n')) return true;
    }
    final harnesses = config.assistHarnesses;
    if (harnesses.length != _harnesses.length) return true;
    for (final row in _harnesses) {
      final was = harnesses[row.name];
      if (was == null || '${row.values}' != '$was') return true;
    }
    final pool = config.assistPool;
    return _idle.text.trim() != '${pool.idleSeconds ?? ''}' || _max.text.trim() != '${pool.maxSessions ?? ''}';
  }

  bool get _changed => _kbChanged || _assistChanged;

  CanoaConfig _next() {
    var next = _kbChanged ? config.withHostKb(_values) : config;
    if (_assistChanged) {
      next = next
          .withAssistOperations(_operationValues)
          .withAssistHarnesses({for (final row in _harnesses) row.name: row.values})
          .withAssistPool(idleSeconds: int.tryParse(_idle.text.trim()), maxSessions: int.tryParse(_max.text.trim()));
    }
    return next;
  }

  Future<void> _save() async {
    setState(() {
      busy = true;
      message = null;
    });
    try {
      final changed = _next();
      final problem = await widget.onSave(changed);
      if (!mounted) return;
      if (problem == null) {
        // What the file holds now, which fills in the backend a role was left to choose.
        final held = changed.hostKb;
        for (final MapEntry(:key, :value) in _fields.entries) {
          if (value.text.trim() != (held[key] ?? '')) value.text = held[key] ?? '';
        }
      }
      setState(() {
        if (problem == null) config = changed;
        saved = problem == null;
        message = problem ?? context.t('kb.saved');
      });
      if (problem == null) unawaited(_load());
    } on Object catch (error) {
      if (mounted) {
        setState(() {
          saved = false;
          message = '$error';
        });
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Widget _field(String key, String label, String hint, {Widget? suffix}) => Padding(
    padding: const EdgeInsets.only(bottom: 10),
    child: TextField(
      key: Key('host-kb-$key'),
      controller: _fields[key],
      decoration: InputDecoration(
        labelText: label,
        hintText: hint,
        border: const OutlineInputBorder(),
        isDense: true,
        suffixIcon: suffix,
      ),
    ),
  );

  /// The offers that can do what a role needs: the operations `ops` name, each offer once.
  List<KbOffer> _offersFor(List<String> ops) {
    final seen = <String>{};
    return [
      for (final id in ops)
        for (final offer in _offers?.operation(id)?.offers ?? const <KbOffer>[])
          if (seen.add(offer.id)) offer,
    ];
  }

  String _offerKind(BuildContext context, KbOffer offer) {
    final kind = context.t('kb.kind.${offer.kind}');
    return '${offer.title} · $kind';
  }

  String _offerLabel(KbOffer offer) => '${offer.title} (${offer.id})';

  Widget _pickOffer(String role, List<String> ops) {
    final offers = _offersFor(ops);
    if (offers.isEmpty) return const SizedBox.shrink();
    return PopupMenuButton<String>(
      key: Key('host-kb-${role}_use-pick'),
      tooltip: context.t('kb.pick_backend'),
      icon: const Icon(Icons.arrow_drop_down),
      onSelected: (id) {
        _fields['${role}_use']!.text = id;
        unawaited(_loadModels(role, ops, id));
      },
      itemBuilder: (context) => [
        for (final offer in offers)
          PopupMenuItem<String>(value: offer.id, child: Text(_offerLabel(offer))),
      ],
    );
  }

  Future<void> _loadModels(String role, List<String> ops, String id) async {
    final host = widget.host;
    if (host == null) return;
    final offers = await host.offers(id);
    if (!mounted || offers == null) return;
    // A backend that ranks natively is asked for rerank models; one that only chats, for chat models.
    final operation = ops.map(offers.operation).whereType<KbOperation>().where((each) => each.offer(id) != null).firstOrNull;
    setState(() => _models[role] = (id, operation?.offer(id)?.models));
  }

  Widget _pickModel(String role, List<String> ops) {
    final current = _fields['${role}_use']!.text.trim();
    final held = _models[role];
    final models = held != null && held.$1 == current ? held.$2 : null;
    if (current.isEmpty || models == null) {
      return current.isEmpty || widget.host == null
          ? const SizedBox.shrink()
          : IconButton(
              key: Key('host-kb-${role}_model-load'),
              tooltip: context.t('kb.pick_model'),
              icon: const Icon(Icons.arrow_drop_down),
              onPressed: () => unawaited(_loadModels(role, ops, current)),
            );
    }
    return PopupMenuButton<String>(
      key: Key('host-kb-${role}_model-pick'),
      tooltip: context.t('kb.pick_model'),
      icon: const Icon(Icons.arrow_drop_down),
      onSelected: (id) => _fields['${role}_model']!.text = id,
      itemBuilder: (context) => [
        for (final model in models.offered) PopupMenuItem<String>(value: model.id, child: Text(model.label)),
        if (models.unreported.isNotEmpty)
          PopupMenuItem<String>(enabled: false, child: Text(context.t('model.unreported'), key: Key('host-kb-${role}_model-unreported'))),
        for (final model in models.unreported) PopupMenuItem<String>(value: model.id, child: Text(model.label)),
        if (!models.listed)
          PopupMenuItem<String>(enabled: false, child: Text(models.why ?? context.t('kb.models_not_listed'))),
      ],
    );
  }

  /// Where the text goes when the role uses [use]: the offer's own words and its address, or what the choice stands for.
  String _where(String role, List<String> ops) {
    final use = _fields['${role}_use']!.text.trim();
    final offers = _offers;
    if (offers == null) return '';
    // The choice `default` is whatever canoa.cfg's `[assist.OPERATION]` puts first, which the host says as `asked`.
    final chosen = use.isEmpty || use == 'default' ? ops.expand((op) => offers.operation(op)?.asked ?? const <String>[]).firstOrNull : use;
    final offer = chosen == null ? null : _offersFor(ops).where((each) => each.id == chosen).firstOrNull;
    if (offer == null) {
      return use.isEmpty && _fields['${role}_model']!.text.trim().isEmpty ? '' : context.t('kb.where_unknown', {'use': use.isEmpty ? 'default' : use});
    }
    final what = role == 'embedder' ? 'kb.where_embedder' : 'kb.where_ranker';
    return context.t(what, {'title': offer.title, 'where': [offer.location, ?offer.address].where((each) => each.isNotEmpty).join(' ')});
  }

  Future<void> _test(String role, List<String> ops) async {
    final host = widget.host;
    final model = _fields['${role}_model']!.text.trim();
    if (host == null || model.isEmpty) return;
    final use = _fields['${role}_use']!.text.trim();
    setState(() {
      _testing.add(role);
      _tests.remove(role);
    });
    String said;
    try {
      final answer = await host.test({'role': role, if (use.isNotEmpty) 'use': use, 'model': model});
      final ms = '${answer['ms'] ?? ''}';
      if (!mounted) return;
      if (answer['ok'] == true) {
        said = role == 'embedder'
            ? context.t('kb.test_embedder_ok', {'length': answer['length'], 'ms': ms})
            : answer['best_first'] == true
            ? context.t('kb.test_ranker_ok', {'ms': ms})
            : context.t('kb.test_ranker_off', {'order': (answer['order'] as List?)?.map((each) => '${(each as num) + 1}').join(', ') ?? '', 'ms': ms});
      } else {
        said = context.t('kb.test_failed', {'why': answer['why'] ?? ''});
      }
    } on Object catch (error) {
      if (!mounted) return;
      said = context.t('kb.test_failed', {'why': error});
    }
    setState(() {
      _testing.remove(role);
      _tests[role] = said;
    });
  }

  Widget _role(BuildContext context, String role, List<String> ops, {required String title, required String modelLabel}) {
    final text = Theme.of(context).textTheme;
    final where = _where(role, ops);
    final testable = widget.host != null && _fields['${role}_model']!.text.trim().isNotEmpty;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(title, style: text.titleSmall),
        const SizedBox(height: 6),
        _field('${role}_use', context.t('kb.backend'), context.t('kb.backend_hint'), suffix: _pickOffer(role, ops)),
        _field('${role}_model', modelLabel, context.t('kb.model_hint'), suffix: _pickModel(role, ops)),
        if (where.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Text(where, key: Key('host-kb-${role}_where'), style: text.bodySmall),
          ),
        if (widget.host != null)
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: Row(
              children: [
                OutlinedButton(
                  key: Key('host-kb-${role}_test'),
                  onPressed: !testable || _testing.contains(role) ? null : () => unawaited(_test(role, ops)),
                  child: Text(_testing.contains(role) ? context.t('kb.testing') : context.t('kb.test')),
                ),
                const SizedBox(width: 12),
                if (_tests[role] case final result?) Expanded(child: Text(result, key: Key('host-kb-${role}_test-result'), style: text.bodySmall)),
              ],
            ),
          ),
      ],
    );
  }

  /// The order the host would ask the backends of [operation] in, or why it can ask none.
  Widget _asked(BuildContext context, KbOperation? operation) {
    if (operation == null) return const SizedBox.shrink();
    final style = Theme.of(context).textTheme.bodySmall;
    final said = operation.asked.isNotEmpty ? context.t('kb.asked', {'order': operation.asked.join(' → ')}) : context.t('kb.nothing_can', {'why': operation.problem ?? ''});
    return Padding(
      padding: const EdgeInsets.only(top: 4),
      child: Text(said, key: Key('assist-${operation.id}-asked'), style: style),
    );
  }

  Widget _operation(BuildContext context, String id) {
    final text = Theme.of(context).textTheme;
    final held = _offers?.operation(id);
    final offers = held?.offers ?? const <KbOffer>[];
    final taken = {_use[id]!.text.trim(), ..._fallback[id]!};
    return Padding(
      padding: const EdgeInsets.only(bottom: 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(context.t('kb.op.$id'), style: text.titleSmall),
          if (offers.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Wrap(
                spacing: 8,
                runSpacing: 4,
                children: [
                  for (final offer in offers)
                    Tooltip(
                      message: [offer.location, ?offer.address].where((each) => each.isNotEmpty).join(' '),
                      child: Chip(
                        key: Key('assist-$id-offer-${offer.id}'),
                        label: Text(_offerKind(context, offer)),
                        visualDensity: VisualDensity.compact,
                      ),
                    ),
                ],
              ),
            )
          else if (_offers != null)
            Text(context.t('kb.no_offers'), key: Key('assist-$id-none'), style: text.bodySmall),
          const SizedBox(height: 6),
          TextField(
            key: Key('assist-$id-use'),
            controller: _use[id],
            decoration: InputDecoration(
              labelText: context.t('kb.asked_first'),
              hintText: context.t('kb.asked_first_hint'),
              border: const OutlineInputBorder(),
              isDense: true,
              suffixIcon: offers.isEmpty
                  ? null
                  : PopupMenuButton<String>(
                      key: Key('assist-$id-use-pick'),
                      tooltip: context.t('kb.pick_backend'),
                      icon: const Icon(Icons.arrow_drop_down),
                      onSelected: (value) => _use[id]!.text = value,
                      itemBuilder: (context) => [for (final offer in offers) PopupMenuItem<String>(value: offer.id, child: Text(_offerLabel(offer)))],
                    ),
            ),
          ),
          const SizedBox(height: 6),
          Wrap(
            spacing: 6,
            runSpacing: 4,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              Text(context.t('kb.then'), style: text.bodySmall),
              for (final (at, backend) in _fallback[id]!.indexed)
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (at > 0)
                      IconButton(
                        key: Key('assist-$id-fallback-$backend-earlier'),
                        tooltip: context.t('kb.earlier'),
                        visualDensity: VisualDensity.compact,
                        iconSize: 16,
                        icon: const Icon(Icons.arrow_back),
                        onPressed: () => setState(() {
                          final list = _fallback[id]!;
                          list
                            ..removeAt(at)
                            ..insert(at - 1, backend);
                          message = null;
                        }),
                      ),
                    InputChip(
                      key: Key('assist-$id-fallback-$backend'),
                      label: Text(backend),
                      visualDensity: VisualDensity.compact,
                      onDeleted: () => setState(() {
                        _fallback[id]!.removeAt(at);
                        message = null;
                      }),
                    ),
                  ],
                ),
              if (offers.any((offer) => !taken.contains(offer.id)))
                PopupMenuButton<String>(
                  key: Key('assist-$id-fallback-add'),
                  tooltip: context.t('kb.add_fallback'),
                  icon: const Icon(Icons.add),
                  onSelected: (value) => setState(() {
                    _fallback[id]!.add(value);
                    message = null;
                  }),
                  itemBuilder: (context) => [
                    for (final offer in offers)
                      if (!taken.contains(offer.id)) PopupMenuItem<String>(value: offer.id, child: Text(_offerLabel(offer))),
                  ],
                ),
            ],
          ),
          _asked(context, held),
        ],
      ),
    );
  }

  Widget _harness(BuildContext context, _HarnessRow row, int at) {
    final sources = _offers?.harnesses ?? const <KbHarnessSource>[];
    final source = sources.where((each) => each.source == row.fields['source']!.text.trim()).firstOrNull;
    Widget field(String key, {List<String> pick = const []}) => SizedBox(
      width: 200,
      child: TextField(
        key: Key('assist-harness-$at-$key'),
        controller: row.fields[key],
        decoration: InputDecoration(
          labelText: context.t('kb.harness.$key'),
          border: const OutlineInputBorder(),
          isDense: true,
          suffixIcon: pick.isEmpty
              ? null
              : PopupMenuButton<String>(
                  key: Key('assist-harness-$at-$key-pick'),
                  icon: const Icon(Icons.arrow_drop_down),
                  onSelected: (value) => row.fields[key]!.text = value,
                  itemBuilder: (context) => [for (final each in pick) PopupMenuItem<String>(value: each, child: Text(each))],
                ),
        ),
      ),
    );
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: Wrap(
        spacing: 8,
        runSpacing: 8,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          SizedBox(
            width: 140,
            child: TextFormField(
              key: Key('assist-harness-$at-name'),
              initialValue: row.name,
              decoration: InputDecoration(labelText: context.t('kb.harness.name'), border: const OutlineInputBorder(), isDense: true),
              onChanged: (value) => setState(() {
                row.name = value.trim();
                message = null;
              }),
            ),
          ),
          field('source', pick: [for (final each in sources) each.source]),
          field('profile', pick: source?.profiles ?? const []),
          field('account', pick: source?.accounts ?? const []),
          field('model'),
          field('effort'),
          IconButton(
            key: Key('assist-harness-$at-remove'),
            tooltip: context.t('kb.harness.remove'),
            icon: const Icon(Icons.delete_outline),
            onPressed: () => setState(() {
              _harnesses.remove(row);
              row.dispose();
              message = null;
            }),
          ),
        ],
      ),
    );
  }

  void _addHarness() => setState(() {
    var number = _harnesses.length + 1;
    while (_harnesses.any((row) => row.name == 'harness$number')) {
      number++;
    }
    _harnesses.add(_HarnessRow('harness$number', const {})..fields.values.forEach((each) => each.addListener(_edited)));
    message = null;
  });

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final problem = _problem;
    final failed = problem != null || (message != null && !saved);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(context.t('kb.intro'), key: const Key('host-kb-intro')),
        const SizedBox(height: 12),
        _role(context, 'embedder', const ['embed'], title: context.t('kb.meaning'), modelLabel: context.t('kb.model.embedder')),
        _role(context, 'ranker', const ['rerank', 'complete'], title: context.t('kb.ordering'), modelLabel: context.t('kb.model.ranker')),
        Text(context.t('kb.server'), style: text.titleSmall),
        const SizedBox(height: 6),
        _field('server', context.t('kb.server.address'), context.t('kb.server.hint')),
        const SizedBox(height: 8),
        Text(context.t('kb.backends'), style: text.titleSmall),
        const SizedBox(height: 4),
        Text(context.t('kb.backends_intro'), style: text.bodySmall),
        const SizedBox(height: 8),
        for (final operation in _operations) _operation(context, operation),
        Text(context.t('kb.harnesses'), style: text.titleSmall),
        const SizedBox(height: 4),
        Text(context.t('kb.harnesses_intro'), style: text.bodySmall),
        const SizedBox(height: 8),
        for (final (at, row) in _harnesses.indexed) _harness(context, row, at),
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton.icon(
            key: const Key('assist-harness-add'),
            onPressed: _addHarness,
            icon: const Icon(Icons.add),
            label: Text(context.t('kb.harness.add')),
          ),
        ),
        const SizedBox(height: 4),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            SizedBox(
              width: 200,
              child: TextField(
                key: const Key('assist-pool-idle'),
                controller: _idle,
                keyboardType: TextInputType.number,
                decoration: InputDecoration(
                  labelText: context.t('kb.pool.idle'),
                  hintText: '${_offers?.idleSeconds ?? 120}',
                  border: const OutlineInputBorder(),
                  isDense: true,
                ),
              ),
            ),
            SizedBox(
              width: 200,
              child: TextField(
                key: const Key('assist-pool-max'),
                controller: _max,
                keyboardType: TextInputType.number,
                decoration: InputDecoration(
                  labelText: context.t('kb.pool.max'),
                  hintText: '${_offers?.maxSessions ?? 2}',
                  border: const OutlineInputBorder(),
                  isDense: true,
                ),
              ),
            ),
          ],
        ),
        const SizedBox(height: 4),
        Text(context.t('kb.pool.hint'), style: text.bodySmall),
        const SizedBox(height: 12),
        Row(
          children: [
            FilledButton(
              key: const Key('host-kb-save'),
              onPressed: busy || problem != null || !_changed ? null : () => unawaited(_save()),
              child: Text(context.t('common.save')),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Text(
                problem ?? message ?? context.t('kb.leave_empty'),
                key: const Key('host-kb-note'),
                style: failed ? text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.error) : text.bodySmall,
              ),
            ),
          ],
        ),
      ],
    );
  }
}
