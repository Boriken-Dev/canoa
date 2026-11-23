import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../config/canoa_config.dart';
import 'hosting_panel.dart' show SaveHosting;

/// Where the host's knowledge search gets meaning and ordering, `[host.kb]`: an embeddings server (search by what is meant) and a
/// chat model (the best passages put in order), each the person's own, named by address and the exact model it loaded. Canoa
/// chooses neither, fetches no model, and sends the knowledge searched only to an address typed here.
class KnowledgeSearchPanel extends StatefulWidget {
  const KnowledgeSearchPanel({super.key, required this.config, required this.onSave});

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
      _fields[key] = TextEditingController(text: saved[key] ?? '')..addListener(() => setState(() => message = null));
    }
  }

  @override
  void dispose() {
    for (final field in _fields.values) {
      field.dispose();
    }
    super.dispose();
  }

  Map<String, String> get _values => {for (final MapEntry(:key, :value) in _fields.entries) key: value.text.trim()};

  /// What is wrong with the four values, or null: an address with no model, or a model with no address, would be refused.
  String? get _problem {
    final values = _values;
    for (final (address, model, what) in const [
      ('embeddings', 'model', 'The embeddings server'),
      ('llm_endpoint', 'llm_model', 'The chat model'),
    ]) {
      final named = values[address]!.isNotEmpty;
      if (named && values[model]!.isEmpty) return '$what needs the exact model its server loaded.';
      if (!named && values[model]!.isNotEmpty) return '$what needs the address of its server.';
      if (named) {
        final uri = Uri.tryParse(values[address]!);
        if (uri == null || !(uri.scheme == 'http' || uri.scheme == 'https') || uri.host.isEmpty) {
          return '$what is an http or https address.';
        }
      }
    }
    return null;
  }

  bool get _changed {
    final saved = config.hostKb;
    final values = _values;
    return CanoaConfig.hostKbKeys.any((key) => (saved[key] ?? '') != values[key]);
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
      decoration: InputDecoration(labelText: label, hintText: hint, border: const OutlineInputBorder(), isDense: true),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final problem = _problem;
    final failed = problem != null || (message != null && !message!.startsWith('Saved'));
    return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
      const Text(
        'An agent can search the knowledge its overlays ship. By default that matches the words it uses. Name a server of your own '
        '(an OpenAI-compatible one, such as a model server on this computer) to match what is meant, and a chat model to put the best passages '
        'first. Searched text goes to these addresses only.',
        key: Key('host-kb-intro'),
      ),
      const SizedBox(height: 12),
      Text('Meaning', style: text.titleSmall),
      const SizedBox(height: 6),
      _field('embeddings', 'Embeddings server', 'http://127.0.0.1:PORT'),
      _field('model', 'Embeddings model', 'the exact id the server loaded'),
      Text('Ordering', style: text.titleSmall),
      const SizedBox(height: 6),
      _field('llm_endpoint', 'Chat model server', 'http://127.0.0.1:PORT'),
      _field('llm_model', 'Chat model', 'the exact id the server loaded'),
      Row(children: [
        FilledButton(
          key: const Key('host-kb-save'),
          onPressed: busy || problem != null || !_changed ? null : () => unawaited(_save()),
          child: const Text('Save'),
        ),
        const SizedBox(width: 12),
        Expanded(
          child: Text(
            problem ?? message ?? 'Leave a pair empty to search without it.',
            key: const Key('host-kb-note'),
            style: failed ? text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.error) : text.bodySmall,
          ),
        ),
      ]),
    ]);
  }
}
