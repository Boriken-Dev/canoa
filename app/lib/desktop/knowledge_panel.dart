import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../config/canoa_config.dart';
import 'hosting_panel.dart' show SaveHosting;

/// Where the host's knowledge search gets meaning and ordering, `[host.kb.embedder]` and `[host.kb.ranker]`: a backend (an account's
/// API such as `openai:work`, a server, or the operation's own choice) and the exact model each asks for, and the one server of the
/// person's own that this panel keeps. Canoa chooses none of them, fetches no model, and sends the knowledge searched only where the
/// backend chosen sends it.
class KnowledgeSearchPanel extends StatefulWidget {
  const KnowledgeSearchPanel({
    super.key,
    required this.config,
    required this.onSave,
  });

  final CanoaConfig config;
  final SaveHosting onSave;

  @override
  State<KnowledgeSearchPanel> createState() => _KnowledgeSearchPanelState();
}

class _KnowledgeSearchPanelState extends State<KnowledgeSearchPanel> {
  late CanoaConfig config = widget.config;
  final _fields = <String, TextEditingController>{};
  bool busy = false;
  String? message;

  @override
  void initState() {
    super.initState();
    final saved = config.hostKb;
    for (final key in CanoaConfig.hostKbKeys) {
      _fields[key] = TextEditingController(text: saved[key] ?? '')
        ..addListener(() => setState(() => message = null));
    }
  }

  @override
  void dispose() {
    for (final field in _fields.values) {
      field.dispose();
    }
    super.dispose();
  }

  Map<String, String> get _values => {
    for (final MapEntry(:key, :value) in _fields.entries)
      key: value.text.trim(),
  };

  /// What is wrong with the values, or null: a server that is no http or https address would be refused.
  String? get _problem {
    final server = _values['server']!;
    if (server.isEmpty) return null;
    final uri = Uri.tryParse(server);
    if (uri == null ||
        !(uri.scheme == 'http' || uri.scheme == 'https') ||
        uri.host.isEmpty) {
      return 'The server is an http or https address.';
    }
    return null;
  }

  bool get _changed {
    final saved = config.hostKb;
    final values = _values;
    return CanoaConfig.hostKbKeys.any(
      (key) => (saved[key] ?? '') != values[key],
    );
  }

  Future<void> _save() async {
    setState(() {
      busy = true;
      message = null;
    });
    try {
      final changed = config.withHostKb(_values);
      final problem = await widget.onSave(changed);
      if (!mounted) return;
      if (problem == null) {
        // What the file holds now, which fills in the backend a role was left to choose.
        final held = changed.hostKb;
        for (final MapEntry(:key, :value) in _fields.entries) {
          if (value.text.trim() != (held[key] ?? '')) {
            value.text = held[key] ?? '';
          }
        }
      }
      setState(() {
        if (problem == null) config = changed;
        message = problem ?? 'Saved. A session started now uses it.';
      });
    } on Object catch (error) {
      if (mounted) setState(() => message = '$error');
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Widget _field(String key, String label, String hint) => Padding(
    padding: const EdgeInsets.only(bottom: 10),
    child: TextField(
      key: Key('host-kb-$key'),
      controller: _fields[key],
      decoration: InputDecoration(
        labelText: label,
        hintText: hint,
        border: const OutlineInputBorder(),
        isDense: true,
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final problem = _problem;
    final failed =
        problem != null || (message != null && !message!.startsWith('Saved'));
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Text(
          'An agent can search the knowledge its overlays ship. By default that matches the words it uses. Name a backend to match what is '
          'meant, and one to put the best passages first: an account with an API key (openai:work), a server of your own, or leave the '
          'backend empty to use the choice in [assist]. Searched text goes where that backend sends it: every passage of the knowledge to '
          'the one that matches what is meant, once and again when it changes, and the query to both.',
          key: Key('host-kb-intro'),
        ),
        const SizedBox(height: 12),
        Text('Meaning', style: text.titleSmall),
        const SizedBox(height: 6),
        _field('embedder_use', 'Backend', 'openai:work, or empty'),
        _field(
          'embedder_model',
          'Embeddings model',
          'the exact id; empty turns it off',
        ),
        Text('Ordering', style: text.titleSmall),
        const SizedBox(height: 6),
        _field('ranker_use', 'Backend', 'openai:work, or empty'),
        _field('ranker_model', 'Model', 'the exact id; empty turns it off'),
        Text('A server of your own', style: text.titleSmall),
        const SizedBox(height: 6),
        _field(
          'server',
          'Server address',
          'http://127.0.0.1:PORT (an OpenAI-compatible server)',
        ),
        Row(
          children: [
            FilledButton(
              key: const Key('host-kb-save'),
              onPressed: busy || problem != null || !_changed
                  ? null
                  : () => unawaited(_save()),
              child: const Text('Save'),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Text(
                problem ??
                    message ??
                    'Leave a model empty to search without it.',
                key: const Key('host-kb-note'),
                style: failed
                    ? text.bodySmall?.copyWith(
                        color: Theme.of(context).colorScheme.error,
                      )
                    : text.bodySmall,
              ),
            ),
          ],
        ),
      ],
    );
  }
}
