import 'dart:async';

import 'package:flutter/material.dart';

import '../config/canoa_config.dart';
import '../settings/settings_controller.dart';
import '../settings/settings_row.dart';
import 'speech_backend.dart';

/// Settings > Voice: where speech is turned into text, said before anything is recorded, and a short test of the microphone.
class VoiceSection extends StatefulWidget {
  const VoiceSection({super.key, required this.controller, required this.backends});

  final SettingsController controller;

  /// The speech services this build has; none says "No speech service here".
  final List<SpeechBackend> backends;

  @override
  State<VoiceSection> createState() => _VoiceSectionState();
}

/// The choice that clears a key, so the service's own default applies.
const _own = '';

class _Choices {
  const _Choices(this.models, this.languages, this.devices);
  final List<SpeechChoice> models, languages, devices;
}

class _VoiceSectionState extends State<VoiceSection> {
  SpeechBackend? _loadedFor;
  Future<_Choices>? _choices;

  SpeechBackend? _backend(VoiceSettings voice) {
    for (final backend in widget.backends) {
      if (backend.id == voice.backend) return backend;
    }
    return null;
  }

  Future<_Choices> _load(SpeechBackend backend) async =>
      _Choices(await backend.models(), await backend.languages(), await backend.devices());

  void _reload(SpeechBackend backend) {
    final next = _load(backend);
    setState(() {
      _choices = next;
    });
  }

  void _set(VoiceSettings Function(VoiceSettings) change) =>
      widget.controller.change((c) => c.copyWith(voice: change(c.voice)));

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.controller,
    builder: (context, _) {
      final voice = widget.controller.config.voice;
      if (widget.backends.isEmpty) return const _NoSpeechService();
      final backend = _backend(voice);
      if (backend != null && !identical(backend, _loadedFor)) {
        _loadedFor = backend;
        _choices = _load(backend);
      }
      return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        SettingsGroup(title: 'Speech to text', children: [
          ChoiceRow<String>(
            title: 'Speech service',
            configKey: const ['voice', 'backend'],
            description: 'Who turns what you say into text',
            fieldKey: const Key('voice-backend'),
            value: backend?.id ?? voice.backend,
            choices: [
              for (final each in widget.backends) (each.id, each.name),
              if (backend == null) (voice.backend, '${voice.backend} (not available here)'),
            ],
            // The microphone stays off while the choice is made; a choice never starts a capture.
            onChanged: (id) => _set((v) => v.copyWith(backend: id, clearModel: true, clearLanguage: true, clearDevice: true)),
          ),
          if (backend == null)
            const SettingsRow(
              title: 'Not available here',
              description: 'The service named in canoa.cfg is not part of this build. Choose another; nothing is sent anywhere meanwhile',
              child: true,
            )
          else ...[
            SettingsRow(
              key: const Key('voice-where'),
              title: 'Where it is processed',
              description: '${backend.where.place.label}. ${backend.where.words}',
              child: true,
            ),
            if (backend.where.endpoint case final endpoint?)
              SettingsRow(
                key: const Key('voice-endpoint'),
                title: 'Account or address',
                description: endpoint,
                child: true,
                machine: true,
              ),
            SettingsRow(
              key: const Key('voice-privacy'),
              title: 'Audio and recordings',
              description: '${switch (backend.where.leaves) {
                AudioLeaves.no => 'The audio stays on this device.',
                AudioLeaves.maybe => 'The audio may leave this device, as the service above decides.',
                AudioLeaves.yes => 'The audio leaves this device for the service above.',
              }} Canoa keeps no recording, and the microphone is on only while you dictate.',
              child: true,
            ),
          ],
        ]),
        if (backend != null)
          FutureBuilder<_Choices>(
            future: _choices,
            builder: (context, snapshot) {
              final choices = snapshot.data ?? const _Choices([], [], []);
              return SettingsGroup(title: 'Input', children: [
                if (choices.models.length > 1)
                  ChoiceRow<String>(
                    title: 'Model',
                    configKey: const ['voice', 'model'],
                    fieldKey: const Key('voice-model'),
                    value: choices.models.any((m) => m.id == voice.model) ? voice.model! : _own,
                    choices: [(_own, 'The service\'s own choice'), for (final m in choices.models) (m.id, m.label)],
                    onChanged: (id) => _set((v) => id == _own ? v.copyWith(clearModel: true) : v.copyWith(model: id)),
                  ),
                ChoiceRow<String>(
                  title: 'Input device',
                  configKey: const ['voice', 'device'],
                  description: choices.devices.isEmpty ? 'This service records from the system\'s default microphone' : null,
                  fieldKey: const Key('voice-device'),
                  value: choices.devices.any((d) => d.id == voice.device) ? voice.device! : _own,
                  enabled: choices.devices.isNotEmpty,
                  choices: [(_own, 'System default'), for (final d in choices.devices) (d.id, d.label)],
                  onChanged: (id) => _set((v) => id == _own ? v.copyWith(clearDevice: true) : v.copyWith(device: id)),
                ),
                ChoiceRow<String>(
                  title: 'Language',
                  configKey: const ['voice', 'language'],
                  description: 'Automatic follows the system\'s language',
                  fieldKey: const Key('voice-language'),
                  value: choices.languages.any((l) => l.id == voice.language) ? voice.language! : _own,
                  choices: [(_own, 'Automatic'), for (final l in choices.languages) (l.id, l.label)],
                  onChanged: (id) => _set((v) => id == _own ? v.copyWith(clearLanguage: true) : v.copyWith(language: id)),
                ),
                ChoiceRow<DictationMode>(
                  title: 'Dictation',
                  configKey: const ['voice', 'mode'],
                  description: 'Click to toggle starts and ends with a press; push to talk lasts while the key is held',
                  fieldKey: const Key('voice-mode'),
                  value: voice.mode,
                  choices: const [(DictationMode.toggle, 'Click to toggle'), (DictationMode.hold, 'Push to talk')],
                  onChanged: (mode) => _set((v) => v.copyWith(mode: mode)),
                ),
                MicrophoneTest(
                  key: ValueKey(backend.id),
                  backend: backend,
                  device: voice.device,
                  language: voice.language,
                  model: voice.model,
                  onDone: () => _reload(backend),
                ),
              ]);
            },
          ),
      ]);
    },
  );
}

