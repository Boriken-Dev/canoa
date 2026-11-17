/// The editor of one profile of canoa.cfg's `[profiles]`: its name and the values a new session starts with when it is chosen: the
/// source it runs on, its model, effort and mode, its starting instructions (text added to the session's initial context after
/// every other layer's), the prompt cache lifetime and the loadout. The account and the permissions are the session's own and
/// are not part of one.
library;

import 'package:flutter/material.dart';

import '../chat/cache_lifetime.dart';
import '../config/canoa_config.dart';

class ProfileEditor extends StatefulWidget {
  const ProfileEditor({
    super.key,
    required this.original,
    required this.preset,
    required this.taken,
    required this.sources,
    required this.loadouts,
    required this.onSave,
    required this.onCancel,
  });

  /// The name the profile has in canoa.cfg; null for a new one.
  final String? original;

  /// What it holds now.
  final Preset preset;

  /// The names the other profiles have.
  final Set<String> taken;

  /// The sources a profile may run on, by label.
  final List<String> sources;

  /// The loadouts canoa.cfg names.
  final List<String> loadouts;

  /// Writes the profile; a refusal is the answer, shown beside Save.
  final Future<String?> Function(String name, Preset preset) onSave;
  final VoidCallback onCancel;

  @override
  State<ProfileEditor> createState() => _ProfileEditorState();
}

class _ProfileEditorState extends State<ProfileEditor> {
  late final _name = TextEditingController(text: widget.original ?? '');
  late final _model = TextEditingController(text: widget.preset.model ?? '');
  late final _effort = TextEditingController(text: widget.preset.effort ?? '');
  late final _mode = TextEditingController(text: widget.preset.mode ?? '');
  late final _context = TextEditingController(text: widget.preset.context ?? '');
  late String? _source = widget.preset.source;
  late String? _cache = widget.preset.cacheLifetime;
  late String? _loadout = widget.preset.loadout;
  String? _problem;
  bool _saving = false;

  @override
  void dispose() {
    for (final controller in [_name, _model, _effort, _mode, _context]) {
      controller.dispose();
    }
    super.dispose();
  }

  String? get _nameProblem {
    final name = _name.text;
    if (name.isEmpty) return null;
    if (!isPresetName(name)) return presetNameRule;
    if (widget.taken.any((other) => other.toLowerCase() == name.toLowerCase() && other != widget.original)) {
      return 'Another profile has that name';
    }
    return null;
  }

  static String? _text(TextEditingController controller) {
    final text = controller.text.trim();
    return text.isEmpty ? null : text;
  }

  Preset _made() => Preset(
    source: _source,
    model: _text(_model),
    effort: _text(_effort),
    mode: _text(_mode),
    // Instructions are kept as written, but a field of only blanks is none.
    context: _context.text.trim().isEmpty ? null : _context.text,
    cacheLifetime: _cache,
    loadout: _loadout,
  );

  Future<void> _save() async {
    setState(() {
      _saving = true;
      _problem = null;
    });
    final refused = await widget.onSave(_name.text, _made());
    if (mounted) {
      setState(() {
        _saving = false;
        _problem = refused;
      });
    }
  }

  DropdownMenuItem<String?> _none(String words) => DropdownMenuItem<String?>(value: null, child: Text(words));

  @override
  Widget build(BuildContext context) {
    final ready = _name.text.isNotEmpty && _nameProblem == null && !_saving;
    // A value the file holds that the list no longer offers stays shown, so saving does not drop it unseen.
    final sources = {...widget.sources, ?_source}.toList();
    final loadouts = {...widget.loadouts, ?_loadout}.toList();
    return Column(
      key: const Key('profile-editor'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(widget.original == null ? 'New profile' : 'Edit ${widget.original}', style: Theme.of(context).textTheme.titleMedium),
        const SizedBox(height: 8),
        TextField(
          key: const Key('profile-edit-name'),
          controller: _name,
          decoration: InputDecoration(labelText: 'Name', errorText: _nameProblem),
          onChanged: (_) => setState(() {}),
        ),
        const SizedBox(height: 8),
        DropdownButtonFormField<String?>(
          key: const Key('profile-edit-source'),
          isExpanded: true,
          initialValue: _source,
          decoration: const InputDecoration(labelText: 'Source'),
          items: [
            _none('The session\'s own'),
            for (final label in sources) DropdownMenuItem<String?>(value: label, child: Text(label)),
          ],
          onChanged: (value) => setState(() => _source = value),
        ),
        const SizedBox(height: 8),
        Row(children: [
          Expanded(
            child: TextField(
              key: const Key('profile-edit-model'),
              controller: _model,
              decoration: const InputDecoration(labelText: 'Model', hintText: 'The agent\'s own'),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: TextField(
              key: const Key('profile-edit-effort'),
              controller: _effort,
              decoration: const InputDecoration(labelText: 'Effort', hintText: 'The agent\'s own'),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: TextField(
              key: const Key('profile-edit-mode'),
              controller: _mode,
              decoration: const InputDecoration(labelText: 'Mode', hintText: 'The agent\'s own'),
            ),
          ),
        ]),
        const SizedBox(height: 8),
        TextField(
          key: const Key('profile-edit-context'),
          controller: _context,
          minLines: 3,
          maxLines: 10,
          decoration: const InputDecoration(
            labelText: 'Starting instructions',
            alignLabelWithHint: true,
            helperText: 'Added to the session\'s initial context after every other layer\'s',
          ),
        ),
        const SizedBox(height: 8),
        Row(children: [
          Expanded(
            child: DropdownButtonFormField<String?>(
              key: const Key('profile-edit-cache'),
              isExpanded: true,
              initialValue: _cache,
              decoration: const InputDecoration(labelText: 'Prompt cache'),
              items: [
                _none('Not set'),
                for (final lifetime in CacheLifetime.values)
                  DropdownMenuItem<String?>(
                    value: lifetime.id,
                    child: Text('${lifetime.label}: ${lifetime.about}', overflow: TextOverflow.ellipsis),
                  ),
              ],
              onChanged: (value) => setState(() => _cache = value),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: DropdownButtonFormField<String?>(
              key: const Key('profile-edit-loadout'),
              isExpanded: true,
              initialValue: _loadout,
              decoration: const InputDecoration(labelText: 'Loadout'),
              items: [
                _none('The default'),
                for (final name in loadouts)
                  DropdownMenuItem<String?>(
                    value: name,
                    child: Text(widget.loadouts.contains(name) ? name : '$name (not in canoa.cfg)', overflow: TextOverflow.ellipsis),
                  ),
              ],
              onChanged: (value) => setState(() => _loadout = value),
            ),
          ),
        ]),
        if (_problem != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(_problem!, key: const Key('profile-edit-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
          ),
        const SizedBox(height: 8),
        Row(mainAxisAlignment: MainAxisAlignment.end, children: [
          TextButton(key: const Key('profile-edit-cancel'), onPressed: widget.onCancel, child: const Text('Cancel')),
          const SizedBox(width: 8),
          FilledButton(key: const Key('profile-edit-save'), onPressed: ready ? _save : null, child: const Text('Save')),
        ]),
      ],
    );
  }
}