/// What Voice says when this build has no speech service, with what would give it one.
class _NoSpeechService extends StatelessWidget {
  const _NoSpeechService();

  @override
  Widget build(BuildContext context) => SettingsGroup(title: 'Speech to text', children: [
    SettingsRow(
      key: const Key('voice-none'),
      title: 'No speech service here',
      description: 'Dictation uses the speech service of the operating system, which Windows and macOS have. This browser or '
          'system has none, and Canoa never sends audio to another service on its own',
      control: TextButton(
        key: const Key('voice-setup'),
        onPressed: () => showDialog<void>(
          context: context,
          builder: (context) => AlertDialog(
            title: const Text('Set up dictation'),
            content: const Text(
              'Use the Canoa desktop app on Windows or macOS and allow the microphone and speech recognition in the '
              'system\'s privacy settings. Another speech service can be added later in Settings > Voice.',
            ),
            actions: [TextButton(onPressed: () => Navigator.of(context).pop(), child: const Text('Close'))],
          ),
        ),
        child: const Text('Set up'),
      ),
    ),
  ]);
}

/// A short test: the microphone opens when Test is pressed, shows its level and what the service heard, and closes when it is
/// stopped, when the service ends it or when the page goes away.
class MicrophoneTest extends StatefulWidget {
  const MicrophoneTest({super.key, required this.backend, this.device, this.language, this.model, this.onDone});

  final SpeechBackend backend;
  final String? device, language, model;

  /// Called when a test ends, for the page to read again what the backend now knows.
  final VoidCallback? onDone;

  @override
  State<MicrophoneTest> createState() => _MicrophoneTestState();
}

class _MicrophoneTestState extends State<MicrophoneTest> {
  StreamSubscription<Transcript>? _heard;
  StreamSubscription<double>? _levels;
  double _level = 0;
  String _text = '';
  String? _problem;

  bool get _running => _heard != null;

  void _start() {
    setState(() {
      _text = '';
      _problem = null;
      _level = 0;
    });
    _levels = widget.backend.levels.listen((level) {
      if (mounted) setState(() => _level = level);
    });
    _heard = widget.backend
        .start(device: widget.device, language: widget.language, model: widget.model)
        .listen(
          (transcript) {
            if (mounted) setState(() => _text = transcript.text);
          },
          onError: (Object error) {
            if (mounted) setState(() => _problem = error is SpeechFailure ? error.message : 'The speech service failed.');
          },
          onDone: _ended,
          cancelOnError: false,
        );
  }

  void _ended() {
    unawaited(_levels?.cancel());
    _levels = null;
    _heard = null;
    if (!mounted) return;
    setState(() => _level = 0);
    widget.onDone?.call();
  }

  Future<void> _stop() => widget.backend.stop();

  @override
  void dispose() {
    if (_running) unawaited(widget.backend.cancel());
    unawaited(_heard?.cancel());
    unawaited(_levels?.cancel());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return SettingsRow(
      title: 'Test the microphone',
      description: _running ? 'Listening. Say something, then stop' : 'Nothing is heard until you press Test',
      control: _running
          ? FilledButton.tonal(key: const Key('voice-test-stop'), onPressed: _stop, child: const Text('Stop'))
          : OutlinedButton(key: const Key('voice-test'), onPressed: _start, child: const Text('Test')),
      below: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        ClipRRect(
          borderRadius: BorderRadius.circular(4),
          child: LinearProgressIndicator(
            key: const Key('voice-level'),
            value: _level,
            minHeight: 8,
            semanticsLabel: 'Microphone level',
            semanticsValue: '${(_level * 100).round()}%',
          ),
        ),
        if (_text.isNotEmpty)
          Padding(padding: const EdgeInsets.only(top: 8), child: Text(_text, key: const Key('voice-heard'))),
        if (_problem case final problem?)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Text(problem, key: const Key('voice-test-error'), style: TextStyle(color: scheme.error)),
          ),
      ]),
    );
  }
}
